use serde::Serialize;
use specta::Type;
use tauri_specta::Event;

#[derive(Serialize, Clone, Type, Event)]
pub enum LogLevel {
    Info,
    Success,
    Warn,
    Error,
}

#[derive(Serialize, Clone, Type, Event)]
pub struct ProxyLogEvent {
    pub level: LogLevel,
    pub message: String,
}

#[derive(Serialize, Clone, Type, Event)]
pub struct SessionStartedEvent;

#[derive(Serialize, Clone, Type, Event)]
pub struct SessionEndedEvent;

#[derive(Serialize, Clone, Type, Event)]
pub enum WhichClient {
    Primary,
    Secondary,
}

#[derive(Serialize, Clone, Type, Event)]
pub struct ClientStatusEvent {
    pub which: WhichClient,
    pub online: bool,
}

#[derive(Serialize, Clone, Type, Event)]
pub struct NickNameEvent(pub String);
