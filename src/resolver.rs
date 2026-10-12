use std::net::{IpAddr, SocketAddr};

use hickory_resolver::{
    Resolver,
    config::{GOOGLE, ResolverConfig},
    net::runtime::TokioRuntimeProvider,
    proto::rr::RData,
};

fn parse_host_port(input: &str, default_port: u16) -> (String, u16) {
    if let Some((host, port_str)) = input.rsplit_once(':') {
        if let Ok(port) = port_str.parse::<u16>() {
            if !host.contains(']') && !host.contains(':') {
                return (host.to_string(), port);
            }
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

    let resolver = Resolver::builder_with_config(
        ResolverConfig::udp_and_tcp(&GOOGLE),
        TokioRuntimeProvider::default(),
    )
    .build()
    .ok()?;

    // SRV lookup
    let srv_name = format!("_{}._{}.{}", service, protocol, host);
    if let Ok(srv_lookup) = resolver.srv_lookup(srv_name.as_str()).await {
        let srv = srv_lookup.answers().iter().find_map(|r| match &r.data {
            RData::SRV(srv) => Some(srv.clone()),
            _ => None,
        });
        if let Some(srv) = srv {
            let target_ip = resolver
                .lookup_ip(srv.target.to_utf8().as_str())
                .await
                .ok()?
                .iter()
                .next()?;
            return Some(SocketAddr::new(target_ip, srv.port));
        }
    }

    // Fallback: standard A/AAAA lookup
    let ip_response = resolver.lookup_ip(host.as_str()).await.ok()?;
    let ip = ip_response.iter().next()?;
    Some(SocketAddr::new(ip, port))
}
