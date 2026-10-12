use crate::events::{AppEvent, EventSink, LogLevel};

#[derive(Clone)]
pub struct Logger {
    sink: EventSink,
}

impl Logger {
    pub fn new(sink: &EventSink) -> Self {
        Self { sink: sink.clone() }
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
        self.sink.emit(AppEvent::Log { level, message });
    }
}
