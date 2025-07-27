use reqwest::{
    header::{HeaderMap, HeaderValue},
    Client,
};
use serde_json::Value;

pub struct NewVersion {
    pub tag: String,
    pub link: String,
}

pub async fn has_update(current_version: &str) -> anyhow::Result<Option<NewVersion>> {
    let client = Client::new();

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

    let response = client
        .get("https://api.github.com/repos/kauri-off/voxelproxy/releases/latest")
        .headers(headers)
        .send()
        .await?
        .error_for_status()?;

    let json: Value = response.json().await?;

    let tag = json
        .get("tag_name")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("Missing or invalid 'tag_name' field"))?;

    if tag == current_version {
        return Ok(None);
    }

    let asset_url = json
        .get("assets")
        .and_then(Value::as_array)
        .and_then(|assets| assets.first())
        .and_then(|asset| asset.get("browser_download_url"))
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("Missing or invalid asset download URL"))?;

    Ok(Some(NewVersion {
        tag: tag.to_string(),
        link: asset_url.to_string(),
    }))
}
