const MANUAL_WARNING_VALUE: &str = "ManualWarningAcknowledged";
const LAST_SEEN_VERSION_VALUE: &str = "LastSeenVersion";
const INSTALL_ID_VALUE: &str = "InstallId";
const LAST_READ_REPLY_VALUE: &str = "LastReadReplyAt";

#[cfg(windows)]
mod store {
    use winreg::RegKey;
    use winreg::enums::HKEY_CURRENT_USER;

    const SUBKEY: &str = "Software\\VoxelProxy";

    pub fn get_string(name: &str) -> Option<String> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        hkcu.open_subkey(SUBKEY).ok()?.get_value(name).ok()
    }

    pub fn set_string(name: &str, value: &str) -> Result<(), String> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let (key, _) = hkcu.create_subkey(SUBKEY).map_err(|e| e.to_string())?;
        key.set_value(name, &value.to_string())
            .map_err(|e| e.to_string())
    }

    pub fn get_flag(name: &str) -> bool {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let Ok(key) = hkcu.open_subkey(SUBKEY) else {
            return false;
        };
        matches!(key.get_value::<u32, _>(name), Ok(v) if v != 0)
    }

    pub fn set_flag(name: &str) -> Result<(), String> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let (key, _) = hkcu.create_subkey(SUBKEY).map_err(|e| e.to_string())?;
        key.set_value(name, &1u32).map_err(|e| e.to_string())
    }
}

#[cfg(not(windows))]
mod store {
    use std::path::PathBuf;

    use serde_json::{Map, Value};

    fn path() -> Option<PathBuf> {
        Some(dirs::config_dir()?.join("voxelproxy").join("prefs.json"))
    }

    fn load() -> Map<String, Value> {
        path()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    fn save(map: &Map<String, Value>) -> Result<(), String> {
        let path = path().ok_or("config dir is unavailable")?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let data = serde_json::to_vec_pretty(map).map_err(|e| e.to_string())?;
        std::fs::write(path, data).map_err(|e| e.to_string())
    }

    pub fn get_string(name: &str) -> Option<String> {
        load().get(name)?.as_str().map(str::to_owned)
    }

    pub fn set_string(name: &str, value: &str) -> Result<(), String> {
        let mut map = load();
        map.insert(name.to_owned(), Value::String(value.to_owned()));
        save(&map)
    }

    pub fn get_flag(name: &str) -> bool {
        load().get(name).and_then(Value::as_bool).unwrap_or(false)
    }

    pub fn set_flag(name: &str) -> Result<(), String> {
        let mut map = load();
        map.insert(name.to_owned(), Value::Bool(true));
        save(&map)
    }
}

pub fn manual_warning_acknowledged() -> bool {
    store::get_flag(MANUAL_WARNING_VALUE)
}

pub fn acknowledge_manual_warning() -> Result<(), String> {
    store::set_flag(MANUAL_WARNING_VALUE)
}

pub fn last_seen_version() -> Option<String> {
    store::get_string(LAST_SEEN_VERSION_VALUE)
}

pub fn set_last_seen_version(value: &str) -> Result<(), String> {
    store::set_string(LAST_SEEN_VERSION_VALUE, value)
}

/// Persistent per-installation id, created on first use.
pub fn install_id() -> String {
    if let Some(id) = store::get_string(INSTALL_ID_VALUE).filter(|s| !s.is_empty()) {
        return id;
    }
    let id = uuid::Uuid::new_v4().to_string();
    let _ = store::set_string(INSTALL_ID_VALUE, &id);
    id
}

pub fn last_read_reply_at() -> i64 {
    store::get_string(LAST_READ_REPLY_VALUE)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

pub fn set_last_read_reply_at(value: i64) -> Result<(), String> {
    store::set_string(LAST_READ_REPLY_VALUE, &value.to_string())
}
