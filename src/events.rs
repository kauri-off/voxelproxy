use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Info,
    Success,
    Warn,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WhichClient {
    Primary,
    Secondary,
}

#[derive(Debug, Clone)]
pub enum AppEvent {
    Log { level: LogLevel, message: String },
    SessionStarted,
    SessionEnded,
    ClientStatus { which: WhichClient, online: bool },
    NickName(String),
    ServerAddr(String),
    UpdateProgress { downloaded: u64, total: u64 },
}

/// Cloneable handle the backend uses to push events to the UI.
#[derive(Clone)]
pub struct EventSink(UnboundedSender<AppEvent>);

impl EventSink {
    pub fn channel() -> (Self, UnboundedReceiver<AppEvent>) {
        let (tx, rx) = unbounded();
        (Self(tx), rx)
    }

    pub fn emit(&self, event: AppEvent) {
        let _ = self.0.unbounded_send(event);
    }

    pub fn client_status(&self, which: WhichClient, online: bool) {
        self.emit(AppEvent::ClientStatus { which, online });
    }
}
