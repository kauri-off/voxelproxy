use reqwest::{
    header::{HeaderMap, HeaderValue},
    Client,
};
use serde_json::Value;
use std::process::Command;

pub async fn check_for_updates(current_version: &str) {
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

    let response: Value = serde_json::from_str(
        &client
            .get("https://api.github.com/repos/kauri-off/voxelproxy/releases/latest")
            .headers(headers)
            .send()
            .await
            .expect("Failed to fetch release")
            .text()
            .await
            .unwrap(),
    )
    .unwrap();

    if response["tag_name"].as_str().unwrap_or("") != current_version {
        println!(
            " Доступна новая версия, пожалуйста обновитесь: {}",
            response["tag_name"].as_str().unwrap_or("")
        );
        let download_url = response["assets"][0]["browser_download_url"]
            .as_str()
            .unwrap_or("Error");
        println!(" Ссылка: {}", download_url);

        let _ = Command::new("cmd")
            .arg("/C")
            .arg("start")
            .arg(download_url)
            .output();
        loop {
            let _: String = dialoguer::Input::new().interact_text().unwrap();
        }
        // download_release(download_url).await;
    } else {
        println!(" У вас последняя версия!");
    }
}

// async fn download_release(url: &str) {
//     let response = reqwest::get(url)
//         .await
//         .expect("Failed to download release")
//         .bytes()
//         .await
//         .expect("Failed to read bytes");

//     std::fs::write("your_release_file", &response).expect("Failed to write file");
// }
