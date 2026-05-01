use serde::Serialize;
use tauri::{AppHandle, State};
use tauri_specta::Event;

use crate::{
    app_state::AppState,
    changelog::{self, ChangelogEntry},
    config,
    events::{SessionEndedEvent, SessionStartedEvent, UpdateProgressEvent},
    logger::Logger,
    prefs,
    protocols::Version,
    session,
    updater::has_update,
};

#[derive(Serialize, specta::Type)]
pub struct UpdateInfo {
    pub tag: String,
    pub link: String,
}

#[tauri::command]
#[specta::specta]
pub async fn start_manual_session(
    server_addr: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    tokio::spawn(config::send_start_manual(server_addr.clone()));
    abort_existing(&state).await;

    SessionStartedEvent {}.emit(&app).ok();

    let handle = tokio::spawn(async move {
        let log = Logger::new(&app);
        if let Err(e) = session::run_manual_mode(server_addr, app.clone()).await {
            log.error(format!("{}", e));
        }
        SessionEndedEvent {}.emit(&app).ok();
    });

    *state.session.lock().await = Some(handle.abort_handle());
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn start_auto_session(
    use_windivert: bool,
    port_min: u16,
    port_max: u16,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    tokio::spawn(config::send_start_auto(use_windivert));
    abort_existing(&state).await;
    let panic_mode = state.panic_mode.clone();

    SessionStartedEvent {}.emit(&app).ok();

    let handle = tokio::spawn(async move {
        let log = Logger::new(&app);
        if let Err(e) = session::run_automatic_mode(
            use_windivert,
            port_min,
            port_max,
            app.clone(),
            panic_mode,
        )
        .await
        {
            log.error(format!("{}", e));
        }
        SessionEndedEvent {}.emit(&app).ok();
    });

    *state.session.lock().await = Some(handle.abort_handle());
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn stop_session(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    abort_existing(&state).await;
    SessionEndedEvent {}.emit(&app).ok();
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn get_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

#[tauri::command]
#[specta::specta]
pub fn get_supported_versions() -> Vec<String> {
    Version::supported_versions()
        .iter()
        .map(|s| s.to_string())
        .collect()
}

#[tauri::command]
#[specta::specta]
pub fn get_local_ip_addr() -> String {
    use std::net::Ipv4Addr;
    crate::local_ip::get_local_ip()
        .unwrap_or(Ipv4Addr::new(127, 0, 0, 1))
        .to_string()
}

#[tauri::command]
#[specta::specta]
pub async fn check_updates() -> Result<Option<UpdateInfo>, String> {
    if cfg!(debug_assertions) || std::env::consts::OS != "windows" {
        return Ok(None);
    }
    let version = env!("CARGO_PKG_VERSION");
    match has_update(version).await {
        Ok(Some(info)) => Ok(Some(UpdateInfo {
            tag: info.tag,
            link: info.link,
        })),
        Ok(None) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

#[tauri::command]
#[specta::specta]
pub async fn download_and_install_update(url: String, app: AppHandle) -> Result<(), String> {
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (url, app);
        return Err("Unsupported platform".into());
    }

    #[cfg(target_os = "windows")]
    {
        use futures_util::StreamExt;
        use tokio::io::AsyncWriteExt;

        let resp = reqwest::Client::new()
            .get(&url)
            .send()
            .await
            .map_err(|e| e.to_string())?
            .error_for_status()
            .map_err(|e| e.to_string())?;

        let total = resp.content_length().unwrap_or(0);

        let mut path = std::env::temp_dir();
        path.push("VoxelProxy-update.msi");

        let mut file = tokio::fs::File::create(&path)
            .await
            .map_err(|e| e.to_string())?;

        let mut stream = resp.bytes_stream();
        let mut downloaded: u64 = 0;

        UpdateProgressEvent { downloaded, total }.emit(&app).ok();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| e.to_string())?;
            file.write_all(&chunk).await.map_err(|e| e.to_string())?;
            downloaded += chunk.len() as u64;
            UpdateProgressEvent { downloaded, total }.emit(&app).ok();
        }

        file.flush().await.map_err(|e| e.to_string())?;
        drop(file);

        std::process::Command::new("msiexec")
            .args(["/i", path.to_string_lossy().as_ref()])
            .spawn()
            .map_err(|e| e.to_string())?;

        app.exit(0);
        Ok(())
    }
}

#[tauri::command]
#[specta::specta]
pub fn open_url(url: String) {
    if url.starts_with("http://") || url.starts_with("https://") {
        let _ = open::that(url);
    }
}

#[tauri::command]
#[specta::specta]
pub fn get_platform() -> String {
    std::env::consts::OS.to_string()
}

#[tauri::command]
#[specta::specta]
pub async fn set_panic_mode(value: bool, state: State<'_, AppState>) -> Result<(), String> {
    let mut panic_mode = state.panic_mode.lock().await;
    *panic_mode = value;
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn get_pending_changelogs() -> Result<Vec<ChangelogEntry>, String> {
    let current = env!("CARGO_PKG_VERSION");
    let last_seen = prefs::last_seen_version();

    if last_seen.is_none() {
        prefs::set_last_seen_version(current)?;
        return Ok(Vec::new());
    }

    let entries = changelog::pending_for(last_seen.as_deref(), current, changelog::bundled());

    if entries.is_empty() {
        if let Some(stored) = last_seen.as_deref() {
            if stored != current {
                prefs::set_last_seen_version(current)?;
            }
        }
    }

    Ok(entries)
}

#[tauri::command]
#[specta::specta]
pub fn acknowledge_changelog() -> Result<(), String> {
    prefs::set_last_seen_version(env!("CARGO_PKG_VERSION"))
}

#[tauri::command]
#[specta::specta]
pub fn get_manual_warning_acknowledged() -> bool {
    prefs::manual_warning_acknowledged()
}

#[tauri::command]
#[specta::specta]
pub fn acknowledge_manual_warning() -> Result<(), String> {
    prefs::acknowledge_manual_warning()
}

async fn abort_existing(state: &State<'_, AppState>) {
    if let Some(h) = state.session.lock().await.take() {
        h.abort();
    }
}
