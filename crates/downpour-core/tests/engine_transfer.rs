//! End-to-end engine tests against the local fault-injecting server.
//!
//! No test touches the public internet: every transfer runs against
//! `downpour-testkit` on loopback.

use std::path::Path;
use std::time::Duration;

use downpour_core::{
    CancelToken, Engine, EngineConfig, EngineError, ProgressSample, ProgressSink, RetryPolicy,
    RunOutcome,
};
use downpour_testkit::{TestServer, pattern_byte, versioned_byte};
use downpour_types::{AddDownloadRequest, DownloadState};

fn test_engine(dir: &Path) -> Engine {
    let config = EngineConfig {
        default_dir: dir.join("downloads"),
        checkpoint_interval: Duration::from_millis(20),
        checkpoint_bytes: 32 * 1024,
        retry: RetryPolicy {
            max_attempts: 6,
            base_delay: Duration::from_millis(5),
            max_delay: Duration::from_millis(50),
        },
        ..EngineConfig::default()
    };
    Engine::open(dir.join("engine.db"), config).expect("open engine")
}

fn add(engine: &mut Engine, dir: &Path, url: String) -> downpour_types::DownloadId {
    add_with(engine, dir, url, None, None)
}

fn add_with(
    engine: &mut Engine,
    dir: &Path,
    url: String,
    connections: Option<u16>,
    speed_limit: Option<u64>,
) -> downpour_types::DownloadId {
    engine
        .add(AddDownloadRequest {
            url,
            dest_dir: Some(dir.join("downloads").to_string_lossy().into_owned()),
            connections,
            speed_limit,
            ..AddDownloadRequest::default()
        })
        .expect("add")
}

fn expected(size: u64, seed: u8) -> Vec<u8> {
    (0..size)
        .map(|offset| versioned_byte(offset, seed))
        .collect()
}

fn final_bytes(engine: &Engine, id: downpour_types::DownloadId) -> Vec<u8> {
    let record = engine.get(id).unwrap().expect("record");
    let path = record.final_path.expect("final path");
    std::fs::read(path).expect("read downloaded file")
}

#[tokio::test]
async fn downloads_a_file_byte_exact() {
    let server = TestServer::start().await.unwrap();
    let temp = tempfile::tempdir().unwrap();
    let mut engine = test_engine(temp.path());

    let size = 262_144;
    let id = add(&mut engine, temp.path(), server.url(&format!("/ok/{size}")));
    let outcome = engine.run(id, CancelToken::new(), None).await.unwrap();

    assert_eq!(outcome, RunOutcome::Completed);
    let record = engine.get(id).unwrap().unwrap();
    assert_eq!(record.state, DownloadState::Completed);
    assert_eq!(record.downloaded_bytes, size);
    assert_eq!(record.total_bytes, Some(size));
    assert!(
        record
            .checksum_sha256
            .as_deref()
            .is_some_and(|value| value.len() == 64)
    );
    assert_eq!(final_bytes(&engine, id), expected(size, 0));

    server.stop().await;
}

#[tokio::test]
async fn empty_file_completes() {
    let server = TestServer::start().await.unwrap();
    let temp = tempfile::tempdir().unwrap();
    let mut engine = test_engine(temp.path());

    let id = add(&mut engine, temp.path(), server.url("/ok/0"));
    let outcome = engine.run(id, CancelToken::new(), None).await.unwrap();

    assert_eq!(outcome, RunOutcome::Completed);
    assert_eq!(final_bytes(&engine, id), Vec::<u8>::new());
    server.stop().await;
}

#[tokio::test]
async fn no_range_server_uses_one_stream() {
    let server = TestServer::start().await.unwrap();
    let temp = tempfile::tempdir().unwrap();
    let mut engine = test_engine(temp.path());

    let size = 200_000;
    let id = add(
        &mut engine,
        temp.path(),
        server.url(&format!("/no-range/{size}")),
    );
    let outcome = engine.run(id, CancelToken::new(), None).await.unwrap();

    assert_eq!(outcome, RunOutcome::Completed);
    let record = engine.get(id).unwrap().unwrap();
    assert_eq!(record.supports_ranges, Some(false));
    assert_eq!(final_bytes(&engine, id), expected(size, 0));
    server.stop().await;
}

#[tokio::test]
async fn pause_then_resume_completes_byte_exact() {
    let server = TestServer::start().await.unwrap();
    let temp = tempfile::tempdir().unwrap();
    let mut engine = test_engine(temp.path());

    let size = 512 * 1024;
    let id = add(&mut engine, temp.path(), server.url(&format!("/ok/{size}")));

    let cancel = CancelToken::new();
    let trigger = cancel.clone();
    let sink = move |sample: ProgressSample| {
        if sample.downloaded_bytes >= 64 * 1024 {
            trigger.cancel();
        }
    };
    let progress: ProgressSink<'_> = &sink;

    let first = engine.run(id, cancel, Some(progress)).await.unwrap();
    assert_eq!(first, RunOutcome::Paused);

    let paused = engine.get(id).unwrap().unwrap();
    assert_eq!(paused.state, DownloadState::Paused);
    assert!(paused.downloaded_bytes > 0);
    assert!(paused.downloaded_bytes < size);

    let second = engine.run(id, CancelToken::new(), None).await.unwrap();
    assert_eq!(second, RunOutcome::Completed);
    assert_eq!(final_bytes(&engine, id), expected(size, 0));

    server.stop().await;
}

#[tokio::test]
async fn resume_survives_engine_restart() {
    let server = TestServer::start().await.unwrap();
    let temp = tempfile::tempdir().unwrap();
    let size = 384 * 1024;

    let id = {
        let mut engine = test_engine(temp.path());
        let id = add(&mut engine, temp.path(), server.url(&format!("/ok/{size}")));
        let cancel = CancelToken::new();
        let trigger = cancel.clone();
        let sink = move |sample: ProgressSample| {
            if sample.downloaded_bytes >= 64 * 1024 {
                trigger.cancel();
            }
        };
        let progress: ProgressSink<'_> = &sink;
        assert_eq!(
            engine.run(id, cancel, Some(progress)).await.unwrap(),
            RunOutcome::Paused
        );
        id
    };

    let mut reopened = test_engine(temp.path());
    assert_eq!(
        reopened.run(id, CancelToken::new(), None).await.unwrap(),
        RunOutcome::Completed
    );
    assert_eq!(final_bytes(&reopened, id), expected(size, 0));

    server.stop().await;
}

#[tokio::test]
async fn recovers_after_a_crash_and_discards_unsynced_tail() {
    let server = TestServer::start().await.unwrap();
    let temp = tempfile::tempdir().unwrap();
    let size = 384 * 1024;

    let id = {
        let mut engine = test_engine(temp.path());
        let id = add(&mut engine, temp.path(), server.url(&format!("/ok/{size}")));
        let cancel = CancelToken::new();
        let trigger = cancel.clone();
        let sink = move |sample: ProgressSample| {
            if sample.downloaded_bytes >= 64 * 1024 {
                trigger.cancel();
            }
        };
        let progress: ProgressSink<'_> = &sink;
        assert_eq!(
            engine.run(id, cancel, Some(progress)).await.unwrap(),
            RunOutcome::Paused
        );
        id
    };

    // Simulate a hard kill: the row still says `downloading`, and the temp file
    // has bytes past the last checkpoint that may not be durable.
    {
        let store = downpour_store::Store::open(temp.path().join("engine.db")).unwrap();
        store
            .connection()
            .execute(
                "UPDATE downloads SET status = 'downloading' WHERE id = ?1",
                [id.to_string()],
            )
            .unwrap();
    }
    let record = {
        let store = downpour_store::Store::open(temp.path().join("engine.db")).unwrap();
        downpour_store::downloads::get(store.connection(), id)
            .unwrap()
            .unwrap()
    };
    let temp_path = std::path::PathBuf::from(record.temp_path.unwrap());
    {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&temp_path)
            .unwrap();
        file.write_all(&[0xAB; 8192]).unwrap();
    }

    let mut recovered = test_engine(temp.path());
    let after_open = recovered.get(id).unwrap().unwrap();
    assert_eq!(after_open.state, DownloadState::Paused);
    assert!(after_open.downloaded_bytes > 0);

    assert_eq!(
        recovered.run(id, CancelToken::new(), None).await.unwrap(),
        RunOutcome::Completed
    );
    assert_eq!(final_bytes(&recovered, id), expected(size, 0));

    server.stop().await;
}

#[tokio::test]
async fn changed_etag_restarts_from_zero() {
    let server = TestServer::start().await.unwrap();
    let temp = tempfile::tempdir().unwrap();
    let mut engine = test_engine(temp.path());

    server.set_etag("\"v1\"");
    server.set_seed(0);

    let size = 256 * 1024;
    let id = add(
        &mut engine,
        temp.path(),
        server.url(&format!("/versioned/{size}")),
    );

    let cancel = CancelToken::new();
    let trigger = cancel.clone();
    let sink = move |sample: ProgressSample| {
        if sample.downloaded_bytes >= 64 * 1024 {
            trigger.cancel();
        }
    };
    let progress: ProgressSink<'_> = &sink;
    assert_eq!(
        engine.run(id, cancel, Some(progress)).await.unwrap(),
        RunOutcome::Paused
    );

    // The representation changes while the download is paused.
    server.set_etag("\"v2\"");
    server.set_seed(1);

    assert_eq!(
        engine.run(id, CancelToken::new(), None).await.unwrap(),
        RunOutcome::Completed
    );
    assert_eq!(final_bytes(&engine, id), expected(size, 1));

    server.stop().await;
}

#[tokio::test]
async fn server_restarting_content_mid_transfer_is_handled() {
    let server = TestServer::start().await.unwrap();
    let temp = tempfile::tempdir().unwrap();
    let mut engine = test_engine(temp.path());

    let size = 256 * 1024;
    let id = add(
        &mut engine,
        temp.path(),
        server.url(&format!("/flip-after-range/{size}")),
    );

    let cancel = CancelToken::new();
    let trigger = cancel.clone();
    let sink = move |sample: ProgressSample| {
        if sample.downloaded_bytes >= 64 * 1024 {
            trigger.cancel();
        }
    };
    let progress: ProgressSink<'_> = &sink;
    assert_eq!(
        engine.run(id, cancel, Some(progress)).await.unwrap(),
        RunOutcome::Paused
    );

    // The resume request gets a full 200 for a different representation.
    assert_eq!(
        engine.run(id, CancelToken::new(), None).await.unwrap(),
        RunOutcome::Completed
    );
    assert_eq!(final_bytes(&engine, id), expected(size, 1));

    server.stop().await;
}

#[tokio::test]
async fn retries_after_throttling() {
    let server = TestServer::start().await.unwrap();
    let temp = tempfile::tempdir().unwrap();
    let mut engine = test_engine(temp.path());

    let size = 128 * 1024;
    let id = add(
        &mut engine,
        temp.path(),
        server.url(&format!("/retry-429/{size}/2")),
    );

    assert_eq!(
        engine.run(id, CancelToken::new(), None).await.unwrap(),
        RunOutcome::Completed
    );
    assert_eq!(final_bytes(&engine, id), expected(size, 0));
    server.stop().await;
}

#[tokio::test]
async fn resumes_after_a_dropped_connection() {
    let server = TestServer::start().await.unwrap();
    let temp = tempfile::tempdir().unwrap();
    let mut engine = test_engine(temp.path());

    let size = 256 * 1024;
    let id = add(
        &mut engine,
        temp.path(),
        server.url(&format!("/drop/{size}/98304")),
    );

    assert_eq!(
        engine.run(id, CancelToken::new(), None).await.unwrap(),
        RunOutcome::Completed
    );
    assert_eq!(final_bytes(&engine, id), expected(size, 0));
    server.stop().await;
}

#[tokio::test]
async fn a_permanently_truncated_response_fails() {
    let server = TestServer::start().await.unwrap();
    let temp = tempfile::tempdir().unwrap();

    let config = EngineConfig {
        default_dir: temp.path().join("downloads"),
        checkpoint_interval: Duration::from_millis(10),
        checkpoint_bytes: 16 * 1024,
        retry: RetryPolicy {
            max_attempts: 3,
            base_delay: Duration::from_millis(5),
            max_delay: Duration::from_millis(20),
        },
        ..EngineConfig::default()
    };
    let mut engine = Engine::open(temp.path().join("engine.db"), config).unwrap();

    let id = add(
        &mut engine,
        temp.path(),
        server.url("/always-truncated/131072"),
    );
    let error = engine
        .run(id, CancelToken::new(), None)
        .await
        .expect_err("must fail");

    // Depending on where hyper notices the short body the surface error is
    // either a truncated stream or a network error; both must be terminal
    // after the retry budget.
    assert!(
        matches!(
            error,
            EngineError::Truncated { .. } | EngineError::Network(_)
        ),
        "unexpected error: {error}"
    );
    let record = engine.get(id).unwrap().unwrap();
    assert_eq!(record.state, DownloadState::Failed);
    assert!(record.error.is_some());
    server.stop().await;
}

#[tokio::test]
async fn hostile_disposition_name_stays_inside_the_destination() {
    let server = TestServer::start().await.unwrap();
    let temp = tempfile::tempdir().unwrap();
    let mut engine = test_engine(temp.path());

    let id = add(
        &mut engine,
        temp.path(),
        server.url("/disposition/4096?name=..%2F..%2Fetc%2Fpasswd"),
    );
    assert_eq!(
        engine.run(id, CancelToken::new(), None).await.unwrap(),
        RunOutcome::Completed
    );

    let record = engine.get(id).unwrap().unwrap();
    let final_path = std::path::PathBuf::from(record.final_path.unwrap());
    assert_eq!(
        final_path.parent().unwrap(),
        temp.path().join("downloads").as_path()
    );
    assert_eq!(final_path.file_name().unwrap(), "passwd");
    assert_eq!(std::fs::read(&final_path).unwrap(), expected(4096, 0));

    server.stop().await;
}

#[tokio::test]
async fn follows_redirects_and_downloads() {
    let server = TestServer::start().await.unwrap();
    let temp = tempfile::tempdir().unwrap();
    let mut engine = test_engine(temp.path());

    let id = add(&mut engine, temp.path(), server.url("/redirect/3/5000"));
    assert_eq!(
        engine.run(id, CancelToken::new(), None).await.unwrap(),
        RunOutcome::Completed
    );
    assert_eq!(final_bytes(&engine, id), expected(5000, 0));
    server.stop().await;
}

#[tokio::test]
async fn authentication_errors_are_reported() {
    let server = TestServer::start().await.unwrap();
    let temp = tempfile::tempdir().unwrap();
    let mut engine = test_engine(temp.path());

    let id = add(&mut engine, temp.path(), server.url("/auth/1000"));
    let error = engine
        .run(id, CancelToken::new(), None)
        .await
        .expect_err("must fail");

    assert!(matches!(error, EngineError::AuthRequired));
    let record = engine.get(id).unwrap().unwrap();
    assert_eq!(record.state, DownloadState::Failed);
    assert_eq!(
        record.error.unwrap().code,
        downpour_types::ErrorCode::AuthRequired
    );
    server.stop().await;
}

#[test]
fn secret_headers_are_not_persisted() {
    let temp = tempfile::tempdir().unwrap();
    let mut engine = test_engine(temp.path());

    let mut headers = std::collections::BTreeMap::new();
    headers.insert("Authorization".to_owned(), "Bearer secret".to_owned());

    let error = engine
        .add(AddDownloadRequest {
            url: "http://127.0.0.1:1/file".to_owned(),
            headers,
            ..AddDownloadRequest::default()
        })
        .expect_err("must reject");

    assert!(matches!(error, EngineError::InvalidHeader(_)));
}

#[tokio::test]
async fn unsupported_scheme_is_rejected_at_add_time() {
    let temp = tempfile::tempdir().unwrap();
    let mut engine = test_engine(temp.path());

    let error = engine
        .add(AddDownloadRequest {
            url: "ftp://example.com/file".to_owned(),
            ..AddDownloadRequest::default()
        })
        .expect_err("must reject");

    assert!(matches!(error, EngineError::UnsupportedScheme(_)));
}

#[tokio::test]
async fn segmented_transfer_uses_multiple_connections() {
    let server = TestServer::start().await.unwrap();
    let temp = tempfile::tempdir().unwrap();
    let mut engine = test_engine(temp.path());

    let size = 4 * 1024 * 1024;
    let id = add_with(
        &mut engine,
        temp.path(),
        server.url(&format!("/slow/{size}/64/2")),
        Some(4),
        None,
    );

    assert_eq!(
        engine.run(id, CancelToken::new(), None).await.unwrap(),
        RunOutcome::Completed
    );
    assert_eq!(final_bytes(&engine, id), expected(size, 0));
    assert_eq!(engine.get(id).unwrap().unwrap().connections, 4);

    let ranges = server.ranges_seen();
    let starts: std::collections::BTreeSet<String> = ranges
        .iter()
        .filter_map(|value| value.strip_prefix("bytes="))
        .filter_map(|value| value.split('-').next())
        .map(str::to_owned)
        .collect();
    assert!(
        starts.len() >= 2,
        "expected several range starts, saw {ranges:?}"
    );
    assert!(ranges.len() >= 4, "expected several range requests");

    server.stop().await;
}

#[tokio::test]
async fn segmented_transfer_pauses_and_resumes() {
    let server = TestServer::start().await.unwrap();
    let temp = tempfile::tempdir().unwrap();
    let mut engine = test_engine(temp.path());

    let size = 4 * 1024 * 1024;
    let id = add_with(
        &mut engine,
        temp.path(),
        // Slow enough that the pause lands well before completion on fast
        // runners; 16 KiB chunks at 10 ms keep the test deterministic.
        server.url(&format!("/slow/{size}/16/10")),
        Some(4),
        None,
    );

    let cancel = CancelToken::new();
    let trigger = cancel.clone();
    let sink = move |sample: ProgressSample| {
        if sample.downloaded_bytes >= 512 * 1024 {
            trigger.cancel();
        }
    };
    let progress: ProgressSink<'_> = &sink;
    assert_eq!(
        engine.run(id, cancel, Some(progress)).await.unwrap(),
        RunOutcome::Paused
    );

    let paused = engine.get(id).unwrap().unwrap();
    assert_eq!(paused.state, DownloadState::Paused);
    assert!(paused.downloaded_bytes > 0);
    assert!(paused.downloaded_bytes < size);

    assert_eq!(
        engine.run(id, CancelToken::new(), None).await.unwrap(),
        RunOutcome::Completed
    );
    assert_eq!(final_bytes(&engine, id), expected(size, 0));

    server.stop().await;
}

#[tokio::test]
async fn speed_limit_throttles_a_single_stream() {
    let server = TestServer::start().await.unwrap();
    let temp = tempfile::tempdir().unwrap();
    let mut engine = test_engine(temp.path());

    let size = 256 * 1024;
    let id = add_with(
        &mut engine,
        temp.path(),
        server.url(&format!("/ok/{size}")),
        None,
        Some(128 * 1024),
    );

    let started = std::time::Instant::now();
    assert_eq!(
        engine.run(id, CancelToken::new(), None).await.unwrap(),
        RunOutcome::Completed
    );
    let elapsed = started.elapsed();
    assert!(
        elapsed >= std::time::Duration::from_millis(1000),
        "256 KiB at 128 KiB/s finished in {elapsed:?}"
    );
    assert_eq!(final_bytes(&engine, id), expected(size, 0));

    server.stop().await;
}

#[test]
fn passive_engines_do_not_recover_running_downloads() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("engine.db");
    let id = {
        let mut engine = Engine::open(&db, EngineConfig::default()).unwrap();
        engine
            .add(AddDownloadRequest {
                url: "http://127.0.0.1:1/file".to_owned(),
                ..AddDownloadRequest::default()
            })
            .unwrap()
    };
    {
        let store = downpour_store::Store::open(&db).unwrap();
        store
            .connection()
            .execute(
                "UPDATE downloads SET status = 'downloading' WHERE id = ?1",
                [id.to_string()],
            )
            .unwrap();
    }

    let passive = Engine::open_without_recovery(&db, EngineConfig::default()).unwrap();
    assert_eq!(
        passive.get(id).unwrap().unwrap().state,
        DownloadState::Downloading
    );
    drop(passive);

    let active = Engine::open(&db, EngineConfig::default()).unwrap();
    assert_eq!(
        active.get(id).unwrap().unwrap().state,
        DownloadState::Paused
    );
}

#[test]
fn run_future_is_send() {
    fn assert_send<F: std::future::Future + Send>(_: F) {}

    let temp = tempfile::tempdir().unwrap();
    let mut engine = test_engine(temp.path());
    let id = engine
        .add(AddDownloadRequest {
            url: "http://127.0.0.1:1/file".to_owned(),
            ..AddDownloadRequest::default()
        })
        .unwrap();

    // Never polled: this is a compile-time bound check that the future can be
    // spawned by the UI runtime (M3).
    assert_send(engine.run(id, CancelToken::new(), None));
}

#[test]
fn pattern_fixture_is_deterministic() {
    assert_eq!(pattern_byte(0), 0);
    assert_eq!(pattern_byte(251), 0);
    assert_eq!(versioned_byte(5, 1), pattern_byte(5) ^ 1);
}
