#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app_state;
mod commands;
mod controller;
#[cfg(target_os = "windows")]
mod hotspot_redirect;
mod local_ip;
mod logger;
#[allow(dead_code)]
mod packets;
mod protocols;
mod proxy;
mod resolver;
mod session;
mod telemetry;
mod updater;

fn main() {
    tauri::Builder::default()
        .setup(|_app| {
            tauri::async_runtime::spawn(telemetry::send_startup_ping());
            Ok(())
        })
        .manage(app_state::AppState::new())
        .invoke_handler(tauri::generate_handler![
            commands::start_manual_session,
            commands::start_auto_session,
            commands::stop_session,
            commands::get_version,
            commands::get_local_ip_addr,
            commands::check_updates,
            commands::open_url,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
