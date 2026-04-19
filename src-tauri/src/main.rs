#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use tauri_specta::{Builder, collect_commands, collect_events};

pub mod app_state;
pub mod commands;
pub mod controller;
pub mod events;
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

fn create_builder() -> Builder<tauri::Wry> {
    Builder::<tauri::Wry>::new()
        .commands(collect_commands![
            commands::start_manual_session,
            commands::start_auto_session,
            commands::stop_session,
            commands::get_version,
            commands::get_local_ip_addr,
            commands::check_updates,
            commands::open_url,
            commands::get_platform,
            commands::set_panic_mode
        ])
        .events(collect_events![
            events::ProxyLogEvent,
            events::SessionStartedEvent,
            events::SessionEndedEvent,
            events::ClientStatusEvent,
            events::NickNameEvent,
        ])
}

pub fn main() {
    let builder = create_builder();
    let invoke_handler = builder.invoke_handler();

    tauri::Builder::default()
        .setup(move |app| {
            builder.mount_events(app);
            tauri::async_runtime::spawn(telemetry::send_startup_ping());
            Ok(())
        })
        .manage(app_state::AppState::new())
        .invoke_handler(invoke_handler)
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_bindings() {
        create_builder()
            .export(
                specta_typescript::Typescript::default(),
                "../src/bindings.ts",
            )
            .expect("Failed to export TypeScript bindings");
    }
}
