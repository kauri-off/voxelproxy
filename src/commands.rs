use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::{app_state::AppState, logger::Logger, session, updater::has_update};

#[derive(Serialize)]
pub struct UpdateInfo {
    pub tag: String,
    pub link: String,
}

// ── Start manual session ──────────────────────────────────────────────────────

#[tauri::command]
pub async fn start_manual_session(
    server_addr: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    abort_existing(&state).await;
    let log = Logger::new(app.clone());
    let app2 = app.clone();

    // Emit BEFORE spawning to avoid a race where session-ended fires first.
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

// ── Start automatic session (Windows) ─────────────────────────────────────────

#[cfg(target_os = "windows")]
#[tauri::command]
pub async fn start_auto_session(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    abort_existing(&state).await;
    let log = Logger::new(app.clone());
    let app2 = app.clone();

    // Emit BEFORE spawning to avoid a race where session-ended fires first.
    app.emit("session-started", "auto")
        .map_err(|e| e.to_string())?;

    let handle = tokio::spawn(async move {
        match session::run_automatic_mode(log.clone()).await {
            Ok(()) => log.info("Автосессия завершена"),
            Err(e) => log.error(format!("{}", e)),
        }
        app2.emit("session-ended", ()).ok();
    });

    *state.session.lock().await = Some(handle.abort_handle());
    Ok(())
}

#[cfg(not(target_os = "windows"))]
#[tauri::command]
pub async fn start_auto_session(
    _app: AppHandle,
    _state: State<'_, AppState>,
) -> Result<(), String> {
    Err("Автоматический режим поддерживается только на Windows".to_string())
}

// ── Stop session ──────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn stop_session(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    abort_existing(&state).await;
    app.emit("session-ended", ()).ok();
    Ok(())
}

// ── Queries ───────────────────────────────────────────────────────────────────

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
    let version = env!("CARGO_PKG_VERSION");
    match has_update(version).await {
        Ok(Some(info)) => Ok(Some(UpdateInfo { tag: info.tag, link: info.link })),
        Ok(None) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

#[tauri::command]
pub fn open_url(url: String) {
    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("cmd")
        .args(["/c", "start", "", &url])
        .spawn();
}

// ── Helper ────────────────────────────────────────────────────────────────────

async fn abort_existing(state: &State<'_, AppState>) {
    if let Some(h) = state.session.lock().await.take() {
        h.abort();
    }
}
