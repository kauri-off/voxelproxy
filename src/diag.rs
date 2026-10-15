//! Hidden in-memory diagnostic log, attached to support messages on request.

use std::{
    borrow::Cow,
    collections::VecDeque,
    fmt::Write as _,
    io::Write as _,
    sync::{LazyLock, Mutex, TryLockError},
    time::{Duration, Instant},
};

use flate2::{Compression, write::GzEncoder};

/// Plain-text cap: hours of session events, about 100-200 KiB once gzipped.
pub const CAPACITY_BYTES: usize = 1024 * 1024;
const MAX_LINE_BYTES: usize = 8 * 1024;
const MAX_PANIC_BYTES: usize = 32 * 1024;
const REDACTED: &str = "[redacted]";
const SECRET_KEYS: &[&str] = &[
    "authorization",
    "bearer",
    "token",
    "password",
    "passwd",
    "secret",
    "api_key",
    "apikey",
    "x-api-key",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Debug,
    Info,
    Warn,
    Error,
}

impl Level {
    fn tag(self) -> &'static str {
        match self {
            Level::Debug => "DEBUG",
            Level::Info => "INFO ",
            Level::Warn => "WARN ",
            Level::Error => "ERROR",
        }
    }
}

/// Byte-bounded ring of formatted lines; the oldest lines are evicted first.
pub struct Recorder {
    started: Instant,
    capacity: usize,
    inner: Mutex<Ring>,
}

#[derive(Default)]
struct Ring {
    lines: VecDeque<String>,
    bytes: usize,
    evicted_lines: u64,
    evicted_bytes: u64,
}

impl Ring {
    fn push(&mut self, line: String, capacity: usize) {
        self.bytes += line.len();
        self.lines.push_back(line);
        while self.bytes > capacity {
            let Some(old) = self.lines.pop_front() else {
                break;
            };
            self.bytes -= old.len();
            self.evicted_lines += 1;
            self.evicted_bytes += old.len() as u64;
        }
    }
}

impl Recorder {
    pub fn new(capacity: usize) -> Self {
        Self {
            started: Instant::now(),
            capacity,
            inner: Mutex::new(Ring::default()),
        }
    }

    pub fn uptime(&self) -> Duration {
        self.started.elapsed()
    }

    fn format(&self, level: Level, source: &str, message: &str, max: usize) -> String {
        let message = redact(message);
        let message = truncate(&message, max);
        let mono = self.started.elapsed();
        let wall = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ");
        let mut line = String::with_capacity(message.len() + source.len() + 48);
        let _ = write!(
            line,
            "{wall} +{:>10.3}s {} [{source}] ",
            mono.as_secs_f64(),
            level.tag()
        );
        line.push_str(message);
        line.push('\n');
        line
    }

    pub fn record(&self, level: Level, source: &str, message: &str) {
        let line = self.format(level, source, message, MAX_LINE_BYTES);
        self.lock().push(line, self.capacity);
    }

    /// Never blocks: a panic may fire while this thread already holds the lock.
    fn record_nonblocking(&self, level: Level, source: &str, message: &str, max: usize) {
        let line = self.format(level, source, message, max);
        let mut ring = match self.inner.try_lock() {
            Ok(ring) => ring,
            Err(TryLockError::Poisoned(e)) => e.into_inner(),
            Err(TryLockError::WouldBlock) => return,
        };
        ring.push(line, self.capacity);
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Ring> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn dump(&self) -> String {
        let ring = self.lock();
        let mut out = String::with_capacity(ring.bytes + 128);
        if ring.evicted_lines > 0 {
            let _ = writeln!(
                out,
                "[{} older lines ({} bytes) evicted, cap {} bytes]",
                ring.evicted_lines, ring.evicted_bytes, self.capacity
            );
        }
        for line in &ring.lines {
            out.push_str(line);
        }
        out
    }

    pub fn bytes(&self) -> usize {
        self.lock().bytes
    }
}

static GLOBAL: LazyLock<Recorder> = LazyLock::new(|| Recorder::new(CAPACITY_BYTES));

pub fn global() -> &'static Recorder {
    &GLOBAL
}

pub fn record(level: Level, source: &str, message: impl AsRef<str>) {
    GLOBAL.record(level, source, message.as_ref());
}

pub fn debug(source: &str, message: impl AsRef<str>) {
    record(Level::Debug, source, message);
}

pub fn info(source: &str, message: impl AsRef<str>) {
    record(Level::Info, source, message);
}

pub fn warn(source: &str, message: impl AsRef<str>) {
    record(Level::Warn, source, message);
}

pub fn error(source: &str, message: impl AsRef<str>) {
    record(Level::Error, source, message);
}

/// Starts the clock, records the environment and hooks panics; call once at startup.
pub fn init() {
    LazyLock::force(&GLOBAL);
    install_panic_hook();
    info("app", "diagnostic recorder started");
    for line in environment() {
        info("env", line);
    }
}

fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        let payload = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "<non-string payload>".to_string());
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_default();
        let backtrace = std::backtrace::Backtrace::force_capture();
        let message = format!(
            "panic in thread {:?} at {location}: {payload}\n{backtrace}",
            thread.name().unwrap_or("<unnamed>")
        );
        GLOBAL.record_nonblocking(Level::Error, "panic", &message, MAX_PANIC_BYTES);
        previous(info);
    }));
}

/// Static and current facts about the app and machine, re-read at export time.
fn environment() -> Vec<String> {
    let cfg = crate::config::get_config()
        .lock()
        .map(|c| {
            format!(
                "install_id={} app_id={} os_user={} telemetry_enabled={} telemetry_url={:?}",
                c.install_id,
                c.app_id,
                c.username,
                c.should_send(),
                c.telemetry_url
            )
        })
        .unwrap_or_else(|_| "config lock poisoned".to_string());
    vec![
        format!(
            "voxelproxy {} ({} build), os={} family={} arch={} os_version={:?}",
            env!("CARGO_PKG_VERSION"),
            if cfg!(debug_assertions) {
                "debug"
            } else {
                "release"
            },
            std::env::consts::OS,
            std::env::consts::FAMILY,
            std::env::consts::ARCH,
            os_version()
        ),
        cfg,
        format!("update_url={:?}", env!("UPDATE_URL")),
        format!(
            "exe={:?} elevated={}",
            std::env::current_exe().ok(),
            crate::backend::is_elevated()
        ),
        format!(
            "prefs: manual_warning_acknowledged={} last_seen_version={:?} last_read_reply_at={}",
            crate::prefs::manual_warning_acknowledged(),
            crate::prefs::last_seen_version(),
            crate::prefs::last_read_reply_at()
        ),
        format!(
            "local_ip={:?} interfaces=[{}]",
            crate::local_ip::get_local_ip(),
            crate::local_ip::describe_interfaces()
        ),
        format!("local_utc_offset={}", chrono::Local::now().format("%:z")),
    ]
}

#[cfg(windows)]
fn os_version() -> Option<String> {
    use winreg::{RegKey, enums::HKEY_LOCAL_MACHINE};
    let key = RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey("SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion")
        .ok()?;
    let get = |name: &str| key.get_value::<String, _>(name).unwrap_or_default();
    Some(format!(
        "{} {} build {}.{}",
        get("ProductName"),
        get("DisplayVersion"),
        get("CurrentBuild"),
        key.get_value::<u32, _>("UBR").unwrap_or_default()
    ))
}

#[cfg(not(windows))]
fn os_version() -> Option<String> {
    let release = std::fs::read_to_string("/etc/os-release").ok()?;
    release
        .lines()
        .find_map(|l| l.strip_prefix("PRETTY_NAME="))
        .map(|v| v.trim_matches('"').to_string())
}

/// Full report sent to the developer: a fresh environment header plus the ring.
pub fn export() -> String {
    let recorder = global();
    let mut out = String::new();
    let _ = writeln!(
        out,
        "=== VoxelProxy diagnostic log, exported {} (uptime {:.3}s) ===",
        chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ"),
        recorder.uptime().as_secs_f64()
    );
    for line in environment() {
        out.push_str(&redact(&line));
        out.push('\n');
    }
    out.push_str("=== events ===\n");
    out.push_str(&recorder.dump());
    out
}

pub fn gzip(text: &str) -> std::io::Result<Vec<u8>> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(text.as_bytes())?;
    encoder.finish()
}

fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// Masks credential-looking values and URL userinfo.
pub fn redact(input: &str) -> Cow<'_, str> {
    let lower = input.to_ascii_lowercase();
    let mut spans: Vec<(usize, usize)> = Vec::new();

    let mut from = 0;
    while let Some(pos) = lower[from..].find("://") {
        let start = from + pos + 3;
        let rest = &lower[start..];
        let end = rest
            .find(|c: char| c == '/' || c == '?' || c == '#' || c.is_whitespace() || c == '"')
            .unwrap_or(rest.len());
        if let Some(at) = rest[..end].rfind('@') {
            spans.push((start, start + at));
        }
        from = start;
    }

    for key in SECRET_KEYS {
        let mut from = 0;
        while let Some(pos) = lower[from..].find(key) {
            let key_end = from + pos + key.len();
            from = key_end;
            let bytes = lower.as_bytes();
            let mut value = key_end;
            while value < bytes.len() && matches!(bytes[value], b'"' | b'\'' | b' ' | b':' | b'=') {
                value += 1;
            }
            if value == key_end {
                continue;
            }
            if lower[value..].starts_with("bearer ") {
                value += "bearer ".len();
            }
            let end = lower[value..]
                .find(|c: char| {
                    c.is_whitespace() || matches!(c, '&' | ',' | ';' | '"' | '\'' | '}' | ')')
                })
                .map_or(lower.len(), |e| value + e);
            if end > value {
                spans.push((value, end));
            }
        }
    }

    if spans.is_empty() {
        return Cow::Borrowed(input);
    }
    spans.sort_unstable();
    let mut out = String::with_capacity(input.len());
    let mut cursor = 0;
    for (start, end) in spans {
        if end <= cursor {
            continue;
        }
        let start = start.max(cursor);
        out.push_str(&input[cursor..start]);
        out.push_str(REDACTED);
        cursor = end;
    }
    out.push_str(&input[cursor..]);
    Cow::Owned(out)
}

#[cfg(test)]
mod tests {
    use std::io::Read;

    use super::*;

    #[test]
    fn evicts_oldest_lines_to_stay_within_cap() {
        let recorder = Recorder::new(1000);
        for i in 0..200 {
            recorder.record(Level::Info, "test", &format!("line {i:04}"));
        }
        assert!(recorder.bytes() <= 1000);
        let dump = recorder.dump();
        assert!(dump.contains("older lines"));
        assert!(dump.contains("line 0199"));
        assert!(!dump.contains("line 0000"));
    }

    #[test]
    fn long_lines_are_truncated_on_char_boundary() {
        let recorder = Recorder::new(CAPACITY_BYTES);
        recorder.record(Level::Warn, "test", &"ж".repeat(MAX_LINE_BYTES));
        let dump = recorder.dump();
        assert!(dump.len() < MAX_LINE_BYTES + 100);
        assert!(dump.ends_with("ж\n"));
    }

    #[test]
    fn lines_have_wall_clock_uptime_level_and_source() {
        let recorder = Recorder::new(CAPACITY_BYTES);
        recorder.record(Level::Error, "session", "boom");
        let line = recorder.dump();
        let (wall, rest) = line.split_once(' ').unwrap();
        assert!(chrono::DateTime::parse_from_rfc3339(wall).is_ok(), "{wall}");
        let rest = rest.trim_start();
        assert!(rest.starts_with('+'));
        assert!(rest.contains("s ERROR [session] boom\n"), "{rest}");
    }

    #[test]
    fn secrets_are_not_recorded() {
        let recorder = Recorder::new(CAPACITY_BYTES);
        recorder.record(
            Level::Info,
            "rpc",
            "Authorization: Bearer abc.def.ghi token=hunter2&x=1 {\"password\": \"p4ss\"}",
        );
        recorder.record(
            Level::Info,
            "rpc",
            "url https://user:s3cret@example.com/path?api_key=zzz",
        );
        let dump = recorder.dump();
        for secret in ["abc.def.ghi", "hunter2", "p4ss", "user:s3cret", "zzz"] {
            assert!(!dump.contains(secret), "{secret} leaked: {dump}");
        }
        assert!(dump.contains("x=1"));
        assert!(dump.contains("example.com/path"));
        assert!(dump.contains(REDACTED));
    }

    #[test]
    fn redact_leaves_plain_text_alone() {
        let text = "Подключено к 1.2.3.4:25565, threshold=256";
        assert!(matches!(redact(text), Cow::Borrowed(_)));
        assert_eq!(redact("tokens are fine"), "tokens are fine");
    }

    #[test]
    fn gzip_round_trips() {
        let text = "hello ".repeat(1000);
        let packed = gzip(&text).unwrap();
        assert!(packed.len() < text.len() / 10);
        let mut unpacked = String::new();
        flate2::read::GzDecoder::new(packed.as_slice())
            .read_to_string(&mut unpacked)
            .unwrap();
        assert_eq!(unpacked, text);
    }

    #[test]
    fn panic_hook_path_does_not_block_on_held_lock() {
        let recorder = Recorder::new(CAPACITY_BYTES);
        let guard = recorder.lock();
        recorder.record_nonblocking(Level::Error, "panic", "while locked", MAX_PANIC_BYTES);
        drop(guard);
        recorder.record_nonblocking(Level::Error, "panic", "after", MAX_PANIC_BYTES);
        let dump = recorder.dump();
        assert!(!dump.contains("while locked"));
        assert!(dump.contains("after"));
    }
}
