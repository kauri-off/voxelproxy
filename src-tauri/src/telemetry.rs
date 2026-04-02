/// Sends a single fire-and-forget ping to the telemetry server on startup.
/// Any error (no network, server down, missing URL) is silently ignored.
/// Does nothing in debug builds.
pub async fn send_startup_ping() {
    if cfg!(debug_assertions) {
        return;
    }
    let url = env!("TELEMETRY_URL");
    if url.is_empty() {
        return;
    }
    let _ = reqwest::Client::new()
        .post(url)
        .json(&serde_json::json!({
            "version":  env!("CARGO_PKG_VERSION"),
            "os":       std::env::consts::OS,
            "username": std::env::var("USERNAME")
                            .or_else(|_| std::env::var("USER"))
                            .unwrap_or_else(|_| "unknown".to_string()),
        }))
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await;
}
