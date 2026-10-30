//! Fault-injecting HTTP server and fixtures for engine tests.
//!
//! The bodies are generated from a deterministic byte pattern so tests can
//! verify byte-exact resumes without shipping fixtures.
//!
//! Routes:
//!
//! | Route | Behaviour |
//! |---|---|
//! | `/health` | plain `ok` |
//! | `/ok/{size}` | ranges, stable ETag, `If-Range` aware |
//! | `/no-range/{size}` | ignores `Range`, always `200` |
//! | `/versioned/{size}` | body and ETag controlled by [`TestServer::set_etag`]/[`set_seed`] |
//! | `/flip-after-range/{size}` | probe and plain GET see v1; a resume request sees v2 and gets a full `200` |
//! | `/drop/{size}/{after}` | first *full* request (no `Range`) drops the connection after `after` bytes |
//! | `/always-truncated/{size}` | declares `Content-Length: size` but always sends half |
//! | `/retry-429/{size}/{fails}` | first request succeeds; next `fails` requests return `429` |
//! | `/disposition/{size}?name=...` | serves a file with that `Content-Disposition` name |
//! | `/redirect/{hops}/{size}` | `hops` redirects, then `/ok/{size}` |
//! | `/auth/{size}` | always `401` |
//!
//! Keep test sizes to a few MiB: bodies are allocated in memory.

use std::collections::HashMap;
use std::io;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};

use axum::{
    Router,
    body::Body,
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use bytes::Bytes;
use futures_util::stream;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

/// Mutable server behaviour that individual tests can steer.
#[derive(Debug)]
pub struct TestState {
    etag: RwLock<String>,
    seed: AtomicU64,
    dropped: AtomicBool,
    retry_requests: AtomicUsize,
}

impl TestState {
    fn new() -> Self {
        Self {
            etag: RwLock::new("\"downpour-test\"".to_owned()),
            seed: AtomicU64::new(0),
            dropped: AtomicBool::new(false),
            retry_requests: AtomicUsize::new(0),
        }
    }

    fn etag(&self) -> String {
        self.etag
            .read()
            .map(|value| value.clone())
            .unwrap_or_else(|poisoned| poisoned.into_inner().clone())
    }

    fn set_etag(&self, value: &str) {
        match self.etag.write() {
            Ok(mut guard) => *guard = value.to_owned(),
            Err(poisoned) => *poisoned.into_inner() = value.to_owned(),
        }
    }

    fn seed(&self) -> u8 {
        self.seed.load(Ordering::SeqCst) as u8
    }
}

/// A running test server. Stops when dropped or when [`stop`](Self::stop) is awaited.
pub struct TestServer {
    pub addr: SocketAddr,
    state: Arc<TestState>,
    shutdown: Option<oneshot::Sender<()>>,
    handle: Option<JoinHandle<()>>,
}

impl TestServer {
    pub async fn start() -> std::io::Result<Self> {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?;
        let addr = listener.local_addr()?;
        let (shutdown, stop) = oneshot::channel();
        let state = Arc::new(TestState::new());

        let app = Router::new()
            .route("/health", get(health))
            .route("/ok/{size}", get(ok))
            .route("/no-range/{size}", get(no_range))
            .route("/versioned/{size}", get(versioned))
            .route("/flip-after-range/{size}", get(flip_after_range))
            .route("/drop/{size}/{after}", get(drop_after))
            .route("/always-truncated/{size}", get(always_truncated))
            .route("/retry-429/{size}/{fails}", get(retry_429))
            .route("/disposition/{size}", get(disposition))
            .route("/redirect/{hops}/{size}", get(redirect))
            .route("/auth/{size}", get(auth))
            .with_state(state.clone());

        let handle = tokio::spawn(async move {
            let _ = axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = stop.await;
                })
                .await;
        });

        Ok(Self {
            addr,
            state,
            shutdown: Some(shutdown),
            handle: Some(handle),
        })
    }

    #[must_use]
    pub fn url(&self, path: &str) -> String {
        format!("http://{}{}", self.addr, path)
    }

    /// Change the ETag (and the byte pattern) served by `/versioned` and friends.
    pub fn set_etag(&self, value: &str) {
        self.state.set_etag(value);
    }

    /// Change the byte-pattern seed used by `/versioned`.
    pub fn set_seed(&self, seed: u64) {
        self.state.seed.store(seed, Ordering::SeqCst);
    }

    pub async fn stop(mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(handle) = self.handle.take() {
            let _ = handle.await;
        }
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
    }
}

async fn health() -> &'static str {
    "ok"
}

async fn ok(
    Path(size): Path<u64>,
    State(state): State<Arc<TestState>>,
    headers: HeaderMap,
) -> Response {
    serve_range(size, &headers, &state, state.seed(), None)
}

async fn versioned(
    Path(size): Path<u64>,
    State(state): State<Arc<TestState>>,
    headers: HeaderMap,
) -> Response {
    serve_range(size, &headers, &state, state.seed(), None)
}

/// A resume request (`If-Range` present) sees a changed representation and
/// gets a full `200`; probe and initial requests see version 1.
async fn flip_after_range(
    Path(size): Path<u64>,
    State(_state): State<Arc<TestState>>,
    headers: HeaderMap,
) -> Response {
    if headers.contains_key(header::IF_RANGE) {
        serve_range_with(size, &headers, "\"downpour-v2\"", 1, None)
    } else {
        serve_range_with(size, &headers, "\"downpour-v1\"", 0, None)
    }
}

async fn no_range(Path(size): Path<u64>) -> Response {
    let body = if size == 0 {
        Vec::new()
    } else {
        pattern_body(0, size - 1, 0)
    };
    with_headers(StatusCode::OK, body, None, false, "\"downpour-test\"", None)
}

async fn drop_after(
    Path((size, after)): Path<(u64, u64)>,
    State(state): State<Arc<TestState>>,
    headers: HeaderMap,
) -> Response {
    // Probes send a Range request; they must not consume the drop.
    if headers.contains_key(header::RANGE) {
        return serve_range(size, &headers, &state, state.seed(), None);
    }
    let first_full = !state.dropped.swap(true, Ordering::SeqCst);
    if first_full {
        return truncated_response(size, after.min(size), true);
    }
    serve_range(size, &headers, &state, state.seed(), None)
}

async fn always_truncated(Path(size): Path<u64>) -> Response {
    truncated_response(size, size / 2, false)
}

fn truncated_response(size: u64, send: u64, drop: bool) -> Response {
    let body = if send == 0 {
        Vec::new()
    } else {
        pattern_body(0, send - 1, 0)
    };
    let chunks = if drop {
        vec![
            Ok::<_, io::Error>(Bytes::from(body)),
            Err(io::Error::new(
                io::ErrorKind::ConnectionReset,
                "downpour-test: connection dropped",
            )),
        ]
    } else {
        vec![Ok::<_, io::Error>(Bytes::from(body))]
    };

    let mut response = Response::new(Body::from_stream(stream::iter(chunks)));
    if let Ok(value) = HeaderValue::from_str(&size.to_string()) {
        response.headers_mut().insert(header::CONTENT_LENGTH, value);
    }
    response
}

async fn retry_429(
    Path((size, fails)): Path<(u64, u64)>,
    State(state): State<Arc<TestState>>,
    headers: HeaderMap,
) -> Response {
    let request_number = state.retry_requests.fetch_add(1, Ordering::SeqCst);
    // Request 0 is the probe; the next `fails` requests are throttled.
    if request_number >= 1 && (request_number as u64) <= fails {
        let mut response = StatusCode::TOO_MANY_REQUESTS.into_response();
        response
            .headers_mut()
            .insert(header::RETRY_AFTER, HeaderValue::from_static("0"));
        return response;
    }
    serve_range(size, &headers, &state, state.seed(), None)
}

async fn disposition(
    Path(size): Path<u64>,
    State(state): State<Arc<TestState>>,
    Query(query): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let name = query
        .get("name")
        .cloned()
        .unwrap_or_else(|| "download".to_owned());
    let mut response = serve_range(size, &headers, &state, state.seed(), None);
    if let Ok(value) = HeaderValue::from_str(&format!("attachment; filename=\"{name}\"")) {
        response
            .headers_mut()
            .insert(header::CONTENT_DISPOSITION, value);
    }
    response
}

async fn redirect(
    Path((hops, size)): Path<(u32, u64)>,
    State(state): State<Arc<TestState>>,
) -> Response {
    if hops == 0 {
        return serve_range(size, &HeaderMap::new(), &state, state.seed(), None);
    }
    let mut response = StatusCode::FOUND.into_response();
    if let Ok(value) = HeaderValue::from_str(&format!("/redirect/{}/{size}", hops - 1)) {
        response.headers_mut().insert(header::LOCATION, value);
    }
    response
}

async fn auth(Path(_size): Path<u64>) -> Response {
    StatusCode::UNAUTHORIZED.into_response()
}

/// Serve a byte range, honouring `If-Range` the way a correct server would.
fn serve_range(
    size: u64,
    headers: &HeaderMap,
    state: &TestState,
    seed: u8,
    disposition: Option<&str>,
) -> Response {
    let etag = state.etag();
    serve_range_with(size, headers, &etag, seed, disposition)
}

fn serve_range_with(
    size: u64,
    headers: &HeaderMap,
    etag: &str,
    seed: u8,
    disposition: Option<&str>,
) -> Response {
    // A validator mismatch means the representation changed: send the whole body.
    let if_range_matches = headers
        .get(header::IF_RANGE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value == etag);
    let if_range_present = headers.contains_key(header::IF_RANGE);
    let ignore_range = if_range_present && !if_range_matches;

    if size == 0 {
        return with_headers(StatusCode::OK, Vec::new(), None, true, etag, disposition);
    }

    let requested = if ignore_range {
        None
    } else {
        headers
            .get(header::RANGE)
            .and_then(|value| value.to_str().ok())
            .and_then(parse_range)
    };

    match requested {
        Some((start, requested_end)) if start < size => {
            let end = requested_end.min(size - 1);
            let body = pattern_body(start, end, seed);
            let content_range = format!("bytes {start}-{end}/{size}");
            with_headers(
                StatusCode::PARTIAL_CONTENT,
                body,
                Some(content_range),
                true,
                etag,
                disposition,
            )
        }
        Some(_) if !ignore_range => {
            let mut response = StatusCode::RANGE_NOT_SATISFIABLE.into_response();
            if let Ok(value) = HeaderValue::from_str(&format!("bytes */{size}")) {
                response.headers_mut().insert(header::CONTENT_RANGE, value);
            }
            response
        }
        _ => with_headers(
            StatusCode::OK,
            pattern_body(0, size - 1, seed),
            None,
            true,
            etag,
            disposition,
        ),
    }
}

fn with_headers(
    status: StatusCode,
    body: Vec<u8>,
    content_range: Option<String>,
    accept_ranges: bool,
    etag: &str,
    disposition: Option<&str>,
) -> Response {
    let mut response = (status, Body::from(body)).into_response();
    let headers = response.headers_mut();

    if let Ok(value) = HeaderValue::from_str(etag) {
        headers.insert(header::ETAG, value);
    }
    if accept_ranges {
        headers.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    }
    if let Some(value) = content_range {
        if let Ok(value) = HeaderValue::from_str(&value) {
            headers.insert(header::CONTENT_RANGE, value);
        }
    }
    if let Some(value) = disposition {
        if let Ok(value) = HeaderValue::from_str(value) {
            headers.insert(header::CONTENT_DISPOSITION, value);
        }
    }

    response
}

/// Deterministic body byte at an absolute offset.
#[must_use]
pub fn pattern_byte(offset: u64) -> u8 {
    (offset % 251) as u8
}

/// Deterministic body byte with a per-version xor seed.
#[must_use]
pub fn versioned_byte(offset: u64, seed: u8) -> u8 {
    pattern_byte(offset) ^ seed
}

fn pattern_body(start: u64, end_inclusive: u64, seed: u8) -> Vec<u8> {
    let length = (end_inclusive - start + 1) as usize;
    (0..length)
        .map(|index| versioned_byte(start + index as u64, seed))
        .collect()
}

/// Parse `bytes=start-end` (end optional).
#[must_use]
pub fn parse_range(value: &str) -> Option<(u64, u64)> {
    let rest = value.strip_prefix("bytes=")?;
    let (start, end) = rest.split_once('-')?;
    let start: u64 = start.trim().parse().ok()?;
    let end: u64 = if end.trim().is_empty() {
        u64::MAX
    } else {
        end.trim().parse().ok()?
    };
    (start <= end).then_some((start, end))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn serves_a_range() {
        let server = TestServer::start().await.unwrap();
        let response = reqwest::Client::new()
            .get(server.url("/ok/1000"))
            .header(header::RANGE, "bytes=10-19")
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
        let body = response.bytes().await.unwrap();
        assert_eq!(body.len(), 10);
        assert_eq!(body[0], pattern_byte(10));
        server.stop().await;
    }

    #[tokio::test]
    async fn no_range_route_ignores_range() {
        let server = TestServer::start().await.unwrap();
        let response = reqwest::Client::new()
            .get(server.url("/no-range/500"))
            .header(header::RANGE, "bytes=0-9")
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.bytes().await.unwrap().len(), 500);
        server.stop().await;
    }

    #[tokio::test]
    async fn if_range_mismatch_returns_full_body() {
        let server = TestServer::start().await.unwrap();
        let response = reqwest::Client::new()
            .get(server.url("/ok/1000"))
            .header(header::RANGE, "bytes=500-999")
            .header(header::IF_RANGE, "\"stale\"")
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.bytes().await.unwrap().len(), 1000);
        server.stop().await;
    }

    #[tokio::test]
    async fn versioned_body_changes_with_seed() {
        let server = TestServer::start().await.unwrap();
        server.set_seed(7);
        server.set_etag("\"v7\"");
        let response = reqwest::Client::new()
            .get(server.url("/versioned/10"))
            .send()
            .await
            .unwrap();
        assert_eq!(response.headers()[header::ETAG], "\"v7\"");
        let body = response.bytes().await.unwrap();
        assert_eq!(body[3], versioned_byte(3, 7));
        server.stop().await;
    }

    #[tokio::test]
    async fn retry_429_throttles_once() {
        let server = TestServer::start().await.unwrap();
        let client = reqwest::Client::new();

        let probe = client
            .get(server.url("/retry-429/100/1"))
            .send()
            .await
            .unwrap();
        assert_eq!(probe.status(), StatusCode::OK);

        let throttled = client
            .get(server.url("/retry-429/100/1"))
            .send()
            .await
            .unwrap();
        assert_eq!(throttled.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(throttled.headers()[header::RETRY_AFTER], "0");

        let ok = client
            .get(server.url("/retry-429/100/1"))
            .send()
            .await
            .unwrap();
        assert_eq!(ok.status(), StatusCode::OK);
        server.stop().await;
    }

    #[test]
    fn parses_ranges() {
        assert_eq!(parse_range("bytes=0-99"), Some((0, 99)));
        assert_eq!(parse_range("bytes=100-"), Some((100, u64::MAX)));
        assert_eq!(parse_range("bytes=99-0"), None);
        assert_eq!(parse_range("items=0-1"), None);
    }
}
