use serde::Serialize;
use tauri::{AppHandle, State};
use tauri_specta::Event;

use crate::{
    app_state::AppState,
    changelog::{self, ChangelogEntry},
    config,
    events::{SessionEndedEvent, SessionStartedEvent},
    logger::Logger,
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
        match session::run_manual_mode(server_addr, app.clone()).await {
            Ok(()) => log.info("Сессия завершена"),
            Err(e) => log.error(format!("{}", e)),
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
        match session::run_automatic_mode(
            use_windivert,
            port_min,
            port_max,
            app.clone(),
            panic_mode,
        )
        .await
        {
            Ok(()) => log.info("Автосессия завершена"),
            Err(e) => log.error(format!("{}", e)),
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
    let last_seen = changelog::read_last_seen();

    if last_seen.is_none() {
        changelog::write_last_seen(current)?;
        return Ok(Vec::new());
    }

    let entries = changelog::pending_for(last_seen.as_deref(), current, changelog::bundled());

    if entries.is_empty() {
        if let Some(stored) = last_seen.as_deref() {
            if stored != current {
                changelog::write_last_seen(current)?;
            }
        }
    }

    Ok(entries)
}

#[tauri::command]
#[specta::specta]
pub fn acknowledge_changelog() -> Result<(), String> {
    changelog::write_last_seen(env!("CARGO_PKG_VERSION"))
}

async fn abort_existing(state: &State<'_, AppState>) {
    if let Some(h) = state.session.lock().await.take() {
        h.abort();
    }
}
