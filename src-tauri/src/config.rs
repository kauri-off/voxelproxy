use serde_json::json;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use uuid::Uuid;

static CONFIG: OnceLock<Mutex<Config>> = OnceLock::new();
static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

pub fn get_config() -> &'static Mutex<Config> {
    CONFIG.get_or_init(|| Mutex::new(Config::load()))
}

fn get_client() -> &'static reqwest::Client {
    CLIENT.get_or_init(|| reqwest::Client::new())
}

pub struct Config {
    pub telemetry_url: &'static str,
    pub version: &'static str,
    pub os: &'static str,
    pub username: String,
    pub uuid: Uuid,
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
            uuid: Uuid::new_v4(),
        }
    }

    pub fn should_send(&self) -> bool {
        !cfg!(debug_assertions) && !self.telemetry_url.is_empty()
    }

    pub fn new_session(&mut self) {
        self.uuid = Uuid::new_v4();
    }
}

async fn post(path: &str, version: &str, payload: serde_json::Value) {
    let (should_send, url) = {
        let cfg = get_config().lock().unwrap();
        (
            cfg.should_send(),
            format!("{}/{}/{}", cfg.telemetry_url, version, path),
        )
    };

    if !should_send {
        return;
    }

    let _ = get_client()
        .post(&url)
        .json(&payload)
        .timeout(Duration::from_secs(5))
        .send()
        .await;
}

pub async fn send_startup_ping() {
    let (version, os, username) = {
        let cfg = get_config().lock().unwrap();
        (cfg.version, cfg.os, cfg.username.clone())
    };

    post(
        "ping",
        "v1",
        json!({ "version": version, "os": os, "username": username }),
    )
    .await;
}

pub async fn send_start_manual(server_addr: String) {
    let username = get_config().lock().unwrap().username.clone();
    post(
        "start_manual",
        "v1",
        json!({ "server_addr": server_addr, "username": username }),
    )
    .await;
}

pub async fn send_start_auto(windivert: bool) {
    let username = get_config().lock().unwrap().username.clone();
    post(
        "start_auto",
        "v1",
        json!({ "windivert": windivert, "username": username }),
    )
    .await;
}

pub async fn send_join(server_addr: String, nickname: String, protocol_version: i32) {
    let (username, uuid) = {
        let mut cfg = get_config().lock().unwrap();
        cfg.new_session();
        (cfg.username.clone(), cfg.uuid.clone())
    };
    post(
        "joined",
        "v2",
        json!({ "server_addr": server_addr, "username": username, "nickname": nickname, "uuid": uuid, "protocol_version": protocol_version }),
    )
    .await;
}

pub async fn send_protocol_metadata(data: String) {
    let uuid = get_config().lock().unwrap().uuid.clone();
    post("data", "v1", json!({ "uuid": uuid, "data": data })).await;
}
