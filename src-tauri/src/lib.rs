pub mod app_state;
pub mod commands;
pub mod controller;
#[cfg(target_os = "windows")]
pub mod hotspot_redirect;
pub mod local_ip;
pub mod logger;
#[allow(dead_code)]
pub mod packets;
pub mod protocols;
pub mod proxy;
pub mod resolver;
pub mod session;
pub mod telemetry;
pub mod updater;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
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
            commands::get_platform,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
