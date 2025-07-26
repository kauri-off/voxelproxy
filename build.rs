fn main() {
    #[cfg(not(debug_assertions))]
    let target = std::env::var("TARGET").unwrap_or_default();

    #[cfg(not(debug_assertions))]
    if target.contains("windows-msvc") {
        println!("cargo:rustc-link-arg=/ENTRY:mainCRTStartup");
        println!("cargo:rustc-link-arg=/SUBSYSTEM:WINDOWS");
    }
}
