//! Browser native-messaging host.
//!
//! Browsers frame messages as a 4-byte little-endian length followed by JSON.
//! Chrome caps host→browser messages at 1 MB, so this binary only ever
//! forwards commands and compact status summaries — never file bytes.
//!
//! Each browser message is translated to one engine IPC request:
//! `add` → `download.add`, `list` → `download.list`, and so on.

use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::time::Duration;

use downpour_ipc::{ClientKind, IpcClient, Request};
use serde_json::{Value, json};

const MAX_HOST_TO_BROWSER: usize = 1024 * 1024;
const MAX_BROWSER_TO_HOST: usize = 64 * 1024 * 1024;

/// Extension-facing compatibility version. Bumped when a host change requires
/// newer extension behaviour; extensions compare it with their own constant and
/// fall back to the browser downloader when the host is too old.
const EXTENSION_PROTOCOL: u32 = 1;

fn main() {
    if let Err(error) = run() {
        eprintln!("downpour-nm-host: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    validate_invocation()?;

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    runtime.block_on(async {
        let mut client = connect_with_retry().await?;
        let mut stdin = io::stdin().lock();
        let mut stdout = io::stdout().lock();
        let mut next_id = 1_u64;

        while let Some(frame) = read_browser_frame(&mut stdin)? {
            let message: Value = match serde_json::from_slice(&frame) {
                Ok(message) => message,
                Err(error) => {
                    write_browser_message(
                        &mut stdout,
                        &json!({"error": {"code": "invalid_json", "message": error.to_string()}}),
                    )?;
                    continue;
                }
            };

            let method = message
                .get("method")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let engine_method = match method {
                "ping" => "ping",
                "add" => "download.add",
                "list" => "download.list",
                "pause" => "download.pause",
                "resume" => "download.resume",
                "cancel" => "download.cancel",
                "remove" => "download.remove",
                other => {
                    write_browser_message(
                        &mut stdout,
                        &json!({
                            "error": {
                                "code": "unknown_method",
                                "message": format!("unknown method: {other}")
                            }
                        }),
                    )?;
                    continue;
                }
            };

            let request = Request {
                id: next_id,
                method: engine_method.to_owned(),
                params: message.get("params").cloned().unwrap_or(Value::Null),
            };
            next_id += 1;

            let response = match client.call(&request).await {
                Ok(response) => response,
                Err(error) => {
                    write_browser_message(
                        &mut stdout,
                        &json!({"error": {"code": "engine_error", "message": error.to_string()}}),
                    )?;
                    continue;
                }
            };

            let envelope = match (response.result, response.error) {
                (Some(result), _) => success_envelope(result),
                (None, Some(error)) => json!({ "error": error }),
                (None, None) => json!({
                    "error": {"code": "internal", "message": "empty engine response"}
                }),
            };
            write_browser_message(&mut stdout, &envelope)?;
        }
        Ok::<(), Box<dyn std::error::Error>>(())
    })
}

/// Chrome passes the extension origin; Firefox passes the manifest path and
/// add-on id. Anything else is not a browser we registered.
fn validate_invocation() -> Result<(), io::Error> {
    let mut args = std::env::args().skip(1);
    let Some(first) = args.next() else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "this binary is launched by a browser, not by hand",
        ));
    };
    validate_invocation_first(&first)
}

fn validate_invocation_first(first: &str) -> Result<(), io::Error> {
    if first.starts_with("chrome-extension://") {
        return Ok(());
    }
    if std::path::Path::new(first).is_file() {
        return Ok(());
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidInput,
        "unrecognized browser invocation arguments",
    ))
}

/// Successful browser responses carry the extension compatibility fields next
/// to the engine result. Older extensions ignore unknown keys; newer ones use
/// `extension_protocol` to decide whether take-over is safe.
fn success_envelope(result: Value) -> Value {
    json!({
        "result": result,
        "extension_protocol": EXTENSION_PROTOCOL,
        "host_version": env!("CARGO_PKG_VERSION"),
    })
}

async fn connect_with_retry() -> Result<IpcClient, String> {
    let first_error = match IpcClient::connect(ClientKind::NativeHost).await {
        Ok(client) => return Ok(client),
        Err(error) => error.to_string(),
    };

    launch_desktop_app();
    for _ in 0..25 {
        tokio::time::sleep(Duration::from_millis(200)).await;
        if let Ok(client) = IpcClient::connect(ClientKind::NativeHost).await {
            return Ok(client);
        }
    }
    Err(format!(
        "cannot reach the Downpour engine ({first_error}); start the app once and retry"
    ))
}

fn launch_desktop_app() {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(path) = std::env::var_os("DOWNPOUR_APP") {
        candidates.push(PathBuf::from(path));
    }
    if let Ok(executable) = std::env::current_exe() {
        if let Some(directory) = executable.parent() {
            for name in ["downpour-desktop", "Downpour", "downpour"] {
                candidates.push(directory.join(name));
            }
        }
    }

    for candidate in candidates {
        if candidate.is_file() {
            let _ = std::process::Command::new(candidate)
                .arg("--background")
                .spawn();
            return;
        }
    }
}

fn read_browser_frame<R: Read>(reader: &mut R) -> io::Result<Option<Vec<u8>>> {
    let mut length = [0_u8; 4];
    match reader.read_exact(&mut length) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(error) => return Err(error),
    }

    let length = u32::from_le_bytes(length) as usize;
    if length > MAX_BROWSER_TO_HOST {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "browser message exceeds 64 MiB",
        ));
    }

    let mut frame = vec![0_u8; length];
    reader.read_exact(&mut frame)?;
    Ok(Some(frame))
}

fn write_browser_message<W: Write>(writer: &mut W, value: &Value) -> io::Result<()> {
    let bytes = serde_json::to_vec(value).map_err(io::Error::other)?;
    if bytes.len() > MAX_HOST_TO_BROWSER {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "response exceeds the 1 MiB host→browser limit",
        ));
    }

    writer.write_all(&(bytes.len() as u32).to_le_bytes())?;
    writer.write_all(&bytes)?;
    writer.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_a_frame() {
        let value = json!({"method": "ping"});
        let mut buffer = Vec::new();
        write_browser_message(&mut buffer, &value).unwrap();

        let mut cursor = io::Cursor::new(buffer);
        let frame = read_browser_frame(&mut cursor).unwrap().unwrap();
        let decoded: Value = serde_json::from_slice(&frame).unwrap();
        assert_eq!(decoded, value);
    }

    #[test]
    fn clean_eof_is_not_an_error() {
        let mut cursor = io::Cursor::new(Vec::new());
        assert!(read_browser_frame(&mut cursor).unwrap().is_none());
    }

    #[test]
    fn rejects_oversized_browser_frames() {
        let mut buffer = Vec::new();
        let huge = (MAX_BROWSER_TO_HOST + 1) as u32;
        buffer.extend_from_slice(&huge.to_le_bytes());
        buffer.extend_from_slice(&[0_u8; 8]);
        let mut cursor = io::Cursor::new(buffer);
        assert!(read_browser_frame(&mut cursor).is_err());
    }

    #[test]
    fn success_envelope_carries_the_extension_protocol() {
        let envelope = success_envelope(json!({"ok": true}));
        assert_eq!(envelope["result"], json!({"ok": true}));
        assert_eq!(envelope["extension_protocol"], EXTENSION_PROTOCOL);
        assert_eq!(envelope["host_version"], env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn accepts_a_chromium_extension_origin() {
        assert!(validate_invocation_first("chrome-extension://abc/").is_ok());
    }

    #[test]
    fn accepts_a_firefox_manifest_path() {
        let path =
            std::env::temp_dir().join(format!("downpour-nm-host-test-{}.json", std::process::id()));
        std::fs::write(&path, b"{}").unwrap();
        let result = validate_invocation_first(path.to_str().unwrap());
        let _ = std::fs::remove_file(&path);
        assert!(result.is_ok());
    }

    #[test]
    fn rejects_unknown_invocation_arguments() {
        assert!(validate_invocation_first("--help").is_err());
    }
}
