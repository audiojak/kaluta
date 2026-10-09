//! The rules server end to end (spec §10.6): an in-process server on
//! loopback, the publisher's REST calls, and agents over REST and over MCP
//! (rmcp's Streamable HTTP client).

mod common;

use common::{MAILBOX, call, snapshot, start};
use reqwest::StatusCode;
use serde_json::{Value, json};

#[tokio::test(flavor = "multi_thread")]
async fn publish_then_read_through_rest_and_mcp_until_revoked() {
    let s = start(0, None).await;
    let publisher = s.registered().await;
    let (status, body) = s.register(" Scout@Agents.Example ").await;
    assert_eq!((status, body["error"].as_str()), (StatusCode::CONFLICT, Some("already_registered")));

    // Version 1, then a push that must name it.
    let (status, body) = s.publish(&publisher, None, snapshot(1).to_json().unwrap()).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["version"], 1);
    let (status, body) = s.publish(&publisher, None, snapshot(2).to_json().unwrap()).await;
    assert_eq!((status, &body["current_version"]), (StatusCode::PRECONDITION_REQUIRED, &json!(1)));
    let (status, body) = s.publish(&publisher, Some("\"5\""), snapshot(6).to_json().unwrap()).await;
    assert_eq!((status, &body["current_version"]), (StatusCode::PRECONDITION_FAILED, &json!(1)), "{body}");
    let (status, body) = s.publish(&publisher, Some("1"), snapshot(1).to_json().unwrap()).await;
    assert_eq!((status, body["error"].as_str()), (StatusCode::CONFLICT, Some("version_not_newer")));
    let (status, body) = s.publish(&publisher, Some("\"1\""), snapshot(2).to_json().unwrap()).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["versions_kept"], json!([2, 1]));

    let r = s
        .http
        .get(s.url(&format!("/v1/mailboxes/{MAILBOX}/snapshot/version")))
        .bearer_auth(&publisher)
        .send()
        .await
        .unwrap();
    assert_eq!(r.headers()["etag"], "\"2\"");
    let body: Value = r.json().await.unwrap();
    assert_eq!((body["version"].clone(), body["published_at"].clone()), (json!(2), json!("2025-10-09T08:55:20Z")));

    let (id, agent) = s.mint(&publisher, "Weekly outreach routine").await;
    let r = s
        .http
        .get(s.url(&format!("/v1/mailboxes/{MAILBOX}/agent-tokens")))
        .bearer_auth(&publisher)
        .send()
        .await
        .unwrap();
    let listed: Value = r.json().await.unwrap();
    assert!(listed["agent_tokens"][0]["last_used_at"].is_null(), "never used yet: {listed}");

    // REST: a customer's person-scoped guideline and audience, matched by hash.
    let r = s.get(Some(&agent), &format!("/v1/m/{MAILBOX}/guide?to=Ann@Acme.com&message_type=reply")).await;
    assert_eq!(r.status(), StatusCode::OK);
    let for_ann: Value = r.json().await.unwrap();
    let text = for_ann["writing_guide"].as_str().unwrap();
    assert!(text.contains("This message: written for Customers; a reply."), "{text}");
    assert!(text.contains("- Call her Annie (to ann@acme.com)") && text.contains("- Be formal (for Customers)"));
    assert!(!text.contains("pricing"), "the rule for Globex's people is not shown for Ann");
    assert!(text.contains("- Work › Calendar: cal.com/scout"));
    assert_eq!(for_ann["mailbox"], MAILBOX);
    assert_eq!(for_ann["sends_as"], "Scout <scout@agents.example>");
    assert_eq!(for_ann["about"], "You send as Scout.");
    assert_eq!((for_ann["guide_version"].clone(), for_ann["version"].clone()), (json!(12), json!(2)));
    assert_eq!(for_ann["published_at"], "2025-10-09T08:55:20Z");
    assert!(for_ann.get("send_mode").is_none(), "the server never sends");

    let r = s.get(Some(&agent), &format!("/v1/m/{MAILBOX}/guide?to=bea@globex.com,vc@fund.com")).await;
    let for_bea: Value = r.json().await.unwrap();
    assert!(for_bea["writing_guide"].as_str().unwrap().contains("- Never mention pricing (to bea@globex.com)"));
    let r = s.get(Some(&agent), &format!("/v1/m/{MAILBOX}/facts?category=WORK&query=cal")).await;
    let facts: Value = r.json().await.unwrap();
    assert_eq!(facts["facts"][0]["value"], "cal.com/scout");
    assert_eq!(facts["version"], 2);

    // MCP: the same tools, arguments and answers as mailbox mode.
    let client = s.mcp(&agent).await.expect("connect");
    let tools = client.list_all_tools().await.unwrap();
    let catalog = agent_mcp::mailbox_catalog();
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
    assert_eq!(names, ["guide_rules", "facts_lookup", "check_draft", "report_send"]);
    // Mailbox mode's two reading tools take the same arguments here.
    for t in tools.iter().take(2) {
        let spec = catalog.iter().find(|c| c.name() == t.name).unwrap();
        assert_eq!(Value::Object(t.input_schema.as_ref().clone()), spec.input_schema, "{}", t.name);
    }
    let answer = call(&client, "guide_rules", json!({ "to": ["Ann@Acme.com"], "message_type": "reply" })).await;
    assert_eq!(answer, for_ann);
    let answer = call(&client, "facts_lookup", json!({ "category": "WORK", "query": "cal" })).await;
    assert_eq!(answer, facts);
    let bad = client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("guide_rules")
                .with_arguments(json!({ "recipients": [] }).as_object().unwrap().clone()),
        )
        .await
        .unwrap();
    assert_eq!(bad.is_error, Some(true), "unknown arguments are refused, as in mailbox mode");

    // Revoked: both surfaces answer 401 with a challenge.
    let r = s
        .http
        .delete(s.url(&format!("/v1/mailboxes/{MAILBOX}/agent-tokens/{id}")))
        .bearer_auth(&publisher)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::NO_CONTENT);
    let r = s.get(Some(&agent), &format!("/v1/m/{MAILBOX}/guide")).await;
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(r.headers()["www-authenticate"], "Bearer realm=\"openagc-rules\", error=\"invalid_token\"");
    let r = s
        .http
        .post(s.url("/mcp"))
        .bearer_auth(&agent)
        .header("Accept", "application/json, text/event-stream")
        .json(&json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
    assert!(r.headers().contains_key("www-authenticate"));
    assert!(client.list_all_tools().await.is_err(), "an open client is cut off at its next request");
    assert!(s.mcp(&agent).await.is_err(), "and cannot connect again");
    let r = s.http.post(s.url("/mcp")).json(&json!({})).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        r.headers()["www-authenticate"],
        format!(
            "Bearer realm=\"openagc-rules\", resource_metadata=\"{}/.well-known/oauth-protected-resource\", \
             scope=\"rules\"",
            s.base
        ),
        "with OAuth on, a 401 at /mcp says where to sign in"
    );

    let r = s
        .http
        .get(s.url(&format!("/v1/mailboxes/{MAILBOX}/agent-tokens")))
        .bearer_auth(&publisher)
        .send()
        .await
        .unwrap();
    let listed: Value = r.json().await.unwrap();
    assert_eq!(listed["agent_tokens"][0]["name"], "Weekly outreach routine");
    assert!(listed["agent_tokens"][0]["revoked_at"].is_string());
    assert!(listed["agent_tokens"][0].get("token").is_none(), "a token is shown once");
    assert!(listed["agent_tokens"][0]["last_used_at"].is_string(), "the app's list says when it was last used");
}

#[tokio::test(flavor = "multi_thread")]
async fn refusals() {
    let s = start(0, None).await;
    let publisher = s.registered().await;
    let (status, body) = s.register("not an address").await;
    assert_eq!((status, body["error"].as_str()), (StatusCode::UNPROCESSABLE_ENTITY, Some("invalid_address")));

    // Snapshots the server will not hold.
    let mut plain: Value = serde_json::from_str(&snapshot(1).to_json().unwrap()).unwrap();
    plain["entries"][2]["scope"]["people"] = json!(["ann@acme.com"]);
    let (status, body) = s.publish(&publisher, None, plain.to_string()).await;
    assert_eq!((status, body["error"].as_str()), (StatusCode::UNPROCESSABLE_ENTITY, Some("invalid_snapshot")));
    assert!(body["message"].as_str().unwrap().contains("salted hashes"));
    let mut newer: Value = serde_json::from_str(&snapshot(1).to_json().unwrap()).unwrap();
    newer["schema_version"] = json!(2);
    let (status, _) = s.publish(&publisher, None, newer.to_string()).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let mut other = snapshot(1);
    other.mailbox.address = "someone@else.example".into();
    let (status, body) = s.publish(&publisher, None, other.to_json().unwrap()).await;
    assert_eq!((status, body["error"].as_str()), (StatusCode::UNPROCESSABLE_ENTITY, Some("mailbox_mismatch")));
    let (status, _) = s.publish(&publisher, None, "{".into()).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let (status, body) = s.publish(&publisher, Some("soon"), snapshot(1).to_json().unwrap()).await;
    assert_eq!((status, body["error"].as_str()), (StatusCode::BAD_REQUEST, Some("invalid_if_match")));

    // Tokens that are not this mailbox's.
    let (status, _) = s.publish("oagc_pub_wrong", None, snapshot(1).to_json().unwrap()).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = s.publish("", None, snapshot(1).to_json().unwrap()).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    // An address that is not registered answers as a wrong token does:
    // nobody learns which addresses are registered here.
    let r = s.http.put(s.url("/v1/mailboxes/nobody@x.example/snapshot")).bearer_auth(&publisher).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
    let unknown: Value = r.json().await.unwrap();
    let r = s.http.put(s.url(&format!("/v1/mailboxes/{MAILBOX}/snapshot"))).bearer_auth("oagc_pub_wrong").send().await;
    let wrong: Value = r.unwrap().json().await.unwrap();
    assert_eq!(unknown, wrong);
    for path in ["/v1/mailboxes/nobody@x.example/agent-tokens", "/v1/mailboxes/nobody@x.example/reports"] {
        assert_eq!(s.get(Some(&publisher), path).await.status(), StatusCode::UNAUTHORIZED, "{path}");
    }

    let (_, agent) = s.mint(&publisher, "Routine").await;
    let r = s.get(Some(&agent), &format!("/v1/m/{MAILBOX}/guide")).await;
    let body: Value = r.json().await.unwrap();
    assert_eq!(body["error"], "not_published");
    let client = s.mcp(&agent).await.unwrap();
    let r = client
        .call_tool(rmcp::model::CallToolRequestParams::new("guide_rules").with_arguments(Default::default()))
        .await
        .unwrap();
    assert_eq!(r.is_error, Some(true));

    let (status, _) = s.publish(&publisher, None, snapshot(1).to_json().unwrap()).await;
    assert_eq!(status, StatusCode::OK);
    let r = s.get(Some(&agent), &format!("/v1/m/{MAILBOX}/guide?message_type=memo")).await;
    assert_eq!(r.status(), StatusCode::BAD_REQUEST);
    let r = s.get(Some(&agent), &format!("/v1/m/{MAILBOX}/guide?recipient=a@b.com")).await;
    assert_eq!(r.status(), StatusCode::BAD_REQUEST);
    let r = s.get(None, &format!("/v1/m/{MAILBOX}/guide?message_type=memo")).await;
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED, "the token is checked first");

    // Another mailbox's agent token reads nothing here.
    let (status, body) = s.register("other@agents.example").await;
    assert_eq!(status, StatusCode::CREATED);
    let other_publisher = body["publisher_token"].as_str().unwrap();
    let r = s
        .http
        .post(s.url("/v1/mailboxes/other@agents.example/agent-tokens"))
        .bearer_auth(other_publisher)
        .json(&json!({ "name": "Other" }))
        .send()
        .await
        .unwrap();
    let other_agent = r.json::<Value>().await.unwrap()["token"].as_str().unwrap().to_owned();
    let r = s.get(Some(&other_agent), &format!("/v1/m/{MAILBOX}/guide")).await;
    assert_eq!(r.status(), StatusCode::NOT_FOUND);
    let r = s.get(Some(&publisher), &format!("/v1/m/{MAILBOX}/guide")).await;
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED, "a publisher token is not an agent token");
    let r = s
        .http
        .put(s.url(&format!("/v1/mailboxes/{MAILBOX}/snapshot")))
        .bearer_auth(other_publisher)
        .body(snapshot(2).to_json().unwrap())
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED, "nor does another mailbox's publisher token publish");
    let r = s
        .http
        .delete(s.url(&format!("/v1/mailboxes/{MAILBOX}/agent-tokens/0123456789abcdef")))
        .bearer_auth(&publisher)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::NOT_FOUND);

    // Forgetting a mailbox takes its tokens with it.
    let r = s.http.delete(s.url(&format!("/v1/mailboxes/{MAILBOX}"))).bearer_auth(&publisher).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::NO_CONTENT);
    let r = s.get(Some(&agent), &format!("/v1/m/{MAILBOX}/guide")).await;
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
    let (status, _) = s.register(MAILBOX).await;
    assert_eq!(status, StatusCode::CREATED, "and it can be registered again");
}

#[tokio::test(flavor = "multi_thread")]
async fn only_the_last_five_versions_are_kept() {
    let s = start(0, None).await;
    let publisher = s.registered().await;
    let mut current: Option<String> = None;
    let mut last = Value::Null;
    for v in [1, 2, 3, 5, 8, 13, 21] {
        let (status, body) = s.publish(&publisher, current.as_deref(), snapshot(v).to_json().unwrap()).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        current = Some(v.to_string());
        last = body;
    }
    assert_eq!(last["versions_kept"], json!([21, 13, 8, 5, 3]));
    let db = rules_server::Db::open(&s.dir).unwrap();
    let rows: i64 = db.run_now(|c| c.query_row("SELECT count(*) FROM snapshots", [], |r| r.get(0))).unwrap();
    assert_eq!(rows, 5);
}

#[tokio::test(flavor = "multi_thread")]
async fn each_token_is_rate_limited_on_its_own() {
    let s = start(4, None).await;
    let publisher = s.registered().await;
    let (status, _) = s.publish(&publisher, None, snapshot(1).to_json().unwrap()).await;
    assert_eq!(status, StatusCode::OK);
    let (_, a) = s.mint(&publisher, "A").await;
    let (_, b) = s.mint(&publisher, "B").await;
    for _ in 0..4 {
        assert_eq!(s.get(Some(&a), &format!("/v1/m/{MAILBOX}/facts")).await.status(), StatusCode::OK);
    }
    let r = s.get(Some(&a), &format!("/v1/m/{MAILBOX}/facts")).await;
    assert_eq!(r.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(r.headers()["retry-after"].to_str().unwrap().parse::<u64>().unwrap() >= 1);
    let r = s
        .http
        .post(s.url("/mcp"))
        .bearer_auth(&a)
        .header("Accept", "application/json, text/event-stream")
        .json(&json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::TOO_MANY_REQUESTS, "MCP shares the token's bucket");
    assert_eq!(s.get(Some(&b), &format!("/v1/m/{MAILBOX}/facts")).await.status(), StatusCode::OK);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_registration_token_closes_registration_to_strangers() {
    let s = start(0, Some("let-me-in")).await;
    let (status, _) = s.register(MAILBOX).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let r = s
        .http
        .post(s.url("/v1/mailboxes"))
        .bearer_auth("let-me-in")
        .json(&json!({ "address": MAILBOX }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::CREATED);
    assert_eq!(s.get(None, "/healthz").await.text().await.unwrap(), "ok\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn failed_sign_ins_are_limited_per_address_before_the_database() {
    let s = common::start_trusting(120, &["127.0.0.1"]).await;
    let publisher = s.registered().await;
    assert_eq!(s.publish(&publisher, None, snapshot(1).to_json().unwrap()).await.0, StatusCode::OK);
    let (_, agent) = s.mint(&publisher, "Routine").await;
    let guide = async |token: &str, address: &str| {
        s.http
            .get(s.url(&format!("/v1/m/{MAILBOX}/guide")))
            .bearer_auth(token)
            .header("X-Forwarded-For", address)
            .send()
            .await
            .unwrap()
            .status()
    };
    let garbage = |i: u32| format!("oagc_oat_garbage{i}.{}", "x".repeat(43));
    for i in 0..rules_server::FAILED_AUTH_PER_MINUTE {
        assert_eq!(guide(&garbage(i), "203.0.113.9").await, StatusCode::UNAUTHORIZED);
    }
    assert_eq!(guide(&garbage(99), "203.0.113.9").await, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(guide(&agent, "203.0.113.9").await, StatusCode::TOO_MANY_REQUESTS, "refused before any look");
    assert_eq!(guide(&agent, "198.51.100.7").await, StatusCode::OK, "another address is not");
    // Publisher calls count alike, unknown addresses included.
    let r = s
        .http
        .get(s.url("/v1/mailboxes/nobody@x.example/agent-tokens"))
        .bearer_auth(&publisher)
        .header("X-Forwarded-For", "203.0.113.9")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::TOO_MANY_REQUESTS);
    // Without a trusted proxy, the header is ignored: every request is the
    // peer's, whatever it claims.
    let open = common::start_trusting(120, &[]).await;
    for i in 0..rules_server::FAILED_AUTH_PER_MINUTE {
        let r = open
            .http
            .get(open.url(&format!("/v1/m/{MAILBOX}/guide")))
            .bearer_auth(garbage(i))
            .header("X-Forwarded-For", format!("203.0.113.{i}"))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
    }
    assert_eq!(
        open.get(Some(&garbage(0)), &format!("/v1/m/{MAILBOX}/guide")).await.status(),
        StatusCode::TOO_MANY_REQUESTS
    );
}
