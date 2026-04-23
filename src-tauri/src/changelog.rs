use serde::Serialize;

const CHANGELOG_MD: &str = include_str!("../../CHANGELOG.md");

#[derive(Debug, Serialize, specta::Type)]
pub struct ChangelogEntry {
    pub version: String,
    pub html: String,
}

pub fn bundled() -> &'static str {
    CHANGELOG_MD
}

fn split_versions(md: &str) -> Vec<(String, String)> {
    let mut result = Vec::new();
    let mut current_header: Option<String> = None;
    let mut current_body = String::new();

    for line in md.lines() {
        if let Some(rest) = line.strip_prefix("## ") {
            if let Some(h) = current_header.take() {
                result.push((h, std::mem::take(&mut current_body)));
            }
            current_header = Some(rest.trim().to_string());
        } else if current_header.is_some() {
            current_body.push_str(line);
            current_body.push('\n');
        }
    }
    if let Some(h) = current_header.take() {
        result.push((h, current_body));
    }
    result
}

fn parse_version(header: &str) -> Option<semver::Version> {
    let token = header.split_whitespace().next()?;
    let token = token.trim_start_matches('[').trim_end_matches(']');
    let token = token.strip_prefix('v').unwrap_or(token);
    semver::Version::parse(token).ok()
}

fn render_markdown(md: &str) -> String {
    let mut opts = pulldown_cmark::Options::empty();
    opts.insert(pulldown_cmark::Options::ENABLE_STRIKETHROUGH);
    opts.insert(pulldown_cmark::Options::ENABLE_TABLES);
    let parser = pulldown_cmark::Parser::new_ext(md, opts);
    let mut html = String::new();
    pulldown_cmark::html::push_html(&mut html, parser);
    html
}

pub fn pending_for(last_seen: Option<&str>, current: &str, md: &str) -> Vec<ChangelogEntry> {
    let Ok(current_v) = semver::Version::parse(current) else {
        return Vec::new();
    };
    let last_v = last_seen.and_then(|s| semver::Version::parse(s).ok());

    let mut entries: Vec<(semver::Version, String)> = split_versions(md)
        .into_iter()
        .filter_map(|(header, body)| parse_version(&header).map(|v| (v, body)))
        .filter(|(v, _)| *v <= current_v)
        .filter(|(v, _)| match &last_v {
            Some(last) => v > last,
            None => false,
        })
        .collect();

    entries.sort_by(|a, b| b.0.cmp(&a.0));
    entries
        .into_iter()
        .map(|(v, body)| ChangelogEntry {
            version: v.to_string(),
            html: render_markdown(&body),
        })
        .collect()
}

#[cfg(windows)]
pub fn read_last_seen() -> Option<String> {
    use winreg::RegKey;
    use winreg::enums::HKEY_CURRENT_USER;
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let key = hkcu.open_subkey("Software\\VoxelProxy").ok()?;
    key.get_value("LastSeenVersion").ok()
}

#[cfg(windows)]
pub fn write_last_seen(value: &str) -> Result<(), String> {
    use winreg::RegKey;
    use winreg::enums::HKEY_CURRENT_USER;
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (key, _) = hkcu
        .create_subkey("Software\\VoxelProxy")
        .map_err(|e| e.to_string())?;
    key.set_value("LastSeenVersion", &value.to_string())
        .map_err(|e| e.to_string())
}

#[cfg(not(windows))]
pub fn read_last_seen() -> Option<String> {
    None
}

#[cfg(not(windows))]
pub fn write_last_seen(_value: &str) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "# Changelog\n\n## 6.2.5\n- five\n\n## 6.2.4\n- four\n\n## 6.2.3\n- three\n";

    #[test]
    fn first_install_returns_empty() {
        let out = pending_for(None, "6.2.5", SAMPLE);
        assert!(out.is_empty());
    }

    #[test]
    fn same_version_returns_empty() {
        let out = pending_for(Some("6.2.5"), "6.2.5", SAMPLE);
        assert!(out.is_empty());
    }

    #[test]
    fn one_version_gap() {
        let out = pending_for(Some("6.2.4"), "6.2.5", SAMPLE);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].version, "6.2.5");
        assert!(out[0].html.contains("five"));
    }

    #[test]
    fn multi_version_gap_sorted_desc() {
        let out = pending_for(Some("6.2.2"), "6.2.5", SAMPLE);
        let versions: Vec<&str> = out.iter().map(|e| e.version.as_str()).collect();
        assert_eq!(versions, vec!["6.2.5", "6.2.4", "6.2.3"]);
    }

    #[test]
    fn downgrade_returns_empty() {
        let out = pending_for(Some("6.2.9"), "6.2.5", SAMPLE);
        assert!(out.is_empty());
    }

    #[test]
    fn caps_at_current_version() {
        let out = pending_for(Some("6.2.2"), "6.2.4", SAMPLE);
        let versions: Vec<&str> = out.iter().map(|e| e.version.as_str()).collect();
        assert_eq!(versions, vec!["6.2.4", "6.2.3"]);
    }

    #[test]
    fn tolerates_trailing_date_on_header() {
        let md = "## 6.2.5 — 2026-04-23\n- x\n\n## 6.2.4 - old\n- y\n";
        let out = pending_for(Some("6.2.3"), "6.2.5", md);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].version, "6.2.5");
    }

    #[test]
    fn skips_unparseable_headers() {
        let md = "## unreleased\n- x\n\n## 6.2.5\n- y\n";
        let out = pending_for(Some("6.2.4"), "6.2.5", md);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].version, "6.2.5");
    }

    #[test]
    fn bundled_file_parses() {
        let out = pending_for(Some("0.0.0"), env!("CARGO_PKG_VERSION"), bundled());
        assert!(!out.is_empty(), "bundled CHANGELOG.md should yield entries");
    }
}
