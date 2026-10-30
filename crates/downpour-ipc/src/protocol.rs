//! Wire messages and framing.
//!
//! One frame is one JSON object followed by `\n`. Maximum frame size is 8 MiB;
//! the native-messaging host has a stricter 1 MiB host→browser limit and must
//! send summaries only.

use downpour_types::{ErrorInfo, PROTOCOL_VERSION};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

pub const MAX_FRAME_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    pub id: u64,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Response {
    pub id: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorInfo>,
}

impl Response {
    #[must_use]
    pub fn ok(id: u64, result: Value) -> Self {
        Self {
            id,
            result: Some(result),
            error: None,
        }
    }

    #[must_use]
    pub fn error(id: u64, error: ErrorInfo) -> Self {
        Self {
            id,
            result: None,
            error: Some(error),
        }
    }
}

/// Server-pushed event (progress, completion, failures).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    pub event: String,
    pub data: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClientKind {
    Ui,
    Cli,
    NativeHost,
}

/// First frame sent by a client.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hello {
    pub protocol_version: u32,
    pub token: String,
    pub client: ClientKind,
}

impl Hello {
    #[must_use]
    pub fn new(token: impl Into<String>, client: ClientKind) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            token: token.into(),
            client,
        }
    }
}

#[derive(Debug, Error)]
pub enum IpcError {
    #[error("frame exceeds {MAX_FRAME_BYTES} bytes")]
    FrameTooLarge,
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("protocol version mismatch (client {client}, engine {engine})")]
    ProtocolMismatch { client: u32, engine: u32 },
    #[error("invalid IPC token")]
    Unauthorized,
}

/// Serialize a value as one newline-terminated frame, enforcing the size cap.
pub fn encode_frame<T: Serialize>(value: &T) -> Result<Vec<u8>, IpcError> {
    let mut bytes = serde_json::to_vec(value)?;
    if bytes.len() > MAX_FRAME_BYTES {
        return Err(IpcError::FrameTooLarge);
    }
    bytes.push(b'\n');
    Ok(bytes)
}

/// Parse one newline-free frame.
pub fn decode_frame(bytes: &[u8]) -> Result<Request, IpcError> {
    if bytes.len() > MAX_FRAME_BYTES {
        return Err(IpcError::FrameTooLarge);
    }
    Ok(serde_json::from_slice(bytes)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_a_request() {
        let request = Request {
            id: 7,
            method: "download.add".into(),
            params: serde_json::json!({"url": "https://example.com/file.zip"}),
        };
        let frame = encode_frame(&request).unwrap();
        assert_eq!(frame.last(), Some(&b'\n'));
        let decoded = decode_frame(&frame[..frame.len() - 1]).unwrap();
        assert_eq!(decoded, request);
    }

    #[test]
    fn rejects_oversized_frames() {
        let big = vec![b'x'; MAX_FRAME_BYTES + 1];
        assert!(matches!(decode_frame(&big), Err(IpcError::FrameTooLarge)));
    }

    #[test]
    fn response_omits_none_fields() {
        let response = Response::ok(1, serde_json::json!({"ok": true}));
        let value = serde_json::to_value(response).unwrap();
        assert!(value.get("error").is_none());
        assert_eq!(value["result"]["ok"], true);
    }

    #[test]
    fn hello_is_versioned() {
        let hello = Hello::new("secret", ClientKind::Cli);
        assert_eq!(hello.protocol_version, PROTOCOL_VERSION);
    }
}
