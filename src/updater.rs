use reqwest::{
    Client,
    header::{HeaderMap, HeaderValue},
};
use semver::Version;
use serde_json::Value;
use std::time::Duration;

use crate::{events::EventSink, logger::Logger};

#[derive(Debug, Clone)]
pub struct NewVersion {
    pub tag: String,
    pub link: String,
}

struct Release {
    version: Version,
    link: String,
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

fn parse_version(raw: &str) -> anyhow::Result<Version> {
    let stripped = raw.strip_prefix('v').unwrap_or(raw);
    Version::parse(stripped).map_err(|e| anyhow::anyhow!("некорректная версия {raw:?}: {e}"))
}

async fn server_release() -> anyhow::Result<Release> {
    let release = crate::config::latest_release().await?;
    if release.download_url.is_empty() {
        anyhow::bail!("сервер не вернул ссылку на установщик");
    }
    Ok(Release {
        version: parse_version(&release.version)?,
        link: release.download_url,
    })
}

async fn github_release() -> anyhow::Result<Release> {
    if env!("UPDATE_URL").is_empty() {
        anyhow::bail!("UPDATE_URL не задан");
    }

    let release: Value = Client::builder()
        .timeout(Duration::from_secs(15))
        .build()?
        .get(env!("UPDATE_URL"))
        .headers(build_github_headers())
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    let tag = release
        .get("tag_name")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("Missing or invalid 'tag_name' field"))?;

    let link = release
        .get("assets")
        .and_then(Value::as_array)
        .and_then(|assets| {
            assets.iter().find(|asset| {
                asset
                    .get("name")
                    .and_then(Value::as_str)
                    .is_some_and(|name| name.to_ascii_lowercase().ends_with(".msi"))
            })
        })
        .and_then(|asset| asset.get("browser_download_url"))
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("Missing or invalid asset download URL"))?;

    Ok(Release {
        version: parse_version(tag)?,
        link: link.to_string(),
    })
}

/// Asks the update server first and falls back to GitHub; fails only if both fail.
pub async fn has_update(current_version: &str, log: &Logger) -> anyhow::Result<Option<NewVersion>> {
    if env!("TELEMETRY_URL").is_empty() && env!("UPDATE_URL").is_empty() {
        return Ok(None);
    }

    let current = parse_version(current_version)?;
    let release = match server_release().await {
        Ok(release) => release,
        Err(server_err) => {
            log.warn(format!(
                "Сервер обновлений недоступен ({server_err}), проверяю через GitHub"
            ));
            github_release().await.map_err(|github_err| {
                anyhow::anyhow!("сервер обновлений: {server_err}; GitHub: {github_err}")
            })?
        }
    };

    Ok((release.version > current).then(|| NewVersion {
        tag: format!("v{}", release.version),
        link: release.link,
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
