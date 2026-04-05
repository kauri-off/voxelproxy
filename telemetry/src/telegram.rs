#[cfg(feature = "telegram")]
use reqwest::Client;
#[cfg(feature = "telegram")]
use std::sync::OnceLock;

#[cfg(feature = "telegram")]
static CLIENT: OnceLock<Client> = OnceLock::new();

#[cfg(feature = "telegram")]
const TOKEN: &str = include_str!("../TG_TOKEN");
#[cfg(feature = "telegram")]
const CHAT_ID: &str = include_str!("../TG_ADMIN_ID");

#[cfg(feature = "telegram")]
pub fn send(msg: String) {
    let client = CLIENT.get_or_init(Client::new);
    let url = format!("https://api.telegram.org/bot{}/sendMessage", TOKEN.trim());

    tokio::spawn(async move {
        let _ = client
            .post(url)
            .json(&serde_json::json!({
                "chat_id": CHAT_ID.trim(),
                "text": msg,
                "parse_mode": "HTML",
                "disable_web_page_preview": true
            }))
            .send()
            .await;
    });
}

#[cfg(not(feature = "telegram"))]
pub fn send(_: String) {}

pub fn format_ping(user: &str, ip: &str, version: &str, os: &str) -> String {
    format!(
        "<b>📡 Ping</b>\n👤 <b>User:</b> {}\n🌐 <b>IP:</b> {}\n🖥 <b>OS:</b> {}\n⚙️ <b>Version:</b> {}",
        user, ip, os, version
    )
}

pub fn format_manual(user: &str, server: &str) -> String {
    format!(
        "<b>🧭 Manual Start</b>\n👤 <b>User:</b> {}\n🔗 <b>Server:</b> {}",
        user, server
    )
}

pub fn format_auto(user: &str, windivert: bool) -> String {
    format!(
        "<b>⚡ Auto Start</b>\n👤 <b>User:</b> {}\n🧩 <b>WinDivert:</b> {}",
        user, windivert
    )
}
