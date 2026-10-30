//! Multi-connection transfer.
//!
//! Workers pull segment indices from a shared queue and each streams one
//! `Range` request into the single writer thread. Segments are finer-grained
//! than the worker count, so a worker that finishes early takes another range
//! instead of idling.
//!
//! A single segment (or a no-range server) uses the engine's single-stream
//! path; this module assumes `total` is known and ranges are supported.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use downpour_store::Store;
use downpour_store::downloads::{self, SegmentRecord};
use downpour_types::{DownloadId, DownloadState};
use futures_util::StreamExt;
use reqwest::header::{
    ACCEPT_ENCODING, CONTENT_RANGE, ETAG, HeaderMap, HeaderName, IF_RANGE, LAST_MODIFIED, RANGE,
    RETRY_AFTER,
};
use reqwest::{Client, StatusCode, Url};
use tokio::task::JoinSet;
use tokio::time::MissedTickBehavior;

use crate::cancel::CancelToken;
use crate::engine::{ProgressSample, ProgressSink, RetryPolicy};
use crate::error::{EngineError, EngineResult};
use crate::probe::parse_content_range;
use crate::ratelimit::RateLimiter;
use crate::writer::ChunkWriter;

/// Everything the workers need that does not change during one attempt.
pub(crate) struct SegmentedRequest<'a> {
    pub client: &'a Client,
    pub url: &'a Url,
    pub headers: &'a HeaderMap,
    pub connections: u16,
    pub total: u64,
    pub segments: &'a [SegmentRecord],
    pub validator: Option<String>,
    pub speed_limit: u64,
    pub checkpoint_interval: Duration,
    pub retry: RetryPolicy,
}

pub(crate) enum TransferOutcome {
    Completed,
    Paused,
}

const MAX_RESTARTS: u32 = 2;
const MAX_SEGMENT_ATTEMPTS: u32 = 200;

struct Shared {
    client: Client,
    url: Url,
    headers: HeaderMap,
    total: u64,
    validator: Option<String>,
    segments: Vec<SegmentRecord>,
    writer: Arc<ChunkWriter>,
    limiter: Option<Arc<RateLimiter>>,
    retry: RetryPolicy,
    cancel: CancelToken,
    stop: CancelToken,
    queue: Mutex<VecDeque<usize>>,
    failure: Mutex<Option<EngineError>>,
    restart: Mutex<Option<(Option<String>, Option<String>)>>,
}

impl Shared {
    fn stopped(&self) -> bool {
        self.cancel.is_cancelled() || self.stop.is_cancelled()
    }

    async fn wait_stop(&self) {
        tokio::select! {
            () = self.cancel.cancelled() => {}
            () = self.stop.cancelled() => {}
        }
    }

    fn set_failure(&self, error: EngineError) {
        let mut slot = lock(&self.failure);
        if slot.is_none() {
            *slot = Some(error);
        }
        drop(slot);
        self.stop.cancel();
    }

    fn request_restart(&self, etag: Option<String>, last_modified: Option<String>) {
        *lock(&self.restart) = Some((etag, last_modified));
        self.stop.cancel();
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

enum WorkerStop {
    Stopped,
    Restart {
        etag: Option<String>,
        last_modified: Option<String>,
    },
    Fatal(EngineError),
}

/// Run the segmented transfer until completion, pause, or failure.
pub(crate) async fn run(
    store: &mut Store,
    writer: Arc<ChunkWriter>,
    request: SegmentedRequest<'_>,
    id: DownloadId,
    cancel: &CancelToken,
    progress: Option<ProgressSink<'_>>,
) -> EngineResult<TransferOutcome> {
    let mut template: Vec<SegmentRecord> = request.segments.to_vec();
    let mut validator = request.validator.clone();
    let mut restarts = 0_u32;
    let initial_downloaded: u64 = template.iter().map(|segment| segment.downloaded).sum();
    let mut sampler = SampleState::new(initial_downloaded);

    loop {
        let stop = CancelToken::new();
        let mut queue = VecDeque::new();
        for (index, segment) in template.iter().enumerate() {
            if !segment.is_complete() {
                queue.push_back(index);
            }
        }
        let active = queue.len();
        let worker_count = active
            .min(usize::from(request.connections.clamp(1, 32)))
            .max(1);

        let shared = Arc::new(Shared {
            client: request.client.clone(),
            url: request.url.clone(),
            headers: request.headers.clone(),
            total: request.total,
            validator: validator.clone(),
            segments: template.clone(),
            writer: writer.clone(),
            limiter: RateLimiter::new(request.speed_limit).map(Arc::new),
            retry: request.retry,
            cancel: cancel.clone(),
            stop: stop.clone(),
            queue: Mutex::new(queue),
            failure: Mutex::new(None),
            restart: Mutex::new(None),
        });

        let mut workers = JoinSet::new();
        for _ in 0..worker_count {
            workers.spawn(worker(shared.clone()));
        }

        let mut ticker = tokio::time::interval(request.checkpoint_interval);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
        let mut paused = false;

        loop {
            tokio::select! {
                biased;
                () = cancel.cancelled() => {
                    paused = true;
                    shared.stop.cancel();
                    break;
                }
                () = shared.stop.cancelled() => break,
                _ = ticker.tick() => {
                    if let Err(error) = checkpoint(store, &shared, id, &mut sampler, progress).await {
                        shared.stop.cancel();
                        while workers.join_next().await.is_some() {}
                        return Err(error);
                    }
                }
                joined = workers.join_next() => {
                    match joined {
                        Some(Ok(Ok(()))) => {
                            if workers.is_empty() {
                                break;
                            }
                        }
                        Some(Ok(Err(error))) => {
                            shared.set_failure(error);
                            break;
                        }
                        Some(Err(join_error)) => {
                            shared.set_failure(EngineError::Internal(format!(
                                "transfer worker panicked: {join_error}"
                            )));
                            break;
                        }
                        None => break,
                    }
                }
            }
        }

        shared.stop.cancel();
        while workers.join_next().await.is_some() {}

        // Final sync for this attempt, then commit whatever is durable.
        let report = shared.writer.sync().await.map_err(EngineError::Io)?;
        let downloaded_total = commit_from_report(store, id, &template, &report)?;

        if paused {
            downloads::set_state(store.connection(), id, DownloadState::Paused)?;
            return Ok(TransferOutcome::Paused);
        }

        let failure = lock(&shared.failure).take();
        if let Some(error) = failure {
            return Err(error);
        }

        let restart = lock(&shared.restart).take();
        if let Some((etag, last_modified)) = restart {
            restarts += 1;
            if restarts > MAX_RESTARTS || template.len() != 1 {
                return Err(EngineError::ContentChanged);
            }
            shared
                .writer
                .reset(0, vec![(0, 0)])
                .await
                .map_err(EngineError::Io)?;
            downloads::update_validators(
                store.connection(),
                id,
                etag.as_deref(),
                last_modified.as_deref(),
            )?;
            downloads::reset_progress(store.connection_mut(), id)?;
            for segment in &mut template {
                segment.downloaded = 0;
            }
            validator = etag
                .filter(|value| !value.starts_with("W/"))
                .or(last_modified);
            continue;
        }

        if downloaded_total < request.total {
            return Err(EngineError::Truncated {
                expected: request.total,
                received: downloaded_total,
            });
        }

        return Ok(TransferOutcome::Completed);
    }
}

async fn worker(shared: Arc<Shared>) -> EngineResult<()> {
    loop {
        if shared.stopped() {
            return Ok(());
        }
        let index = lock(&shared.queue).pop_front();
        let Some(index) = index else {
            return Ok(());
        };

        match download_segment(&shared, index).await {
            Ok(()) => {}
            Err(WorkerStop::Stopped) => return Ok(()),
            Err(WorkerStop::Restart {
                etag,
                last_modified,
            }) => {
                shared.request_restart(etag, last_modified);
                return Ok(());
            }
            Err(WorkerStop::Fatal(error)) => {
                shared.set_failure(error);
                return Ok(());
            }
        }
    }
}

async fn download_segment(shared: &Shared, index: usize) -> Result<(), WorkerStop> {
    let segment = &shared.segments[index];
    let mut cursor = segment.start + segment.downloaded;
    let mut attempts = 0_u32;
    let mut total_attempts = 0_u32;

    while cursor <= segment.end {
        if shared.stopped() {
            return Err(WorkerStop::Stopped);
        }
        attempts += 1;
        total_attempts += 1;
        if total_attempts > MAX_SEGMENT_ATTEMPTS {
            return Err(WorkerStop::Fatal(EngineError::Internal(
                "segment exceeded its total retry budget".to_owned(),
            )));
        }

        let mut request = shared
            .client
            .get(shared.url.clone())
            .headers(shared.headers.clone())
            .header(ACCEPT_ENCODING, "identity")
            .header(RANGE, format!("bytes={cursor}-{}", segment.end));
        if let Some(validator) = shared.validator.as_deref() {
            request = request.header(IF_RANGE, validator);
        }

        let response = tokio::select! {
            biased;
            () = shared.wait_stop() => return Err(WorkerStop::Stopped),
            response = request.send() => response,
        };

        let response = match response {
            Ok(response) => response,
            Err(error) => {
                if attempts >= shared.retry.max_attempts {
                    return Err(WorkerStop::Fatal(EngineError::Network(error)));
                }
                if !sleep_or_stop(shared, backoff(&shared.retry, attempts)).await {
                    return Err(WorkerStop::Stopped);
                }
                continue;
            }
        };

        let status = response.status();

        if status == StatusCode::PARTIAL_CONTENT {
            let parsed = response
                .headers()
                .get(CONTENT_RANGE)
                .and_then(|value| value.to_str().ok())
                .and_then(parse_content_range);
            match parsed {
                Some((start, _, Some(total))) if start == cursor && total == shared.total => {}
                _ => {
                    return Err(WorkerStop::Fatal(EngineError::ContentChanged));
                }
            }
        } else if status == StatusCode::OK {
            if cursor > segment.start {
                let etag = header_string(response.headers(), ETAG);
                let last_modified = header_string(response.headers(), LAST_MODIFIED);
                return Err(WorkerStop::Restart {
                    etag,
                    last_modified,
                });
            }
            return Err(WorkerStop::Fatal(EngineError::ContentChanged));
        } else if status == StatusCode::RANGE_NOT_SATISFIABLE {
            return Err(WorkerStop::Fatal(EngineError::ContentChanged));
        } else if status == StatusCode::UNAUTHORIZED {
            return Err(WorkerStop::Fatal(EngineError::AuthRequired));
        } else if status == StatusCode::PROXY_AUTHENTICATION_REQUIRED {
            return Err(WorkerStop::Fatal(EngineError::ProxyAuthRequired));
        } else if status == StatusCode::TOO_MANY_REQUESTS
            || status == StatusCode::REQUEST_TIMEOUT
            || status.is_server_error()
        {
            if attempts >= shared.retry.max_attempts {
                return Err(WorkerStop::Fatal(EngineError::Http {
                    status: status.as_u16(),
                }));
            }
            let delay = retry_after(&response).unwrap_or_else(|| backoff(&shared.retry, attempts));
            drop(response);
            if !sleep_or_stop(shared, delay).await {
                return Err(WorkerStop::Stopped);
            }
            continue;
        } else {
            return Err(WorkerStop::Fatal(EngineError::Http {
                status: status.as_u16(),
            }));
        }

        let mut stream = response.bytes_stream();
        let mut stream_error: Option<EngineError> = None;

        loop {
            let next = tokio::select! {
                biased;
                () = shared.wait_stop() => return Err(WorkerStop::Stopped),
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
                    if cursor.saturating_add(len) > segment.end + 1 {
                        return Err(WorkerStop::Fatal(EngineError::ContentChanged));
                    }
                    if let Some(limiter) = &shared.limiter {
                        tokio::select! {
                            biased;
                            () = shared.wait_stop() => return Err(WorkerStop::Stopped),
                            () = limiter.acquire(len) => {}
                        }
                    }
                    if shared.stopped() {
                        return Err(WorkerStop::Stopped);
                    }
                    shared
                        .writer
                        .write(segment.index, cursor, chunk)
                        .await
                        .map_err(|error| WorkerStop::Fatal(EngineError::Io(error)))?;
                    cursor += len;
                    // Progress means the connection is healthy again.
                    attempts = 0;
                }
                Err(error) => {
                    stream_error = Some(EngineError::Network(error));
                    break;
                }
            }
        }

        if cursor > segment.end {
            return Ok(());
        }

        let error = stream_error.unwrap_or(EngineError::Truncated {
            expected: segment.end + 1,
            received: cursor,
        });
        if attempts >= shared.retry.max_attempts {
            return Err(WorkerStop::Fatal(error));
        }
        if !sleep_or_stop(shared, backoff(&shared.retry, attempts)).await {
            return Err(WorkerStop::Stopped);
        }
    }

    Ok(())
}

struct SampleState {
    last_at: Instant,
    last_bytes: u64,
}

impl SampleState {
    fn new(last_bytes: u64) -> Self {
        Self {
            last_at: Instant::now(),
            last_bytes,
        }
    }
}

async fn checkpoint(
    store: &mut Store,
    shared: &Shared,
    id: DownloadId,
    sampler: &mut SampleState,
    progress: Option<ProgressSink<'_>>,
) -> EngineResult<()> {
    let report = shared.writer.sync().await.map_err(EngineError::Io)?;
    let downloaded = commit_from_report(store, id, &shared.segments, &report)?;

    if let Some(sink) = progress {
        let now = Instant::now();
        let elapsed = now.duration_since(sampler.last_at).as_secs_f64().max(0.001);
        let speed = (downloaded.saturating_sub(sampler.last_bytes) as f64 / elapsed) as u64;
        sink(ProgressSample {
            downloaded_bytes: downloaded,
            total_bytes: Some(shared.total),
            speed_bps: speed,
            eta_seconds: shared.total.saturating_sub(downloaded).checked_div(speed),
        });
        sampler.last_at = now;
        sampler.last_bytes = downloaded;
    }
    Ok(())
}

fn commit_from_report(
    store: &mut Store,
    id: DownloadId,
    segments: &[SegmentRecord],
    report: &[(u32, u64)],
) -> EngineResult<u64> {
    let mut rows = Vec::with_capacity(segments.len());
    let mut total = 0_u64;

    for segment in segments {
        let end = report
            .iter()
            .find(|(index, _)| *index == segment.index)
            .map(|(_, end)| *end)
            .unwrap_or(segment.start + segment.downloaded);
        let downloaded = end.saturating_sub(segment.start).min(segment.len());
        total = total.saturating_add(downloaded);
        rows.push(SegmentRecord {
            downloaded,
            ..segment.clone()
        });
    }

    downloads::commit_segments(store.connection_mut(), id, total, &rows)?;
    Ok(total)
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

async fn sleep_or_stop(shared: &Shared, delay: Duration) -> bool {
    tokio::select! {
        biased;
        () = shared.wait_stop() => false,
        () = tokio::time::sleep(delay) => true,
    }
}
