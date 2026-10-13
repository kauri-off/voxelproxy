use reqwest::{
    Client,
    header::{HeaderMap, HeaderValue},
};
use serde_json::Value;

use crate::events::EventSink;

#[derive(Debug, Clone)]
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

#[cfg(not(windows))]
pub async fn download_and_install(
    _url: String,
    _tag: String,
    _events: EventSink,
) -> anyhow::Result<()> {
    anyhow::bail!("Автообновление поддерживается только на Windows")
}

#[cfg(windows)]
const SCAN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(180);

#[cfg(windows)]
const INVALID_GRACE: std::time::Duration = std::time::Duration::from_secs(15);

#[cfg(windows)]
pub async fn download_and_install(
    url: String,
    tag: String,
    events: EventSink,
) -> anyhow::Result<()> {
    let path = std::env::temp_dir().join(format!("VoxelProxy-update-{tag}.msi"));
    if !path.exists() {
        download(&url, &path, &events).await?;
    }

    let started = std::time::Instant::now();
    loop {
        match package_state(&path) {
            PackageState::Ready => break,
            PackageState::Missing => anyhow::bail!(
                "Файл обновления пропал — вероятно, его удалил антивирус. Нажмите «Повторить» или скачайте обновление со страницы релизов"
            ),
            PackageState::Busy if started.elapsed() < SCAN_TIMEOUT => wait(started, &events).await,
            PackageState::Invalid if started.elapsed() < INVALID_GRACE => {
                wait(started, &events).await
            }
            PackageState::Busy => anyhow::bail!(
                "Антивирус слишком долго проверяет установщик. Подождите и нажмите «Повторить»"
            ),
            PackageState::Invalid => {
                let _ = std::fs::remove_file(&path);
                anyhow::bail!(
                    "Установщик повреждён или заблокирован антивирусом. Нажмите «Повторить»"
                )
            }
        }
    }

    std::process::Command::new("msiexec")
        .args(["/i", path.to_string_lossy().as_ref()])
        .spawn()?;
    Ok(())
}

#[cfg(windows)]
async fn download(url: &str, path: &std::path::Path, events: &EventSink) -> anyhow::Result<()> {
    use crate::events::AppEvent;
    use futures_util::StreamExt;
    use tokio::io::AsyncWriteExt;

    let resp = Client::new().get(url).send().await?.error_for_status()?;
    let total = resp.content_length().unwrap_or(0);

    let part = path.with_extension("msi.part");
    let mut file = tokio::fs::File::create(&part).await?;

    let mut stream = resp.bytes_stream();
    let mut downloaded: u64 = 0;
    events.emit(AppEvent::UpdateProgress { downloaded, total });

    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        file.write_all(&chunk).await?;
        downloaded += chunk.len() as u64;
        events.emit(AppEvent::UpdateProgress { downloaded, total });
    }

    file.flush().await?;
    drop(file);

    // The antivirus may still hold the .part file right after close, so retry the rename.
    let started = std::time::Instant::now();
    loop {
        match std::fs::rename(&part, path) {
            Ok(()) => return Ok(()),
            Err(e) if started.elapsed() < SCAN_TIMEOUT && is_busy(&e) => {
                wait(started, events).await
            }
            Err(e) => return Err(e.into()),
        }
    }
}

#[cfg(windows)]
async fn wait(started: std::time::Instant, events: &EventSink) {
    events.emit(crate::events::AppEvent::UpdateVerifying {
        waited_secs: started.elapsed().as_secs(),
    });
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
}

#[cfg(windows)]
enum PackageState {
    Ready,
    Busy,
    Invalid,
    Missing,
}

#[cfg(windows)]
fn is_busy(e: &std::io::Error) -> bool {
    const ERROR_ACCESS_DENIED: i32 = 5;
    const ERROR_SHARING_VIOLATION: i32 = 32;
    const ERROR_LOCK_VIOLATION: i32 = 33;
    matches!(
        e.raw_os_error(),
        Some(ERROR_ACCESS_DENIED | ERROR_SHARING_VIOLATION | ERROR_LOCK_VIOLATION)
    )
}

#[cfg(windows)]
fn package_state(path: &std::path::Path) -> PackageState {
    use std::os::windows::{ffi::OsStrExt, fs::OpenOptionsExt};
    use windows::{
        Win32::System::ApplicationInstallationAndServicing::MsiVerifyPackageW, core::PCWSTR,
    };

    // Exclusive open fails while a scanner still has the file open.
    match std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(path)
    {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return PackageState::Missing,
        Err(e) if is_busy(&e) => return PackageState::Busy,
        Err(_) => return PackageState::Invalid,
    }

    let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    match unsafe { MsiVerifyPackageW(PCWSTR(wide.as_ptr())) } {
        0 => PackageState::Ready,
        _ if !path.exists() => PackageState::Missing,
        _ => PackageState::Invalid,
    }
}
