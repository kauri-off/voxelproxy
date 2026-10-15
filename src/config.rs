use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use tokio::{
    runtime::Handle,
    sync::mpsc::{self, UnboundedSender},
    task::JoinHandle,
};
use tonic::transport::{Channel, ClientTlsConfig, Endpoint};
use uuid::Uuid;

/// Generated gRPC types/client for the `worker.v1` service (see `proto/worker.proto`).
pub mod pb {
    tonic::include_proto!("worker.v1");
}

use pb::worker_client::WorkerClient;
use pb::*;

use crate::{diag, events::ClientId};

static CONFIG: OnceLock<Mutex<Config>> = OnceLock::new();
static CHANNEL: OnceLock<Option<Channel>> = OnceLock::new();
static REPORTER: Mutex<Option<Reporter>> = Mutex::new(None);

pub fn get_config() -> &'static Mutex<Config> {
    CONFIG.get_or_init(|| Mutex::new(Config::load()))
}

pub struct Config {
    pub telemetry_url: &'static str,
    pub version: &'static str,
    pub os: &'static str,
    pub username: String,
    pub install_id: String,
    /// Identifies this run of the app.
    pub app_id: Uuid,
    pub started: Instant,
    pub session: Option<Session>,
}

/// The running proxy session, see `begin_session`.
#[derive(Clone, Copy)]
pub struct Session {
    pub id: Uuid,
    pub started: Instant,
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
            app_id: Uuid::new_v4(),
            started: Instant::now(),
            session: None,
        }
    }

    pub fn should_send(&self) -> bool {
        !cfg!(debug_assertions) && !self.telemetry_url.is_empty()
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
                    diag::error("rpc", format!("invalid TELEMETRY_URL: {e:#}"));
                    return None;
                }
            };
            let endpoint = match endpoint.tls_config(ClientTlsConfig::new().with_webpki_roots()) {
                Ok(endpoint) => endpoint,
                Err(e) => {
                    eprintln!("telemetry: failed to configure TLS: {e}");
                    diag::error("rpc", format!("failed to configure TLS: {e:#}"));
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

/// Awaits an RPC and records its outcome and timing in the diagnostic log.
async fn traced<T>(
    name: &str,
    call: impl Future<Output = Result<tonic::Response<T>, tonic::Status>>,
) -> Result<T, tonic::Status> {
    let started = Instant::now();
    let result = call.await;
    let elapsed = started.elapsed();
    match &result {
        Ok(_) => diag::debug("rpc", format!("{name} ok in {elapsed:?}")),
        Err(status) => diag::warn(
            "rpc",
            format!(
                "{name} failed after {elapsed:?}: code={:?} message={:?}",
                status.code(),
                status.message()
            ),
        ),
    }
    result.map(tonic::Response::into_inner)
}

/// A lifecycle report; the reporter sends them one by one so the server gets them in order.
enum Report {
    AppStarted(AppStartedRequest),
    AppClosed(AppClosedRequest),
    SessionStarted(SessionStartedRequest),
    SessionJoined(SessionJoinedRequest),
    ActiveClientSwitched(ActiveClientSwitchedRequest),
    SessionEnded(SessionEndedRequest),
    ChatMessageSent(ChatMessageSentRequest),
}

struct Reporter {
    tx: UnboundedSender<Report>,
    task: JoinHandle<()>,
}

/// Starts sending reports on `rt`; reports made before are dropped.
pub fn start_reporter(rt: &Handle) {
    let Some(mut client) = client() else { return };
    let (tx, mut rx) = mpsc::unbounded_channel();
    let task = rt.spawn(async move {
        while let Some(report) = rx.recv().await {
            let _ = match report {
                Report::AppStarted(r) => {
                    traced("AppStarted", client.app_started(r)).await.map(drop)
                }
                Report::AppClosed(r) => traced("AppClosed", client.app_closed(r)).await.map(drop),
                Report::SessionStarted(r) => traced("SessionStarted", client.session_started(r))
                    .await
                    .map(drop),
                Report::SessionJoined(r) => traced("SessionJoined", client.session_joined(r))
                    .await
                    .map(drop),
                Report::ActiveClientSwitched(r) => {
                    traced("ActiveClientSwitched", client.active_client_switched(r))
                        .await
                        .map(drop)
                }
                Report::SessionEnded(r) => traced("SessionEnded", client.session_ended(r))
                    .await
                    .map(drop),
                Report::ChatMessageSent(r) => {
                    traced("ChatMessageSent", client.chat_message_sent(r))
                        .await
                        .map(drop)
                }
            };
        }
    });
    *REPORTER.lock().unwrap() = Some(Reporter { tx, task });
}

fn report(report: Report) {
    if let Some(reporter) = REPORTER.lock().unwrap().as_ref() {
        let _ = reporter.tx.send(report);
    }
}

/// Sends what is still queued, giving up after `limit`; later reports are dropped.
pub async fn flush_reports(limit: Duration) {
    let Some(Reporter { tx, task }) = REPORTER.lock().unwrap().take() else {
        return;
    };
    drop(tx);
    if tokio::time::timeout(limit, task).await.is_err() {
        diag::warn("rpc", format!("reports not sent within {limit:?}"));
    }
}

fn millis(d: Duration) -> i64 {
    d.as_millis().try_into().unwrap_or(i64::MAX)
}

fn app_id() -> String {
    get_config().lock().unwrap().app_id.to_string()
}

fn current_session() -> Option<Session> {
    get_config().lock().unwrap().session
}

pub fn report_app_started() {
    report(Report::AppStarted(AppStartedRequest {
        client: Some(client_info()),
        app_id: app_id(),
    }));
}

pub fn report_app_closed() {
    let uptime = get_config().lock().unwrap().started.elapsed();
    report(Report::AppClosed(AppClosedRequest {
        app_id: app_id(),
        uptime_ms: millis(uptime),
    }));
}

#[derive(Debug, Clone)]
pub enum SessionKind {
    Auto { use_windivert: bool },
    Manual { server_addr: String },
}

/// Makes a new session current; joins, chat and switches are reported under it until it ends.
pub fn begin_session(kind: &SessionKind) -> Session {
    let session = Session {
        id: Uuid::new_v4(),
        started: Instant::now(),
    };
    get_config().lock().unwrap().session = Some(session);
    diag::info("rpc", format!("session_id={} {kind:?}", session.id));
    let mut request = SessionStartedRequest {
        client: Some(client_info()),
        app_id: app_id(),
        session_id: session.id.to_string(),
        ..Default::default()
    };
    match kind {
        SessionKind::Auto { use_windivert } => {
            request.set_mode(SessionMode::Auto);
            request.use_windivert = *use_windivert;
        }
        SessionKind::Manual { server_addr } => {
            request.set_mode(SessionMode::Manual);
            request.server_addr = server_addr.clone();
        }
    }
    report(Report::SessionStarted(request));
    session
}

pub enum SessionEnd {
    Finished,
    Stopped,
    Failed(String),
    Panicked,
}

pub fn end_session(session: Session, end: SessionEnd) {
    {
        let mut cfg = get_config().lock().unwrap();
        if cfg.session.is_some_and(|s| s.id == session.id) {
            cfg.session = None;
        }
    }
    let mut request = SessionEndedRequest {
        session_id: session.id.to_string(),
        duration_ms: millis(session.started.elapsed()),
        ..Default::default()
    };
    let reason = match end {
        SessionEnd::Finished => SessionEndReason::Finished,
        SessionEnd::Stopped => SessionEndReason::Stopped,
        SessionEnd::Failed(error) => {
            request.error = error;
            SessionEndReason::Failed
        }
        SessionEnd::Panicked => SessionEndReason::Panicked,
    };
    request.set_reason(reason);
    report(Report::SessionEnded(request));
}

pub fn report_join(server_addr: String, nickname: String, protocol_version: i32) {
    let Some(session) = current_session() else {
        return;
    };
    report(Report::SessionJoined(SessionJoinedRequest {
        client: Some(client_info()),
        session_id: session.id.to_string(),
        server_addr,
        nickname,
        protocol_version,
    }));
}

pub fn report_active_client_switched(active: ClientId, replayed_packets: usize) {
    let Some(session) = current_session() else {
        return;
    };
    let mut request = ActiveClientSwitchedRequest {
        session_id: session.id.to_string(),
        replayed_packets: replayed_packets.try_into().unwrap_or(u32::MAX),
        session_uptime_ms: millis(session.started.elapsed()),
        ..Default::default()
    };
    request.set_active(match active {
        ClientId::Primary => ClientRole::Primary,
        ClientId::Secondary => ClientRole::Secondary,
    });
    report(Report::ActiveClientSwitched(request));
}

pub fn report_chat(text: String, is_command: bool) {
    let Some(session) = current_session() else {
        return;
    };
    report(Report::ChatMessageSent(ChatMessageSentRequest {
        session_id: session.id.to_string(),
        text,
        is_command,
    }));
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatMessage {
    pub id: String,
    pub from_developer: bool,
    pub text: String,
    pub created_at_ms: i64,
    pub has_log: bool,
}

impl From<DeveloperMessage> for ChatMessage {
    fn from(m: DeveloperMessage) -> Self {
        Self {
            from_developer: m.author() == MessageAuthor::Developer,
            id: m.id,
            text: m.text,
            created_at_ms: m.created_at_ms,
            has_log: m.has_diagnostic_log,
        }
    }
}

pub fn chat_available() -> bool {
    client().is_some()
}

fn chat_client() -> anyhow::Result<WorkerClient<Channel>> {
    client().ok_or_else(|| anyhow::anyhow!("Связь с разработчиком недоступна в этой сборке"))
}

/// The typed text goes in `message`; the hidden log only ever in `diagnostic_log`.
fn developer_message_request(
    client: ClientInfo,
    message: String,
    log: Option<&str>,
) -> SendDeveloperMessageRequest {
    let mut request = SendDeveloperMessageRequest {
        client: Some(client),
        message,
        ..Default::default()
    };
    if let Some(log) = log {
        match diag::gzip(log) {
            Ok(packed) => {
                request.diagnostic_log = packed;
                request.set_diagnostic_log_encoding(DiagnosticLogEncoding::Gzip);
            }
            Err(_) => {
                request.diagnostic_log = log.as_bytes().to_vec();
                request.set_diagnostic_log_encoding(DiagnosticLogEncoding::Plain);
            }
        }
    }
    request
}

pub async fn send_developer_message(
    message: String,
    attach_log: bool,
) -> anyhow::Result<Option<ChatMessage>> {
    let mut client = chat_client()?;
    let log = attach_log.then(diag::export);
    let request = developer_message_request(client_info(), message, log.as_deref());
    diag::info(
        "support",
        format!(
            "sending message: text_chars={} log_plain_bytes={} log_sent_bytes={} encoding={:?}",
            request.message.chars().count(),
            log.as_ref().map_or(0, String::len),
            request.diagnostic_log.len(),
            request.diagnostic_log_encoding()
        ),
    );
    let response = traced(
        "SendDeveloperMessage",
        client.send_developer_message(request),
    )
    .await?;
    Ok(response.message.map(ChatMessage::from))
}

pub async fn list_developer_messages() -> anyhow::Result<Vec<ChatMessage>> {
    let mut client = chat_client()?;
    let request = ListDeveloperMessagesRequest {
        client: Some(client_info()),
    };
    let response = traced(
        "ListDeveloperMessages",
        client.list_developer_messages(request),
    )
    .await?;
    let mut messages: Vec<ChatMessage> = response
        .messages
        .into_iter()
        .map(ChatMessage::from)
        .collect();
    messages.sort_by_key(|m| m.created_at_ms);
    Ok(messages)
}

pub struct LatestRelease {
    pub version: String,
    pub download_url: String,
}

pub async fn latest_release() -> anyhow::Result<LatestRelease> {
    let mut client = client().ok_or_else(|| anyhow::anyhow!("сервер не настроен"))?;
    let request = GetLatestReleaseRequest {
        client: Some(client_info()),
    };
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        traced("GetLatestRelease", client.get_latest_release(request)),
    )
    .await
    .map_err(|_| {
        diag::warn("rpc", "GetLatestRelease timed out after 5s");
        anyhow::anyhow!("таймаут")
    })?
    .map_err(|status| anyhow::anyhow!("{}", status.message()))?;
    Ok(LatestRelease {
        version: response.version,
        download_url: response.download_url,
    })
}

#[cfg(test)]
mod tests {
    use std::io::Read;

    use super::*;

    fn info() -> ClientInfo {
        ClientInfo {
            version: "1.0.0".into(),
            os: "linux".into(),
            username: "u".into(),
            install_id: "id".into(),
        }
    }

    #[test]
    fn log_goes_to_its_own_field_not_the_text() {
        let log = "=== diagnostic ===\nsecret-ish detail line\n";
        let request = developer_message_request(info(), "Не работает".into(), Some(log));
        assert_eq!(request.message, "Не работает");
        assert!(!request.message.contains("diagnostic"));
        assert_eq!(
            request.diagnostic_log_encoding(),
            DiagnosticLogEncoding::Gzip
        );
        let mut unpacked = String::new();
        flate2::read::GzDecoder::new(request.diagnostic_log.as_slice())
            .read_to_string(&mut unpacked)
            .unwrap();
        assert_eq!(unpacked, log);
    }

    #[test]
    fn no_log_leaves_fields_empty() {
        let request = developer_message_request(info(), "hi".into(), None);
        assert_eq!(request.message, "hi");
        assert!(request.diagnostic_log.is_empty());
        assert_eq!(
            request.diagnostic_log_encoding(),
            DiagnosticLogEncoding::Unspecified
        );
    }

    #[test]
    fn full_size_log_fits_grpc_limit() {
        let log: String = (0..)
            .map(|i| format!("line {i} {}\n", uuid::Uuid::new_v4()))
            .take_while({
                let mut total = 0;
                move |l: &String| {
                    total += l.len();
                    total <= diag::CAPACITY_BYTES + 64 * 1024
                }
            })
            .collect();
        let request = developer_message_request(info(), "x".into(), Some(&log));
        assert!(prost::Message::encoded_len(&request) < 4 * 1024 * 1024);
    }
}
