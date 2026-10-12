//! The Agents workflow (spec §14.13): what agents wait on the user for,
//! across accounts. Proposed actions (sends end on the editable email,
//! §10.4), verification codes arriving in agent mailboxes, and the
//! latest run of a sorting routine to keep or undo; and what agent
//! mailboxes sent on their own since the user last looked.

use mail_domain::Millis;

use crate::registry::AccountKind;
use crate::{Core, CoreError, runtime};

/// Codes this recent are shown.
const CODE_WINDOW_MS: Millis = 30 * 60 * 1000;
/// `workflow_meta` key: when the user last looked at the Agents workflow.
const LOOKED_KEY: &str = "agents_looked_at";

/// A verification code that arrived in an agent mailbox.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AgentCode {
    pub account_id: String,
    /// The agent mailbox's address.
    pub mailbox: String,
    pub message_id: String,
    pub from: String,
    pub subject: String,
    pub code: String,
    pub received_at: i64,
    /// From "expires in 10 minutes", if the message says.
    pub expires_at: Option<i64>,
}

/// What an agent mailbox sent on its own.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AgentSent {
    pub account_id: String,
    pub mailbox: String,
    pub count: u32,
}

/// Everything the Agents workflow shows besides the proposed actions,
/// which the app's agent panels hold.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AgentsWork {
    pub codes: Vec<AgentCode>,
    pub sent_on_their_own: Vec<AgentSent>,
    /// Since when `sent_on_their_own` counts.
    pub since: i64,
}

/// A six-digit code in `text`: standing alone, or as two groups of three
/// ("482 913", "482-913").
pub(crate) fn code_in(text: &str) -> Option<String> {
    if let Some(code) = crate::agent_mailbox::find_code(text) {
        return Some(code);
    }
    let b = text.as_bytes();
    (0..b.len().saturating_sub(6)).find_map(|i| {
        let w = &b[i..i + 7];
        let digits = |s: &[u8]| s.iter().all(u8::is_ascii_digit);
        let alone_before = i == 0 || !b[i - 1].is_ascii_alphanumeric();
        let alone_after = i + 7 == b.len() || !b[i + 7].is_ascii_alphanumeric();
        (digits(&w[..3]) && (w[3] == b' ' || w[3] == b'-') && digits(&w[4..]) && alone_before && alone_after)
            .then(|| format!("{}{}", &text[i..i + 3], &text[i + 4..i + 7]))
    })
}

/// "expires in 10 minutes" → ten minutes after `received`.
fn expiry(text: &str, received: Millis) -> Option<Millis> {
    let lower = text.to_lowercase();
    let rest = lower.split("expires in ").nth(1)?;
    let mut words = rest.split_whitespace();
    let n: i64 = words.next()?.parse().ok()?;
    let unit = words.next().unwrap_or("");
    let ms = if unit.starts_with("hour") { n * 3_600_000 } else { n * 60_000 };
    Some(received + ms)
}

impl Core {
    /// The agent mailboxes on this Mac: (account id, address).
    async fn agent_mailboxes(&self) -> Vec<(String, String)> {
        let data_dir = self.data_path();
        runtime::run(async move {
            tokio::task::spawn_blocking(move || crate::registry::load_index(&data_dir))
                .await
                .map_err(|e| CoreError::new(crate::ErrorKind::Internal, e.to_string()))
        })
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|e| e.kind == AccountKind::Agent)
        .map(|e| (e.id, e.email))
        .collect()
    }

    /// When the user last looked at the Agents workflow (else a day back).
    async fn agents_looked_at(&self, now: Millis) -> Result<Millis, CoreError> {
        let db = self.db()?;
        runtime::run(async move {
            Ok(db
                .read(|c| mail_store::workflows::meta(c, LOOKED_KEY))
                .await?
                .and_then(|v| v.parse().ok())
                .unwrap_or(now - 86_400_000))
        })
        .await
    }
}

#[uniffi::export]
impl Core {
    /// Codes from the last half hour in every agent mailbox, newest first,
    /// and what agent mailboxes sent on their own since the user last
    /// looked (spec §14.13).
    pub async fn agents_work(&self) -> Result<AgentsWork, CoreError> {
        let now = mail_sync::now_millis();
        let since = self.agents_looked_at(now).await?;
        let mut codes = Vec::new();
        let mut sent = Vec::new();
        for (account, mailbox) in self.agent_mailboxes().await {
            let found = crate::registry::scoped(Some(account.clone()), async {
                let db = self.db()?;
                runtime::run(async move {
                    Ok(db
                        .read(move |c| {
                            let mut stmt = c.prepare_cached(
                                "SELECT m.gmail_id, COALESCE(m.from_name, m.from_email, ''), m.subject, m.snippet,
                                        m.internal_date, m.is_sent_by_me
                                   FROM messages m
                                  WHERE m.is_draft = 0 AND m.internal_date >= ?1
                                  ORDER BY m.internal_date DESC",
                            )?;
                            let rows = stmt
                                .query_map([since.min(now - CODE_WINDOW_MS)], |r| {
                                    Ok((
                                        r.get::<_, String>(0)?,
                                        r.get::<_, String>(1)?,
                                        r.get::<_, String>(2)?,
                                        r.get::<_, String>(3)?,
                                        r.get::<_, i64>(4)?,
                                        r.get::<_, bool>(5)?,
                                    ))
                                })?
                                .collect::<Result<Vec<_>, _>>()?;
                            Ok(rows)
                        })
                        .await?)
                })
                .await
            })
            .await;
            let Ok(rows) = found else { continue };
            let mut own = 0;
            for (id, from, subject, snippet, at, mine) in rows {
                if mine {
                    if at >= since {
                        own += 1;
                    }
                    continue;
                }
                if at < now - CODE_WINDOW_MS {
                    continue;
                }
                let text = format!("{subject}\n{snippet}");
                if let Some(code) = code_in(&text) {
                    codes.push(AgentCode {
                        account_id: account.clone(),
                        mailbox: mailbox.clone(),
                        message_id: id,
                        from,
                        subject,
                        expires_at: expiry(&text, at),
                        code,
                        received_at: at,
                    });
                }
            }
            if own > 0 {
                sent.push(AgentSent { account_id: account, mailbox, count: own });
            }
        }
        codes.sort_by_key(|c| -c.received_at);
        Ok(AgentsWork { codes, sent_on_their_own: sent, since })
    }

    /// The user looked at the Agents workflow: what agent mailboxes send
    /// from now on counts as new.
    pub async fn mark_agents_looked(&self) -> Result<(), CoreError> {
        let db = self.db()?;
        let now = mail_sync::now_millis().to_string();
        runtime::run(
            async move { Ok(db.write(move |tx| mail_store::workflows::set_meta(tx, LOOKED_KEY, &now)).await?) },
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_stand_alone_or_in_two_groups() {
        assert_eq!(code_in("Your verification code is 482 913. It expires"), Some("482913".into()));
        assert_eq!(code_in("Code: 482913"), Some("482913".into()));
        assert_eq!(code_in("Use 482-913 to sign in"), Some("482913".into()));
        assert_eq!(code_in("Order 1482 9134 shipped"), None);
        assert_eq!(code_in("Call 555 0100"), None);
        assert_eq!(expiry("It expires in 10 minutes.", 1_000), Some(601_000));
        assert_eq!(expiry("No expiry here", 1_000), None);
    }
}
