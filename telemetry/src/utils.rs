use axum::http::HeaderMap;
use chrono::{FixedOffset, Utc};
use std::net::SocketAddr;

pub fn now() -> String {
    let offset = FixedOffset::east_opt(3 * 3600).unwrap();
    Utc::now().with_timezone(&offset).to_rfc3339()
}

pub fn extract_ip(headers: &HeaderMap, addr: SocketAddr) -> String {
    headers
        .get("x-forwarded-for")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.split(',').next())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| addr.ip().to_string())
}
