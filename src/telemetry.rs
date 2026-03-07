/// Sends a single fire-and-forget ping to the telemetry server on startup.
/// Any error (no network, server down) is silently ignored.
pub async fn send_startup_ping() {
    let _ = reqwest::Client::new()
        .post("https://endarise.isgood.host:4444/voxelproxy/ping")
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
