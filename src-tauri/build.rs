fn main() {
    tauri_build::build();

    println!("cargo:rerun-if-env-changed=TELEMETRY_URL");
    let telemetry_url = std::env::var("TELEMETRY_URL").unwrap_or_default();
    println!("cargo:rustc-env=TELEMETRY_URL={}", telemetry_url);

    println!("cargo:rerun-if-env-changed=UPDATE_URL");
    let update_url = std::env::var("UPDATE_URL").unwrap_or_default();
    println!("cargo:rustc-env=UPDATE_URL={}", update_url);

    #[cfg(target_os = "windows")]
    copy_windivert_files();
}

#[cfg(target_os = "windows")]
fn copy_windivert_files() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let out_dir = std::env::var("OUT_DIR").unwrap();

    let profile_dir = std::path::Path::new(&out_dir)
        .ancestors()
        .nth(3)
        .expect("Failed to resolve profile dir from OUT_DIR")
        .to_path_buf();

    let src_dir = std::path::Path::new(&manifest_dir).join("windivert");

    for file in &["WinDivert.dll", "WinDivert64.sys"] {
        let src = src_dir.join(file);
        let dst = profile_dir.join(file);
        if src.exists() {
            std::fs::copy(&src, &dst).ok();
        }
    }

    println!("cargo:rerun-if-changed=windivert/WinDivert.dll");
    println!("cargo:rerun-if-changed=windivert/WinDivert64.sys");
}
