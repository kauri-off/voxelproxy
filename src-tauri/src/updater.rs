use reqwest::{
    Client,
    header::{HeaderMap, HeaderValue},
};
use serde_json::Value;

pub struct NewVersion {
    pub tag: String,
    pub link: String,
}

fn build_github_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert("User-Agent", HeaderValue::from_static("VP Updater"));
    headers.insert(
        "Accept",
        HeaderValue::from_static("application/vnd.github+json"),
    );
    headers.insert(
        "X-GitHub-Api-Version",
        HeaderValue::from_static("2022-11-28"),
    );
    headers
}

pub async fn has_update(current_version: &str) -> anyhow::Result<Option<NewVersion>> {
    if env!("UPDATE_URL").is_empty() {
        return Ok(None);
    }

    let client = Client::new();

    let release: Value = client
        .get(env!("UPDATE_URL"))
        .headers(build_github_headers())
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    let latest_version = release
        .get("tag_name")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("Missing or invalid 'tag_name' field"))?;

    let latest_stripped = latest_version.strip_prefix('v').unwrap_or(latest_version);
    if latest_stripped == current_version {
        return Ok(None);
    }

    let download_url = release
        .get("assets")
        .and_then(Value::as_array)
        .and_then(|assets| assets.first())
        .and_then(|asset| asset.get("browser_download_url"))
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("Missing or invalid asset download URL"))?;

    Ok(Some(NewVersion {
        tag: latest_version.to_string(),
        link: download_url.to_string(),
    }))
}
