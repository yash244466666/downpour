//! Tauri shell for Downpour.
//!
//! The UI never touches SQLite, HTTP, or the filesystem directly: every action
//! goes through these commands, which drive a short-lived `downpour-core`
//! engine instance. Each running download owns its own engine (and SQLite
//! connection), so the UI stays responsive while a transfer runs.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use downpour_core::{CancelToken, Engine, EngineConfig, ProgressSample, RunOutcome};
use downpour_store::downloads::DownloadRecord;
use downpour_types::{AddDownloadRequest, DownloadId, EngineInfo};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};

struct AppState {
    db_path: PathBuf,
    config: Mutex<EngineConfig>,
    running: Mutex<HashMap<String, RunHandle>>,
    speeds: Mutex<HashMap<String, u64>>,
}

#[derive(Clone)]
struct RunHandle {
    cancel: CancelToken,
    cancel_after: Arc<AtomicBool>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn parse_id(raw: &str) -> Result<DownloadId, String> {
    DownloadId::from_str(raw).map_err(|_| format!("invalid download id: {raw}"))
}

fn open_engine(state: &AppState) -> Result<Engine, String> {
    let config = lock(&state.config).clone();
    // Recovery runs once in `load_state`; per-command engines must not touch
    // the state of a transfer that another engine is running.
    Engine::open_without_recovery(&state.db_path, config).map_err(|error| error.to_string())
}

/// UI-facing shape of a download.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DownloadView {
    id: String,
    url: String,
    name: String,
    directory: String,
    state: String,
    total_bytes: Option<u64>,
    downloaded_bytes: u64,
    speed_bps: u64,
    connections: u16,
    eta_seconds: Option<u64>,
    error: Option<String>,
    final_path: Option<String>,
    created_at_ms: i64,
    completed_at_ms: Option<i64>,
}

impl DownloadView {
    fn from_record(record: &DownloadRecord, speed_bps: u64) -> Self {
        let eta_seconds = match (record.total_bytes, speed_bps) {
            (Some(total), speed) if speed > 0 && total > record.downloaded_bytes => {
                Some((total - record.downloaded_bytes) / speed)
            }
            _ => None,
        };

        Self {
            id: record.id.to_string(),
            url: record.url.clone(),
            name: record.file_name.clone(),
            directory: record.dest_dir.clone(),
            state: record.state.as_str().to_owned(),
            total_bytes: record.total_bytes,
            downloaded_bytes: record.downloaded_bytes,
            speed_bps,
            connections: record.connections,
            eta_seconds,
            error: record.error.as_ref().map(ToString::to_string),
            final_path: record.final_path.clone(),
            created_at_ms: record.created_at_ms,
            completed_at_ms: record.completed_at_ms,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProgressEvent {
    id: String,
    downloaded_bytes: u64,
    speed_bps: u64,
    eta_seconds: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SettingsView {
    download_dir: String,
    connections: u16,
    speed_limit: u64,
    theme: String,
}

/// Identity of the embedded engine, shown in the UI status bar.
#[tauri::command]
fn engine_info() -> EngineInfo {
    EngineInfo {
        version: env!("CARGO_PKG_VERSION").to_owned(),
        protocol_version: downpour_types::PROTOCOL_VERSION,
        os: std::env::consts::OS.to_owned(),
        arch: std::env::consts::ARCH.to_owned(),
    }
}

#[tauri::command]
async fn list_downloads(state: State<'_, AppState>) -> Result<Vec<DownloadView>, String> {
    let engine = open_engine(&state)?;
    let records = engine.list(500).map_err(|error| error.to_string())?;
    let speeds = lock(&state.speeds).clone();
    Ok(records
        .iter()
        .map(|record| {
            let speed = speeds.get(&record.id.to_string()).copied().unwrap_or(0);
            DownloadView::from_record(record, speed)
        })
        .collect())
}

#[tauri::command]
async fn add_download(
    app: AppHandle,
    state: State<'_, AppState>,
    url: String,
    name: Option<String>,
    directory: Option<String>,
    connections: Option<u16>,
    start_now: bool,
) -> Result<String, String> {
    let mut engine = open_engine(&state)?;
    let id = engine
        .add(AddDownloadRequest {
            url,
            file_name: name,
            dest_dir: directory,
            connections,
            start_immediately: start_now,
            ..AddDownloadRequest::default()
        })
        .map_err(|error| error.to_string())?;

    if start_now {
        start_run(&app, &id.to_string())?;
    }
    let _ = app.emit("downloads-changed", ());
    Ok(id.to_string())
}

#[tauri::command]
async fn start_download(app: AppHandle, id: String) -> Result<(), String> {
    start_run(&app, &id)
}

#[tauri::command]
async fn pause_download(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let running = lock(&state.running).get(&id).cloned();
    if let Some(handle) = running {
        handle.cancel.cancel();
        return Ok(());
    }
    let mut engine = open_engine(&state)?;
    engine
        .pause(parse_id(&id)?)
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn cancel_download(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let running = lock(&state.running).get(&id).cloned();
    if let Some(handle) = running {
        handle.cancel_after.store(true, Ordering::SeqCst);
        handle.cancel.cancel();
        return Ok(());
    }
    let mut engine = open_engine(&state)?;
    engine
        .cancel(parse_id(&id)?)
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn remove_download(state: State<'_, AppState>, id: String) -> Result<(), String> {
    if lock(&state.running).contains_key(&id) {
        return Err("download is running; cancel it first".to_owned());
    }
    let mut engine = open_engine(&state)?;
    engine
        .remove(parse_id(&id)?)
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn get_settings(state: State<'_, AppState>) -> Result<SettingsView, String> {
    let engine = open_engine(&state)?;
    let config = lock(&state.config).clone();
    let download_dir = engine
        .setting("downloads.default_dir")
        .ok()
        .flatten()
        .and_then(|value| value.as_str().map(str::to_owned))
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| config.default_dir.to_string_lossy().into_owned());
    let connections = engine
        .setting("downloads.default_connections")
        .ok()
        .flatten()
        .and_then(|value| value.as_u64())
        .and_then(|value| u16::try_from(value).ok())
        .unwrap_or(config.default_connections)
        .clamp(1, 32);
    let speed_limit = engine
        .setting("network.speed_limit")
        .ok()
        .flatten()
        .and_then(|value| value.as_u64())
        .unwrap_or(config.global_speed_limit);
    let theme = engine
        .setting("ui.theme")
        .ok()
        .flatten()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| "system".to_owned());

    Ok(SettingsView {
        download_dir,
        connections,
        speed_limit,
        theme,
    })
}

#[tauri::command]
async fn set_settings(state: State<'_, AppState>, settings: SettingsView) -> Result<(), String> {
    if settings.download_dir.trim().is_empty() {
        return Err("download directory cannot be empty".to_owned());
    }
    if !(1..=32).contains(&settings.connections) {
        return Err("connections must be between 1 and 32".to_owned());
    }
    if !matches!(settings.theme.as_str(), "system" | "light" | "dark") {
        return Err("unknown theme".to_owned());
    }

    let mut engine = open_engine(&state)?;
    engine
        .set_setting(
            "downloads.default_dir",
            &serde_json::Value::String(settings.download_dir.clone()),
        )
        .map_err(|error| error.to_string())?;
    engine
        .set_setting(
            "downloads.default_connections",
            &serde_json::json!(settings.connections),
        )
        .map_err(|error| error.to_string())?;
    engine
        .set_setting(
            "network.speed_limit",
            &serde_json::json!(settings.speed_limit),
        )
        .map_err(|error| error.to_string())?;
    engine
        .set_setting(
            "ui.theme",
            &serde_json::Value::String(settings.theme.clone()),
        )
        .map_err(|error| error.to_string())?;

    {
        let mut config = lock(&state.config);
        config.default_dir = PathBuf::from(&settings.download_dir);
        config.default_connections = settings.connections;
        config.global_speed_limit = settings.speed_limit;
    }
    Ok(())
}

#[tauri::command]
async fn reveal_path(path: String) -> Result<(), String> {
    let path = PathBuf::from(path);
    let target = if path.is_dir() && path.exists() {
        path
    } else {
        path.parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."))
    };
    if !target.exists() {
        return Err(format!("{} does not exist yet", target.display()));
    }

    let spawn = if cfg!(target_os = "windows") {
        std::process::Command::new("explorer").arg(&target).spawn()
    } else if cfg!(target_os = "macos") {
        std::process::Command::new("open").arg(&target).spawn()
    } else {
        std::process::Command::new("xdg-open").arg(&target).spawn()
    };
    spawn.map(|_| ()).map_err(|error| error.to_string())
}

fn start_run(app: &AppHandle, id: &str) -> Result<(), String> {
    let parsed = parse_id(id)?;
    let handle = RunHandle {
        cancel: CancelToken::new(),
        cancel_after: Arc::new(AtomicBool::new(false)),
    };

    let (db_path, config) = {
        let state = app.state::<AppState>();
        {
            let mut running = lock(&state.running);
            if running.contains_key(id) {
                return Ok(());
            }
            running.insert(id.to_owned(), handle.clone());
        }
        (state.db_path.clone(), lock(&state.config).clone())
    };

    let app = app.clone();
    let id = id.to_owned();
    tauri::async_runtime::spawn(async move {
        run_download(app, id, parsed, db_path, config, handle).await;
    });
    Ok(())
}

async fn run_download(
    app: AppHandle,
    id: String,
    parsed: DownloadId,
    db_path: PathBuf,
    config: EngineConfig,
    handle: RunHandle,
) {
    let mut engine = match Engine::open_without_recovery(&db_path, config) {
        Ok(engine) => engine,
        Err(error) => {
            tracing::error!(%error, "cannot open engine for download");
            finish_run(&app, &id);
            return;
        }
    };

    let sink = {
        let app = app.clone();
        let id = id.clone();
        move |sample: ProgressSample| {
            let state = app.state::<AppState>();
            if sample.speed_bps > 0 {
                lock(&state.speeds).insert(id.clone(), sample.speed_bps);
            }
            let _ = app.emit(
                "download-progress",
                ProgressEvent {
                    id: id.clone(),
                    downloaded_bytes: sample.downloaded_bytes,
                    speed_bps: sample.speed_bps,
                    eta_seconds: sample.eta_seconds,
                },
            );
        }
    };

    let outcome = engine.run(parsed, handle.cancel.clone(), Some(&sink)).await;

    if handle.cancel_after.load(Ordering::SeqCst) {
        let _ = engine.cancel(parsed);
    }

    if matches!(outcome, Ok(RunOutcome::Completed)) {
        if let Ok(Some(record)) = engine.get(parsed) {
            notify_complete(&app, &record.file_name);
        }
    }

    finish_run(&app, &id);
    let _ = app.emit("downloads-changed", ());
}

fn finish_run(app: &AppHandle, id: &str) {
    let state = app.state::<AppState>();
    lock(&state.speeds).remove(id);
    lock(&state.running).remove(id);
}

fn notify_complete(app: &AppHandle, name: &str) {
    use tauri_plugin_notification::NotificationExt;
    let _ = app
        .notification()
        .builder()
        .title("Download complete")
        .body(name)
        .show();
}

fn load_state() -> AppState {
    let db_path = downpour_core::paths::database_path();
    let mut config = EngineConfig::default();

    if let Ok(engine) = Engine::open(&db_path, config.clone()) {
        if let Ok(Some(value)) = engine.setting("downloads.default_dir") {
            if let Some(dir) = value.as_str().filter(|dir| !dir.is_empty()) {
                config.default_dir = PathBuf::from(dir);
            }
        }
        if let Ok(Some(value)) = engine.setting("downloads.default_connections") {
            if let Some(connections) = value.as_u64() {
                config.default_connections = u16::try_from(connections).unwrap_or(8).clamp(1, 32);
            }
        }
        if let Ok(Some(value)) = engine.setting("network.speed_limit") {
            config.global_speed_limit = value.as_u64().unwrap_or(0);
        }
    }

    AppState {
        db_path,
        config: Mutex::new(config),
        running: Mutex::new(HashMap::new()),
        speeds: Mutex::new(HashMap::new()),
    }
}

fn build_tray(app: &mut tauri::App) -> tauri::Result<()> {
    use tauri::menu::{Menu, MenuItem};
    use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

    let show = MenuItem::with_id(app, "show", "Show Downpour", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &quit])?;

    let mut builder = TrayIconBuilder::with_id("downpour-tray")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => show_window(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_window(tray.app_handle());
            }
        });

    if let Some(icon) = app.default_window_icon().cloned() {
        builder = builder.icon(icon);
    }
    builder.build(app)?;
    Ok(())
}

fn show_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
            app.manage(load_state());
            build_tray(app)?;

            if let Some(window) = app.get_webview_window("main") {
                let window_for_event = window.clone();
                window.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        let _ = window_for_event.hide();
                    }
                });
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            engine_info,
            list_downloads,
            add_download,
            start_download,
            pause_download,
            cancel_download,
            remove_download,
            get_settings,
            set_settings,
            reveal_path
        ])
        .run(tauri::generate_context!())
        .expect("error while running Downpour");
}
