use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tokio::runtime::Runtime;
use tokio::task::{AbortHandle, JoinHandle};

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

/// Owns the tokio runtime and the currently running proxy session.
pub struct Backend {
    rt: Runtime,
    events: EventSink,
    session: Mutex<Option<AbortHandle>>,
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
        self.abort_existing();
        self.events.emit(AppEvent::SessionStarted);

        let events = self.events.clone();
        let handle = self.rt.spawn(async move {
            if let Err(e) = fut.await {
                Logger::new(&events).error(e.to_string());
            }
            events.emit(AppEvent::SessionEnded);
        });
        *self.session.lock().unwrap() = Some(handle.abort_handle());
    }

    pub fn stop(&self) {
        self.abort_existing();
        self.events.emit(AppEvent::SessionEnded);
    }

    pub fn set_check_mode(&self, value: bool) {
        self.check_mode.store(value, Ordering::Relaxed);
    }

    fn abort_existing(&self) {
        if let Some(h) = self.session.lock().unwrap().take() {
            h.abort();
        }
    }

    pub fn check_updates(&self) -> JoinHandle<Result<Option<UpdateInfo>, String>> {
        self.spawn(async {
            if cfg!(debug_assertions) || !cfg!(windows) {
                return Ok(None);
            }
            crate::updater::has_update(env!("CARGO_PKG_VERSION"))
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
