//! Media downloads: yt-dlp when available, native HLS fallback otherwise.
//!
//! These jobs update the same `downloads` rows as regular transfers so the UI
//! shows them uniformly. Pausing a media job stops the process; resuming
//! restarts it (yt-dlp/HLS have no cross-process resume here).
//!
//! SQLite is opened in short scopes, never held across an await: the task is
//! spawned on the Tauri runtime and must stay `Send`.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use downpour_media::{MediaKind, hls, ytdlp};
use downpour_store::Store;
use downpour_store::downloads;
use downpour_types::{DownloadId, DownloadState, ErrorCode, ErrorInfo};
use tauri::{AppHandle, Emitter, Manager};

use crate::{AppState, RunHandle, finish_run, lock, notify_complete, open_engine};

pub(crate) fn start_media(
    app: &AppHandle,
    id: DownloadId,
    url: String,
    kind: MediaKind,
    cookie_header: Option<String>,
) -> Result<(), String> {
    let id_string = id.to_string();
    let handle = RunHandle {
        cancel: downpour_core::CancelToken::new(),
        cancel_after: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
    };

    {
        let state = app.state::<AppState>();
        let mut running = lock(&state.running);
        if running.contains_key(&id_string) {
            return Ok(());
        }
        running.insert(id_string.clone(), handle.clone());
    }

    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        run_media(app, id, id_string, url, kind, cookie_header, handle).await;
    });
    Ok(())
}

async fn run_media(
    app: AppHandle,
    id: DownloadId,
    id_string: String,
    url: String,
    kind: MediaKind,
    cookie_header: Option<String>,
    handle: RunHandle,
) {
    let (db_path, config) = {
        let state = app.state::<AppState>();
        (state.db_path.clone(), lock(&state.config).clone())
    };

    let record = match open_engine(&app.state::<AppState>())
        .and_then(|engine| engine.get(id).map_err(|error| error.to_string()))
    {
        Ok(Some(record)) => record,
        Ok(None) => {
            finish_run(&app, &id_string);
            return;
        }
        Err(error) => {
            tracing::warn!(%error, "cannot load media download");
            finish_run(&app, &id_string);
            return;
        }
    };

    update_db(&db_path, |store| {
        let _ = downloads::mark_started(store.connection(), id);
    });

    let result = if let Some(binary) = ytdlp::detect(None) {
        run_ytdlp(
            &app,
            &db_path,
            &record,
            &url,
            &binary,
            cookie_header.as_deref(),
            &handle,
        )
        .await
    } else if kind == MediaKind::Hls {
        run_hls(&app, &db_path, &record, &url, &config.user_agent, &handle).await
    } else {
        Err("DASH downloads need yt-dlp installed".to_owned())
    };

    let canceled = handle.cancel.is_cancelled();
    let cancel_after = handle
        .cancel_after
        .load(std::sync::atomic::Ordering::SeqCst);

    if canceled {
        if cancel_after {
            if let Some(temp) = record.temp_path.as_deref() {
                let _ = std::fs::remove_file(temp);
            }
            if let Some(final_path) = record.final_path.as_deref() {
                let _ = std::fs::remove_file(final_path);
            }
            update_db(&db_path, |store| {
                let _ = downloads::set_state(store.connection(), id, DownloadState::Canceled);
            });
        } else {
            update_db(&db_path, |store| {
                let _ = downloads::set_state(store.connection(), id, DownloadState::Paused);
            });
        }
        finish_run(&app, &id_string);
        let _ = app.emit("downloads-changed", ());
        return;
    }

    match result {
        Ok(output) => {
            let bytes = std::fs::metadata(&output)
                .map(|meta| meta.len())
                .unwrap_or(0);
            let checksum = sha256_file(&output).await.unwrap_or_default();
            update_db(&db_path, |store| {
                let _ = downloads::set_paths(
                    store.connection(),
                    id,
                    &record.file_name,
                    &output.to_string_lossy(),
                    &output.to_string_lossy(),
                );
            });
            if let Ok(mut store) = Store::open(&db_path) {
                let _ =
                    downloads::mark_completed(store.connection_mut(), id, bytes, &checksum, None);
            }
            notify_complete(&app, &record.file_name);
        }
        Err(error) => {
            update_db(&db_path, |store| {
                let _ = downloads::set_error(
                    store.connection(),
                    id,
                    &ErrorInfo::new(ErrorCode::Internal, error),
                );
            });
        }
    }

    finish_run(&app, &id_string);
    let _ = app.emit("downloads-changed", ());
}

fn update_db(db_path: &Path, handler: impl FnOnce(&Store)) {
    if let Ok(store) = Store::open(db_path) {
        handler(&store);
    }
}

fn record_progress(db_path: &Path, id: DownloadId, downloaded: u64, total: Option<u64>) {
    update_db(db_path, |store| {
        if let Some(total) = total {
            let _ = downloads::set_total_bytes(store.connection(), id, total);
        }
        let _ = downloads::commit_progress(store.connection(), id, downloaded, None);
    });
}

async fn run_ytdlp(
    app: &AppHandle,
    db_path: &Path,
    record: &downpour_store::downloads::DownloadRecord,
    url: &str,
    binary: &Path,
    cookie_header: Option<&str>,
    handle: &RunHandle,
) -> Result<PathBuf, String> {
    let cookie_file = match cookie_header {
        Some(header) => {
            Some(ytdlp::write_cookie_file(url, header).map_err(|error| error.to_string())?)
        }
        None => None,
    };
    let output = PathBuf::from(
        record
            .final_path
            .as_deref()
            .ok_or_else(|| "download has no destination".to_owned())?,
    );

    let id = record.id;
    let id_string = id.to_string();
    let db_path = db_path.to_path_buf();
    let mut last_db_write = Instant::now() - Duration::from_secs(5);

    ytdlp::run(
        binary,
        url,
        &output,
        cookie_file.as_ref().map(|file| file.path()),
        None,
        &|| handle.cancel.is_cancelled(),
        |sample| {
            let state = app.state::<AppState>();
            if let Some(speed) = sample.speed_bps {
                lock(&state.speeds).insert(id_string.clone(), speed);
            }
            if last_db_write.elapsed() >= Duration::from_millis(500) {
                record_progress(&db_path, id, sample.downloaded_bytes, sample.total_bytes);
                last_db_write = Instant::now();
            }
        },
    )
    .await
    .map_err(|error| error.to_string())?;

    resolve_output(record, &output).ok_or_else(|| "yt-dlp produced no output file".to_owned())
}

/// yt-dlp may append or change the extension; find the newest matching file.
fn resolve_output(
    record: &downpour_store::downloads::DownloadRecord,
    expected: &Path,
) -> Option<PathBuf> {
    if expected.is_file() {
        return Some(expected.to_path_buf());
    }
    let directory = Path::new(&record.dest_dir);
    let stem = record
        .file_name
        .rsplit_once('.')
        .map(|(stem, _)| stem)
        .unwrap_or(&record.file_name);
    let mut best: Option<(std::time::SystemTime, PathBuf)> = None;
    for entry in std::fs::read_dir(directory).ok()? {
        let entry = entry.ok()?;
        let path = entry.path();
        let name = path.file_name()?.to_string_lossy();
        if !name.starts_with(stem) {
            continue;
        }
        let modified = entry.metadata().ok()?.modified().ok()?;
        if best
            .as_ref()
            .map(|(time, _)| modified > *time)
            .unwrap_or(true)
        {
            best = Some((modified, path));
        }
    }
    best.map(|(_, path)| path)
}

async fn run_hls(
    app: &AppHandle,
    db_path: &Path,
    record: &downpour_store::downloads::DownloadRecord,
    url: &str,
    user_agent: &str,
    handle: &RunHandle,
) -> Result<PathBuf, String> {
    let client = reqwest::Client::builder()
        .user_agent(user_agent.to_owned())
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()
        .map_err(|error| error.to_string())?;
    let playlist = url::Url::parse(url).map_err(|error| error.to_string())?;
    let temp = PathBuf::from(
        record
            .temp_path
            .as_deref()
            .ok_or_else(|| "download has no temp path".to_owned())?,
    );
    let final_path = PathBuf::from(
        record
            .final_path
            .as_deref()
            .ok_or_else(|| "download has no destination".to_owned())?,
    );

    let id = record.id;
    let id_string = id.to_string();
    let db_path = db_path.to_path_buf();
    let mut last_db_write = Instant::now() - Duration::from_secs(5);
    let mut last_bytes = 0_u64;
    let mut last_at = Instant::now();

    hls::download_hls(
        &client,
        &playlist,
        &temp,
        &|| handle.cancel.is_cancelled(),
        |bytes, _segments| {
            if last_db_write.elapsed() >= Duration::from_millis(500) {
                let elapsed = last_at.elapsed().as_secs_f64().max(0.001);
                let speed = (bytes.saturating_sub(last_bytes) as f64 / elapsed) as u64;
                last_bytes = bytes;
                last_at = Instant::now();
                let state = app.state::<AppState>();
                lock(&state.speeds).insert(id_string.clone(), speed);
                record_progress(&db_path, id, bytes, None);
                last_db_write = Instant::now();
            }
        },
    )
    .await
    .map_err(|error| error.to_string())?;

    tokio::fs::rename(&temp, &final_path)
        .await
        .map_err(|error| error.to_string())?;
    Ok(final_path)
}

async fn sha256_file(path: &Path) -> Result<String, std::io::Error> {
    use sha2::{Digest, Sha256};
    use tokio::io::AsyncReadExt;

    let mut file = tokio::fs::File::open(path).await?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 256 * 1024];
    loop {
        let read = file.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}
