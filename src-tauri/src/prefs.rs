#[cfg(windows)]
const SUBKEY: &str = "Software\\VoxelProxy";
#[cfg(windows)]
const MANUAL_WARNING_VALUE: &str = "ManualWarningAcknowledged";
#[cfg(windows)]
const LAST_SEEN_VERSION_VALUE: &str = "LastSeenVersion";

#[cfg(windows)]
pub fn manual_warning_acknowledged() -> bool {
    use winreg::RegKey;
    use winreg::enums::HKEY_CURRENT_USER;
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let Ok(key) = hkcu.open_subkey(SUBKEY) else {
        return false;
    };
    let val: Result<u32, _> = key.get_value(MANUAL_WARNING_VALUE);
    matches!(val, Ok(v) if v != 0)
}

#[cfg(windows)]
pub fn acknowledge_manual_warning() -> Result<(), String> {
    use winreg::RegKey;
    use winreg::enums::HKEY_CURRENT_USER;
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (key, _) = hkcu
        .create_subkey(SUBKEY)
        .map_err(|e| e.to_string())?;
    key.set_value(MANUAL_WARNING_VALUE, &1u32)
        .map_err(|e| e.to_string())
}

#[cfg(not(windows))]
pub fn manual_warning_acknowledged() -> bool {
    true
}

#[cfg(not(windows))]
pub fn acknowledge_manual_warning() -> Result<(), String> {
    Ok(())
}

#[cfg(windows)]
pub fn last_seen_version() -> Option<String> {
    use winreg::RegKey;
    use winreg::enums::HKEY_CURRENT_USER;
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let key = hkcu.open_subkey(SUBKEY).ok()?;
    key.get_value(LAST_SEEN_VERSION_VALUE).ok()
}

#[cfg(windows)]
pub fn set_last_seen_version(value: &str) -> Result<(), String> {
    use winreg::RegKey;
    use winreg::enums::HKEY_CURRENT_USER;
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (key, _) = hkcu.create_subkey(SUBKEY).map_err(|e| e.to_string())?;
    key.set_value(LAST_SEEN_VERSION_VALUE, &value.to_string())
        .map_err(|e| e.to_string())
}

#[cfg(not(windows))]
pub fn last_seen_version() -> Option<String> {
    None
}

#[cfg(not(windows))]
pub fn set_last_seen_version(_value: &str) -> Result<(), String> {
    Ok(())
}
