fn main() {
    let target = std::env::var("TARGET").unwrap_or_default();

    // Только для Windows + MSVC
    if target.contains("windows-msvc") {
        println!("cargo:rustc-link-arg=/ENTRY:mainCRTStartup");
        println!("cargo:rustc-link-arg=/SUBSYSTEM:WINDOWS");
    }

    // Если хочешь также поддерживать gnu:
    // else if target.contains("windows-gnu") {
    //     println!("cargo:rustc-link-arg=-Wl,--subsystem,windows");
    // }
}
