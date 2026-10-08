//! Unsubscribe from Clean Up (spec §14.12): from the ticked lists or
//! senders, by the `List-Unsubscribe` header of each group's newest
//! message. With `List-Unsubscribe-Post: List-Unsubscribe=One-Click` and
//! an https address, one POST from here (RFC 8058) after the user
//! confirms; with a `mailto:` address, a message the app opens in the
//! composer for the user to send. Never automatic, and never an agent's
//! tool: only the window calls these.
//!
//! The POST carries nothing of the user's: no cookies (the client keeps
//! none), no credentials (an address with a user name is refused), no
//! redirects followed, and a short timeout. Its address is read from the
//! store when the user acts, never taken from the caller.

use std::collections::BTreeMap;
use std::time::Duration;

use mail_store::cleanup::{self, ListHeadersOf, UnsubscribeKind};
use reqwest::Url;

use super::{CleanupScope, CleanupView, query};
use crate::{Core, CoreError, ErrorKind, runtime};

/// How long the one-click POST may take, connecting included.
const TIMEOUT: Duration = Duration::from_secs(10);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// RFC 8058's body, sent as `application/x-www-form-urlencoded`.
pub(crate) const ONE_CLICK_BODY: &str = "List-Unsubscribe=One-Click";

/// How a list is left.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CleanupUnsubscribeMethod {
    /// One POST to an https address on `host` (RFC 8058), from the core.
    OneClick { host: String },
    /// A message to send: the composer opens filled in with it.
    Mailto { to: Vec<String>, cc: Vec<String>, subject: String, body: String },
}

/// One list (or sender) to leave, for the confirmation.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CleanupUnsubscribeTarget {
    /// The ticked groups it covers: groups that share a list are one
    /// target, so each list is asked once.
    pub keys: Vec<String>,
    /// The list's name, else the sender's name, else its id or address.
    pub name: String,
    pub method: CleanupUnsubscribeMethod,
}

/// What one one-click unsubscribe did.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CleanupUnsubscribeResult {
    pub name: String,
    pub host: String,
    /// Why it failed, in words for the window; none when the list
    /// accepted it.
    pub error: Option<String>,
}

/// A `mailto:` address taken apart (RFC 6068).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Mailto {
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub subject: String,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Method {
    OneClick(Url),
    Mailto(Mailto),
}

/// The URIs of a `List-Unsubscribe` header, in order (RFC 2369: each in
/// angle brackets, commas between; whitespace inside is folding).
pub(crate) fn uris(header: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = header;
    while let Some(start) = rest.find('<') {
        let after = &rest[start + 1..];
        let Some(end) = after.find('>') else { break };
        let uri: String = after[..end].chars().filter(|c| !c.is_whitespace()).collect();
        if !uri.is_empty() {
            out.push(uri);
        }
        rest = &after[end + 1..];
    }
    out
}

/// `List-Unsubscribe-Post` asks for RFC 8058's one-click POST.
pub(crate) fn is_one_click(post: Option<&str>) -> bool {
    post.is_some_and(|p| p.trim().eq_ignore_ascii_case(ONE_CLICK_BODY))
}

/// An address the one-click POST may go to: https, a host, no user name
/// or password. (Tests also allow plain http to the loopback address,
/// where their local server listens.)
pub(crate) fn postable(url: &Url) -> bool {
    let secure =
        url.scheme() == "https" || (cfg!(test) && url.scheme() == "http" && url.host_str() == Some("127.0.0.1"));
    secure && url.host_str().is_some_and(|h| !h.is_empty()) && url.username().is_empty() && url.password().is_none()
}

/// Percent-decoding as RFC 6068 means it: `+` stays a plus.
fn decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Some(byte) =
                std::str::from_utf8(&bytes[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(byte);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn addresses(s: &str) -> impl Iterator<Item = String> + '_ {
    s.split(',').map(|a| decode(a).trim().to_owned()).filter(|a| a.contains('@'))
}

/// A `mailto:` URI's recipients, subject and body; none unless it names
/// at least one address. Headers other than To, Cc, Subject and Body are
/// left out.
pub(crate) fn parse_mailto(uri: &str) -> Option<Mailto> {
    let rest = uri.get(..7).filter(|s| s.eq_ignore_ascii_case("mailto:")).map(|_| &uri[7..])?;
    let (to, query) = rest.split_once('?').unwrap_or((rest, ""));
    let mut m = Mailto { to: addresses(to).collect(), ..Mailto::default() };
    for pair in query.split('&').filter(|p| !p.is_empty()) {
        let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
        match decode(name).to_ascii_lowercase().as_str() {
            "to" => m.to.extend(addresses(value)),
            "cc" => m.cc.extend(addresses(value)),
            "subject" => m.subject = decode(value),
            "body" => m.body = decode(value),
            _ => {}
        }
    }
    (!m.to.is_empty()).then_some(m)
}

/// How to leave a list from its headers: the one-click POST when offered
/// over https, else a mailto; none when neither is there (a web page
/// alone is not offered: it would mean opening it).
pub(crate) fn method(unsubscribe: &str, post: Option<&str>) -> Option<Method> {
    let uris = uris(unsubscribe);
    if is_one_click(post)
        && let Some(url) = uris.iter().find_map(|u| Url::parse(u).ok().filter(postable))
    {
        return Some(Method::OneClick(url));
    }
    uris.iter().find_map(|u| parse_mailto(u)).map(Method::Mailto)
}

/// A target with what acting on it needs: the address and what to record.
pub(crate) struct Resolved {
    pub target: CleanupUnsubscribeTarget,
    pub method: Method,
    /// Remembered once it succeeds (`cleanup::unsubscribe_identity`).
    pub identities: Vec<String>,
}

/// The ticked groups' newest headers as targets, one per list: groups
/// sharing a list id (or, without one, the same unsubscribe address) are
/// one target. Groups without a usable header are left out.
pub(crate) fn resolve(view: CleanupView, rows: Vec<ListHeadersOf>) -> Vec<Resolved> {
    let Some(kind) = cleanup::unsubscribe_kind(view.into()) else { return vec![] };
    let mut by_list: BTreeMap<String, Resolved> = BTreeMap::new();
    let mut order = Vec::new();
    for row in rows {
        let Some(method) = row.unsubscribe.as_deref().and_then(|h| method(h, row.unsubscribe_post.as_deref())) else {
            continue;
        };
        let list = match (&row.list_id, &method) {
            (Some(id), _) => format!("list:{}", id.to_lowercase()),
            (None, Method::OneClick(url)) => format!("url:{url}"),
            (None, Method::Mailto(m)) => format!("mailto:{}", m.to.join(",").to_lowercase()),
        };
        let mut identities = vec![cleanup::unsubscribe_identity(kind, &row.key)];
        if let (UnsubscribeKind::Sender, Some(id)) = (kind, &row.list_id) {
            identities.push(cleanup::unsubscribe_identity(UnsubscribeKind::List, id));
        }
        if let Some(existing) = by_list.get_mut(&list) {
            existing.target.keys.push(row.key);
            existing.identities.extend(identities);
            continue;
        }
        let from_name = row.from.as_ref().and_then(|f| f.name.clone()).filter(|n| !n.trim().is_empty());
        let name = [row.list_name.clone().filter(|n| !n.trim().is_empty()), from_name, row.list_id.clone()]
            .into_iter()
            .flatten()
            .next()
            .or_else(|| row.from.as_ref().map(|f| f.email.clone()))
            .unwrap_or_else(|| row.key.clone());
        let shown = match &method {
            Method::OneClick(url) => {
                CleanupUnsubscribeMethod::OneClick { host: url.host_str().unwrap_or("").to_owned() }
            }
            Method::Mailto(m) => CleanupUnsubscribeMethod::Mailto {
                to: m.to.clone(),
                cc: m.cc.clone(),
                subject: m.subject.clone(),
                body: m.body.clone(),
            },
        };
        order.push(list.clone());
        by_list.insert(
            list,
            Resolved {
                target: CleanupUnsubscribeTarget { keys: vec![row.key], name, method: shown },
                method,
                identities,
            },
        );
    }
    order.into_iter().filter_map(|k| by_list.remove(&k)).collect()
}

/// The one-click POST (RFC 8058): the fixed body, no cookies, no
/// credentials, no redirects followed, a short timeout. A 2xx or 3xx
/// answer is taken as done (a redirect to a "you are unsubscribed" page
/// is common, and is not followed).
pub(crate) async fn post_one_click(url: &Url) -> Result<(), String> {
    let host = url.host_str().unwrap_or("the list").to_owned();
    if !postable(url) {
        return Err(format!("{host} is not a secure address"));
    }
    let client = reqwest::Client::builder()
        .timeout(TIMEOUT)
        .connect_timeout(CONNECT_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .user_agent("OpenAGC")
        .build()
        .map_err(|e| e.to_string())?;
    let answer = client
        .post(url.clone())
        .header(reqwest::header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(ONE_CLICK_BODY)
        .send()
        .await;
    match answer {
        Ok(r) if r.status().is_success() || r.status().is_redirection() => Ok(()),
        Ok(r) => Err(format!("{host} answered {}", r.status())),
        Err(e) if e.is_timeout() => Err(format!("{host} did not answer in time")),
        Err(_) => Err(format!("{host} could not be reached")),
    }
}

#[uniffi::export]
impl Core {
    /// What Unsubscribe would do for the groups named by `keys` (Mailing
    /// Lists, Sender, People): one target per list, from each group's
    /// newest message. Empty when none carries `List-Unsubscribe`.
    pub async fn cleanup_unsubscribe_targets(
        &self,
        account_id: String,
        view: CleanupView,
        scope: CleanupScope,
        keys: Vec<String>,
    ) -> Result<Vec<CleanupUnsubscribeTarget>, CoreError> {
        let db = self.store_for(&account_id).await?;
        let q = query(view, scope);
        runtime::run(async move {
            let rows = db.read(move |c| cleanup::newest_list_headers(c, &q, &keys)).await?;
            Ok(resolve(view, rows).into_iter().map(|r| r.target).collect())
        })
        .await
    }

    /// Unsubscribe from the one-click lists among the groups named by
    /// `keys`, once each, after the user confirmed: the addresses are read
    /// from the store now. Mailto targets are the composer's and are left
    /// out. Successes are remembered, so the groups say "Unsubscribed".
    pub async fn cleanup_unsubscribe(
        &self,
        account_id: String,
        view: CleanupView,
        scope: CleanupScope,
        keys: Vec<String>,
    ) -> Result<Vec<CleanupUnsubscribeResult>, CoreError> {
        if keys.is_empty() {
            return Err(CoreError::new(ErrorKind::InvalidInput, "no groups given"));
        }
        let db = self.store_for(&account_id).await?;
        let q = query(view, scope);
        runtime::run(async move {
            let rows = db.read(move |c| cleanup::newest_list_headers(c, &q, &keys)).await?;
            let mut results = Vec::new();
            let mut done = Vec::new();
            for resolved in resolve(view, rows) {
                let Method::OneClick(url) = &resolved.method else { continue };
                let error = post_one_click(url).await.err();
                if let Some(e) = &error {
                    tracing::info!(host = url.host_str().unwrap_or(""), error = %e, "one-click unsubscribe failed");
                } else {
                    done.extend(resolved.identities.iter().cloned());
                }
                results.push(CleanupUnsubscribeResult {
                    name: resolved.target.name.clone(),
                    host: url.host_str().unwrap_or("").to_owned(),
                    error,
                });
            }
            if !done.is_empty() {
                let at = mail_sync::now_millis();
                db.write(move |tx| cleanup::record_unsubscribed(tx, &done, at)).await?;
            }
            Ok(results)
        })
        .await
    }
}

#[cfg(test)]
mod tests;
