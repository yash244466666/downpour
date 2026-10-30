use downpour_types::EngineInfo;

/// Identity of the embedded engine, shown in the UI status bar.
///
/// The real engine is wired in M1; this proves the command bridge works.
#[tauri::command]
fn engine_info() -> EngineInfo {
    EngineInfo {
        version: env!("CARGO_PKG_VERSION").to_owned(),
        protocol_version: downpour_types::PROTOCOL_VERSION,
        os: std::env::consts::OS.to_owned(),
        arch: std::env::consts::ARCH.to_owned(),
    }
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![engine_info])
        .run(tauri::generate_context!())
        .expect("error while running Downpour");
}
