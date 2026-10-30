//! Process-level crash test: SIGKILL a running download, restart, and verify
//! the file is byte-exact.
//!
//! Unix only: Windows uses a different kill mechanism and the test harness is
//! covered by the engine restart tests there.

#![cfg(unix)]

use std::process::{Command, Stdio};
use std::time::Duration;

use downpour_testkit::{TestServer, pattern_byte};

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_downpour")
}

fn cli(db: &std::path::Path, dir: &std::path::Path) -> Command {
    let mut command = Command::new(binary());
    command.args([
        "--db",
        db.to_str().expect("utf-8 db path"),
        "--dir",
        dir.to_str().expect("utf-8 dir path"),
    ]);
    command
}

#[tokio::test(flavor = "multi_thread")]
async fn resumes_after_sigkill() {
    let server = TestServer::start().await.unwrap();
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("crash.db");
    let dir = temp.path().join("downloads");

    let size = 16 * 1024 * 1024;
    let url = server.url(&format!("/slow/{size}/16/30"));

    let queued = cli(&db, &dir)
        .args(["add", &url, "-n", "crash.bin"])
        .output()
        .expect("spawn add");
    assert!(
        queued.status.success(),
        "add failed: {}",
        String::from_utf8_lossy(&queued.stderr)
    );
    let stdout = String::from_utf8_lossy(&queued.stdout);
    let id = stdout
        .split_whitespace()
        .last()
        .expect("download id")
        .to_owned();

    let mut child = cli(&db, &dir)
        .args(["run", &id, "--quiet"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn run");

    tokio::time::sleep(Duration::from_millis(1500)).await;

    // SIGKILL: no graceful shutdown, no final checkpoint.
    child.kill().expect("kill child");
    let _ = child.wait();

    let listed = cli(&db, &dir)
        .args(["list", "--json"])
        .output()
        .expect("spawn list");
    assert!(
        listed.status.success(),
        "list failed: {}",
        String::from_utf8_lossy(&listed.stderr)
    );
    let record: serde_json::Value = serde_json::from_slice(&listed.stdout).expect("json record");
    let downloaded = record["downloaded_bytes"].as_u64().unwrap_or(0);
    assert!(
        downloaded > 0,
        "the process was killed before any checkpoint committed"
    );
    assert!(downloaded < size);

    let resumed = cli(&db, &dir)
        .args(["run", &id, "--quiet"])
        .status()
        .expect("spawn resume");
    assert!(resumed.success(), "resume failed");

    let contents = std::fs::read(dir.join("crash.bin")).expect("downloaded file");
    assert_eq!(contents.len(), size as usize);
    let expected: Vec<u8> = (0..size).map(pattern_byte).collect();
    assert_eq!(contents, expected);

    server.stop().await;
}
