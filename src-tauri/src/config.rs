use std::sync::{Mutex, OnceLock};
use tonic::transport::{Channel, Endpoint};
use uuid::Uuid;

/// Generated gRPC types/client for the `worker.v1` service (see `proto/worker.proto`).
pub mod pb {
    tonic::include_proto!("worker.v1");
}

use pb::worker_client::WorkerClient;
use pb::*;

static CONFIG: OnceLock<Mutex<Config>> = OnceLock::new();
static CHANNEL: OnceLock<Option<Channel>> = OnceLock::new();

pub fn get_config() -> &'static Mutex<Config> {
    CONFIG.get_or_init(|| Mutex::new(Config::load()))
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
        // !cfg!(debug_assertions) && !self.telemetry_url.is_empty()
        true
    }

    pub fn new_session(&mut self) {
        self.uuid = Uuid::new_v4();
    }
}

/// Lazily-initialised, process-wide gRPC channel to the telemetry worker.
///
/// `connect_lazy` never blocks: the TCP/HTTP2 connection is established on the
/// first RPC and transparently re-established on failure. Returns `None` when
/// telemetry is disabled (debug builds / empty URL) or the URL is invalid, so
/// every caller becomes a cheap no-op in those cases.
fn channel() -> Option<Channel> {
    CHANNEL
        .get_or_init(|| {
            let cfg = get_config().lock().unwrap();
            if !cfg.should_send() {
                return None;
            }
            Endpoint::from_shared(cfg.telemetry_url.to_string())
                .ok()
                .map(|endpoint| endpoint.connect_lazy())
        })
        .clone()
}

/// The shared singleton client. Clone-cheap; call from anywhere.
fn client() -> Option<WorkerClient<Channel>> {
    channel().map(WorkerClient::new)
}

fn client_info() -> ClientInfo {
    let cfg = get_config().lock().unwrap();
    ClientInfo {
        version: cfg.version.to_string(),
        os: cfg.os.to_string(),
        username: cfg.username.clone(),
    }
}

fn session_id() -> String {
    get_config().lock().unwrap().uuid.to_string()
}

pub async fn send_startup_ping() {
    let Some(mut client) = client() else { return };
    let _ = client
        .ping(PingRequest {
            client: Some(client_info()),
        })
        .await;
}

pub async fn send_start_manual(server_addr: String) {
    let Some(mut client) = client() else { return };
    let _ = client
        .manual_start(ManualStartRequest {
            client: Some(client_info()),
            server_addr,
        })
        .await;
}

pub async fn send_start_auto(windivert: bool) {
    let Some(mut client) = client() else { return };
    let _ = client
        .auto_start(AutoStartRequest {
            client: Some(client_info()),
            use_windivert: windivert,
        })
        .await;
}

pub async fn send_join(server_addr: String, nickname: String, protocol_version: i32) {
    let Some(mut client) = client() else { return };
    let _ = client
        .join(JoinRequest {
            client: Some(client_info()),
            session_id: session_id(),
            server_addr,
            nickname,
            protocol_version,
        })
        .await;
}

pub async fn send_protocol_metadata(data: String) {
    let Some(mut client) = client() else { return };
    let _ = client
        .send_protocol_metadata(SendProtocolMetadataRequest {
            session_id: session_id(),
            payload: data,
        })
        .await;
}

pub async fn send_developer_message(message: String) {
    let Some(mut client) = client() else { return };
    let _ = client
        .send_developer_message(SendDeveloperMessageRequest {
            client: Some(client_info()),
            message,
        })
        .await;
}
