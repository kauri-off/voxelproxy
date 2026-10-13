use std::sync::{Mutex, OnceLock};
use tonic::transport::{Channel, ClientTlsConfig, Endpoint};
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
    pub install_id: String,
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
            install_id: crate::prefs::install_id(),
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

/// Lazy telemetry channel; `None` when disabled. webpki roots are required for `https://`.
fn channel() -> Option<Channel> {
    CHANNEL
        .get_or_init(|| {
            let cfg = get_config().lock().unwrap();
            if !cfg.should_send() {
                return None;
            }
            let endpoint = match Endpoint::from_shared(cfg.telemetry_url.to_string()) {
                Ok(endpoint) => endpoint,
                Err(e) => {
                    eprintln!(
                        "telemetry: invalid TELEMETRY_URL {:?}: {e}",
                        cfg.telemetry_url
                    );
                    return None;
                }
            };
            let endpoint = match endpoint.tls_config(ClientTlsConfig::new().with_webpki_roots()) {
                Ok(endpoint) => endpoint,
                Err(e) => {
                    eprintln!("telemetry: failed to configure TLS: {e}");
                    return None;
                }
            };
            Some(endpoint.connect_lazy())
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
        install_id: cfg.install_id.clone(),
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

pub async fn send_protocol_metadata(data: String, custom: bool) {
    let Some(mut client) = client() else { return };
    let _ = client
        .send_protocol_metadata(SendProtocolMetadataRequest {
            session_id: session_id(),
            payload: data,
            custom,
        })
        .await;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatMessage {
    pub id: String,
    pub from_developer: bool,
    pub text: String,
    pub created_at_ms: i64,
}

impl From<DeveloperMessage> for ChatMessage {
    fn from(m: DeveloperMessage) -> Self {
        Self {
            from_developer: m.author() == MessageAuthor::Developer,
            id: m.id,
            text: m.text,
            created_at_ms: m.created_at_ms,
        }
    }
}

pub fn chat_available() -> bool {
    client().is_some()
}

fn chat_client() -> anyhow::Result<WorkerClient<Channel>> {
    client().ok_or_else(|| anyhow::anyhow!("Связь с разработчиком недоступна в этой сборке"))
}

pub async fn send_developer_message(message: String) -> anyhow::Result<Option<ChatMessage>> {
    let response = chat_client()?
        .send_developer_message(SendDeveloperMessageRequest {
            client: Some(client_info()),
            message,
        })
        .await?
        .into_inner();
    Ok(response.message.map(ChatMessage::from))
}

pub async fn list_developer_messages() -> anyhow::Result<Vec<ChatMessage>> {
    let response = chat_client()?
        .list_developer_messages(ListDeveloperMessagesRequest {
            client: Some(client_info()),
        })
        .await?
        .into_inner();
    let mut messages: Vec<ChatMessage> = response
        .messages
        .into_iter()
        .map(ChatMessage::from)
        .collect();
    messages.sort_by_key(|m| m.created_at_ms);
    Ok(messages)
}
