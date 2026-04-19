use tauri::AppHandle;
use tauri_specta::Event;

use crate::events::{LogLevel, ProxyLogEvent};

#[derive(Clone)]
pub struct Logger<'a> {
    app: &'a AppHandle,
}

impl<'a> Logger<'a> {
    pub fn new(app: &'a AppHandle) -> Self {
        Self { app }
    }

    pub fn info(&self, msg: impl Into<String>) {
        self.send(LogLevel::Info, msg.into());
    }

    pub fn success(&self, msg: impl Into<String>) {
        self.send(LogLevel::Success, msg.into());
    }

    pub fn warn(&self, msg: impl Into<String>) {
        self.send(LogLevel::Warn, msg.into());
    }

    pub fn error(&self, msg: impl Into<String>) {
        self.send(LogLevel::Error, msg.into());
    }

    fn send(&self, level: LogLevel, message: String) {
        ProxyLogEvent { level, message }.emit(self.app).ok();
    }
}
