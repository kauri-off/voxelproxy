pub async fn send_startup_ping() {
    if cfg!(debug_assertions) {
        return;
    }
    let url = env!("TELEMETRY_URL");
    if url.is_empty() {
        return;
    }
    let ping_url = format!("{}/v1/ping", url);
    let _ = reqwest::Client::new()
        .post(ping_url)
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

pub async fn send_start_manual(server_addr: String) {
    if cfg!(debug_assertions) {
        return;
    }
    let url = env!("TELEMETRY_URL");
    if url.is_empty() {
        return;
    }
    let ping_url = format!("{}/v1/start_manual", url);
    let _ = reqwest::Client::new()
        .post(ping_url)
        .json(&serde_json::json!({
            "username": std::env::var("USERNAME")
                .or_else(|_| std::env::var("USER"))
                .unwrap_or_else(|_| "unknown".to_string()),
            "server_addr": server_addr
        }))
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await;
}

pub async fn send_start_auto(windivert: bool) {
    if cfg!(debug_assertions) {
        return;
    }
    let url = env!("TELEMETRY_URL");
    if url.is_empty() {
        return;
    }
    let ping_url = format!("{}/v1/start_auto", url);
    let _ = reqwest::Client::new()
        .post(ping_url)
        .json(&serde_json::json!({
            "username": std::env::var("USERNAME")
                .or_else(|_| std::env::var("USER"))
                .unwrap_or_else(|_| "unknown".to_string()),
            "windivert": windivert
        }))
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await;
}
