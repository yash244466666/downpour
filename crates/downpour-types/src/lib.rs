//! Shared data types and protocol DTOs for Downpour.
//!
//! This crate deliberately has no engine, database, or Tauri dependencies so the
//! CLI, native-messaging host, and desktop UI can all speak the same protocol.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

/// Version of the local IPC protocol. Bump on breaking changes.
pub const PROTOCOL_VERSION: u32 = 1;

/// Stable identifier for a download job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DownloadId(pub Uuid);

impl DownloadId {
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::now_v7())
    }
}

impl Default for DownloadId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for DownloadId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl std::str::FromStr for DownloadId {
    type Err = uuid::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Ok(Self(Uuid::parse_str(value)?))
    }
}

/// Lifecycle state of a download.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DownloadState {
    Queued,
    Probing,
    Downloading,
    Paused,
    Finalizing,
    Completed,
    Failed,
    Canceled,
}

impl DownloadState {
    #[must_use]
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Canceled)
    }

    #[must_use]
    pub fn is_running(self) -> bool {
        matches!(self, Self::Probing | Self::Downloading | Self::Finalizing)
    }

    /// Stable database / wire spelling.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Probing => "probing",
            Self::Downloading => "downloading",
            Self::Paused => "paused",
            Self::Finalizing => "finalizing",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Canceled => "canceled",
        }
    }
}

impl std::fmt::Display for DownloadState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for DownloadState {
    type Err = UnknownEnumValue;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "queued" => Ok(Self::Queued),
            "probing" => Ok(Self::Probing),
            "downloading" => Ok(Self::Downloading),
            "paused" => Ok(Self::Paused),
            "finalizing" => Ok(Self::Finalizing),
            "completed" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            "canceled" => Ok(Self::Canceled),
            other => Err(UnknownEnumValue::new("download state", other)),
        }
    }
}

/// A byte range assigned to one worker.
///
/// `start` and `end` are inclusive, matching HTTP `Range: bytes=start-end`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SegmentSpec {
    pub index: u32,
    pub start: u64,
    /// Inclusive end offset.
    pub end: u64,
    /// Bytes already durably written for this segment.
    #[serde(default)]
    pub downloaded: u64,
}

impl SegmentSpec {
    #[must_use]
    pub fn new(index: u32, start: u64, end: u64) -> Self {
        Self {
            index,
            start,
            end,
            downloaded: 0,
        }
    }

    #[must_use]
    pub fn len(&self) -> u64 {
        self.end.saturating_sub(self.start).saturating_add(1)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.end < self.start
    }

    #[must_use]
    pub fn remaining(&self) -> u64 {
        self.len().saturating_sub(self.downloaded)
    }
}

/// Per-segment progress, sent only for the detail view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SegmentProgress {
    pub index: u32,
    pub start: u64,
    pub end: u64,
    pub downloaded: u64,
    pub speed_bps: u64,
}

/// Coalesced download summary used by list views and progress events.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DownloadSnapshot {
    pub id: DownloadId,
    pub url: String,
    pub file_name: String,
    pub state: DownloadState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_bytes: Option<u64>,
    #[serde(default)]
    pub downloaded_bytes: u64,
    #[serde(default)]
    pub speed_bps: u64,
    #[serde(default)]
    pub connections: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eta_seconds: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorInfo>,
    pub created_at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub segments: Option<Vec<SegmentProgress>>,
}

/// Where a download request originated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DownloadSource {
    #[default]
    Ui,
    BrowserExtension,
    Clipboard,
    Batch,
    Api,
}

impl DownloadSource {
    /// Stable database / wire spelling.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ui => "ui",
            Self::BrowserExtension => "browser_extension",
            Self::Clipboard => "clipboard",
            Self::Batch => "batch",
            Self::Api => "api",
        }
    }
}

impl std::fmt::Display for DownloadSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for DownloadSource {
    type Err = UnknownEnumValue;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "ui" => Ok(Self::Ui),
            "browser_extension" => Ok(Self::BrowserExtension),
            "clipboard" => Ok(Self::Clipboard),
            "batch" => Ok(Self::Batch),
            "api" => Ok(Self::Api),
            other => Err(UnknownEnumValue::new("download source", other)),
        }
    }
}

/// Error returned when a database or wire value names an unknown enum variant.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("unknown {kind}: {value}")]
pub struct UnknownEnumValue {
    kind: &'static str,
    value: String,
}

impl UnknownEnumValue {
    #[must_use]
    pub fn new(kind: &'static str, value: impl Into<String>) -> Self {
        Self {
            kind,
            value: value.into(),
        }
    }
}

/// Request to enqueue a new download.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AddDownloadRequest {
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dest_dir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connections: Option<u16>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub cookies: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub referer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_agent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed_limit: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checksum_sha256: Option<String>,
    #[serde(default)]
    pub start_immediately: bool,
    #[serde(default)]
    pub source: DownloadSource,
}

/// Browser context handed over by the extension for authenticated downloads.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BrowserContext {
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub final_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub referrer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_agent: Option<String>,
    /// Raw `Cookie` header assembled by the extension.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cookie_header: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra_headers: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    #[serde(default)]
    pub incognito: bool,
}

/// Machine-readable error codes; the UI translates these.
#[derive(Debug, Clone, PartialEq, Eq, Error, Serialize, Deserialize)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum ErrorCode {
    #[error("invalid URL")]
    InvalidUrl,
    #[error("unsupported URL scheme")]
    UnsupportedScheme,
    #[error("destination path is outside the download directory")]
    PathTraversal,
    #[error("private or link-local address requires confirmation")]
    PrivateAddress,
    #[error("HTTP error {status}")]
    Http { status: u16 },
    #[error("authentication required")]
    AuthRequired,
    #[error("proxy authentication required")]
    ProxyAuthRequired,
    #[error("server does not support byte ranges")]
    RangeUnsupported,
    #[error("server content changed since the download started")]
    ContentChanged,
    #[error("network error")]
    Network,
    #[error("I/O error")]
    Io,
    #[error("disk is full")]
    DiskFull,
    #[error("checksum mismatch")]
    ChecksumMismatch,
    #[error("database error")]
    Database,
    #[error("download was canceled")]
    Canceled,
    #[error("feature not implemented")]
    NotImplemented,
    #[error("internal error")]
    Internal,
}

/// A wire-safe error with a code and a human-readable detail message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorInfo {
    pub code: ErrorCode,
    pub message: String,
}

impl ErrorInfo {
    #[must_use]
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for ErrorInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for ErrorInfo {}

/// Engine and protocol identity returned by a handshake.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngineInfo {
    pub version: String,
    pub protocol_version: u32,
    pub os: String,
    pub arch: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segment_len_is_inclusive() {
        let seg = SegmentSpec::new(0, 10, 19);
        assert_eq!(seg.len(), 10);
        assert_eq!(seg.remaining(), 10);
    }

    #[test]
    fn download_id_is_uuid_v7_monotonic_enough() {
        let a = DownloadId::new();
        let b = DownloadId::new();
        assert_ne!(a, b);
        assert_eq!(a.0.get_version_num(), 7);
    }

    #[test]
    fn state_terminality() {
        assert!(DownloadState::Completed.is_terminal());
        assert!(DownloadState::Failed.is_terminal());
        assert!(DownloadState::Canceled.is_terminal());
        assert!(!DownloadState::Paused.is_terminal());
        assert!(DownloadState::Downloading.is_running());
    }

    #[test]
    fn error_code_serializes_with_tag() {
        let value = serde_json::to_value(ErrorCode::Http { status: 404 }).unwrap();
        assert_eq!(value["code"], "http");
        assert_eq!(value["status"], 404);
    }

    #[test]
    fn states_round_trip_through_strings() {
        for state in [
            DownloadState::Queued,
            DownloadState::Probing,
            DownloadState::Downloading,
            DownloadState::Paused,
            DownloadState::Finalizing,
            DownloadState::Completed,
            DownloadState::Failed,
            DownloadState::Canceled,
        ] {
            assert_eq!(state.as_str().parse::<DownloadState>().unwrap(), state);
        }
        assert!("bogus".parse::<DownloadState>().is_err());
    }

    #[test]
    fn sources_round_trip_through_strings() {
        for source in [
            DownloadSource::Ui,
            DownloadSource::BrowserExtension,
            DownloadSource::Clipboard,
            DownloadSource::Batch,
            DownloadSource::Api,
        ] {
            assert_eq!(source.as_str().parse::<DownloadSource>().unwrap(), source);
        }
        assert!("bogus".parse::<DownloadSource>().is_err());
    }
}
