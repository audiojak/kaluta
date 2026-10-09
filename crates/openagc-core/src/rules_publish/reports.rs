//! Reports from cloud agents (spec §10.6, ADR 0016, decisions 8 and 10;
//! oagc-gmn7.6). A cloud agent checks its draft with the rules server's
//! `check_draft`, sends through the mailbox's service, then files
//! `report_send`. At each sync of a publishing agent mailbox (and on
//! *Publish Now*) the app pulls the reports, keeps each one in the
//! account's store, records it as an AI composition (ADR 0013, agent
//! `cloud:<agent name>`) when the account records compositions, and only
//! then acknowledges them, which deletes them on the server. Each report is
//! matched to the sent message in the mailbox by its Message-ID, else by
//! recipient, subject and time; one whose mail has not synced yet is
//! matched when it does, and after a day is "reported, not seen in the
//! mailbox". A send with no report is reviewed as before (decision 8's
//! net).
//!
//! A report is the agent's own account of what it sent: untrusted text.
//! It is sized here as on the server, stored as data, rendered escaped and
//! reaches the daily review's agent only fenced, as every AI text does.

use std::time::Instant;

use mail_store::cloud_reports::{self, NOT_SEEN_AFTER_MS, NewReport};
use mail_store::compositions::{self, CLOUD_AGENT_PREFIX, Kind, Recipients, ReportedComposition};

use super::*;

/// Reports are pulled at a sync at most this often; *Publish Now* pulls at
/// once.
pub(crate) const PULL_EVERY: Duration = Duration::from_secs(60);
/// Reports asked for at once, and pages per pull.
const PAGE: u32 = 200;
const MAX_PAGES: usize = 20;
/// Pulls a report that does not open or read is tried at before it is
/// given up on (acknowledged unread, and counted in the status).
pub(crate) const MAX_REPORT_TRIES: u32 = 5;
/// What a report may hold here, whatever the server let through.
const MAX_BODY: usize = 256 * 1024;
const MAX_LINE: usize = 998;
const MAX_RECIPIENTS: usize = 100;
const MAX_CHECKS: usize = 50;

/// How a report stands against the mailbox's sent mail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum CloudReportMatch {
    /// The sent message with the Message-ID the report names.
    MessageId,
    /// A message to its recipient, with its subject, sent at its time.
    RecipientAndSubject,
    /// Not in the mailbox yet; looked for at each sync.
    Waiting,
    /// Reported a day ago or more, and not seen in the mailbox.
    NotSeen,
}

/// A cloud agent's report, as an agent mailbox's Settings lists it. Every
/// string is the agent's own words: shown as text, never followed.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CloudReportInfo {
    pub id: i64,
    pub agent_name: String,
    pub agent_kind: RulesAgentKind,
    pub to: Vec<String>,
    pub subject: String,
    /// When it was sent, as the report says (else when the server heard),
    /// in milliseconds since the Unix epoch.
    pub sent_at: i64,
    /// What the guide's check found in the body when the report came.
    pub guide_check: Vec<String>,
    /// The published version the agent checked its draft against, if it
    /// says.
    pub checked_version: Option<i64>,
    pub matched: CloudReportMatch,
    /// The sent message it was matched to.
    pub message_id: Option<String>,
    /// Recorded as an AI composition (the account records them).
    pub recorded: bool,
}

fn info(r: cloud_reports::Report, now: i64) -> CloudReportInfo {
    let matched = match r.match_method.as_deref() {
        Some(cloud_reports::BY_MESSAGE_ID) => CloudReportMatch::MessageId,
        Some(_) => CloudReportMatch::RecipientAndSubject,
        None if now - r.received_at >= NOT_SEEN_AFTER_MS => CloudReportMatch::NotSeen,
        None => CloudReportMatch::Waiting,
    };
    CloudReportInfo {
        id: r.id,
        sent_at: r.at(),
        agent_name: r.agent_name,
        agent_kind: if r.agent_kind == "oauth" { RulesAgentKind::Connector } else { RulesAgentKind::Token },
        to: r.to,
        subject: r.subject,
        guide_check: r.guide_check,
        checked_version: r.checked_version,
        matched,
        message_id: r.matched_message_id,
        recorded: r.composition_id.is_some(),
    }
}

/// One line of untrusted text: control characters dropped, at most
/// `max` characters.
fn line(s: &str, max: usize) -> String {
    s.chars().filter(|c| !c.is_control()).take(max).collect::<String>().trim().to_owned()
}

/// At most `max` bytes, cut at a character.
fn capped(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_owned();
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_owned()
}

/// A report from the server's list, read defensively; `None` when it is
/// not one.
fn parse(v: &Value, server: &str) -> Option<NewReport> {
    let strings = |v: &Value, max_items: usize, max: usize| -> Vec<String> {
        v.as_array()
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(|s| line(s, max))
                    .filter(|s| !s.is_empty())
                    .take(max_items)
                    .collect()
            })
            .unwrap_or_default()
    };
    let message_id = v["message_id"]
        .as_str()
        .map(cloud_reports::normalize_message_id)
        .map(|m| line(&m, MAX_LINE))
        .filter(|m| !m.is_empty() && !m.contains(char::is_whitespace));
    Some(NewReport {
        server: server.to_owned(),
        server_id: v["id"].as_i64().filter(|id| *id > 0)?,
        agent_id: line(v["agent_id"].as_str()?, 64),
        agent_name: Some(line(v["agent_name"].as_str().unwrap_or_default(), 100))
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| "A cloud agent".to_owned()),
        agent_kind: if v["agent_kind"] == "oauth" { "oauth" } else { "token" }.to_owned(),
        received_at: millis(&v["received_at"])?,
        message_id,
        to: strings(&v["to"], MAX_RECIPIENTS, MAX_LINE),
        subject: line(v["subject"].as_str().unwrap_or_default(), MAX_LINE),
        sent_at: millis(&v["sent_at"]),
        body_markdown: capped(v["body_markdown"].as_str().unwrap_or_default(), MAX_BODY),
        checked_version: v["checked_version"].as_i64(),
        check_version: v["check"]["version"].as_i64(),
        guide_check: strings(&v["check"]["guide_check"], MAX_CHECKS, 500),
    })
}

/// The AI composition a report is recorded as: the agent's body as text and
/// as sanitized HTML (raw HTML in it shown as text).
fn composition_of(r: &NewReport) -> ReportedComposition {
    let html = mail_mime::markdown_to_html(&r.body_markdown);
    ReportedComposition {
        agent: format!("{CLOUD_AGENT_PREFIX}{}", r.agent_name),
        kind: Kind::of(&r.subject, r.subject.trim().to_lowercase().starts_with("re:")),
        recipients: Recipients::new(r.to.iter().map(String::as_str), []),
        subject: r.subject.clone(),
        ai_text: mail_mime::html_to_text(&html).trim().to_owned(),
        ai_html: Some(html),
        rfc822_message_id: r.message_id.clone(),
        at: r.sent_at.unwrap_or(r.received_at),
    }
}

/// Keep reports pulled for the first time, record each as an AI composition
/// when the account records them, and match what can be matched. How many
/// were new.
pub(crate) fn record_reports(
    tx: &mail_store::Transaction<'_>,
    reports: &[NewReport],
    now: i64,
) -> mail_store::StoreResult<u32> {
    let recording = compositions::recording(tx)?;
    let mut new = 0;
    for r in reports {
        let Some(id) = cloud_reports::insert(tx, r, now)? else { continue };
        new += 1;
        if recording {
            let composition = compositions::record_reported(tx, &composition_of(r), now)?;
            cloud_reports::set_composition(tx, id, composition)?;
        }
    }
    cloud_reports::match_reports(tx, now)?;
    Ok(new)
}

impl RulesState {
    /// How far a page of reports may be acknowledged: up to the last of the
    /// leading run that read (or was given up on after [`MAX_REPORT_TRIES`]
    /// pulls), so a report that failed is never deleted unread while it is
    /// still tried. And how many were given up on now.
    pub(crate) fn acknowledgeable(
        &self,
        account: &str,
        server: &str,
        read: &[(i64, Option<NewReport>)],
    ) -> (Option<i64>, u32) {
        let mut failures = self.report_failures.lock().unwrap_or_else(|e| e.into_inner());
        let mut up_to = None;
        let mut given_up = 0;
        let mut held = false;
        for (id, report) in read {
            let key = (account.to_owned(), server.to_owned(), *id);
            if report.is_some() {
                failures.remove(&key);
            } else {
                let tries = failures.entry(key.clone()).or_insert(0);
                *tries += 1;
                if *tries >= MAX_REPORT_TRIES && !held {
                    failures.remove(&key);
                    given_up += 1;
                } else {
                    held = true;
                }
            }
            if !held {
                up_to = Some(*id);
            }
        }
        (up_to, given_up)
    }
}

impl Core {
    /// After a sync of `account_id`: pull its reports if it publishes and
    /// the last pull was a while ago, then match those still waiting for
    /// their sent mail. Quiet: problems wait for the next sync.
    pub(crate) async fn rules_after_sync(&self, account_id: &str) {
        if self.headless {
            return;
        }
        let Some(record) = self.rules_record(account_id).filter(|r| !r.sample) else { return };
        let due = {
            let mut pulled = self.rules.pulled.lock().unwrap_or_else(|e| e.into_inner());
            let due = record.enabled && pulled.get(account_id).is_none_or(|at| at.elapsed() >= PULL_EVERY);
            if due {
                pulled.insert(account_id.to_owned(), Instant::now());
            }
            due
        };
        if due {
            if let Err(f) = self.rules_pull_reports(account_id).await {
                tracing::warn!(
                    account = account_id,
                    transient = matches!(f, Failure::Transient(_)),
                    "reports not pulled"
                );
            }
        } else if let Err(e) = self.rules_match_reports(account_id).await {
            tracing::warn!(account = account_id, error = %e, "reports not matched");
        }
    }

    /// Match `account_id`'s reports still waiting to its sent mail.
    async fn rules_match_reports(&self, account_id: &str) -> Result<(), CoreError> {
        let db = self.store_for(account_id).await?;
        let now = mail_sync::now_millis();
        let matched = db.write(move |tx| cloud_reports::match_reports(tx, now)).await?;
        if !matched.is_empty() {
            self.rules_event(account_id);
        }
        Ok(())
    }

    /// Pull `account_id`'s reports from its rules server, record them, then
    /// acknowledge them (never before they are recorded: one pulled twice is
    /// recorded once). How many were new.
    pub(crate) async fn rules_pull_reports(&self, account_id: &str) -> Result<u32, Failure> {
        let _guard = self.rules.pulling.lock().await;
        let Some(record) = self.rules_record(account_id).filter(|r| r.enabled && !r.sample) else { return Ok(0) };
        let server = parse_server(&record.server_url)?;
        let host = server.host.clone();
        let address = self.agent_meta_or_err(account_id)?.address;
        let Some(token) = self.secrets.get(keys::rules_publish_token(&server.key, account_id))? else {
            return Ok(0);
        };
        let db = self.store_for(account_id).await?;
        let client = client(&self.rules)?;
        // Reports sealed to this Mac (spec §10.6, encryption at rest).
        let app = self.rules_app_key(account_id, false)?;
        let mut after = 0_i64;
        let mut new = 0;
        for _ in 0..MAX_PAGES {
            let mut url = server.endpoint(&["v1", "mailboxes", &address, "reports"]);
            url.query_pairs_mut().append_pair("after", &after.to_string()).append_pair("limit", &PAGE.to_string());
            let a = send(client.get(url).bearer_auth(&token), &host).await?;
            if let Some(f) = transient(&host, &a) {
                return Err(f);
            }
            match a.status {
                200 => {}
                // An older server, without reports, or one that forgot the
                // mailbox (the next push registers it again): nothing to pull.
                404 | 405 => return Ok(new),
                401 => return Err(Failure::Final(format!("{host} no longer accepts this Mac's publisher token"))),
                _ => return Err(Failure::Final(format!("{host} did not give the reports: {}", a.says()))),
            }
            // The server's database's epoch keys its report ids: a database
            // restored or made again reuses ids, never its epoch.
            let key = match a.body["epoch"].as_str().filter(|e| !e.is_empty()) {
                Some(epoch) => format!("{}#{}", server.url, line(epoch, 64)),
                None => server.url.clone(),
            };
            let mut listed: Vec<(i64, Value)> = a.body["reports"]
                .as_array()
                .map(|l| l.iter().filter_map(|r| Some((r["id"].as_i64().filter(|id| *id > 0)?, r.clone()))).collect())
                .unwrap_or_default();
            listed.sort_by_key(|(id, _)| *id);
            let Some(last) = listed.last().map(|(id, _)| *id) else { break };
            // Each opened and read, or `None`.
            let read: Vec<(i64, Option<NewReport>)> = listed
                .into_iter()
                .map(|(id, r)| {
                    let report = encryption::opened_report(app.as_ref(), &address, r).and_then(|r| parse(&r, &key));
                    (id, report)
                })
                .collect();
            if a.body["dropped"].as_i64().is_some_and(|n| n > 0) {
                tracing::warn!(account = account_id, dropped = %a.body["dropped"], "the server dropped reports for room");
            }
            let reports: Vec<NewReport> = read.iter().filter_map(|(_, r)| r.clone()).collect();
            let now = mail_sync::now_millis();
            new += db
                .write(move |tx| record_reports(tx, &reports, now))
                .await
                .map_err(|e| Failure::Final(e.to_string()))?;
            // Recorded: they may go from the server, up to the first that
            // did not read and is still being tried.
            let (up_to, given_up) = self.rules.acknowledgeable(account_id, &key, &read);
            if given_up > 0 {
                tracing::warn!(account = account_id, given_up, "reports that do not open or read, given up on");
                self.update_rules_record(account_id, |r| {
                    r.unreadable_reports = r.unreadable_reports.saturating_add(given_up);
                })?;
            }
            if let Some(up_to) = up_to {
                let ack = client
                    .post(server.endpoint(&["v1", "mailboxes", &address, "reports", "ack"]))
                    .bearer_auth(&token)
                    .json(&json!({ "up_to_id": up_to }));
                let acked = send(ack, &host).await?;
                if acked.status != 200 {
                    return Err(transient(&host, &acked).unwrap_or_else(|| {
                        Failure::Final(format!("{host} did not take the acknowledgement: {}", acked.says()))
                    }));
                }
            }
            // One held back: what follows it comes again at the next pull.
            if up_to != Some(last) || a.body["more"] != true {
                break;
            }
            after = last;
        }
        // Agents that connected since the last push get the newest snapshot
        // key (spec §10.6); at most once a minute, with the pull.
        if record.key_id.is_some()
            && let Err(f) = self.rules_rewrap(account_id).await
        {
            tracing::warn!(account = account_id, error = f.message(), "snapshot key not wrapped for new agents");
        }
        // Reports pulled before whose mail has come in since.
        let now = mail_sync::now_millis();
        let matched = db
            .write(move |tx| cloud_reports::match_reports(tx, now))
            .await
            .map_err(|e| Failure::Final(e.to_string()))?;
        if new > 0 || !matched.is_empty() {
            tracing::info!(account = account_id, new, matched = matched.len(), "cloud agents' reports recorded");
            self.rules_event(account_id);
        }
        Ok(new)
    }
}

#[uniffi::export]
impl Core {
    /// The reports cloud agents filed about what they sent from an agent
    /// mailbox (spec §10.6), newest first, with how each matched its sent
    /// mail.
    pub async fn rules_reports(&self, account_id: String, limit: u32) -> Result<Vec<CloudReportInfo>, CoreError> {
        let db = self.store_for(&account_id).await?;
        runtime::run(async move {
            let now = mail_sync::now_millis();
            Ok(db.read(move |c| cloud_reports::recent(c, limit)).await?.into_iter().map(|r| info(r, now)).collect())
        })
        .await
    }

    /// How many reports cloud agents filed from an agent mailbox since
    /// `since` (milliseconds since the Unix epoch): "12 reports this week".
    pub async fn rules_report_count(&self, account_id: String, since: i64) -> Result<u32, CoreError> {
        let db = self.store_for(&account_id).await?;
        runtime::run(async move { Ok(db.read(move |c| cloud_reports::count_since(c, since)).await?) }).await
    }
}
