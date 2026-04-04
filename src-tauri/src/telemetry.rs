use lazy_static::lazy_static;
use serde_json::json;
use std::time::Duration;

lazy_static! {
    pub static ref CONFIG: Config = Config::load();
}

pub struct Config {
    pub telemetry_url: &'static str,
    pub version: &'static str,
    pub os: &'static str,
    pub username: String,
}

impl Config {
    fn load() -> Self {
        Self {
            telemetry_url: env!("TELEMETRY_URL"),
            version: env!("CARGO_PKG_VERSION"),
            os: std::env::consts::OS,
            username: std::env::var("USERNAME")
                .or_else(|_| std::env::var("USER"))
                .unwrap_or_else(|_| "unknown".to_string()),
        }
    }

    pub fn should_send(&self) -> bool {
        !cfg!(debug_assertions) && !self.telemetry_url.is_empty()
    }
}

async fn post_telemetry(path: &str, payload: serde_json::Value) {
    if !CONFIG.should_send() {
        return;
    }

    let url = format!("{}/v1/{}", CONFIG.telemetry_url, path);
    let client = reqwest::Client::new();

    let _ = client
        .post(url)
        .json(&payload)
        .timeout(Duration::from_secs(5))
        .send()
        .await;
}

pub async fn send_startup_ping() {
    post_telemetry(
        "ping",
        json!({
            "version": CONFIG.version,
            "os": CONFIG.os,
            "username": CONFIG.username,
        }),
    )
    .await;
}

pub async fn send_start_manual(server_addr: String) {
    post_telemetry(
        "start_manual",
        json!({
            "server_addr": server_addr,
            "username": CONFIG.username,
        }),
    )
    .await;
}

pub async fn send_start_auto(windivert: bool) {
    post_telemetry(
        "start_auto",
        json!({
            "windivert": windivert,
            "username": CONFIG.username,
        }),
    )
    .await;
}
