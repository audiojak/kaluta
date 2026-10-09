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
//! store when the user acts, never taken from the caller, and only from
//! the very messages the confirmation was made from: if newer mail of the
//! list arrived since, nothing is posted and the user is asked to look
//! again. One-click is offered only to an address on the list's own site
//! (the same registrable domain as its `List-Id` or its sender), so a
//! message cannot send the POST to an unrelated host, and never to the
//! local network: https on 443 at a public-looking name, whose addresses
//! are all public when looked up, and the POST connects to exactly those.

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
    /// For each key, in order, the newest message whose headers this was
    /// read from. Unsubscribing checks they are still the newest.
    pub message_ids: Vec<String>,
    /// The list's name, else the sender's name, else its id or address.
    pub name: String,
    pub method: CleanupUnsubscribeMethod,
}

/// What one one-click unsubscribe did.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CleanupUnsubscribeResult {
    /// The groups it covered (the target's keys).
    pub keys: Vec<String>,
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

/// Name suffixes that only ever mean a private network: mDNS, loopback
/// names, and the names people and routers give their own networks.
const PRIVATE_SUFFIXES: &[&str] = &["local", "localhost", "internal", "lan", "home.arpa", "intranet", "corp"];

/// An address the one-click POST may go to: https on the default port, a
/// public-looking host name (not an IP address, not a single label, not a
/// private suffix such as `.local` or `.internal`), no user name or
/// password. The name's addresses are checked again when posting
/// ([`checked_addresses`]). (Unit tests also allow plain http to the
/// loopback port their local server registered; that allowance is not
/// compiled into the app.)
pub(crate) fn postable(url: &Url) -> bool {
    if url.username() != "" || url.password().is_some() {
        return false;
    }
    if test_server::allowed(url) {
        return true;
    }
    let Some(host) = url.host_str() else { return false };
    if host.trim_start_matches('[').trim_end_matches(']').parse::<std::net::IpAddr>().is_ok() {
        return false;
    }
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    let private = PRIVATE_SUFFIXES.iter().any(|s| host == *s || host.ends_with(&format!(".{s}")));
    url.scheme() == "https" && url.port().is_none() && host.contains('.') && !private
}

/// What the confirmation names: the host, with its port if it is not the
/// default (never, for an address [`postable`] lets through, outside tests).
pub(crate) fn shown_host(url: &Url) -> String {
    let host = url.host_str().unwrap_or("").to_owned();
    match url.port() {
        Some(port) => format!("{host}:{port}"),
        None => host,
    }
}

/// Whether the one-click POST may connect to `ip`: a public unicast
/// address. Loopback, unspecified, private (RFC 1918), shared (CGNAT
/// 100.64/10), link-local, multicast, broadcast, benchmarking, reserved,
/// unique-local (fc00::/7) and site-local addresses are not, nor IPv6
/// addresses that carry one of those (IPv4-mapped, NAT64).
pub(crate) fn public_address(ip: std::net::IpAddr) -> bool {
    use std::net::{IpAddr, Ipv4Addr};
    fn v4(ip: Ipv4Addr) -> bool {
        let [a, b, c, _] = ip.octets();
        !(ip.is_loopback()
            || ip.is_unspecified()
            || ip.is_private()
            || ip.is_link_local()
            || ip.is_multicast()
            || ip.is_broadcast()
            || ip.is_documentation()
            || a == 0
            || (a == 100 && (64..128).contains(&b))
            || (a == 192 && b == 0 && c == 0)
            || (a == 198 && (18..20).contains(&b))
            || a >= 240)
    }
    match ip {
        IpAddr::V4(ip) => v4(ip),
        IpAddr::V6(ip) => {
            if let Some(mapped) = ip.to_ipv4_mapped() {
                return v4(mapped);
            }
            let s = ip.segments();
            if s[0] == 0x64 && s[1] == 0xff9b && s[2..6] == [0, 0, 0, 0] {
                let [_, _, _, _, _, _, hi, lo] = s;
                return v4(Ipv4Addr::new((hi >> 8) as u8, hi as u8, (lo >> 8) as u8, lo as u8));
            }
            !(ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_multicast()
                || (s[0] & 0xfe00) == 0xfc00
                || (s[0] & 0xffc0) == 0xfe80
                || (s[0] & 0xffc0) == 0xfec0
                || s[0..6] == [0, 0, 0, 0, 0, 0]
                || (s[0] == 0x2001 && s[1] == 0xdb8))
        }
    }
}

/// The addresses `host` resolved to, if every one is public; a single
/// private answer refuses them all (a rebinding name may give both).
pub(crate) fn checked_addresses(
    host: &str,
    addrs: &[std::net::SocketAddr],
) -> Result<Vec<std::net::SocketAddr>, String> {
    if addrs.is_empty() {
        return Err(format!("{host} could not be found"));
    }
    if addrs.iter().any(|a| !public_address(a.ip())) {
        return Err(format!("{host} leads to a private address"));
    }
    Ok(addrs.to_vec())
}

/// The unit tests' local servers: plain http to 127.0.0.1 on a port a test
/// registered. Outside `cfg(test)` nothing is ever allowed, so the app
/// cannot be made to post to the loopback address.
pub(crate) mod test_server {
    use reqwest::Url;

    #[cfg(test)]
    static PORTS: std::sync::Mutex<Vec<u16>> = std::sync::Mutex::new(Vec::new());

    /// Allow posting to `http://127.0.0.1:<port>` in this test run.
    #[cfg(test)]
    pub(crate) fn register(port: u16) {
        PORTS.lock().unwrap().push(port);
    }

    #[cfg(test)]
    pub(crate) fn allowed(url: &Url) -> bool {
        url.scheme() == "http"
            && url.host_str() == Some("127.0.0.1")
            && url.port().is_some_and(|p| PORTS.lock().unwrap().contains(&p))
    }

    #[cfg(not(test))]
    pub(crate) fn allowed(_url: &Url) -> bool {
        false
    }
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

/// Second-level labels under a two-letter country code that are
/// registries' own (`example.co.uk`), so the registrable domain takes one
/// more label. A short list, not the Public Suffix List: a miss makes a
/// one-click address look foreign (not offered), never the reverse for a
/// different `.com` or `.org` site.
const COUNTRY_SECOND_LEVELS: &[&str] = &["ac", "co", "com", "edu", "gov", "ne", "net", "or", "org", "go"];

/// A host's registrable domain, roughly: its last two labels, or three
/// under a country's own second level (`co.uk`). IP addresses are their
/// own.
pub(crate) fn registrable(host: &str) -> String {
    let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
    if host.parse::<std::net::IpAddr>().is_ok() {
        return host;
    }
    let labels: Vec<&str> = host.split('.').filter(|l| !l.is_empty()).collect();
    let n = labels.len();
    let take = if n >= 3 && labels[n - 1].len() == 2 && COUNTRY_SECOND_LEVELS.contains(&labels[n - 2]) { 3 } else { 2 };
    labels[n.saturating_sub(take)..].join(".")
}

/// The domains a list's mail says it is from: its `List-Id` (`name.domain`)
/// and its sender's address.
pub(crate) fn owners(list_id: Option<&str>, from_email: Option<&str>) -> Vec<String> {
    let from = from_email.and_then(|e| e.rsplit_once('@')).map(|(_, domain)| domain);
    list_id.into_iter().chain(from).map(registrable).filter(|d| !d.is_empty()).collect()
}

/// How to leave a list from its headers: the one-click POST when offered
/// over https to the list's own site (`owners`: the registrable domains of
/// its `List-Id` and sender), else a mailto; none when neither is there.
/// A web page alone is not offered (it would mean opening it), nor a
/// one-click address on another site: the confirmation names the host,
/// but a message from one sender should not make the Mac post to an
/// unrelated one.
pub(crate) fn method(unsubscribe: &str, post: Option<&str>, owners: &[String]) -> Option<Method> {
    let uris = uris(unsubscribe);
    if is_one_click(post)
        && let Some(url) = uris.iter().find_map(|u| {
            Url::parse(u)
                .ok()
                .filter(postable)
                .filter(|url| url.host_str().is_some_and(|h| owners.contains(&registrable(h))))
        })
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
        let owners = owners(row.list_id.as_deref(), row.from.as_ref().map(|f| f.email.as_str()));
        let Some(method) = row.unsubscribe.as_deref().and_then(|h| method(h, row.unsubscribe_post.as_deref(), &owners))
        else {
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
            existing.target.message_ids.push(row.message_id.0);
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
            Method::OneClick(url) => CleanupUnsubscribeMethod::OneClick { host: shown_host(url) },
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
                target: CleanupUnsubscribeTarget {
                    keys: vec![row.key],
                    message_ids: vec![row.message_id.0],
                    name,
                    method: shown,
                },
                method,
                identities,
            },
        );
    }
    order.into_iter().filter_map(|k| by_list.remove(&k)).collect()
}

/// Why a confirmed target is not posted to: the list's newest message is
/// not the one the confirmation showed.
pub(crate) const REVIEW_AGAIN: &str = "New mail arrived from this list; review it again";

/// The one-click POST (RFC 8058): the fixed body, no cookies, no
/// credentials, no redirects followed, a short timeout. Only a 2xx answer
/// is done. A redirect is not followed and not taken as done: the list
/// wants a page opened to finish, which is the user's to do, so the
/// answer names where it points.
pub(crate) async fn post_one_click(url: &Url) -> Result<(), String> {
    post_resolved(url, |host: String| async move {
        tokio::net::lookup_host((host.as_str(), 443)).await.map(|a| a.collect())
    })
    .await
}

/// A client that connects to `addrs` for `host` and nowhere else: the name
/// is not looked up again (no window for it to change between the check
/// and the connection), and no proxy is used (a proxy would resolve it
/// itself).
pub(crate) fn pinned_client(host: &str, addrs: &[std::net::SocketAddr]) -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(TIMEOUT)
        .connect_timeout(CONNECT_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .resolve_to_addrs(host, addrs)
        .user_agent("Kaluta")
        .build()
        .map_err(|e| e.without_url().to_string())
}

/// [`post_one_click`] with the name lookup given: the host's addresses are
/// looked up once, refused unless all are public, and the POST connects to
/// exactly those.
pub(crate) async fn post_resolved<L, F>(url: &Url, lookup: L) -> Result<(), String>
where
    L: FnOnce(String) -> F,
    F: std::future::Future<Output = std::io::Result<Vec<std::net::SocketAddr>>>,
{
    let host = url.host_str().unwrap_or("the list").to_owned();
    let shown = shown_host(url);
    if !postable(url) {
        return Err(format!("{shown} is not a secure address"));
    }
    let addrs = if test_server::allowed(url) {
        let port = url.port().unwrap_or(80);
        vec![std::net::SocketAddr::from(([127, 0, 0, 1], port))]
    } else {
        let found = lookup(host.clone()).await.map_err(|_| format!("{host} could not be found"))?;
        checked_addresses(&host, &found)?
    };
    let client = pinned_client(&host, &addrs)?;
    let answer = client
        .post(url.clone())
        .header(reqwest::header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(ONE_CLICK_BODY)
        .send()
        .await;
    match answer {
        Ok(r) if r.status().is_success() => Ok(()),
        Ok(r) if r.status().is_redirection() => {
            let to = r
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|l| l.to_str().ok())
                .and_then(|l| url.join(l).ok())
                .and_then(|l| l.host_str().map(str::to_owned));
            Err(match to {
                Some(to) => format!("the list wants you to open a page at {to} to finish"),
                None => "the list wants you to open a page to finish".to_owned(),
            })
        }
        Ok(r) => Err(format!("{shown} answered {}", r.status())),
        Err(e) if e.is_timeout() => Err(format!("{shown} did not answer in time")),
        Err(_) => Err(format!("{shown} could not be reached")),
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

    /// Unsubscribe from the one-click lists among `targets`, the ones the
    /// user confirmed (as [`Self::cleanup_unsubscribe_targets`] gave them),
    /// once each. Each address is read from the store again, from the same
    /// messages the confirmation was made from: if a group's newest message
    /// is another one now (new mail of the list arrived), or the address no
    /// longer leads to the host shown, nothing is posted and the result
    /// says to review it again. Mailto targets are the composer's and are
    /// left out. Successes are remembered, so the groups say "Unsubscribed".
    pub async fn cleanup_unsubscribe(
        &self,
        account_id: String,
        view: CleanupView,
        scope: CleanupScope,
        targets: Vec<CleanupUnsubscribeTarget>,
    ) -> Result<Vec<CleanupUnsubscribeResult>, CoreError> {
        let targets: Vec<CleanupUnsubscribeTarget> =
            targets.into_iter().filter(|t| matches!(t.method, CleanupUnsubscribeMethod::OneClick { .. })).collect();
        if targets.is_empty() || targets.iter().any(|t| t.keys.is_empty() || t.keys.len() != t.message_ids.len()) {
            return Err(CoreError::new(ErrorKind::InvalidInput, "no one-click lists given"));
        }
        let db = self.store_for(&account_id).await?;
        let q = query(view, scope);
        runtime::run(async move {
            let mut results = Vec::new();
            let mut done = Vec::new();
            for confirmed in targets {
                let CleanupUnsubscribeMethod::OneClick { host: shown } = &confirmed.method else { continue };
                let keys = confirmed.keys.clone();
                let rows = db.read(move |c| cleanup::newest_list_headers(c, &q, &keys)).await?;
                let newest: Vec<&str> = rows.iter().map(|r| r.message_id.as_str()).collect();
                let still = newest == confirmed.message_ids.iter().map(String::as_str).collect::<Vec<_>>();
                let resolved = if still { resolve(view, rows) } else { vec![] };
                let url = match resolved.as_slice() {
                    [one] => match &one.method {
                        Method::OneClick(url) if shown_host(url) == *shown => Some((url.clone(), one)),
                        _ => None,
                    },
                    _ => None,
                };
                let Some((url, resolved)) = url else {
                    tracing::info!(host = %shown, "one-click unsubscribe not sent: the list's newest mail changed");
                    results.push(CleanupUnsubscribeResult {
                        keys: confirmed.keys.clone(),
                        name: confirmed.name.clone(),
                        host: shown.clone(),
                        error: Some(REVIEW_AGAIN.to_owned()),
                    });
                    continue;
                };
                let url = &url;
                let error = post_one_click(url).await.err();
                if let Some(e) = &error {
                    tracing::info!(host = %shown_host(url), error = %e, "one-click unsubscribe failed");
                } else {
                    done.extend(resolved.identities.iter().cloned());
                }
                results.push(CleanupUnsubscribeResult {
                    keys: resolved.target.keys.clone(),
                    name: resolved.target.name.clone(),
                    host: shown_host(url),
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
