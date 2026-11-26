//! Optional secure DNS (DNS over HTTPS, RFC 8484) for BDownloader's own
//! connections.
//!
//! Every HTTP client of the app uses [`resolver`]. While secure DNS is off it
//! asks the system, exactly like before. When the user turns it on, names
//! are asked to the chosen DoH server instead. Nothing changes for Windows,
//! the network card or other programs.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::{Arc, LazyLock, RwLock};
use std::time::{Duration, Instant};

use reqwest::dns::{Addrs, Name, Resolve, Resolving};

type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// Known DoH servers: (id, name, endpoint). The endpoints use IP addresses so
/// that reaching them needs no DNS at all.
pub const PROVIDERS: &[(&str, &str, &str)] = &[
    ("cloudflare", "Cloudflare (1.1.1.1)", "https://1.1.1.1/dns-query"),
    ("quad9", "Quad9 (9.9.9.9)", "https://9.9.9.9/dns-query"),
    ("google", "Google (8.8.8.8)", "https://8.8.8.8/dns-query"),
];

/// Endpoint of a provider id, or the id itself when it is a custom https URL.
pub fn endpoint_of(provider: &str) -> Option<String> {
    PROVIDERS
        .iter()
        .find(|(id, _, _)| *id == provider)
        .map(|(_, _, url)| (*url).to_owned())
        .or_else(|| provider.starts_with("https://").then(|| provider.to_owned()))
}

struct State {
    endpoint: Option<String>,
    cache: HashMap<String, (Vec<IpAddr>, Instant)>,
}

static STATE: LazyLock<RwLock<State>> = LazyLock::new(|| RwLock::new(State { endpoint: None, cache: HashMap::new() }));

/// Plain client (system DNS is never needed: endpoints are IP addresses).
static DOH_CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(8))
        .timeout(Duration::from_secs(10))
        .build()
        .expect("static client configuration")
});

/// Turns secure DNS on with this DoH endpoint, or off with `None`.
pub fn set_secure_dns(endpoint: Option<String>) {
    let mut state = STATE.write().unwrap_or_else(|e| e.into_inner());
    if state.endpoint != endpoint {
        state.cache.clear();
    }
    state.endpoint = endpoint;
}

pub fn secure_dns() -> Option<String> {
    STATE.read().unwrap_or_else(|e| e.into_inner()).endpoint.clone()
}

/// Resolver to give to every `reqwest::ClientBuilder::dns_resolver`.
pub fn resolver() -> Arc<dyn Resolve> {
    static RESOLVER: LazyLock<Arc<dyn Resolve>> = LazyLock::new(|| Arc::new(Switchable));
    RESOLVER.clone()
}

struct Switchable;

impl Resolve for Switchable {
    fn resolve(&self, name: Name) -> Resolving {
        let host = name.as_str().to_owned();
        Box::pin(async move {
            let addrs: Vec<SocketAddr> = match secure_dns() {
                Some(endpoint) => {
                    lookup(&endpoint, &host).await?.into_iter().map(|ip| SocketAddr::new(ip, 0)).collect()
                }
                None => tokio::net::lookup_host((host.as_str(), 0)).await?.collect(),
            };
            Ok(Box::new(addrs.into_iter()) as Addrs)
        })
    }
}

/// Addresses of `host` from the DoH server, IPv4 first, cached for the TTL.
pub async fn lookup(endpoint: &str, host: &str) -> Result<Vec<IpAddr>, BoxError> {
    if let Ok(ip) = host.parse::<IpAddr>() {
        return Ok(vec![ip]);
    }
    let key = host.to_ascii_lowercase();
    if let Some((ips, until)) = STATE.read().unwrap_or_else(|e| e.into_inner()).cache.get(&key) {
        if *until > Instant::now() {
            return Ok(ips.clone());
        }
    }
    let (v4, v6) = tokio::join!(query(endpoint, &key, TYPE_A), query(endpoint, &key, TYPE_AAAA));
    let mut ips = Vec::new();
    let mut ttl = u32::MAX;
    let mut first_error = None;
    for result in [v4, v6] {
        match result {
            Ok((found, t)) => {
                if !found.is_empty() {
                    ttl = ttl.min(t);
                }
                ips.extend(found);
            }
            Err(e) => first_error = first_error.or(Some(e)),
        }
    }
    if ips.is_empty() {
        return Err(first_error.unwrap_or_else(|| format!("no address for {host} (secure DNS)").into()));
    }
    let ttl = Duration::from_secs(u64::from(ttl.clamp(60, 3600)));
    STATE.write().unwrap_or_else(|e| e.into_inner()).cache.insert(key, (ips.clone(), Instant::now() + ttl));
    Ok(ips)
}

const TYPE_A: u16 = 1;
const TYPE_AAAA: u16 = 28;

async fn query(endpoint: &str, host: &str, kind: u16) -> Result<(Vec<IpAddr>, u32), BoxError> {
    let response = DOH_CLIENT
        .post(endpoint)
        .header(reqwest::header::CONTENT_TYPE, "application/dns-message")
        .header(reqwest::header::ACCEPT, "application/dns-message")
        .body(build_query(host, kind)?)
        .send()
        .await?
        .error_for_status()?;
    let bytes = response.bytes().await?;
    parse_answer(&bytes, kind)
}

/// DNS query message for one name and record type.
fn build_query(host: &str, kind: u16) -> Result<Vec<u8>, BoxError> {
    // Id 0 is recommended for DoH (better caching); recursion desired.
    let mut message = vec![0, 0, 0x01, 0x00, 0, 1, 0, 0, 0, 0, 0, 0];
    for label in host.trim_end_matches('.').split('.') {
        if label.is_empty() || label.len() > 63 {
            return Err(format!("invalid host name {host}").into());
        }
        message.push(label.len() as u8);
        message.extend_from_slice(label.as_bytes());
    }
    message.push(0);
    message.extend_from_slice(&kind.to_be_bytes());
    message.extend_from_slice(&1u16.to_be_bytes());
    Ok(message)
}

fn read_u16(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes([*bytes.get(at)?, *bytes.get(at + 1)?]))
}

/// Skips a (possibly compressed) name and returns the position after it.
fn skip_name(bytes: &[u8], mut at: usize) -> Option<usize> {
    loop {
        let len = *bytes.get(at)?;
        if len & 0xC0 == 0xC0 {
            return Some(at + 2);
        }
        if len == 0 {
            return Some(at + 1);
        }
        at += 1 + len as usize;
    }
}

/// Addresses of the wanted type in a DNS answer, with the smallest TTL.
fn parse_answer(bytes: &[u8], kind: u16) -> Result<(Vec<IpAddr>, u32), BoxError> {
    let bad = || -> BoxError { "malformed DNS answer".into() };
    let flags = read_u16(bytes, 2).ok_or_else(bad)?;
    let rcode = flags & 0x000F;
    if rcode == 3 {
        return Ok((Vec::new(), 0)); // NXDOMAIN
    }
    if rcode != 0 {
        return Err(format!("DNS error code {rcode}").into());
    }
    let questions = read_u16(bytes, 4).ok_or_else(bad)?;
    let answers = read_u16(bytes, 6).ok_or_else(bad)?;
    let mut at = 12;
    for _ in 0..questions {
        at = skip_name(bytes, at).ok_or_else(bad)? + 4;
    }
    let mut ips = Vec::new();
    let mut ttl = u32::MAX;
    for _ in 0..answers {
        at = skip_name(bytes, at).ok_or_else(bad)?;
        let record = read_u16(bytes, at).ok_or_else(bad)?;
        let record_ttl = u32::from_be_bytes(bytes.get(at + 4..at + 8).ok_or_else(bad)?.try_into()?);
        let len = read_u16(bytes, at + 8).ok_or_else(bad)? as usize;
        let data = bytes.get(at + 10..at + 10 + len).ok_or_else(bad)?;
        at += 10 + len;
        if record != kind {
            continue; // CNAME and others
        }
        let ip = match (kind, data.len()) {
            (TYPE_A, 4) => IpAddr::V4(Ipv4Addr::new(data[0], data[1], data[2], data[3])),
            (TYPE_AAAA, 16) => IpAddr::V6(Ipv6Addr::from(<[u8; 16]>::try_from(data)?)),
            _ => continue,
        };
        ttl = ttl.min(record_ttl);
        ips.push(ip);
    }
    Ok((ips, ttl))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_queries() {
        let q = build_query("a.bc", TYPE_A).unwrap();
        assert_eq!(&q[12..], &[1, b'a', 2, b'b', b'c', 0, 0, 1, 0, 1]);
        assert!(build_query("a..b", TYPE_A).is_err());
    }

    #[test]
    fn parses_answers_with_cname() {
        let mut answer = vec![0, 0, 0x81, 0x80, 0, 1, 0, 2, 0, 0, 0, 0];
        answer.extend_from_slice(&[1, b'a', 2, b'b', b'c', 0, 0, 1, 0, 1]);
        // CNAME record pointing elsewhere (data: compressed pointer).
        answer.extend_from_slice(&[0xC0, 12, 0, 5, 0, 1, 0, 0, 0, 30, 0, 2, 0xC0, 12]);
        // A record 5.253.86.36, TTL 300.
        answer.extend_from_slice(&[0xC0, 12, 0, 1, 0, 1, 0, 0, 1, 44, 0, 4, 5, 253, 86, 36]);
        let (ips, ttl) = parse_answer(&answer, TYPE_A).unwrap();
        assert_eq!(ips, vec![IpAddr::V4(Ipv4Addr::new(5, 253, 86, 36))]);
        assert_eq!(ttl, 300);
    }

    #[test]
    fn knows_providers() {
        assert_eq!(endpoint_of("quad9").as_deref(), Some("https://9.9.9.9/dns-query"));
        assert_eq!(endpoint_of("https://dns.example/q").as_deref(), Some("https://dns.example/q"));
        assert_eq!(endpoint_of("nope"), None);
    }

    #[tokio::test]
    #[ignore]
    async fn resolves_live() {
        let ips = lookup("https://1.1.1.1/dns-query", "example.com").await.unwrap();
        assert!(!ips.is_empty());
    }
}
