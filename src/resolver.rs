//! Mirrors the vanilla client's `ServerNameResolver`, including the host it puts in the handshake.

use std::{
    net::{IpAddr, SocketAddr},
    time::Duration,
};

use hickory_resolver::{
    TokioResolver,
    proto::rr::{Name, RData, RecordType},
};

const DEFAULT_PORT: u16 = 25565;

/// Where to connect and what to send in the handshake, as the vanilla client would.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerTarget {
    pub host: String,
    pub port: u16,
    pub addr: SocketAddr,
}

/// Resolves an address typed by the user, like the multiplayer screen does.
pub async fn resolve_server(input: &str) -> Option<ServerTarget> {
    let (host, port) = parse_address(input)?;
    resolve_host_port(&host, port).await
}

/// Resolves an already split host and port, e.g. taken from a client's handshake.
pub async fn resolve_host_port(host: &str, port: u16) -> Option<ServerTarget> {
    // Modded clients append markers like "\0FML3\0" to the handshake host.
    let host = to_ascii(host.split('\0').next().unwrap_or(host));
    if host.is_empty() {
        return None;
    }
    // Vanilla has no fallback: an SRV target that fails to resolve fails the connection.
    match srv_redirect(&host, port).await {
        Some((target, target_port)) => resolve_system(&target, target_port).await,
        None => resolve_system(&host, port).await,
    }
}

/// Guava `HostAndPort.fromString(..).withDefaultPort(25565)`.
fn parse_address(input: &str) -> Option<(String, u16)> {
    let (host, port) = if let Some(rest) = input.strip_prefix('[') {
        let (host, after) = rest.split_once(']')?;
        match after {
            "" => (host, None),
            _ => (host, Some(after.strip_prefix(':')?)),
        }
    } else {
        match input.split_once(':') {
            Some((host, port)) if !port.contains(':') => (host, Some(port)),
            _ => (input, None),
        }
    };
    let port = match port {
        None | Some("") => DEFAULT_PORT,
        Some(p) if p.bytes().all(|b| b.is_ascii_digit()) => p.parse().ok()?,
        Some(_) => return None,
    };
    (!host.is_empty()).then(|| (host.to_string(), port))
}

/// `java.net.IDN.toASCII`: only labels with non-ASCII characters are converted.
fn to_ascii(host: &str) -> String {
    if host.is_ascii() {
        return host.to_string();
    }
    host.split(['.', '\u{3002}', '\u{ff0e}', '\u{ff61}'])
        .map(|label| {
            if label.is_ascii() {
                label.to_string()
            } else {
                idna::domain_to_ascii(label).unwrap_or_default()
            }
        })
        .collect::<Vec<_>>()
        .join(".")
}

/// `InetAddress.getByName` plus `InetSocketAddress.getHostName`, with Java's IPv4-first order.
async fn resolve_system(host: &str, port: u16) -> Option<ServerTarget> {
    let bare = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host);
    if let Ok(ip) = bare.parse::<IpAddr>() {
        let ip = ip.to_canonical();
        return Some(ServerTarget {
            host: reverse_name(ip).await,
            port,
            addr: SocketAddr::new(ip, port),
        });
    }
    let addrs: Vec<SocketAddr> = tokio::net::lookup_host((bare, port)).await.ok()?.collect();
    let addr = addrs
        .iter()
        .find(|a| a.is_ipv4())
        .or_else(|| addrs.first())
        .copied()?;
    Some(ServerTarget {
        host: host.to_string(),
        port,
        addr,
    })
}

/// Java's reverse lookup for literal addresses: the PTR name if it resolves back, else the address.
async fn reverse_name(ip: IpAddr) -> String {
    let literal = java_host_address(ip);
    let Some(resolver) = system_resolver() else {
        return literal;
    };
    let Ok(lookup) = resolver.lookup(Name::from(ip), RecordType::PTR).await else {
        return literal;
    };
    let Some(name) = lookup.answers().iter().find_map(|r| match &r.data {
        RData::PTR(ptr) => Some(ptr.0.to_ascii()),
        _ => None,
    }) else {
        return literal;
    };
    let name = name.trim_end_matches('.').to_string();
    let confirmed = tokio::net::lookup_host((name.as_str(), 0))
        .await
        .is_ok_and(|mut addrs| addrs.any(|a| a.ip().to_canonical() == ip));
    if confirmed { name } else { literal }
}

/// `InetAddress.getHostAddress`: IPv6 is printed without zero compression.
fn java_host_address(ip: IpAddr) -> String {
    match ip {
        IpAddr::V4(v4) => v4.to_string(),
        IpAddr::V6(v6) => v6
            .segments()
            .iter()
            .map(|s| format!("{s:x}"))
            .collect::<Vec<_>>()
            .join(":"),
    }
}

/// `ServerRedirectHandler.createDnsSrvRedirectHandler`: first SRV record, only for port 25565.
async fn srv_redirect(host: &str, port: u16) -> Option<(String, u16)> {
    if port != DEFAULT_PORT || host.parse::<IpAddr>().is_ok() {
        return None;
    }
    let resolver = system_resolver()?;
    let lookup = resolver
        .lookup(format!("_minecraft._tcp.{host}"), RecordType::SRV)
        .await
        .ok()?;
    lookup.answers().iter().find_map(|r| match &r.data {
        RData::SRV(srv) => Some((srv.target.to_ascii(), srv.port)),
        _ => None,
    })
}

/// Reads the system DNS config on every call, like JNDI's `dns:` provider; no fallback servers.
fn system_resolver() -> Option<TokioResolver> {
    let mut builder = TokioResolver::builder_tokio().ok()?;
    let opts = builder.options_mut();
    opts.timeout = Duration::from_secs(1);
    opts.attempts = 1;
    builder.build().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_like_guava_host_and_port() {
        let parsed = |s: &str| parse_address(s);
        assert_eq!(
            parsed("mc.example.net"),
            Some(("mc.example.net".into(), 25565))
        );
        assert_eq!(
            parsed("mc.example.net:25570"),
            Some(("mc.example.net".into(), 25570))
        );
        assert_eq!(
            parsed("mc.example.net:"),
            Some(("mc.example.net".into(), 25565))
        );
        assert_eq!(parsed("[::1]:25570"), Some(("::1".into(), 25570)));
        assert_eq!(parsed("[::1]"), Some(("::1".into(), 25565)));
        assert_eq!(parsed("::1"), Some(("::1".into(), 25565)));
        assert_eq!(parsed("mc.example.net:+1"), None);
        assert_eq!(parsed("mc.example.net:70000"), None);
        assert_eq!(parsed(":25565"), None);
        assert_eq!(parsed(""), None);
    }

    #[test]
    fn idn_keeps_ascii_labels_untouched() {
        assert_eq!(to_ascii("Mc.Example.NET."), "Mc.Example.NET.");
        assert_eq!(to_ascii("сервер.рф"), "xn--b1afb6bcb.xn--p1ai");
        assert_eq!(to_ascii("play.сервер.рф"), "play.xn--b1afb6bcb.xn--p1ai");
    }

    #[test]
    fn ipv6_is_printed_like_java() {
        let ip: IpAddr = "2001:db8::1".parse().unwrap();
        assert_eq!(java_host_address(ip), "2001:db8:0:0:0:0:0:1");
    }

    #[tokio::test]
    async fn srv_only_for_default_port() {
        assert_eq!(srv_redirect("mc.example.net", 25570).await, None);
        assert_eq!(srv_redirect("10.1.2.3", 25565).await, None);
    }

    #[tokio::test]
    async fn hostname_is_sent_as_typed() {
        let target = resolve_server("localhost:25570").await.unwrap();
        assert_eq!(target.host, "localhost");
        assert_eq!(target.port, 25570);
        assert!(target.addr.ip().is_loopback());
        assert_eq!(target.addr.port(), 25570);
    }

    #[tokio::test]
    async fn literal_ip_keeps_port_and_strips_mod_marker() {
        let target = resolve_host_port("127.0.0.1\0FML3\0", 25570).await.unwrap();
        assert_eq!(target.addr, "127.0.0.1:25570".parse().unwrap());
        assert_eq!(target.port, 25570);
    }
}
