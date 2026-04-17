use serde::Serialize;
use tauri::{AppHandle, Emitter};

#[derive(Serialize, Clone, Debug)]
pub struct LogEntry {
    pub level: String,
    pub message: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct ClientStatusPayload {
    pub which: String,
    pub online: bool,
}

#[derive(Clone)]
pub struct Logger {
    app: AppHandle,
}

impl Logger {
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }

    pub fn info(&self, msg: impl Into<String>) {
        self.send("info", msg.into());
    }

    pub fn success(&self, msg: impl Into<String>) {
        self.send("success", msg.into());
    }

    pub fn warn(&self, msg: impl Into<String>) {
        self.send("warn", msg.into());
    }

    pub fn error(&self, msg: impl Into<String>) {
        self.send("error", msg.into());
    }

    pub fn client_status(&self, which: &str, online: bool) {
        self.app
            .emit(
                "client-status",
                ClientStatusPayload {
                    which: which.to_string(),
                    online,
                },
            )
            .ok();
    }

    pub fn nick_name(&self, nickname: &str) {
        self.app.emit("nick-name", nickname).ok();
    }

    fn send(&self, level: &str, message: String) {
        let entry = LogEntry {
            level: level.to_string(),
            message,
        };
        self.app.emit("proxy-log", entry).ok();
    }
}
