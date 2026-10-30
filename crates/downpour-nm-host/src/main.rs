//! Browser native-messaging host.
//!
//! Browsers frame messages as a 4-byte little-endian length followed by JSON.
//! Chrome caps host→browser messages at 1 MB, so this binary only ever forwards
//! commands and compact status summaries — never file bytes or full lists.

use std::io::{self, Read, Write};

const MAX_HOST_TO_BROWSER: usize = 1024 * 1024;
const MAX_BROWSER_TO_HOST: usize = 64 * 1024 * 1024;

fn main() {
    // NOTE(M5): validate argv[1] — Chromium passes the extension origin,
    // Firefox passes the manifest path and add-on id.
    if let Err(error) = run() {
        eprintln!("downpour-nm-host: {error}");
        std::process::exit(1);
    }
}

fn run() -> io::Result<()> {
    let mut stdin = io::stdin().lock();
    let mut stdout = io::stdout().lock();

    while let Some(frame) = read_frame(&mut stdin)? {
        let request: serde_json::Value = match serde_json::from_slice(&frame) {
            Ok(value) => value,
            Err(error) => {
                write_json(
                    &mut stdout,
                    &serde_json::json!({
                        "error": {"code": "invalid_json", "message": error.to_string()}
                    }),
                )?;
                continue;
            }
        };

        let method = request
            .get("method")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();

        let response = match method {
            "ping" => serde_json::json!({
                "result": {
                    "pong": true,
                    "version": env!("CARGO_PKG_VERSION"),
                    "protocol_version": downpour_types::PROTOCOL_VERSION,
                }
            }),
            _ => serde_json::json!({
                "error": {
                    "code": "engine_not_connected",
                    "message": "engine IPC is not wired yet"
                }
            }),
        };

        write_json(&mut stdout, &response)?;
    }

    Ok(())
}

fn read_frame<R: Read>(reader: &mut R) -> io::Result<Option<Vec<u8>>> {
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

fn write_json<W: Write>(writer: &mut W, value: &serde_json::Value) -> io::Result<()> {
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
        let value = serde_json::json!({"method": "ping"});
        let mut buffer = Vec::new();
        write_json(&mut buffer, &value).unwrap();

        let mut cursor = io::Cursor::new(buffer);
        let frame = read_frame(&mut cursor).unwrap().unwrap();
        let decoded: serde_json::Value = serde_json::from_slice(&frame).unwrap();
        assert_eq!(decoded, value);
    }

    #[test]
    fn clean_eof_is_not_an_error() {
        let mut cursor = io::Cursor::new(Vec::new());
        assert!(read_frame(&mut cursor).unwrap().is_none());
    }
}
