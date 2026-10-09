//! MCP over Streamable HTTP at `/mcp` (spec §10.6). Stateless: every
//! request carries `Authorization: Bearer <agent token>` or an OAuth access
//! token, checked before rmcp sees it, and the token alone says which
//! mailbox is served, so a revoked agent stops working at its next request.
//! A 401 names the protected resource metadata when OAuth is on.
//!
//! The tools are mailbox mode's `guide_rules` and `facts_lookup` (§10.1):
//! the same names, arguments and answers, plus the snapshot's `version`
//! and `published_at`. `check_draft` and `report_send` come later.

use std::sync::Arc;

use axum::Router;
use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation, ListToolsResult,
    PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool, ToolAnnotations,
};
use rmcp::service::{MaybeSendFuture, RequestContext};
use rmcp::transport::streamable_http_server::session::never::NeverSessionManager;
use rmcp::transport::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::{ErrorData as McpError, RoleServer, ServerHandler};
use serde_json::{Value, json};

use crate::answers::{self, FactsArgs, GuideArgs};
use crate::{AgentAuth, AppState, TokenSlot, agent_auth};

const INSTRUCTIONS: &str = "The writing guide and shared facts of one agent mailbox, published from OpenAGC. Read \
                            guide_rules before writing, and use only the facts facts_lookup gives. Answers are as of \
                            the version and time they name. Mail is read and sent through the mailbox's service, \
                            not here.";

pub const GUIDE_RULES: &str = "guide_rules";
pub const FACTS_LOOKUP: &str = "facts_lookup";

fn object(properties: Value) -> serde_json::Map<String, Value> {
    match json!({ "type": "object", "properties": properties, "required": [], "additionalProperties": false }) {
        Value::Object(map) => map,
        _ => serde_json::Map::new(),
    }
}

/// The two tools' input schemas: mailbox mode's (kept in step by a test
/// against `agent-mcp`'s catalog).
pub fn input_schemas() -> [(&'static str, serde_json::Map<String, Value>); 2] {
    [
        (
            GUIDE_RULES,
            object(json!({
                "to": { "type": "array", "items": { "type": "string" },
                        "description": "Recipients, for rules about particular people." },
                "message_type": { "type": "string", "enum": answers::MESSAGE_TYPES },
            })),
        ),
        (
            FACTS_LOOKUP,
            object(json!({
                "category": { "type": "string", "description": "A category name or key, such as Work." },
                "query": { "type": "string", "description": "Words to look for in labels and values." },
            })),
        ),
    ]
}

fn tools() -> Vec<Tool> {
    input_schemas()
        .into_iter()
        .map(|(name, schema)| {
            let description = match name {
                GUIDE_RULES => {
                    "The mailbox's writing guide: how mail from it is written (tone, length, phrases to use and \
                     avoid) and the facts drafts may use, for the given recipients and message type. Read it before \
                     writing. Also says whose mailbox this is, the name mail goes out as, and the service's sending \
                     limits. It was published from OpenAGC: version and published_at say how current it is."
                }
                _ => {
                    "Look up facts about the user that drafts may use (their role, time zone, calendar link, the \
                     people they mention), as shared with cloud agents. Use only these facts; never invent others. \
                     A fact marked ask_before_using needs the user's yes before it goes in a message."
                }
            };
            Tool::new(name, description, schema)
                .with_annotations(ToolAnnotations::new().read_only(true).destructive(false))
        })
        .collect()
}

/// One agent request's server: it learns its mailbox from the token the
/// request was accepted with.
#[derive(Clone)]
struct RulesMcp {
    state: AppState,
    tools: Arc<Vec<Tool>>,
}

fn tool_error(code: &str, message: impl std::fmt::Display) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(format!("{code}: {message}"))])
}

impl RulesMcp {
    async fn call(&self, auth: &AgentAuth, name: &str, arguments: Value) -> CallToolResult {
        enum Call {
            Guide(GuideArgs),
            Facts(FactsArgs),
        }
        let parsed = match name {
            GUIDE_RULES => serde_json::from_value::<GuideArgs>(arguments)
                .map_err(|e| e.to_string())
                .and_then(|a| answers::check_guide_args(&a).map(|()| Call::Guide(a))),
            _ => serde_json::from_value::<FactsArgs>(arguments).map(Call::Facts).map_err(|e| e.to_string()),
        };
        // The tool's name and the token's id; never the arguments.
        tracing::info!(token = %auth.token_id, tool = name, "tool call");
        let call = match parsed {
            Ok(c) => c,
            Err(e) => return tool_error("invalid_arguments", e),
        };
        let snapshot = match crate::rest::latest(&self.state, auth.mailbox_id).await {
            Ok(Some(s)) => s,
            Ok(None) => return tool_error("not_published", "nothing has been published to this mailbox yet"),
            Err(_) => return tool_error("failed", "the server failed; see its log"),
        };
        let answer = match call {
            Call::Guide(a) => answers::guide_rules(&snapshot, &a),
            Call::Facts(a) => answers::facts_lookup(&snapshot, &a),
        };
        CallToolResult::structured(answer)
    }
}

impl ServerHandler for RulesMcp {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("openagc-rules", env!("CARGO_PKG_VERSION")))
            .with_instructions(INSTRUCTIONS)
    }

    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListToolsResult, McpError>> + MaybeSendFuture + '_ {
        std::future::ready(Ok(ListToolsResult::with_all_items(self.tools.as_ref().clone())))
    }

    fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<CallToolResponse, McpError>> + MaybeSendFuture + '_ {
        let auth = context
            .extensions
            .get::<http::request::Parts>()
            .and_then(|parts| parts.extensions.get::<AgentAuth>())
            .cloned();
        async move {
            if !self.tools.iter().any(|t| t.name == request.name) {
                return Err(McpError::invalid_params(format!("no tool named {}", request.name), None));
            }
            // The token was checked before the request reached rmcp; a
            // request without one never gets here.
            let Some(auth) = auth else {
                return Ok(tool_error("unauthorized", "no agent token").into());
            };
            let arguments = Value::Object(request.arguments.unwrap_or_default());
            Ok(self.call(&auth, &request.name, arguments).await.into())
        }
    }
}

/// Every `/mcp` request needs a live agent token.
async fn require_agent_token(State(state): State<AppState>, mut req: Request, next: Next) -> Response {
    let slot = req.extensions().get::<TokenSlot>().cloned();
    match agent_auth(&state, req.headers(), slot.as_ref(), true).await {
        Ok(auth) => {
            req.extensions_mut().insert(auth);
            next.run(req).await
        }
        Err(mut e) => {
            // With OAuth on, a 401 says where to sign in (RFC 9728 §5.1).
            if let Some(o) = &state.oauth {
                e.resource_metadata = Some(o.resource_metadata_url());
            }
            e.into_response()
        }
    }
}

pub(crate) fn router(state: AppState) -> Router {
    let tools = Arc::new(tools());
    let server = RulesMcp { state: state.clone(), tools };
    // The server sits behind the operator's proxy under its own name, and
    // every request carries a bearer token, so the Host check (against DNS
    // rebinding of a local server) is off.
    let config = StreamableHttpServerConfig::default()
        .with_legacy_session_mode(false)
        .with_json_response(true)
        .with_sse_keep_alive(None)
        .with_max_request_body_bytes(crate::MAX_BODY_BYTES)
        .disable_allowed_hosts();
    let service =
        StreamableHttpService::new(move || Ok(server.clone()), Arc::new(NeverSessionManager::default()), config);
    Router::new()
        .route_service("/mcp", service)
        .route_layer(axum::middleware::from_fn_with_state(state, require_agent_token))
}
