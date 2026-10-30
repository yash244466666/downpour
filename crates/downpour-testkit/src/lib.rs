//! Fault-injecting HTTP server and fixtures for engine tests.
//!
//! The bodies are generated from a deterministic byte pattern so tests can
//! verify byte-exact resumes without shipping fixtures.
//!
//! Keep test sizes to a few MiB: bodies are allocated in memory.

use std::net::SocketAddr;

use axum::{
    Router,
    body::Body,
    extract::Path,
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

/// A running test server. Stops when dropped or when [`stop`](Self::stop) is awaited.
pub struct TestServer {
    pub addr: SocketAddr,
    shutdown: Option<oneshot::Sender<()>>,
    handle: Option<JoinHandle<()>>,
}

impl TestServer {
    pub async fn start() -> std::io::Result<Self> {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?;
        let addr = listener.local_addr()?;
        let (shutdown, stop) = oneshot::channel();

        let app = Router::new()
            .route("/health", get(health))
            .route("/ok/{size}", get(ok))
            .route("/no-range/{size}", get(no_range));

        let handle = tokio::spawn(async move {
            let _ = axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = stop.await;
                })
                .await;
        });

        Ok(Self {
            addr,
            shutdown: Some(shutdown),
            handle: Some(handle),
        })
    }

    #[must_use]
    pub fn url(&self, path: &str) -> String {
        format!("http://{}{}", self.addr, path)
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

async fn ok(Path(size): Path<u64>, headers: HeaderMap) -> Response {
    if size == 0 {
        return with_headers(StatusCode::OK, Vec::new(), None, true);
    }

    let requested = headers
        .get(header::RANGE)
        .and_then(|value| value.to_str().ok())
        .and_then(parse_range);

    match requested {
        Some((start, requested_end)) if start < size => {
            let end = requested_end.min(size - 1);
            let body = pattern_body(start, end);
            let content_range = format!("bytes {start}-{end}/{size}");
            with_headers(StatusCode::PARTIAL_CONTENT, body, Some(content_range), true)
        }
        Some(_) => {
            let mut response = StatusCode::RANGE_NOT_SATISFIABLE.into_response();
            if let Ok(value) = HeaderValue::from_str(&format!("bytes */{size}")) {
                response.headers_mut().insert(header::CONTENT_RANGE, value);
            }
            response
        }
        None => with_headers(StatusCode::OK, pattern_body(0, size - 1), None, true),
    }
}

async fn no_range(Path(size): Path<u64>) -> Response {
    let body = if size == 0 {
        Vec::new()
    } else {
        pattern_body(0, size - 1)
    };
    with_headers(StatusCode::OK, body, None, false)
}

fn with_headers(
    status: StatusCode,
    body: Vec<u8>,
    content_range: Option<String>,
    accept_ranges: bool,
) -> Response {
    let mut response = (status, Body::from(body)).into_response();
    let headers = response.headers_mut();

    headers.insert(header::ETAG, HeaderValue::from_static("\"downpour-test\""));
    if accept_ranges {
        headers.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    }
    if let Some(value) = content_range {
        if let Ok(value) = HeaderValue::from_str(&value) {
            headers.insert(header::CONTENT_RANGE, value);
        }
    }

    response
}

/// Deterministic body byte at an absolute offset.
#[must_use]
pub fn pattern_byte(offset: u64) -> u8 {
    (offset % 251) as u8
}

fn pattern_body(start: u64, end_inclusive: u64) -> Vec<u8> {
    let length = (end_inclusive - start + 1) as usize;
    (0..length)
        .map(|index| pattern_byte(start + index as u64))
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

    #[test]
    fn parses_ranges() {
        assert_eq!(parse_range("bytes=0-99"), Some((0, 99)));
        assert_eq!(parse_range("bytes=100-"), Some((100, u64::MAX)));
        assert_eq!(parse_range("bytes=99-0"), None);
        assert_eq!(parse_range("items=0-1"), None);
    }
}
