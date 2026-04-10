use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::{app_state::AppState, logger::Logger, session, telemetry, updater::has_update};

#[derive(Serialize)]
pub struct UpdateInfo {
    pub tag: String,
    pub link: String,
}

#[tauri::command]
pub async fn start_manual_session(
    server_addr: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    tokio::spawn(telemetry::send_start_manual(server_addr.clone()));
    abort_existing(&state).await;
    let log = Logger::new(app.clone());
    let app2 = app.clone();

    app.emit("session-started", "manual")
        .map_err(|e| e.to_string())?;

    let handle = tokio::spawn(async move {
        match session::run_manual_mode(server_addr, log.clone()).await {
            Ok(()) => log.info("Сессия завершена"),
            Err(e) => log.error(format!("{}", e)),
        }
        app2.emit("session-ended", ()).ok();
    });

    *state.session.lock().await = Some(handle.abort_handle());
    Ok(())
}

#[tauri::command]
pub async fn start_auto_session(
    use_windivert: bool,
    port_min: u16,
    port_max: u16,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    tokio::spawn(telemetry::send_start_auto(use_windivert));
    abort_existing(&state).await;
    let log = Logger::new(app.clone());
    let app2 = app.clone();
    let panic_mode = state.panic_mode.clone();

    app.emit("session-started", "auto")
        .map_err(|e| e.to_string())?;

    let handle = tokio::spawn(async move {
        match session::run_automatic_mode(
            use_windivert,
            port_min,
            port_max,
            log.clone(),
            panic_mode,
        )
        .await
        {
            Ok(()) => log.info("Автосессия завершена"),
            Err(e) => log.error(format!("{}", e)),
        }
        app2.emit("session-ended", ()).ok();
    });

    *state.session.lock().await = Some(handle.abort_handle());
    Ok(())
}

#[tauri::command]
pub async fn stop_session(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    abort_existing(&state).await;
    app.emit("session-ended", ()).ok();
    Ok(())
}

#[tauri::command]
pub fn get_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

#[tauri::command]
pub fn get_local_ip_addr() -> String {
    use std::net::Ipv4Addr;
    crate::local_ip::get_local_ip()
        .unwrap_or(Ipv4Addr::new(127, 0, 0, 1))
        .to_string()
}

#[tauri::command]
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
pub fn open_url(url: String) {
    if url.starts_with("http://") || url.starts_with("https://") {
        let _ = open::that(url);
    }
}

#[tauri::command]
pub fn get_platform() -> String {
    std::env::consts::OS.to_string()
}

#[tauri::command]
pub async fn set_panic_mode(value: bool, state: State<'_, AppState>) -> Result<(), String> {
    let mut panic_mode = state.panic_mode.lock().await;
    *panic_mode = value;
    Ok(())
}

async fn abort_existing(state: &State<'_, AppState>) {
    if let Some(h) = state.session.lock().await.take() {
        h.abort();
    }
}
