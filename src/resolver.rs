use std::net::{IpAddr, SocketAddr};
use trust_dns_resolver::{config::*, TokioAsyncResolver};

fn parse_host_port(input: &str, default_port: u16) -> (String, u16) {
    if let Some((host, port)) = input.rsplit_once(':') {
        // Проверка: может ли `port` быть числом, а `host` — не IPv6
        if port.parse::<u16>().is_ok() && !host.contains(']') && !host.contains(':') {
            return (host.to_string(), port.parse().unwrap());
        }
    }
    (input.to_string(), default_port)
}

pub async fn resolve_host_port(
    input: &str,
    default_port: u16,
    service: &str,
    protocol: &str,
) -> Option<SocketAddr> {
    let (host, port) = parse_host_port(input, default_port);

    if let Ok(ip) = host.parse::<IpAddr>() {
        return Some(SocketAddr::new(ip, port));
    }

    let resolver = TokioAsyncResolver::tokio(ResolverConfig::default(), ResolverOpts::default());

    // SRV-запрос
    let srv_name = format!("_{}._{}.{}", service, protocol, host);
    if let Ok(srv_lookup) = resolver.srv_lookup(&srv_name).await {
        if let Some(record) = srv_lookup.iter().next() {
            let target_ip = resolver
                .lookup_ip(record.target().to_utf8())
                .await
                .ok()?
                .iter()
                .next()?;
            return Some(SocketAddr::new(target_ip, record.port()));
        }
    }

    // Fallback: обычный A/AAAA
    let ip_response = resolver.lookup_ip(&host).await.ok()?;
    let ip = ip_response.iter().next()?;
    Some(SocketAddr::new(ip, port))
}
