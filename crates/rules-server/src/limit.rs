//! Token buckets in memory, a burst of a minute's allowance refilled
//! evenly: one per bearer token, and for what strangers can do without one
//! (register, open the consent page, try connect codes, fail to sign in)
//! one per client address, under a larger ceiling for the whole server.
//!
//! A client's address is the peer's, or, when the peer is a proxy the
//! operator trusts (`KALUTA_RULES_TRUSTED_PROXY`), the nearest address in
//! `X-Forwarded-For` that is not one of those proxies. IPv6 addresses count
//! by their /64, which one host usually holds whole.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Beyond this many buckets, full ones are dropped (they hold nothing).
const PRUNE_AT: usize = 10_000;

pub struct RateLimiter {
    per_minute: u32,
    buckets: Mutex<HashMap<String, Bucket>>,
}

struct Bucket {
    tokens: f64,
    last: Instant,
}

impl RateLimiter {
    /// `per_minute` 0 turns limiting off.
    pub fn new(per_minute: u32) -> Self {
        Self { per_minute, buckets: Mutex::new(HashMap::new()) }
    }

    /// Take one request for `key`, or say how long until one is allowed.
    pub fn take(&self, key: &str) -> Result<(), Duration> {
        self.bucket(key, true)
    }

    /// Whether `key` has a request left, without taking it: for limits
    /// counted only after the fact (wrong codes, failed sign-ins).
    pub fn peek(&self, key: &str) -> Result<(), Duration> {
        self.bucket(key, false)
    }

    fn bucket(&self, key: &str, take: bool) -> Result<(), Duration> {
        if self.per_minute == 0 {
            return Ok(());
        }
        let capacity = f64::from(self.per_minute);
        let per_second = capacity / 60.0;
        let now = Instant::now();
        let mut buckets = self.buckets.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if buckets.len() > PRUNE_AT {
            buckets.retain(|_, b| b.tokens + now.duration_since(b.last).as_secs_f64() * per_second < capacity);
        }
        let b = buckets.entry(key.to_owned()).or_insert(Bucket { tokens: capacity, last: now });
        b.tokens = (b.tokens + now.duration_since(b.last).as_secs_f64() * per_second).min(capacity);
        b.last = now;
        if b.tokens >= 1.0 {
            if take {
                b.tokens -= 1.0;
            }
            Ok(())
        } else {
            Err(Duration::from_secs_f64((1.0 - b.tokens) / per_second))
        }
    }
}

/// A network the operator trusts as a proxy: `10.0.0.0/8`, `::1/128`, or
/// an address alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cidr {
    net: IpAddr,
    prefix: u8,
}

impl Cidr {
    pub fn parse(s: &str) -> Result<Self, String> {
        let s = s.trim();
        let (addr, prefix) = s.split_once('/').map_or((s, None), |(a, p)| (a, Some(p)));
        let net: IpAddr = addr.parse().map_err(|_| format!("{s:?} is not an address or CIDR network"))?;
        let max = if net.is_ipv4() { 32 } else { 128 };
        let prefix = match prefix {
            None => max,
            Some(p) => p.parse::<u8>().ok().filter(|p| *p <= max).ok_or_else(|| format!("{s:?}: bad prefix length"))?,
        };
        Ok(Self { net, prefix })
    }

    pub fn contains(&self, ip: IpAddr) -> bool {
        let ip = canonical(ip);
        match (self.net, ip) {
            (IpAddr::V4(n), IpAddr::V4(a)) => prefix_eq(&n.octets(), &a.octets(), self.prefix),
            (IpAddr::V6(n), IpAddr::V6(a)) => prefix_eq(&n.octets(), &a.octets(), self.prefix),
            _ => false,
        }
    }
}

fn prefix_eq(a: &[u8], b: &[u8], bits: u8) -> bool {
    let bits = usize::from(bits);
    let (whole, rest) = (bits / 8, bits % 8);
    if a[..whole] != b[..whole] {
        return false;
    }
    rest == 0 || (a[whole] ^ b[whole]) >> (8 - rest) == 0
}

/// An IPv4 address mapped into IPv6 (`::ffff:1.2.3.4`) as itself.
fn canonical(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(ip, IpAddr::V4),
        v4 => v4,
    }
}

/// The networks in `KALUTA_RULES_TRUSTED_PROXY`: comma- or space-separated.
pub fn parse_trusted(list: &[String]) -> Result<Vec<Cidr>, String> {
    list.iter().flat_map(|s| s.split([',', ' '])).filter(|s| !s.trim().is_empty()).map(Cidr::parse).collect()
}

/// The client's address as a bucket key: the peer, or behind a trusted
/// proxy the nearest untrusted address in `X-Forwarded-For` (read right to
/// left, so a client cannot choose it by sending the header itself).
/// `unknown` when the server was not told its peers.
pub fn client_key(peer: Option<SocketAddr>, headers: &http::HeaderMap, trusted: &[Cidr]) -> String {
    let Some(peer) = peer.map(|p| canonical(p.ip())) else { return "unknown".to_owned() };
    let is_trusted = |ip: IpAddr| trusted.iter().any(|c| c.contains(ip));
    let mut client = peer;
    if is_trusted(peer) {
        let forwarded: Vec<Option<IpAddr>> = headers
            .get_all("x-forwarded-for")
            .iter()
            .filter_map(|v| v.to_str().ok())
            .flat_map(|v| v.split(','))
            .map(|a| a.trim().parse::<IpAddr>().ok().map(canonical))
            .collect();
        for ip in forwarded.into_iter().rev() {
            // Something that is not an address: stop at what we trust.
            let Some(ip) = ip else { break };
            client = ip;
            if !is_trusted(ip) {
                break;
            }
        }
    }
    match client {
        IpAddr::V4(v4) => v4.to_string(),
        IpAddr::V6(v6) => {
            let s = v6.segments();
            format!("{:x}:{:x}:{:x}:{:x}::/64", s[0], s[1], s[2], s[3])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peek_does_not_take() {
        let l = RateLimiter::new(1);
        assert!(l.peek("a").is_ok() && l.peek("a").is_ok());
        assert!(l.take("a").is_ok());
        assert!(l.peek("a").is_err());
    }

    #[test]
    fn the_client_is_the_peer_or_behind_a_trusted_proxy_the_nearest_untrusted_forwarded_address() {
        let headers = |xff: &str| {
            let mut h = http::HeaderMap::new();
            h.insert("x-forwarded-for", xff.parse().unwrap());
            h
        };
        let peer: SocketAddr = "127.0.0.1:5000".parse().unwrap();
        let trusted = parse_trusted(&["127.0.0.1/32, 10.0.0.0/8".into()]).unwrap();
        // Not trusted: the header is the client's to choose, so ignored.
        assert_eq!(client_key(Some(peer), &headers("203.0.113.9"), &[]), "127.0.0.1");
        assert_eq!(client_key(Some(peer), &headers("203.0.113.9"), &trusted), "203.0.113.9");
        // A forged left-hand entry does not win over what the proxy added.
        assert_eq!(client_key(Some(peer), &headers("1.2.3.4, 203.0.113.9, 10.0.0.2"), &trusted), "203.0.113.9");
        assert_eq!(client_key(Some(peer), &headers("10.0.0.3, 10.0.0.2"), &trusted), "10.0.0.3");
        assert_eq!(client_key(Some(peer), &http::HeaderMap::new(), &trusted), "127.0.0.1");
        assert_eq!(client_key(Some(peer), &headers("junk, 203.0.113.9"), &trusted), "203.0.113.9");
        assert_eq!(client_key(Some(peer), &headers("203.0.113.9, junk"), &trusted), "127.0.0.1");
        let v6: SocketAddr = "[2001:db8:1:2:3:4:5:6]:5000".parse().unwrap();
        assert_eq!(client_key(Some(v6), &http::HeaderMap::new(), &trusted), "2001:db8:1:2::/64");
        let mapped: SocketAddr = "[::ffff:127.0.0.1]:5000".parse().unwrap();
        assert_eq!(client_key(Some(mapped), &headers("203.0.113.9"), &trusted), "203.0.113.9");
        assert_eq!(client_key(None, &headers("203.0.113.9"), &trusted), "unknown");
        assert!(Cidr::parse("10.0.0.0/33").is_err() && Cidr::parse("nope").is_err());
        assert!(Cidr::parse("172.16.0.0/12").unwrap().contains("172.31.255.1".parse().unwrap()));
        assert!(!Cidr::parse("172.16.0.0/12").unwrap().contains("172.32.0.1".parse().unwrap()));
        assert!(Cidr::parse("::1").unwrap().contains("::1".parse().unwrap()));
    }

    #[test]
    fn a_burst_of_a_minutes_allowance_then_waits() {
        let l = RateLimiter::new(3);
        assert!((0..3).all(|_| l.take("a").is_ok()));
        let wait = l.take("a").unwrap_err();
        assert!(wait > Duration::from_secs(15) && wait <= Duration::from_secs(20), "{wait:?}");
        assert!(l.take("b").is_ok(), "each key has its own bucket");
        assert!(RateLimiter::new(0).take("a").is_ok());
    }
}
