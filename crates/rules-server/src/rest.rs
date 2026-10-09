//! The REST API (spec §10.6).
//!
//! Publisher (the app), `Authorization: Bearer <publisher token>`:
//! - `POST /v1/mailboxes` `{"address"}`: register; answers the publisher
//!   token once. The first registration of an address wins; later ones are
//!   refused (409). With a registration token set, registering needs it.
//! - `DELETE /v1/mailboxes/{address}`: forget the mailbox, its snapshots
//!   and its agent tokens.
//! - `PUT /v1/mailboxes/{address}/snapshot` with `If-Match: <current
//!   version>` (none for the first): a full snapshot whose version is
//!   higher. 412 with `current_version` when the version does not match.
//! - `GET /v1/mailboxes/{address}/snapshot/version`.
//! - `POST /v1/mailboxes/{address}/agent-tokens` `{"name"}`: mint; answers
//!   the token once. `GET` lists them; `DELETE …/agent-tokens/{id}`
//!   revokes one.
//!
//! Agents and scripts, `Authorization: Bearer <agent token>`:
//! - `GET /v1/m/{address}/guide?to=&to=&message_type=` and
//!   `GET /v1/m/{address}/facts?category=&query=`: `guide_rules` and
//!   `facts_lookup`, answered as over MCP.

use axum::extract::{Extension, Path, RawQuery, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};
use writing_guide::Snapshot;

use crate::answers::{self, FactsArgs, GuideArgs};
use crate::db::{self, AgentTokenRow, SnapshotRow};
use crate::{ApiError, AppState, TokenSlot, agent_auth, normalize_address, tokens};

pub(crate) fn router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/v1/mailboxes", post(register))
        .route("/v1/mailboxes/{address}", delete(forget))
        .route("/v1/mailboxes/{address}/snapshot", put(publish))
        .route("/v1/mailboxes/{address}/snapshot/version", get(version))
        .route("/v1/mailboxes/{address}/agent-tokens", post(mint).get(list_tokens))
        .route("/v1/mailboxes/{address}/agent-tokens/{id}", delete(revoke))
        .route("/v1/m/{address}/guide", get(guide))
        .route("/v1/m/{address}/facts", get(facts))
        .with_state(state)
}

async fn healthz(State(state): State<AppState>) -> Response {
    match state.db.run(|c| c.query_row("SELECT 1", [], |r| r.get::<_, i64>(0))).await {
        Ok(_) => (StatusCode::OK, "ok\n").into_response(),
        Err(e) => ApiError::internal(e).into_response(),
    }
}

/// A body that is not the JSON asked for.
fn bad_json(e: impl std::fmt::Display) -> ApiError {
    ApiError::new(StatusCode::BAD_REQUEST, "invalid_body", e.to_string())
}

fn parse_json<T: serde::de::DeserializeOwned>(body: &str) -> Result<T, ApiError> {
    serde_json::from_str(body).map_err(bad_json)
}

fn address_in_path(address: &str) -> Result<String, ApiError> {
    normalize_address(address)
        .ok_or_else(|| ApiError::new(StatusCode::BAD_REQUEST, "invalid_address", "not a mailbox address"))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RegisterBody {
    address: String,
}

async fn register(
    State(state): State<AppState>,
    Extension(slot): Extension<TokenSlot>,
    headers: HeaderMap,
    body: String,
) -> Result<Response, ApiError> {
    state.limiter.take("register").map_err(ApiError::too_many)?;
    if let Some(expected) = &state.registration_token_hash {
        let given = tokens::bearer(&headers);
        if !given.is_some_and(|t| tokens::same(&tokens::hash(t), expected)) {
            return Err(ApiError::unauthorized(given.is_some()));
        }
        slot.set("registration".into());
    }
    let b: RegisterBody = parse_json(&body)?;
    let address = normalize_address(&b.address)
        .ok_or_else(|| ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, "invalid_address", "not a mailbox address"))?;
    let token = tokens::publisher_token();
    let hash = tokens::hash(&token);
    let stored = address.clone();
    let id = state.db.run(move |c| db::insert_mailbox(c, &stored, &hash, db::now_ms())).await?;
    let Some(id) = id else {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "already_registered",
            "this address is already registered on this server; publish with its publisher token, or ask the \
             server's operator to forget it",
        ));
    };
    tracing::info!(mailbox = id, "mailbox registered");
    Ok((StatusCode::CREATED, Json(json!({ "address": address, "publisher_token": token }))).into_response())
}

/// The mailbox in the path, if the publisher token is its own.
async fn publisher(
    state: &AppState,
    headers: &HeaderMap,
    address: &str,
    slot: &TokenSlot,
) -> Result<db::MailboxRow, ApiError> {
    let address = address_in_path(address)?;
    let token = tokens::bearer(headers).ok_or_else(|| ApiError::unauthorized(false))?;
    let presented = tokens::hash(token);
    let row = state.db.run(move |c| db::mailbox_by_address(c, &address)).await?;
    let Some(row) = row else {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "not_registered", "no such mailbox on this server"));
    };
    if !tokens::same(&presented, &row.publisher_token_hash) {
        return Err(ApiError::unauthorized(true));
    }
    slot.set(format!("publisher:{}", row.id));
    state.limiter.take(&format!("publisher:{}", row.id)).map_err(ApiError::too_many)?;
    Ok(row)
}

async fn forget(
    State(state): State<AppState>,
    Extension(slot): Extension<TokenSlot>,
    Path(address): Path<String>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    let m = publisher(&state, &headers, &address, &slot).await?;
    state.db.run(move |c| db::delete_mailbox(c, m.id)).await?;
    tracing::info!(mailbox = m.id, "mailbox forgotten");
    Ok(StatusCode::NO_CONTENT)
}

/// `If-Match`: a version, bare or as a quoted entity tag.
fn if_match(headers: &HeaderMap) -> Result<Option<i64>, ApiError> {
    let Some(value) = headers.get(header::IF_MATCH) else { return Ok(None) };
    let s = value.to_str().unwrap_or_default().trim();
    let s = s.strip_prefix("W/").unwrap_or(s).trim_matches('"');
    s.parse().map(Some).map_err(|_| {
        ApiError::new(StatusCode::BAD_REQUEST, "invalid_if_match", "If-Match must be the current snapshot version")
    })
}

fn etag(version: i64) -> HeaderValue {
    HeaderValue::from_str(&format!("\"{version}\"")).unwrap_or_else(|_| HeaderValue::from_static("\"0\""))
}

fn version_body(latest: Option<&SnapshotRow>, kept: &[i64]) -> Value {
    json!({
        "version": latest.map(|s| s.version),
        "published_at": latest.map_or(Value::Null, |s| answers::time(s.published_at)),
        "versions_kept": kept,
    })
}

/// What a push came to, decided in one transaction.
enum Pushed {
    Stored(Vec<i64>),
    Mismatch(Option<i64>),
    NeedsIfMatch(i64),
    NotNewer(i64),
}

async fn publish(
    State(state): State<AppState>,
    Extension(slot): Extension<TokenSlot>,
    Path(address): Path<String>,
    headers: HeaderMap,
    body: String,
) -> Result<Response, ApiError> {
    let m = publisher(&state, &headers, &address, &slot).await?;
    let expected = if_match(&headers)?;
    let invalid = |code, message: String| ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, code, message);
    let snapshot = Snapshot::from_json(&body).map_err(|e| invalid("invalid_snapshot", e.to_string()))?;
    if normalize_address(&snapshot.mailbox.address).as_deref() != Some(m.address.as_str()) {
        return Err(invalid("mailbox_mismatch", "the snapshot is for another mailbox".into()));
    }
    if snapshot.version < 1 {
        return Err(invalid("invalid_snapshot", "a snapshot's version starts at 1".into()));
    }
    // Stored as this build writes it: fields it does not know are dropped.
    let json = snapshot.to_json().map_err(ApiError::internal)?;
    let row = SnapshotRow { version: snapshot.version, json, published_at: snapshot.published_at };
    let pushed = state
        .db
        .run(move |c| {
            let tx = c.transaction()?;
            let current = db::latest_snapshot(&tx, m.id)?.map(|s| s.version);
            let outcome = match (current, expected) {
                (None, Some(v)) if v != 0 => Pushed::Mismatch(None),
                (Some(c), None) => Pushed::NeedsIfMatch(c),
                (Some(c), Some(v)) if v != c => Pushed::Mismatch(Some(c)),
                (Some(c), _) if row.version <= c => Pushed::NotNewer(c),
                _ => {
                    db::insert_snapshot(&tx, m.id, &row, db::now_ms())?;
                    Pushed::Stored(db::versions(&tx, m.id)?)
                }
            };
            tx.commit()?;
            Ok(outcome)
        })
        .await?;
    let current = |v: Option<i64>| json!({ "current_version": v });
    match pushed {
        Pushed::Stored(kept) => {
            tracing::info!(mailbox = m.id, version = snapshot.version, "snapshot published");
            let latest =
                SnapshotRow { version: snapshot.version, json: String::new(), published_at: snapshot.published_at };
            let mut response = Json(version_body(Some(&latest), &kept)).into_response();
            response.headers_mut().insert(header::ETAG, etag(snapshot.version));
            Ok(response)
        }
        Pushed::Mismatch(v) => Err(ApiError::new(
            StatusCode::PRECONDITION_FAILED,
            "version_mismatch",
            "If-Match is not the current version; read it and push again",
        )
        .with(current(v))),
        Pushed::NeedsIfMatch(v) => Err(ApiError::new(
            StatusCode::PRECONDITION_REQUIRED,
            "if_match_required",
            "send If-Match with the current version",
        )
        .with(current(Some(v)))),
        Pushed::NotNewer(v) => {
            Err(ApiError::new(StatusCode::CONFLICT, "version_not_newer", "a snapshot's version only goes up")
                .with(current(Some(v))))
        }
    }
}

async fn version(
    State(state): State<AppState>,
    Extension(slot): Extension<TokenSlot>,
    Path(address): Path<String>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let m = publisher(&state, &headers, &address, &slot).await?;
    let (latest, kept) = state.db.run(move |c| Ok((db::latest_snapshot(c, m.id)?, db::versions(c, m.id)?))).await?;
    let mut response = Json(version_body(latest.as_ref(), &kept)).into_response();
    if let Some(s) = &latest {
        response.headers_mut().insert(header::ETAG, etag(s.version));
    }
    Ok(response)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MintBody {
    name: String,
}

fn token_json(t: &AgentTokenRow) -> Value {
    json!({
        "id": t.id,
        "name": t.name,
        "created_at": answers::time(t.created_at),
        "revoked_at": t.revoked_at.map_or(Value::Null, answers::time),
    })
}

async fn mint(
    State(state): State<AppState>,
    Extension(slot): Extension<TokenSlot>,
    Path(address): Path<String>,
    headers: HeaderMap,
    body: String,
) -> Result<Response, ApiError> {
    let m = publisher(&state, &headers, &address, &slot).await?;
    let b: MintBody = parse_json(&body)?;
    let name = b.name.trim().to_owned();
    if name.is_empty() || name.chars().count() > 100 || name.chars().any(char::is_control) {
        return Err(ApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_name",
            "a token's name is 1 to 100 characters on one line",
        ));
    }
    let id = tokens::new_id();
    let token = tokens::agent_token(&id);
    let row = AgentTokenRow {
        id,
        mailbox_id: m.id,
        name,
        token_hash: tokens::hash(&token),
        created_at: db::now_ms(),
        revoked_at: None,
    };
    let stored = row.clone();
    state.db.run(move |c| db::insert_agent_token(c, &stored)).await?;
    tracing::info!(mailbox = m.id, token = %row.id, "agent token minted");
    let mut answer = token_json(&row);
    answer["token"] = json!(token);
    Ok((StatusCode::CREATED, Json(answer)).into_response())
}

async fn list_tokens(
    State(state): State<AppState>,
    Extension(slot): Extension<TokenSlot>,
    Path(address): Path<String>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let m = publisher(&state, &headers, &address, &slot).await?;
    let rows = state.db.run(move |c| db::agent_tokens(c, m.id)).await?;
    Ok(Json(json!({ "agent_tokens": rows.iter().map(token_json).collect::<Vec<_>>() })))
}

async fn revoke(
    State(state): State<AppState>,
    Extension(slot): Extension<TokenSlot>,
    Path((address, id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    let m = publisher(&state, &headers, &address, &slot).await?;
    let token = id.clone();
    let found = state.db.run(move |c| db::revoke_agent_token(c, m.id, &token, db::now_ms())).await?;
    if !found {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "not_found", "no such agent token for this mailbox"));
    }
    tracing::info!(mailbox = m.id, token = %id, "agent token revoked");
    Ok(StatusCode::NO_CONTENT)
}

/// The agent token's mailbox, which must be the one in the path.
async fn agent_mailbox(
    state: &AppState,
    headers: &HeaderMap,
    address: &str,
    slot: &TokenSlot,
) -> Result<i64, ApiError> {
    let auth = agent_auth(state, headers, Some(slot)).await?;
    if normalize_address(address).as_deref() != Some(auth.address.as_str()) {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "not_found", "this token is for another mailbox"));
    }
    Ok(auth.mailbox_id)
}

/// The newest snapshot, or why there is none to read.
async fn published(state: &AppState, mailbox_id: i64) -> Result<Snapshot, ApiError> {
    latest(state, mailbox_id).await?.ok_or_else(|| {
        ApiError::new(StatusCode::NOT_FOUND, "not_published", "nothing has been published to this mailbox yet")
    })
}

/// A mailbox's newest snapshot, read back.
pub(crate) async fn latest(state: &AppState, mailbox_id: i64) -> Result<Option<Snapshot>, ApiError> {
    let row = state.db.run(move |c| db::latest_snapshot(c, mailbox_id)).await?;
    row.map(|r| Snapshot::from_json(&r.json).map_err(ApiError::internal)).transpose()
}

/// The query's pairs, in order (`to` may repeat).
fn query_pairs(query: Option<&str>) -> Vec<(String, String)> {
    url::form_urlencoded::parse(query.unwrap_or_default().as_bytes()).into_owned().collect()
}

fn unknown_parameter(name: &str) -> ApiError {
    ApiError::new(StatusCode::BAD_REQUEST, "invalid_arguments", format!("unknown parameter {name}"))
}

async fn guide(
    State(state): State<AppState>,
    Extension(slot): Extension<TokenSlot>,
    Path(address): Path<String>,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let mailbox = agent_mailbox(&state, &headers, &address, &slot).await?;
    let mut args = GuideArgs::default();
    for (k, v) in query_pairs(query.as_deref()) {
        match k.as_str() {
            // Repeated, or comma-separated.
            "to" => args.to.extend(v.split(',').map(str::trim).filter(|t| !t.is_empty()).map(str::to_owned)),
            "message_type" if !v.is_empty() => args.message_type = Some(v),
            "message_type" => {}
            other => return Err(unknown_parameter(other)),
        }
    }
    answers::check_guide_args(&args).map_err(|m| ApiError::new(StatusCode::BAD_REQUEST, "invalid_arguments", m))?;
    Ok(Json(answers::guide_rules(&published(&state, mailbox).await?, &args)))
}

async fn facts(
    State(state): State<AppState>,
    Extension(slot): Extension<TokenSlot>,
    Path(address): Path<String>,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let mailbox = agent_mailbox(&state, &headers, &address, &slot).await?;
    let mut args = FactsArgs::default();
    for (k, v) in query_pairs(query.as_deref()) {
        let v = Some(v).filter(|v| !v.is_empty());
        match k.as_str() {
            "category" => args.category = v,
            "query" => args.query = v,
            other => return Err(unknown_parameter(other)),
        }
    }
    Ok(Json(answers::facts_lookup(&published(&state, mailbox).await?, &args)))
}
