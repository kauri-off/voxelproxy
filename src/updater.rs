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
    headers.append("User-Agent", HeaderValue::from_str("VP Updater").unwrap());
    headers.append(
        "Accept",
        HeaderValue::from_str("application/vnd.github+json").unwrap(),
    );
    headers.append(
        "X-GitHub-Api-Version",
        HeaderValue::from_str("2022-11-28").unwrap(),
    );

    let response: Value = client
        .get("https://api.github.com/repos/kauri-off/voxelproxy/releases/latest")
        .headers(headers)
        .send()
        .await?
        .json()
        .await?;

    if response["tag_name"].as_str().unwrap_or("") != current_version {
        let tag = response["tag_name"].as_str().unwrap_or("").to_string();
        let link = response["assets"][0]["browser_download_url"]
            .as_str()
            .unwrap_or("Error")
            .to_string();
        return Ok(Some(NewVersion { tag, link }));
    }

    Ok(None)
}
