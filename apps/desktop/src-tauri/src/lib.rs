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

use chrono::{Datelike, Timelike};
use downpour_core::schedule::{parse_minutes, window_allows};
use downpour_core::{CancelToken, Engine, EngineConfig, ProgressSample, RunOutcome};
use downpour_ipc::transport::{self, Stream};
use downpour_ipc::{IpcServer, Request, Response, read_frame, server_handshake, write_frame};
use downpour_store::downloads::DownloadRecord;
use downpour_types::{
    AddDownloadRequest, DownloadId, DownloadSource, DownloadState, EngineInfo, ErrorCode, ErrorInfo,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_autostart::ManagerExt;
use tokio::io::BufReader;

mod media;

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
    priority: i64,
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
            priority: record.priority,
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
    max_concurrent: u16,
    max_per_host: u16,
    schedule_enabled: bool,
    schedule_start: String,
    schedule_end: String,
    schedule_days: u8,
    autostart: bool,
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
    let records = engine.list(2000).map_err(|error| error.to_string())?;
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
    let _ = &state;
    queue_add(
        &app,
        AddDownloadRequest {
            url,
            file_name: name,
            dest_dir: directory,
            connections,
            start_immediately: start_now,
            ..AddDownloadRequest::default()
        },
        start_now,
        None,
    )
}

/// Queue a download and route HLS/DASH to the media pipeline.
fn queue_add(
    app: &AppHandle,
    mut request: AddDownloadRequest,
    start_now: bool,
    cookie_header: Option<String>,
) -> Result<String, String> {
    let url = request.url.clone();
    let kind = downpour_media::classify_url(&url).filter(|kind| {
        matches!(
            kind,
            downpour_media::MediaKind::Hls | downpour_media::MediaKind::Dash
        )
    });

    if kind.is_some() && request.file_name.is_none() {
        if let Ok(parsed) = url::Url::parse(&url) {
            let name = downpour_core::probe::filename_from_url(&parsed)
                .unwrap_or_else(|| "video".to_owned());
            let stem = name.rsplit_once('.').map(|(stem, _)| stem).unwrap_or(&name);
            request.file_name = Some(format!("{stem}.mp4"));
        }
    }

    let state = app.state::<AppState>();
    let mut engine = open_engine(&state)?;
    let id = engine.add(request).map_err(|error| error.to_string())?;

    if start_now {
        if let Some(kind) = kind {
            // Media jobs carry the browser cookie header, which is not
            // persisted; start them directly.
            media::start_media(app, id, url, kind, cookie_header)?;
        } else {
            // The scheduler enforces concurrency and per-host limits.
            scheduler_tick(app)?;
        }
    }
    let _ = app.emit("downloads-changed", ());
    Ok(id.to_string())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct MediaTools {
    yt_dlp: Option<String>,
    ffmpeg: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct UpdateStatus {
    configured: bool,
    version: Option<String>,
    message: String,
}

#[tauri::command]
async fn check_for_updates(app: AppHandle) -> UpdateStatus {
    use tauri_plugin_updater::UpdaterExt;

    let updater = match app.updater() {
        Ok(updater) => updater,
        Err(error) => {
            return UpdateStatus {
                configured: false,
                version: None,
                message: format!("Updates are not configured in this build: {error}"),
            };
        }
    };
    match updater.check().await {
        Ok(Some(update)) => UpdateStatus {
            configured: true,
            version: Some(update.version.clone()),
            message: format!("Version {} is available.", update.version),
        },
        Ok(None) => UpdateStatus {
            configured: true,
            version: None,
            message: "You are on the latest version.".to_owned(),
        },
        Err(error) => UpdateStatus {
            configured: false,
            version: None,
            message: format!("Update check failed: {error}"),
        },
    }
}

#[tauri::command]
fn expand_urls(input: String) -> Vec<String> {
    downpour_grabber::expand_lines(&input, 500)
}

#[tauri::command]
fn get_media_tools() -> MediaTools {
    let tools = downpour_media::detect_tools();
    MediaTools {
        yt_dlp: tools.yt_dlp.map(|path| path.display().to_string()),
        ffmpeg: tools.ffmpeg.map(|path| path.display().to_string()),
    }
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
async fn remove_download(
    state: State<'_, AppState>,
    id: String,
    delete_file: Option<bool>,
) -> Result<(), String> {
    if lock(&state.running).contains_key(&id) {
        return Err("download is running; cancel it first".to_owned());
    }
    let mut engine = open_engine(&state)?;
    engine
        .remove(parse_id(&id)?, delete_file.unwrap_or(false))
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn clear_finished(state: State<'_, AppState>) -> Result<usize, String> {
    let mut engine = open_engine(&state)?;
    engine.clear_finished().map_err(|error| error.to_string())
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BatchResult {
    added: usize,
    failed: Vec<String>,
}

#[tauri::command]
async fn add_downloads(
    app: AppHandle,
    state: State<'_, AppState>,
    urls: Vec<String>,
    directory: Option<String>,
    connections: Option<u16>,
) -> Result<BatchResult, String> {
    let mut engine = open_engine(&state)?;
    let mut added = 0_usize;
    let mut failed = Vec::new();

    for url in urls.into_iter().take(500) {
        let request = AddDownloadRequest {
            url: url.clone(),
            dest_dir: directory.clone(),
            connections,
            start_immediately: false,
            source: DownloadSource::Batch,
            ..AddDownloadRequest::default()
        };
        match engine.add(request) {
            Ok(_) => added += 1,
            Err(error) => failed.push(format!("{url}: {error}")),
        }
    }
    if added > 0 {
        scheduler_tick(&app)?;
    }
    let _ = app.emit("downloads-changed", ());
    Ok(BatchResult { added, failed })
}

fn setting_u64(engine: &Engine, key: &str) -> Option<u64> {
    engine
        .setting(key)
        .ok()
        .flatten()
        .and_then(|value| value.as_u64())
}

fn setting_bool(engine: &Engine, key: &str) -> Option<bool> {
    engine
        .setting(key)
        .ok()
        .flatten()
        .and_then(|value| value.as_bool())
}

fn setting_string(engine: &Engine, key: &str) -> Option<String> {
    engine
        .setting(key)
        .ok()
        .flatten()
        .and_then(|value| value.as_str().map(str::to_owned))
}

fn load_settings(app: &AppHandle, state: &AppState) -> Result<SettingsView, String> {
    let engine = open_engine(state)?;
    let config = lock(&state.config).clone();

    let download_dir = setting_string(&engine, "downloads.default_dir")
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| config.default_dir.to_string_lossy().into_owned());
    let connections = setting_u64(&engine, "downloads.default_connections")
        .and_then(|value| u16::try_from(value).ok())
        .unwrap_or(config.default_connections)
        .clamp(1, 32);
    let speed_limit =
        setting_u64(&engine, "network.speed_limit").unwrap_or(config.global_speed_limit);
    let theme = setting_string(&engine, "ui.theme").unwrap_or_else(|| "system".to_owned());
    let max_concurrent = setting_u64(&engine, "downloads.max_concurrent")
        .unwrap_or(3)
        .clamp(1, 16) as u16;
    let max_per_host = setting_u64(&engine, "downloads.max_per_host")
        .unwrap_or(4)
        .clamp(1, 32) as u16;
    let schedule_enabled = setting_bool(&engine, "schedule.enabled").unwrap_or(false);
    let schedule_start =
        setting_string(&engine, "schedule.start").unwrap_or_else(|| "00:00".to_owned());
    let schedule_end =
        setting_string(&engine, "schedule.end").unwrap_or_else(|| "23:59".to_owned());
    let schedule_days = setting_u64(&engine, "schedule.days")
        .unwrap_or(0b0111_1111)
        .min(0b0111_1111) as u8;
    let autostart = app.autolaunch().is_enabled().unwrap_or(false);

    Ok(SettingsView {
        download_dir,
        connections,
        speed_limit,
        theme,
        max_concurrent,
        max_per_host,
        schedule_enabled,
        schedule_start,
        schedule_end,
        schedule_days,
        autostart,
    })
}

#[tauri::command]
async fn get_settings(app: AppHandle, state: State<'_, AppState>) -> Result<SettingsView, String> {
    load_settings(&app, &state)
}

#[tauri::command]
async fn set_settings(
    app: AppHandle,
    state: State<'_, AppState>,
    settings: SettingsView,
) -> Result<(), String> {
    if settings.download_dir.trim().is_empty() {
        return Err("download directory cannot be empty".to_owned());
    }
    if !(1..=32).contains(&settings.connections) {
        return Err("connections must be between 1 and 32".to_owned());
    }
    if !(1..=16).contains(&settings.max_concurrent) {
        return Err("maximum concurrent downloads must be between 1 and 16".to_owned());
    }
    if !(1..=32).contains(&settings.max_per_host) {
        return Err("per-host limit must be between 1 and 32".to_owned());
    }
    if settings.schedule_enabled
        && (parse_minutes(&settings.schedule_start).is_none()
            || parse_minutes(&settings.schedule_end).is_none())
    {
        return Err("schedule times must use HH:MM".to_owned());
    }
    if settings.schedule_enabled && settings.schedule_days == 0 {
        return Err("choose at least one schedule day".to_owned());
    }
    if !matches!(settings.theme.as_str(), "system" | "light" | "dark") {
        return Err("unknown theme".to_owned());
    }

    let mut engine = open_engine(&state)?;
    let entries: [(&str, serde_json::Value); 10] = [
        (
            "downloads.default_dir",
            serde_json::Value::String(settings.download_dir.clone()),
        ),
        (
            "downloads.default_connections",
            serde_json::json!(settings.connections),
        ),
        (
            "network.speed_limit",
            serde_json::json!(settings.speed_limit),
        ),
        (
            "ui.theme",
            serde_json::Value::String(settings.theme.clone()),
        ),
        (
            "downloads.max_concurrent",
            serde_json::json!(settings.max_concurrent),
        ),
        (
            "downloads.max_per_host",
            serde_json::json!(settings.max_per_host),
        ),
        (
            "schedule.enabled",
            serde_json::json!(settings.schedule_enabled),
        ),
        (
            "schedule.start",
            serde_json::Value::String(settings.schedule_start.clone()),
        ),
        (
            "schedule.end",
            serde_json::Value::String(settings.schedule_end.clone()),
        ),
        ("schedule.days", serde_json::json!(settings.schedule_days)),
    ];
    for (key, value) in entries {
        engine
            .set_setting(key, &value)
            .map_err(|error| error.to_string())?;
    }

    {
        let mut config = lock(&state.config);
        config.default_dir = PathBuf::from(&settings.download_dir);
        config.default_connections = settings.connections;
        config.global_speed_limit = settings.speed_limit;
    }

    let autolaunch = app.autolaunch();
    let result = if settings.autostart {
        autolaunch.enable()
    } else {
        autolaunch.disable()
    };
    result.map_err(|error| error.to_string())?;
    Ok(())
}

#[tauri::command]
async fn set_priority(state: State<'_, AppState>, id: String, priority: i64) -> Result<(), String> {
    let mut engine = open_engine(&state)?;
    engine
        .set_priority(parse_id(&id)?, priority)
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn pause_all(state: State<'_, AppState>) -> Result<(), String> {
    let handles: Vec<RunHandle> = lock(&state.running).values().cloned().collect();
    for handle in handles {
        handle.cancel.cancel();
    }
    let mut engine = open_engine(&state)?;
    for record in engine.list_runnable().map_err(|error| error.to_string())? {
        if record.state == DownloadState::Queued {
            engine.pause(record.id).map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

#[tauri::command]
async fn start_all(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    let mut engine = open_engine(&state)?;
    for record in engine.list_runnable().map_err(|error| error.to_string())? {
        if record.state == DownloadState::Paused {
            engine
                .requeue(record.id)
                .map_err(|error| error.to_string())?;
        }
    }
    scheduler_tick(&app)
}

fn host_of(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|parsed| parsed.host_str().map(str::to_owned))
        .unwrap_or_else(|| "direct".to_owned())
}

/// Start queued downloads within the concurrency, per-host, and schedule
/// limits. Safe to call often; it does nothing when no slot is free.
fn scheduler_tick(app: &AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    let settings = load_settings(app, &state)?;

    let now = chrono::Local::now();
    let weekday = now.weekday().num_days_from_monday() as u8;
    let minutes = now.hour() as u16 * 60 + now.minute() as u16;
    let start = parse_minutes(&settings.schedule_start).unwrap_or(0);
    let end = parse_minutes(&settings.schedule_end).unwrap_or(1439);
    let inside_window = !settings.schedule_enabled
        || window_allows(settings.schedule_days, start, end, weekday, minutes);

    if !inside_window {
        let handles: Vec<RunHandle> = lock(&state.running).values().cloned().collect();
        for handle in handles {
            handle.cancel.cancel();
        }
        return Ok(());
    }

    let engine = open_engine(&state)?;
    let records = engine.list(2000).map_err(|error| error.to_string())?;
    let running_ids: std::collections::HashSet<String> =
        lock(&state.running).keys().cloned().collect();

    let mut slots = usize::from(settings.max_concurrent);
    let mut per_host: HashMap<String, usize> = HashMap::new();
    for record in &records {
        if running_ids.contains(&record.id.to_string()) {
            slots = slots.saturating_sub(1);
            *per_host.entry(host_of(&record.url)).or_default() += 1;
        }
    }

    for record in engine.list_runnable().map_err(|error| error.to_string())? {
        if record.state != DownloadState::Queued {
            continue;
        }
        let id = record.id.to_string();
        if running_ids.contains(&id) {
            continue;
        }
        if slots == 0 {
            break;
        }
        let host = host_of(&record.url);
        if per_host.get(&host).copied().unwrap_or(0) >= usize::from(settings.max_per_host) {
            continue;
        }
        let media_kind = downpour_media::classify_url(&record.url).filter(|kind| {
            matches!(
                kind,
                downpour_media::MediaKind::Hls | downpour_media::MediaKind::Dash
            )
        });
        let started = if let Some(kind) = media_kind {
            media::start_media(app, record.id, record.url.clone(), kind, None).is_ok()
        } else {
            start_run(app, &id).is_ok()
        };
        if started {
            slots -= 1;
            *per_host.entry(host).or_default() += 1;
        }
    }
    Ok(())
}

fn expand_tilde(value: &str) -> PathBuf {
    if value == "~" {
        return std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(value));
    }
    if let Some(rest) = value.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(value)
}

/// Command for a system helper.
///
/// AppImages prepend their mount to `PATH` and set `LD_LIBRARY_PATH` (plus
/// toolkit paths) to their bundled libraries. Running `xdg-open`/`gio` with
/// those variables makes the system opener load the AppImage's libraries and
/// fail silently, so they are stripped here.
fn system_command(program: &str) -> std::process::Command {
    let mut command = std::process::Command::new(program);
    for variable in [
        "LD_LIBRARY_PATH",
        "LD_PRELOAD",
        "GIO_MODULE_DIR",
        "GTK_PATH",
        "GDK_PIXBUF_MODULE_FILE",
        "GSETTINGS_SCHEMA_DIR",
    ] {
        command.env_remove(variable);
    }
    command
}

#[cfg(target_os = "linux")]
fn reveal_in_file_manager(path: &Path) -> Result<(), String> {
    // 1. The freedesktop file-manager interface selects the file (or opens the
    //    folder) reliably under Nemo, Nautilus, Dolphin, Thunar, and PCManFM.
    if let Ok(uri) = url::Url::from_file_path(path) {
        let argument = format!("array:string:{uri}");
        let status = system_command("dbus-send")
            .args([
                "--session",
                "--dest=org.freedesktop.FileManager1",
                "--type=method_call",
                "/org/freedesktop/FileManager1",
                "org.freedesktop.FileManager1.ShowItems",
                &argument,
                "string:",
            ])
            .status();
        if matches!(status, Ok(status) if status.success()) {
            return Ok(());
        }
    }

    // 2. Fall back to opening the containing folder.
    let target = if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| path.to_path_buf())
    };
    let target = target.to_string_lossy().into_owned();
    let mut last_error = None;
    for (program, args) in [
        ("gio", vec!["open".to_owned(), target.clone()]),
        ("xdg-open", vec![target.clone()]),
    ] {
        match system_command(program).args(&args).status() {
            Ok(status) if status.success() => return Ok(()),
            Ok(status) => last_error = Some(format!("{program} exited with {status}")),
            Err(error) => last_error = Some(format!("{program}: {error}")),
        }
    }
    Err(last_error.unwrap_or_else(|| "no file manager is available".to_owned()))
}

#[cfg(target_os = "macos")]
fn reveal_in_file_manager(path: &Path) -> Result<(), String> {
    let mut command = system_command("open");
    if !path.is_dir() {
        command.arg("-R");
    }
    command
        .arg(path)
        .status()
        .map_err(|error| error.to_string())
        .and_then(|status| {
            if status.success() {
                Ok(())
            } else {
                Err(format!("open exited with {status}"))
            }
        })
}

#[cfg(target_os = "windows")]
fn reveal_in_file_manager(path: &Path) -> Result<(), String> {
    let mut command = system_command("explorer");
    if path.is_dir() {
        command.arg(path);
    } else {
        command.arg(format!("/select,{}", path.display()));
    }
    // Explorer commonly exits non-zero even when it opens the window.
    command
        .spawn()
        .map(|_| ())
        .map_err(|error| error.to_string())
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn reveal_in_file_manager(_path: &Path) -> Result<(), String> {
    Err("revealing files is not supported on this platform".to_owned())
}

#[tauri::command]
async fn reveal_path(path: String) -> Result<(), String> {
    let path = expand_tilde(&path);
    let target = if path.exists() {
        path
    } else if let Some(parent) = path.parent().filter(|parent| parent.exists()) {
        parent.to_path_buf()
    } else {
        return Err(format!("{} does not exist yet", path.display()));
    };
    reveal_in_file_manager(&target)
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

#[derive(Debug, Deserialize)]
struct IpcAddParams {
    url: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    directory: Option<String>,
    #[serde(default)]
    connections: Option<u16>,
    #[serde(default)]
    start_now: bool,
    #[serde(default)]
    referer: Option<String>,
    #[serde(default)]
    user_agent: Option<String>,
    #[serde(default)]
    cookie_header: Option<String>,
    #[serde(default)]
    headers: BTreeMap<String, String>,
}

fn parse_cookie_header(value: &str) -> BTreeMap<String, String> {
    value
        .split(';')
        .filter_map(|pair| {
            let (name, value) = pair.trim().split_once('=')?;
            (!name.is_empty()).then(|| (name.to_owned(), value.to_owned()))
        })
        .collect()
}

fn ipc_error(id: u64, message: impl Into<String>) -> Response {
    Response::error(id, ErrorInfo::new(ErrorCode::Internal, message.into()))
}

fn ipc_dispatch(app: &AppHandle, request: &Request) -> Response {
    let id = request.id;
    match request.method.as_str() {
        "ping" | "engine.info" => match serde_json::to_value(engine_info()) {
            Ok(value) => Response::ok(id, value),
            Err(error) => ipc_error(id, error.to_string()),
        },
        "download.list" => {
            let state = app.state::<AppState>();
            let engine = match open_engine(&state) {
                Ok(engine) => engine,
                Err(error) => return ipc_error(id, error),
            };
            let records = match engine.list(500) {
                Ok(records) => records,
                Err(error) => return ipc_error(id, error.to_string()),
            };
            let speeds = lock(&state.speeds).clone();
            let views: Vec<DownloadView> = records
                .iter()
                .map(|record| {
                    let speed = speeds.get(&record.id.to_string()).copied().unwrap_or(0);
                    DownloadView::from_record(record, speed)
                })
                .collect();
            match serde_json::to_value(views) {
                Ok(value) => Response::ok(id, value),
                Err(error) => ipc_error(id, error.to_string()),
            }
        }
        "download.add" => {
            let params: IpcAddParams = match serde_json::from_value(request.params.clone()) {
                Ok(params) => params,
                Err(error) => return ipc_error(id, format!("invalid params: {error}")),
            };
            let state = app.state::<AppState>();
            let mut engine = match open_engine(&state) {
                Ok(engine) => engine,
                Err(error) => return ipc_error(id, error),
            };
            let cookies = params
                .cookie_header
                .as_deref()
                .map(parse_cookie_header)
                .unwrap_or_default();
            let _ = &mut engine;
            let cookie_header = params.cookie_header.clone();
            match queue_add(
                app,
                AddDownloadRequest {
                    url: params.url,
                    file_name: params.name,
                    dest_dir: params.directory,
                    connections: params.connections,
                    referer: params.referer,
                    user_agent: params.user_agent,
                    cookies,
                    ephemeral_headers: params.headers,
                    start_immediately: params.start_now,
                    source: DownloadSource::BrowserExtension,
                    ..AddDownloadRequest::default()
                },
                params.start_now,
                cookie_header,
            ) {
                Ok(download_id) => Response::ok(id, serde_json::json!({ "id": download_id })),
                Err(error) => ipc_error(id, error),
            }
        }
        "download.pause" => ipc_id_method(app, request, |state, parsed, raw| {
            if let Some(handle) = lock(&state.running).get(raw).cloned() {
                handle.cancel.cancel();
                return Ok(());
            }
            let mut engine = open_engine(state)?;
            engine.pause(parsed).map_err(|error| error.to_string())
        }),
        "download.resume" => ipc_id_method(app, request, |state, parsed, raw| {
            let mut engine = open_engine(state)?;
            engine.requeue(parsed).map_err(|error| error.to_string())?;
            start_run(app, raw)
        }),
        "download.cancel" => ipc_id_method(app, request, |state, parsed, raw| {
            if let Some(handle) = lock(&state.running).get(raw).cloned() {
                handle
                    .cancel_after
                    .store(true, std::sync::atomic::Ordering::SeqCst);
                handle.cancel.cancel();
                return Ok(());
            }
            let mut engine = open_engine(state)?;
            engine.cancel(parsed).map_err(|error| error.to_string())
        }),
        "download.remove" => {
            let delete_file = request
                .params
                .get("delete_file")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            ipc_id_method(app, request, |state, parsed, _raw| {
                let mut engine = open_engine(state)?;
                engine
                    .remove(parsed, delete_file)
                    .map_err(|error| error.to_string())
            })
        }
        other => ipc_error(id, format!("unknown method: {other}")),
    }
}

fn ipc_id_method(
    app: &AppHandle,
    request: &Request,
    handler: impl FnOnce(&AppState, DownloadId, &str) -> Result<(), String>,
) -> Response {
    let id = request.id;
    let raw = match request.params.get("id").and_then(serde_json::Value::as_str) {
        Some(raw) => raw.to_owned(),
        None => return ipc_error(id, "missing id"),
    };
    let parsed = match parse_id(&raw) {
        Ok(parsed) => parsed,
        Err(error) => return ipc_error(id, error),
    };
    let state = app.state::<AppState>();
    match handler(&state, parsed, &raw) {
        Ok(()) => {
            let _ = app.emit("downloads-changed", ());
            Response::ok(id, serde_json::json!({ "ok": true }))
        }
        Err(error) => ipc_error(id, error),
    }
}

async fn run_ipc_server(app: AppHandle) {
    let token = match transport::generate_token().and_then(|token| {
        transport::write_token(&token)?;
        Ok(token)
    }) {
        Ok(token) => token,
        Err(error) => {
            tracing::error!(%error, "cannot create the IPC token");
            return;
        }
    };

    let server = match IpcServer::bind() {
        Ok(server) => server,
        Err(error) => {
            tracing::error!(%error, "cannot bind the IPC socket");
            return;
        }
    };

    loop {
        match server.accept().await {
            Ok(stream) => {
                let app = app.clone();
                let token = token.clone();
                tauri::async_runtime::spawn(handle_ipc_client(app, stream, token));
            }
            Err(error) => {
                tracing::warn!(%error, "IPC accept failed");
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
        }
    }
}

async fn handle_ipc_client(app: AppHandle, stream: Stream, token: String) {
    let mut connection = BufReader::new(stream);
    let hello = match server_handshake(&mut connection, &token).await {
        Ok(hello) => hello,
        Err(error) => {
            tracing::warn!(%error, "IPC handshake rejected");
            return;
        }
    };
    tracing::debug!(client = ?hello.client, "IPC client connected");

    loop {
        let frame = match read_frame(&mut connection).await {
            Ok(Some(frame)) => frame,
            Ok(None) => break,
            Err(error) => {
                tracing::warn!(%error, "IPC frame error");
                break;
            }
        };
        let request: Request = match serde_json::from_slice(&frame) {
            Ok(request) => request,
            Err(error) => {
                let response = ipc_error(0, format!("invalid request: {error}"));
                let _ = write_frame(&mut connection, &response).await;
                continue;
            }
        };
        let response = ipc_dispatch(&app, &request);
        if write_frame(&mut connection, &response).await.is_err() {
            break;
        }
    }
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
    let context = tauri::generate_context!();
    // The updater plugin refuses to initialize without a `plugins.updater`
    // config. Register it only when a release build provides one; otherwise
    // `check_for_updates` reports that updates are not configured.
    let updater_configured = context.config().plugins.0.contains_key("updater");

    let mut builder = tauri::Builder::default()
        // Must be the first plugin: a second launch focuses the running app.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            show_window(app);
        }))
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init());

    if updater_configured {
        builder = builder.plugin(tauri_plugin_updater::Builder::new().build());
    }

    builder
        .setup(|app| {
            app.manage(load_state());
            build_tray(app)?;

            let ipc_app = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                run_ipc_server(ipc_app).await;
            });

            let scheduler_app = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
                    if let Err(error) = scheduler_tick(&scheduler_app) {
                        tracing::warn!(%error, "scheduler tick failed");
                    }
                }
            });

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
            get_media_tools,
            expand_urls,
            check_for_updates,
            list_downloads,
            add_download,
            add_downloads,
            clear_finished,
            start_download,
            pause_download,
            cancel_download,
            remove_download,
            get_settings,
            set_settings,
            set_priority,
            pause_all,
            start_all,
            reveal_path
        ])
        .run(context)
        .expect("error while running Downpour");
}
