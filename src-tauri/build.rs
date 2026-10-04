fn main() {
    // export the gpu hints declared in main.rs
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        println!("cargo:rustc-link-arg-bins=/EXPORT:NvOptimusEnablement");
        println!("cargo:rustc-link-arg-bins=/EXPORT:AmdPowerXpressRequestHighPerformance");
    }
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(tauri_build::AppManifest::new().commands(&[
        "launcher_state",
        "check",
        "sync",
        "verify",
        "cancel_sync",
        "play",
        "prewarm",
        "presence",
        "game_fullscreen",
        "notes",
        "set_setting",
        "pick_install_dir",
        "clear_cache",
        "open_link",
        "desktop_shortcut",
        "quit_app",
        "install_launcher_update",
    ])))
    .expect("tauri build");
}
