use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Info,
    Success,
    Warn,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientId {
    Primary,
    Secondary,
}

impl ClientId {
    pub fn opposite(self) -> ClientId {
        match self {
            ClientId::Primary => ClientId::Secondary,
            ClientId::Secondary => ClientId::Primary,
        }
    }

    pub fn index(self) -> usize {
        match self {
            ClientId::Primary => 0,
            ClientId::Secondary => 1,
        }
    }
}

#[derive(Debug, Clone)]
pub enum AppEvent {
    Log { level: LogLevel, message: String },
    SessionStarted,
    SessionEnded,
    ClientStatus { which: ClientId, online: bool },
    NickName(String),
    ServerAddr(String),
    UpdateProgress { downloaded: u64, total: u64 },
    UpdateVerifying { waited_secs: u64 },
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

    pub fn client_status(&self, which: ClientId, online: bool) {
        self.emit(AppEvent::ClientStatus { which, online });
    }
}
