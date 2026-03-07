fn main() {
    #[cfg(not(debug_assertions))]
    let target = std::env::var("TARGET").unwrap_or_default();

    #[cfg(not(debug_assertions))]
    if target.contains("windows-msvc") {
        println!("cargo:rustc-link-arg=/ENTRY:mainCRTStartup");
        println!("cargo:rustc-link-arg=/SUBSYSTEM:WINDOWS");
    }

    // Copy WinDivert runtime files to the exe output directory
    copy_windivert_files();
    tauri_build::build()
}

fn copy_windivert_files() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let out_dir = std::env::var("OUT_DIR").unwrap();

    // OUT_DIR is .../target/{profile}/build/{crate}-{hash}/out
    // Go up 3 levels to reach .../target/{profile}
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
            std::fs::copy(&src, &dst).unwrap_or_else(|e| {
                panic!(
                    "Failed to copy {} to {}: {}",
                    src.display(),
                    dst.display(),
                    e
                )
            });
        }
    }

    // Rerun if any windivert file changes
    println!("cargo:rerun-if-changed=windivert/WinDivert.dll");
    println!("cargo:rerun-if-changed=windivert/WinDivert64.sys");
}
