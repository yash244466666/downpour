//! Engine orchestration: probe -> plan -> transfer -> checkpoint -> finalize.
//!
//! M1 runs a single stream per download. The I/O shape is already the one the
//! segmented engine needs: one writer thread owns the file and receives
//! `(offset, bytes)` messages, so M2 can add segment workers without touching
//! the crash-safety rules.
//!
//! Concurrency across downloads arrives with the queue (M4); one `Engine`
//! drives one download at a time.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use downpour_store::Store;
use downpour_store::downloads::{
    self, DownloadRecord, NewDownload, ProbeUpdate, SegmentRecord, SegmentState,
};
use downpour_types::{AddDownloadRequest, DownloadId, DownloadState, SegmentSpec};
use futures_util::StreamExt;
use reqwest::header::{
    self, ACCEPT_ENCODING, AUTHORIZATION, CONTENT_RANGE, COOKIE, ETAG, HeaderMap, HeaderName,
    HeaderValue, IF_RANGE, LAST_MODIFIED, PROXY_AUTHORIZATION, RANGE, RETRY_AFTER,
};
use reqwest::{Client, StatusCode, Url};

use crate::cancel::CancelToken;
use crate::error::{EngineError, EngineResult};
use crate::filename::{sanitize_file_name, unique_file_name};
use crate::probe::{
    ProbeResult, filename_from_content_disposition, filename_from_url, parse_content_range, probe,
};
use crate::writer::ChunkWriter;

/// Backoff policy for transient network failures.
#[derive(Debug, Clone, Copy)]
pub struct RetryPolicy {
    /// Consecutive failures tolerated without checkpoint progress.
    pub max_attempts: u32,
    pub base_delay: Duration,
    pub max_delay: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 5,
            base_delay: Duration::from_millis(500),
            max_delay: Duration::from_secs(30),
        }
    }
}

/// Engine-wide configuration.
#[derive(Debug, Clone)]
pub struct EngineConfig {
    pub default_dir: PathBuf,
    pub user_agent: String,
    pub retry: RetryPolicy,
    pub checkpoint_interval: Duration,
    pub checkpoint_bytes: u64,
    pub connect_timeout: Duration,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            default_dir: crate::paths::default_download_dir(),
            user_agent: format!("Downpour/{}", env!("CARGO_PKG_VERSION")),
            retry: RetryPolicy::default(),
            checkpoint_interval: Duration::from_secs(1),
            checkpoint_bytes: 8 * 1024 * 1024,
            connect_timeout: Duration::from_secs(30),
        }
    }
}

/// Coalesced progress sample; emitted at most once per checkpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProgressSample {
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub speed_bps: u64,
    pub eta_seconds: Option<u64>,
}

/// Progress observer. Called on the transfer task; keep it cheap.
pub type ProgressSink<'a> = &'a (dyn Fn(ProgressSample) + Send + Sync);

/// How a run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunOutcome {
    Completed,
    Paused,
}

/// The download engine.
pub struct Engine {
    store: Store,
    client: Client,
    config: EngineConfig,
}

impl Engine {
    /// Open (or create) a database and recover interrupted downloads.
    pub fn open(db_path: impl AsRef<Path>, config: EngineConfig) -> EngineResult<Self> {
        if let Some(parent) = db_path.as_ref().parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let store = Store::open(db_path)?;
        Self::with_store(store, config)
    }

    /// In-memory engine for tests.
    pub fn open_in_memory(config: EngineConfig) -> EngineResult<Self> {
        let store = Store::open_in_memory()?;
        Self::with_store(store, config)
    }

    fn with_store(store: Store, config: EngineConfig) -> EngineResult<Self> {
        let recovered = downloads::recover_interrupted(store.connection())?;
        if recovered > 0 {
            tracing::info!(recovered, "marked interrupted downloads as paused");
        }

        let client = Client::builder()
            .user_agent(config.user_agent.clone())
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(config.connect_timeout)
            .build()?;

        Ok(Self {
            store,
            client,
            config,
        })
    }

    /// Effective engine configuration.
    #[must_use]
    pub fn config(&self) -> &EngineConfig {
        &self.config
    }

    /// Queue a download. Probing happens when it is first run.
    pub fn add(&mut self, request: AddDownloadRequest) -> EngineResult<DownloadId> {
        let url =
            Url::parse(&request.url).map_err(|err| EngineError::InvalidUrl(err.to_string()))?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err(EngineError::UnsupportedScheme(url.scheme().to_owned()));
        }
        if !request.cookies.is_empty() {
            return Err(EngineError::Internal(
                "cookie handoff is not available until the browser integration lands".to_owned(),
            ));
        }

        let mut headers = Vec::with_capacity(request.headers.len());
        for (name, value) in &request.headers {
            HeaderName::from_bytes(name.as_bytes())
                .map_err(|_| EngineError::InvalidHeader(name.clone()))?;
            HeaderValue::from_str(value).map_err(|_| EngineError::InvalidHeader(name.clone()))?;

            // These are secrets. Until the keyring-backed `headers_enc` path
            // lands, refuse to persist them in plaintext metadata.
            let lowered = name.trim().to_ascii_lowercase();
            if matches!(
                lowered.as_str(),
                "authorization" | "cookie" | "proxy-authorization"
            ) {
                return Err(EngineError::InvalidHeader(format!(
                    "{name} is a secret and is not persisted in M1"
                )));
            }
            headers.push((name.clone(), value.clone()));
        }

        let file_name = request
            .file_name
            .as_deref()
            .map(sanitize_file_name)
            .filter(|name| !name.is_empty())
            .or_else(|| filename_from_url(&url).map(|name| sanitize_file_name(&name)))
            .unwrap_or_else(|| "download".to_owned());

        let dest_dir = request
            .dest_dir
            .as_deref()
            .map(PathBuf::from)
            .unwrap_or_else(|| self.config.default_dir.clone());
        std::fs::create_dir_all(&dest_dir)?;

        let final_path = unique_file_name(&dest_dir, &file_name);
        let temp_path = temp_path_for(&final_path);
        let id = DownloadId::new();

        downloads::insert(
            self.store.connection(),
            &NewDownload {
                id,
                url: url.to_string(),
                file_name,
                dest_dir: dest_dir.to_string_lossy().into_owned(),
                temp_path: temp_path.to_string_lossy().into_owned(),
                final_path: final_path.to_string_lossy().into_owned(),
                connections: request.connections.unwrap_or(8).clamp(1, 32),
                source: request.source,
                checksum_sha256: request.checksum_sha256,
                referer: request.referer,
                user_agent: request.user_agent,
                speed_limit: request.speed_limit,
                created_at_ms: downloads::now_ms(),
                name_locked: request.file_name.is_some(),
                headers,
            },
        )?;

        Ok(id)
    }

    pub fn get(&self, id: DownloadId) -> EngineResult<Option<DownloadRecord>> {
        Ok(downloads::get(self.store.connection(), id)?)
    }

    pub fn list(&self, limit: usize) -> EngineResult<Vec<DownloadRecord>> {
        Ok(downloads::list(self.store.connection(), limit)?)
    }

    pub fn list_runnable(&self) -> EngineResult<Vec<DownloadRecord>> {
        Ok(downloads::list_runnable(self.store.connection())?)
    }

    /// Mark a queued or paused download as paused.
    ///
    /// A running download can only be paused by the process that owns it
    /// (Ctrl-C in the CLI today, the IPC command in M5).
    pub fn pause(&mut self, id: DownloadId) -> EngineResult<()> {
        let record = self.record(id)?;
        match record.state {
            DownloadState::Queued | DownloadState::Paused => {
                downloads::set_state(self.store.connection(), id, DownloadState::Paused)?;
                Ok(())
            }
            DownloadState::Completed | DownloadState::Canceled | DownloadState::Failed => {
                Err(EngineError::Internal("download is not active".to_owned()))
            }
            _ => Err(EngineError::Internal(
                "download is running in another process; pause it there".to_owned(),
            )),
        }
    }

    /// Cancel a download that is not running and delete its partial file.
    pub fn cancel(&mut self, id: DownloadId) -> EngineResult<()> {
        let record = self.record(id)?;
        if record.state.is_running() {
            return Err(EngineError::Internal(
                "download is running in another process; cancel it there".to_owned(),
            ));
        }
        if record.state == DownloadState::Completed {
            return Err(EngineError::Internal(
                "download is already completed".to_owned(),
            ));
        }
        if let Some(temp) = record.temp_path.as_deref() {
            let _ = std::fs::remove_file(temp);
        }
        downloads::set_state(self.store.connection(), id, DownloadState::Canceled)?;
        Ok(())
    }

    fn record(&self, id: DownloadId) -> EngineResult<DownloadRecord> {
        self.get(id)?
            .ok_or_else(|| EngineError::Internal(format!("unknown download {id}")))
    }

    /// Download (or resume) one item to completion.
    pub async fn run(
        &mut self,
        id: DownloadId,
        cancel: CancelToken,
        progress: Option<ProgressSink<'_>>,
    ) -> EngineResult<RunOutcome> {
        match self.run_inner(id, &cancel, progress).await {
            Err(EngineError::Canceled) => {
                let _ = downloads::set_state(self.store.connection(), id, DownloadState::Paused);
                Ok(RunOutcome::Paused)
            }
            Err(error) => {
                let info = error.info();
                let _ = downloads::set_error(self.store.connection(), id, &info);
                tracing::warn!(download = %id, code = ?info.code, "download failed");
                Err(error)
            }
            Ok(outcome) => Ok(outcome),
        }
    }

    async fn run_inner(
        &mut self,
        id: DownloadId,
        cancel: &CancelToken,
        progress: Option<ProgressSink<'_>>,
    ) -> EngineResult<RunOutcome> {
        let record = self.record(id)?;
        match record.state {
            DownloadState::Completed => return Ok(RunOutcome::Completed),
            DownloadState::Canceled => return Err(EngineError::Canceled),
            _ => {}
        }
        if cancel.is_cancelled() {
            return Ok(RunOutcome::Paused);
        }

        downloads::set_state(self.store.connection(), id, DownloadState::Probing)?;

        let url =
            Url::parse(&record.url).map_err(|err| EngineError::InvalidUrl(err.to_string()))?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err(EngineError::UnsupportedScheme(url.scheme().to_owned()));
        }
        let headers = request_headers(&record);

        let probe =
            probe_with_retry(&self.client, &self.config.retry, &url, &headers, cancel).await?;
        downloads::record_probe(
            self.store.connection(),
            id,
            &ProbeUpdate {
                final_url: probe.final_url.clone(),
                total_bytes: probe.total_bytes,
                supports_ranges: probe.ranges_supported,
                etag: probe.etag.clone(),
                last_modified: probe.last_modified.clone(),
                mime: probe.content_type.clone(),
            },
        )?;

        let content_changed = record.downloaded_bytes > 0 && validators_differ(&record, &probe);
        let total = probe.total_bytes;
        let ranges = probe.ranges_supported && total.is_some_and(|value| value > 0);
        // Redirects are resolved once by the probe; the transfer targets the
        // final URL directly because the client does not follow redirects.
        let transfer_url =
            Url::parse(&probe.final_url).map_err(|err| EngineError::InvalidUrl(err.to_string()))?;
        // Credentials must not follow a redirect to another origin.
        let headers = if transfer_url.origin() != url.origin() {
            strip_sensitive_headers(headers)
        } else {
            headers
        };

        let (file_name, final_path) = resolve_paths(&record, &probe);
        let temp_path = temp_path_for(&final_path);
        downloads::set_paths(
            self.store.connection(),
            id,
            &file_name,
            &temp_path.to_string_lossy(),
            &final_path.to_string_lossy(),
        )?;

        if total == Some(0) {
            return self
                .finish_empty(id, &temp_path, &final_path, progress)
                .await;
        }

        let mut resume = 0_u64;
        let segment_end = total.map(|value| value - 1);
        if ranges {
            let total_bytes = total.unwrap_or(0);
            let spec = SegmentSpec::new(0, 0, total_bytes.saturating_sub(1));
            let existing = downloads::load_segments(self.store.connection(), id)?
                .pop()
                .filter(|segment| segment.start == 0 && segment.end == segment_end.unwrap_or(0));

            let segment = match existing {
                Some(segment) => segment,
                None => {
                    downloads::upsert_segment(
                        self.store.connection(),
                        id,
                        &spec,
                        SegmentState::Pending,
                    )?;
                    SegmentRecord {
                        index: 0,
                        start: 0,
                        end: total_bytes.saturating_sub(1),
                        downloaded: 0,
                        state: SegmentState::Pending,
                    }
                }
            };

            let temp_len = std::fs::metadata(&temp_path)
                .map(|meta| meta.len())
                .unwrap_or(0);
            resume = if content_changed {
                0
            } else {
                segment.downloaded.min(temp_len).min(total_bytes)
            };
            if content_changed || resume != segment.downloaded {
                downloads::reset_progress(self.store.connection_mut(), id)?;
                if content_changed {
                    resume = 0;
                } else {
                    // Keep the bytes that survived; record the reconciled offset.
                    let segment = SegmentRecord {
                        downloaded: resume,
                        ..segment
                    };
                    downloads::commit_progress(
                        self.store.connection_mut(),
                        id,
                        resume,
                        Some(&segment),
                    )?;
                }
                if content_changed {
                    downloads::upsert_segment(
                        self.store.connection(),
                        id,
                        &spec,
                        SegmentState::Pending,
                    )?;
                }
            }
        } else {
            // Without range support a restart is the only correct resume.
            downloads::reset_progress(self.store.connection_mut(), id)?;
        }

        if let Some(parent) = final_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // Never truncate on open: the resume offset is applied explicitly
        // below so a crash between open and the transfer cannot destroy
        // committed bytes.
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&temp_path)?;
        if file.metadata()?.len() != resume {
            file.set_len(resume)?;
        }

        downloads::mark_started(self.store.connection(), id)?;
        let writer = ChunkWriter::spawn(file, resume)?;
        let started = Instant::now();

        let transfer = transfer(
            &self.client,
            &mut self.store,
            &self.config,
            id,
            &transfer_url,
            &headers,
            total,
            ranges,
            &writer,
            cancel,
            progress,
            resume,
            probe.validator(),
        )
        .await;

        let outcome = match transfer {
            Ok(outcome) => outcome,
            Err(error) => {
                drop(writer);
                return Err(error);
            }
        };

        if outcome == TransferOutcome::Paused {
            drop(writer);
            return Ok(RunOutcome::Paused);
        }

        writer.finish().await.map_err(EngineError::Io)?;

        let length = tokio::fs::metadata(&temp_path).await?.len();
        if let Some(expected) = total {
            if length != expected {
                return Err(EngineError::Truncated {
                    expected,
                    received: length,
                });
            }
        }

        let checksum = sha256_file(&temp_path).await?;
        if let Some(expected) = record.checksum_sha256.as_deref() {
            if !expected.eq_ignore_ascii_case(&checksum) {
                return Err(EngineError::ChecksumMismatch {
                    expected: expected.to_owned(),
                    actual: checksum,
                });
            }
        }

        downloads::set_state(self.store.connection(), id, DownloadState::Finalizing)?;

        // A different file may have claimed the name while we were downloading.
        let final_path = unique_file_name(Path::new(&record.dest_dir), &file_name);
        tokio::fs::rename(&temp_path, &final_path).await?;
        fsync_parent(&final_path).await?;

        let elapsed = started.elapsed().as_secs_f64().max(0.001);
        let average_speed = (length as f64 / elapsed) as u64;
        downloads::mark_completed(
            self.store.connection_mut(),
            id,
            length,
            &checksum,
            Some(average_speed),
        )?;

        if let Some(sink) = progress {
            sink(ProgressSample {
                downloaded_bytes: length,
                total_bytes: Some(length),
                speed_bps: 0,
                eta_seconds: Some(0),
            });
        }

        Ok(RunOutcome::Completed)
    }

    async fn finish_empty(
        &mut self,
        id: DownloadId,
        temp_path: &Path,
        final_path: &Path,
        progress: Option<ProgressSink<'_>>,
    ) -> EngineResult<RunOutcome> {
        if let Some(parent) = final_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(temp_path, [])?;
        let checksum = sha256_file(temp_path).await?;
        tokio::fs::rename(temp_path, final_path).await?;
        fsync_parent(final_path).await?;
        downloads::mark_completed(self.store.connection_mut(), id, 0, &checksum, None)?;
        if let Some(sink) = progress {
            sink(ProgressSample {
                downloaded_bytes: 0,
                total_bytes: Some(0),
                speed_bps: 0,
                eta_seconds: Some(0),
            });
        }
        Ok(RunOutcome::Completed)
    }
}

/// Retry the probe on transient failures.
///
/// Deliberately a free function: `&Engine` is not `Send` because the SQLite
/// connection is not `Sync`, so an `async fn(&self)` would make the whole run
/// future non-spawnable.
async fn probe_with_retry(
    client: &Client,
    policy: &RetryPolicy,
    url: &Url,
    headers: &HeaderMap,
    cancel: &CancelToken,
) -> EngineResult<ProbeResult> {
    let mut attempts = 0_u32;

    loop {
        attempts += 1;
        match probe(client, url, headers).await {
            Ok(result) => return Ok(result),
            Err(error) => {
                if !error.is_retryable() || attempts >= policy.max_attempts {
                    return Err(error);
                }
                if !sleep_or_cancel(cancel, backoff(policy, attempts)).await {
                    return Err(EngineError::Canceled);
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TransferOutcome {
    Completed,
    Paused,
}

// The transfer state is explicit on purpose: offset, validator, and checkpoint
// bookkeeping are exactly the values that must survive a crash, and a context
// struct would make it easier to drift from the invariant.
#[allow(clippy::too_many_arguments)]
async fn transfer(
    client: &Client,
    store: &mut Store,
    config: &EngineConfig,
    id: DownloadId,
    url: &Url,
    headers: &HeaderMap,
    total: Option<u64>,
    ranges: bool,
    writer: &ChunkWriter,
    cancel: &CancelToken,
    progress: Option<ProgressSink<'_>>,
    resume: u64,
    validator: Option<String>,
) -> EngineResult<TransferOutcome> {
    const MAX_RESTARTS: u32 = 2;
    const MAX_TOTAL_ATTEMPTS: u32 = 500;

    let mut offset = resume;
    let mut attempts = 0_u32;
    let mut restarts = 0_u32;
    let mut validator = validator;
    let mut last_checkpoint_at = Instant::now();
    let mut last_checkpoint_bytes = resume;
    let mut last_sample_at = Instant::now();
    let mut last_sample_bytes = resume;
    let segment_end = total.map(|value| value - 1);

    loop {
        if cancel.is_cancelled() {
            return pause_transfer(writer, store, id, segment_end).await;
        }

        // Without range support a retry can only restart at byte zero; leaving
        // the offset at the truncation point would write the new body in the
        // wrong place.
        if !ranges && offset > 0 {
            writer.truncate(0).await.map_err(EngineError::Io)?;
            downloads::reset_progress(store.connection_mut(), id)?;
            offset = 0;
            last_checkpoint_bytes = 0;
            last_sample_bytes = 0;
            last_checkpoint_at = Instant::now();
            last_sample_at = Instant::now();
        }

        attempts += 1;
        if attempts > MAX_TOTAL_ATTEMPTS {
            return Err(EngineError::Internal(
                "gave up after too many retry attempts".to_owned(),
            ));
        }

        let mut request = client.get(url.clone()).headers(headers.clone());
        request = request.header(ACCEPT_ENCODING, "identity");
        if offset > 0 && ranges {
            request = request.header(RANGE, format!("bytes={offset}-"));
            if let Some(validator) = validator.as_deref() {
                request = request.header(IF_RANGE, validator);
            }
        }

        let response = tokio::select! {
            biased;
            () = cancel.cancelled() => {
                return pause_transfer(writer, store, id, segment_end).await;
            }
            result = request.send() => result,
        };

        let response = match response {
            Ok(response) => response,
            Err(error) => {
                let engine_error = EngineError::Network(error);
                if attempts >= config.retry.max_attempts {
                    return Err(engine_error);
                }
                if !sleep_or_cancel(cancel, backoff(&config.retry, attempts)).await {
                    return pause_transfer(writer, store, id, segment_end).await;
                }
                continue;
            }
        };

        let status = response.status();

        if status == StatusCode::RANGE_NOT_SATISFIABLE {
            if total == Some(offset) {
                return Ok(TransferOutcome::Completed);
            }
            if attempts >= config.retry.max_attempts {
                return Err(EngineError::ContentChanged);
            }
            if !sleep_or_cancel(cancel, backoff(&config.retry, attempts)).await {
                return pause_transfer(writer, store, id, segment_end).await;
            }
            continue;
        }

        if status == StatusCode::OK && offset > 0 {
            // The server restarted the representation: either the validator
            // changed or it ignores ranges. Start over from byte zero.
            restarts += 1;
            if restarts > MAX_RESTARTS {
                return Err(EngineError::ContentChanged);
            }
            writer.truncate(0).await.map_err(EngineError::Io)?;
            offset = 0;
            last_checkpoint_bytes = 0;
            last_sample_bytes = 0;
            last_checkpoint_at = Instant::now();
            last_sample_at = Instant::now();
            downloads::reset_progress(store.connection_mut(), id)?;

            let new_etag = header_string(response.headers(), ETAG);
            let new_last_modified = header_string(response.headers(), LAST_MODIFIED);
            downloads::update_validators(
                store.connection(),
                id,
                new_etag.as_deref(),
                new_last_modified.as_deref(),
            )?;
            validator = new_etag
                .filter(|etag| !etag.starts_with("W/"))
                .or(new_last_modified);
        } else if status == StatusCode::PARTIAL_CONTENT {
            let parsed = response
                .headers()
                .get(CONTENT_RANGE)
                .and_then(|value| value.to_str().ok())
                .and_then(parse_content_range);
            match parsed {
                Some((start, _, response_total)) if start == offset => {
                    if let Some(expected) = total {
                        if response_total != Some(expected) {
                            return Err(EngineError::ContentChanged);
                        }
                    }
                }
                _ => return Err(EngineError::ContentChanged),
            }
        } else if status == StatusCode::UNAUTHORIZED {
            return Err(EngineError::AuthRequired);
        } else if status == StatusCode::PROXY_AUTHENTICATION_REQUIRED {
            return Err(EngineError::ProxyAuthRequired);
        } else if status == StatusCode::TOO_MANY_REQUESTS
            || status == StatusCode::REQUEST_TIMEOUT
            || status.is_server_error()
        {
            if attempts >= config.retry.max_attempts {
                return Err(EngineError::Http {
                    status: status.as_u16(),
                });
            }
            let delay = retry_after(&response).unwrap_or_else(|| backoff(&config.retry, attempts));
            if !sleep_or_cancel(cancel, delay).await {
                return pause_transfer(writer, store, id, segment_end).await;
            }
            continue;
        } else if status != StatusCode::OK {
            return Err(EngineError::Http {
                status: status.as_u16(),
            });
        }

        let mut stream = response.bytes_stream();
        let mut stream_error: Option<EngineError> = None;

        loop {
            let next = tokio::select! {
                biased;
                () = cancel.cancelled() => {
                    return pause_transfer(writer, store, id, segment_end).await;
                }
                next = stream.next() => next,
            };
            let Some(item) = next else {
                break;
            };

            match item {
                Ok(chunk) => {
                    if chunk.is_empty() {
                        continue;
                    }
                    let len = chunk.len() as u64;
                    if let Some(expected) = total {
                        if offset.saturating_add(len) > expected {
                            return Err(EngineError::ContentChanged);
                        }
                    }
                    writer.write(offset, chunk).await.map_err(EngineError::Io)?;
                    offset += len;

                    let checkpoint_due = last_checkpoint_at.elapsed() >= config.checkpoint_interval
                        || offset.saturating_sub(last_checkpoint_bytes) >= config.checkpoint_bytes;
                    if checkpoint_due {
                        let durable = writer.sync().await.map_err(EngineError::Io)?;
                        commit_durable(store, id, durable, segment_end)?;
                        last_checkpoint_at = Instant::now();
                        last_checkpoint_bytes = offset;
                        // With ranges, progress means a retry resumes instead
                        // of starting over, so the failure budget resets. A
                        // no-range restart re-downloads every byte, so it must
                        // keep consuming the budget.
                        if ranges {
                            attempts = 0;
                        }

                        if let Some(sink) = progress {
                            let now = Instant::now();
                            let elapsed =
                                now.duration_since(last_sample_at).as_secs_f64().max(0.001);
                            let speed =
                                (offset.saturating_sub(last_sample_bytes) as f64 / elapsed) as u64;
                            sink(ProgressSample {
                                downloaded_bytes: offset,
                                total_bytes: total,
                                speed_bps: speed,
                                eta_seconds: estimate_eta(offset, total, speed),
                            });
                            last_sample_at = now;
                            last_sample_bytes = offset;
                        }
                    }
                }
                Err(error) => {
                    stream_error = Some(EngineError::Network(error));
                    break;
                }
            }
        }

        let retry_error = match stream_error {
            Some(error) => Some(error),
            None => match total {
                Some(expected) if offset < expected => Some(EngineError::Truncated {
                    expected,
                    received: offset,
                }),
                Some(expected) if offset > expected => {
                    return Err(EngineError::ContentChanged);
                }
                _ => None,
            },
        };

        if let Some(error) = retry_error {
            if attempts >= config.retry.max_attempts {
                return Err(error);
            }
            let durable = writer.sync().await.map_err(EngineError::Io)?;
            commit_durable(store, id, durable, segment_end)?;
            offset = durable;
            last_checkpoint_bytes = durable;
            last_sample_bytes = durable;
            last_checkpoint_at = Instant::now();
            last_sample_at = Instant::now();

            if !sleep_or_cancel(cancel, backoff(&config.retry, attempts)).await {
                return pause_transfer(writer, store, id, segment_end).await;
            }
            continue;
        }

        // The writer may have accepted bytes past the last checkpoint; flush
        // before declaring success so the completed length is durable.
        let durable = writer.sync().await.map_err(EngineError::Io)?;
        if let Some(expected) = total {
            if durable != expected {
                return Err(EngineError::Truncated {
                    expected,
                    received: durable,
                });
            }
        }
        commit_durable(store, id, durable, segment_end)?;
        return Ok(TransferOutcome::Completed);
    }
}

async fn pause_transfer(
    writer: &ChunkWriter,
    store: &mut Store,
    id: DownloadId,
    segment_end: Option<u64>,
) -> EngineResult<TransferOutcome> {
    let durable = writer.sync().await.map_err(EngineError::Io)?;
    commit_durable(store, id, durable, segment_end)?;
    downloads::set_state(store.connection(), id, DownloadState::Paused)?;
    Ok(TransferOutcome::Paused)
}

fn commit_durable(
    store: &mut Store,
    id: DownloadId,
    durable: u64,
    segment_end: Option<u64>,
) -> EngineResult<()> {
    let segment = segment_end.map(|end| SegmentRecord {
        index: 0,
        start: 0,
        end,
        downloaded: durable,
        state: SegmentState::Active,
    });
    downloads::commit_progress(store.connection_mut(), id, durable, segment.as_ref())?;
    Ok(())
}

fn strip_sensitive_headers(mut headers: HeaderMap) -> HeaderMap {
    headers.remove(AUTHORIZATION);
    headers.remove(COOKIE);
    headers.remove(PROXY_AUTHORIZATION);
    headers
}

fn request_headers(record: &DownloadRecord) -> HeaderMap {
    let mut headers = HeaderMap::new();

    if let Some(user_agent) = record.user_agent.as_deref() {
        if let Ok(value) = HeaderValue::from_str(user_agent) {
            headers.insert(header::USER_AGENT, value);
        }
    }
    if let Some(referer) = record.referer.as_deref() {
        if let Ok(value) = HeaderValue::from_str(referer) {
            headers.insert(header::REFERER, value);
        }
    }
    for (name, value) in record.extra_headers() {
        let Ok(name) = HeaderName::from_bytes(name.as_bytes()) else {
            continue;
        };
        // The engine owns these: range and encoding semantics, plus framing
        // headers a stored value must never override.
        if matches!(
            name.as_str(),
            "range"
                | "accept-encoding"
                | "if-range"
                | "host"
                | "content-length"
                | "transfer-encoding"
                | "connection"
                | "te"
        ) {
            continue;
        }
        if let Ok(value) = HeaderValue::from_str(&value) {
            headers.insert(name, value);
        }
    }

    headers
}

fn resolve_paths(record: &DownloadRecord, probe: &ProbeResult) -> (String, PathBuf) {
    let dest_dir = PathBuf::from(&record.dest_dir);
    let mut file_name = record.file_name.clone();

    if !record.name_locked() && record.downloaded_bytes == 0 {
        if let Some(candidate) = probe_file_name(probe) {
            file_name = candidate;
        }
    }

    let final_path = unique_file_name(&dest_dir, &file_name);

    // Drop an empty temp file left under a previous name; never delete one
    // that holds data.
    if let Some(old_temp) = record.temp_path.as_deref() {
        let old_temp = PathBuf::from(old_temp);
        let new_temp = temp_path_for(&final_path);
        if old_temp != new_temp {
            if let Ok(meta) = std::fs::metadata(&old_temp) {
                if meta.len() == 0 {
                    let _ = std::fs::remove_file(old_temp);
                }
            }
        }
    }

    (file_name, final_path)
}

fn probe_file_name(probe: &ProbeResult) -> Option<String> {
    let raw = probe
        .content_disposition
        .as_deref()
        .and_then(filename_from_content_disposition)
        .or_else(|| {
            Url::parse(&probe.final_url)
                .ok()
                .and_then(|url| filename_from_url(&url))
        })?;
    let name = sanitize_file_name(&raw);
    (!name.is_empty()).then_some(name)
}

fn validators_differ(record: &DownloadRecord, probe: &ProbeResult) -> bool {
    match (&record.etag, &probe.etag) {
        (Some(old), Some(new)) => old != new,
        (Some(_), None) => true,
        (None, _) => match (&record.last_modified, &probe.last_modified) {
            (Some(old), Some(new)) => old != new,
            (Some(_), None) => true,
            _ => false,
        },
    }
}

fn header_string(headers: &HeaderMap, name: HeaderName) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
        .filter(|value| !value.is_empty())
}

fn retry_after(response: &reqwest::Response) -> Option<Duration> {
    let value = response.headers().get(RETRY_AFTER)?.to_str().ok()?;
    let seconds: u64 = value.trim().parse().ok()?;
    Some(Duration::from_secs(seconds.min(60)))
}

fn estimate_eta(downloaded: u64, total: Option<u64>, speed_bps: u64) -> Option<u64> {
    let total = total?;
    if downloaded >= total {
        return Some(0);
    }
    if speed_bps == 0 {
        return None;
    }
    Some((total - downloaded) / speed_bps)
}

fn backoff(policy: &RetryPolicy, attempt: u32) -> Duration {
    let shift = attempt.saturating_sub(1).min(10);
    let base_ms = policy.base_delay.as_millis().max(1);
    let exponential = base_ms.saturating_mul(1_u128 << shift);
    let capped = exponential.min(policy.max_delay.as_millis().max(1));
    let half = (capped / 2).max(1);
    let jitter = u128::from(jitter_nanos()) % (half + 1);
    Duration::from_millis(u64::try_from(half + jitter).unwrap_or(u64::MAX))
}

fn jitter_nanos() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| u64::from(elapsed.subsec_nanos()))
        .unwrap_or(0)
}

async fn sleep_or_cancel(cancel: &CancelToken, delay: Duration) -> bool {
    tokio::select! {
        biased;
        () = cancel.cancelled() => false,
        () = tokio::time::sleep(delay) => true,
    }
}

fn temp_path_for(final_path: &Path) -> PathBuf {
    let mut raw = final_path.as_os_str().to_os_string();
    raw.push(".downpour-part");
    PathBuf::from(raw)
}

async fn sha256_file(path: &Path) -> EngineResult<String> {
    use sha2::{Digest, Sha256};
    use tokio::io::AsyncReadExt;

    let mut file = tokio::fs::File::open(path).await?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let read = file.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}

#[cfg(unix)]
async fn fsync_parent(path: &Path) -> EngineResult<()> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    let parent = parent.to_path_buf();
    let result = tokio::task::spawn_blocking(move || -> std::io::Result<()> {
        let directory = std::fs::File::open(parent)?;
        directory.sync_all()
    })
    .await
    .map_err(|error| EngineError::Internal(error.to_string()))?;
    result.map_err(EngineError::Io)
}

#[cfg(not(unix))]
async fn fsync_parent(_path: &Path) -> EngineResult<()> {
    Ok(())
}
