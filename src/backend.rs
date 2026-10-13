use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use futures::FutureExt;
use futures::future::{AbortHandle, abortable};
use tokio::runtime::Runtime;
use tokio::task::JoinHandle;

use crate::{
    changelog::{self, ChangelogEntry},
    config,
    events::{AppEvent, EventSink},
    logger::Logger,
    prefs, session,
};

pub use crate::updater::NewVersion as UpdateInfo;

pub const RELEASES_URL: &str = "https://github.com/kauri-off/voxelproxy/releases";
pub const REPO_URL: &str = "https://github.com/kauri-off/voxelproxy";

struct Session {
    task: JoinHandle<()>,
    abort: AbortHandle,
}

/// Owns the tokio runtime and the currently running proxy session.
pub struct Backend {
    rt: Runtime,
    events: EventSink,
    session: Mutex<Option<Session>>,
    check_mode: Arc<AtomicBool>,
}

impl Backend {
    pub fn new(events: EventSink) -> Arc<Self> {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .thread_name("voxelproxy-worker")
            .build()
            .expect("failed to start tokio runtime");
        Arc::new(Self {
            rt,
            events,
            session: Mutex::new(None),
            check_mode: Arc::new(AtomicBool::new(false)),
        })
    }

    pub fn spawn<F>(&self, fut: F) -> JoinHandle<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        self.rt.spawn(fut)
    }

    pub fn start_manual(&self, server_addr: String) {
        self.spawn(config::send_start_manual(server_addr.clone()));
        let events = self.events.clone();
        self.start_session(async move { session::run_manual_mode(server_addr, events).await });
    }

    pub fn start_auto(&self, use_windivert: bool, port_min: u16, port_max: u16) {
        self.spawn(config::send_start_auto(use_windivert));
        self.set_check_mode(false);
        let events = self.events.clone();
        let check_mode = self.check_mode.clone();
        self.start_session(async move {
            session::run_auto_mode(use_windivert, port_min, port_max, events, check_mode).await
        });
    }

    fn start_session<F>(&self, fut: F)
    where
        F: Future<Output = anyhow::Result<()>> + Send + 'static,
    {
        let mut session = self.session.lock().unwrap();
        let prev = session.take();
        if let Some(prev) = &prev {
            prev.abort.abort();
        }

        let (fut, abort) = abortable(fut);
        let events = self.events.clone();
        let task = self.rt.spawn(async move {
            if let Some(prev) = prev {
                let _ = prev.task.await;
            }
            events.emit(AppEvent::SessionStarted);
            match AssertUnwindSafe(fut).catch_unwind().await {
                Ok(Ok(Err(e))) => Logger::new(&events).error(e.to_string()),
                Err(_) => Logger::new(&events).error("Сессия аварийно завершилась"),
                _ => {}
            }
            events.emit(AppEvent::SessionEnded);
        });
        *session = Some(Session { task, abort });
    }

    /// The session task emits `SessionEnded` itself once fully torn down.
    pub fn stop(&self) {
        self.abort_existing();
    }

    pub fn set_check_mode(&self, value: bool) {
        self.check_mode.store(value, Ordering::Relaxed);
    }

    fn abort_existing(&self) {
        if let Some(s) = self.session.lock().unwrap().as_ref() {
            s.abort.abort();
        }
    }

    pub fn check_updates(&self) -> JoinHandle<Result<Option<UpdateInfo>, String>> {
        let log = Logger::new(&self.events);
        self.spawn(async move {
            if cfg!(debug_assertions) || !cfg!(windows) {
                return Ok(None);
            }
            crate::updater::has_update(env!("CARGO_PKG_VERSION"), &log)
                .await
                .map_err(|e| e.to_string())
        })
    }

    /// Downloads the MSI and launches the installer; the caller should quit afterwards.
    pub fn download_and_install_update(
        &self,
        url: String,
        tag: String,
    ) -> JoinHandle<Result<(), String>> {
        let events = self.events.clone();
        self.spawn(async move {
            crate::updater::download_and_install(url, tag, events)
                .await
                .map_err(|e| e.to_string())
        })
    }
}

impl Drop for Backend {
    fn drop(&mut self) {
        self.abort_existing();
    }
}

pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

pub fn supported_versions() -> Vec<&'static str> {
    crate::protocols::Version::supported_versions().to_vec()
}

pub fn local_ip() -> String {
    crate::local_ip::get_local_ip()
        .unwrap_or(std::net::Ipv4Addr::LOCALHOST)
        .to_string()
}

pub fn is_windows() -> bool {
    cfg!(windows)
}

#[cfg(windows)]
pub fn is_elevated() -> bool {
    use windows::Win32::{
        Foundation::{CloseHandle, HANDLE},
        Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation},
        System::Threading::{GetCurrentProcess, OpenProcessToken},
    };

    unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
            return false;
        }

        let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
        let mut return_length = 0u32;
        let result = GetTokenInformation(
            token,
            TokenElevation,
            Some(&mut elevation as *mut _ as *mut _),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut return_length,
        );

        let _ = CloseHandle(token);
        result.is_ok() && elevation.TokenIsElevated != 0
    }
}

#[cfg(not(windows))]
pub fn is_elevated() -> bool {
    true
}

pub fn open_url(url: &str) {
    if url.starts_with("http://") || url.starts_with("https://") {
        let _ = open::that(url);
    }
}

/// Starts an elevated copy of the app; the caller should quit on success.
pub fn relaunch_as_admin() -> Result<(), String> {
    #[cfg(not(windows))]
    {
        Err("Перезапуск с правами администратора поддерживается только на Windows".into())
    }

    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows::Win32::UI::Shell::ShellExecuteW;
        use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
        use windows::core::PCWSTR;

        let exe = std::env::current_exe().map_err(|e| e.to_string())?;

        let exe_wide: Vec<u16> = exe
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let verb: Vec<u16> = std::ffi::OsStr::new("runas")
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        let result = unsafe {
            ShellExecuteW(
                None,
                PCWSTR(verb.as_ptr()),
                PCWSTR(exe_wide.as_ptr()),
                PCWSTR::null(),
                PCWSTR::null(),
                SW_SHOWNORMAL,
            )
        };

        // Values <= 32 are error codes (e.g. the user dismissed the UAC prompt).
        if result.0 as isize <= 32 {
            return Err("Не удалось перезапустить с правами администратора".into());
        }
        Ok(())
    }
}

pub fn pending_changelogs() -> Vec<ChangelogEntry> {
    let current = version();
    let last_seen = prefs::last_seen_version();

    if last_seen.is_none() {
        let _ = prefs::set_last_seen_version(current);
        return Vec::new();
    }

    let entries = changelog::pending_for(last_seen.as_deref(), current, changelog::bundled());
    if entries.is_empty() && last_seen.as_deref() != Some(current) {
        let _ = prefs::set_last_seen_version(current);
    }
    entries
}

pub fn acknowledge_changelog() {
    let _ = prefs::set_last_seen_version(version());
}

pub fn manual_warning_acknowledged() -> bool {
    prefs::manual_warning_acknowledged()
}

pub fn acknowledge_manual_warning() -> Result<(), String> {
    prefs::acknowledge_manual_warning()
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;
    use futures::channel::oneshot;
    use futures::executor::block_on;
    use tokio::net::TcpListener;

    fn start_listener(backend: &Backend, port: u16) -> oneshot::Receiver<bool> {
        let (tx, rx) = oneshot::channel();
        backend.start_session(async move {
            let listener = TcpListener::bind(("127.0.0.1", port)).await;
            let _ = tx.send(listener.is_ok());
            let _listener = listener?;
            std::future::pending::<()>().await;
            Ok(())
        });
        rx
    }

    #[test]
    fn restart_rebinds_port_after_old_session_is_torn_down() {
        let (events, mut rx) = EventSink::channel();
        let backend = Backend::new(events);
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();

        assert!(block_on(start_listener(&backend, port)).unwrap());
        for _ in 0..20 {
            backend.stop();
            assert!(block_on(start_listener(&backend, port)).unwrap());
        }
        backend.stop();

        let mut expected_started = true;
        for _ in 0..42 {
            match block_on(rx.next()).unwrap() {
                AppEvent::SessionStarted => assert!(expected_started),
                AppEvent::SessionEnded => assert!(!expected_started),
                other => panic!("unexpected event {other:?}"),
            }
            expected_started = !expected_started;
        }
    }

    #[test]
    fn panicking_session_still_ends() {
        let (events, mut rx) = EventSink::channel();
        let backend = Backend::new(events);
        backend.start_session(async { panic!("boom") });

        assert!(matches!(
            block_on(rx.next()),
            Some(AppEvent::SessionStarted)
        ));
        assert!(matches!(
            block_on(rx.next()),
            Some(AppEvent::Log {
                level: crate::events::LogLevel::Error,
                ..
            })
        ));
        assert!(matches!(block_on(rx.next()), Some(AppEvent::SessionEnded)));
    }
}
