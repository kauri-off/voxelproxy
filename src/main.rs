#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

pub mod backend;
pub mod changelog;
pub mod config;
pub mod controller;
pub mod events;
#[cfg(target_os = "windows")]
pub mod hotspot_redirect;
pub mod local_ip;
pub mod logger;
#[allow(dead_code)]
pub mod packets;
pub mod prefs;
pub mod protocols;
pub mod proxy;
pub mod resolver;
pub mod session;
mod ui;
pub mod updater;

fn main() {
    ui::run();
}
