//! Engine errors and their mapping to wire-safe error codes.

use downpour_types::{ErrorCode, ErrorInfo};
use thiserror::Error;

pub type EngineResult<T> = Result<T, EngineError>;

#[derive(Debug, Error)]
pub enum EngineError {
    #[error("invalid URL: {0}")]
    InvalidUrl(String),
    #[error("unsupported URL scheme: {0}")]
    UnsupportedScheme(String),
    #[error("invalid request header: {0}")]
    InvalidHeader(String),
    #[error("HTTP {status}")]
    Http { status: u16 },
    #[error("authentication required")]
    AuthRequired,
    #[error("proxy authentication required")]
    ProxyAuthRequired,
    #[error("server changed content since the transfer started")]
    ContentChanged,
    #[error("server response was truncated: expected {expected} bytes, received {received}")]
    Truncated { expected: u64, received: u64 },
    #[error("network error: {0}")]
    Network(#[from] reqwest::Error),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("database error: {0}")]
    Database(#[from] downpour_store::StoreError),
    #[error("disk is full")]
    DiskFull,
    #[error("checksum mismatch: expected {expected}, got {actual}")]
    ChecksumMismatch { expected: String, actual: String },
    #[error("download canceled")]
    Canceled,
    #[error("probe failed: {0}")]
    Probe(String),
    #[error("invalid destination: {0}")]
    InvalidDestination(String),
    #[error("internal error: {0}")]
    Internal(String),
}

impl EngineError {
    /// Map to the stable error code the UI translates.
    #[must_use]
    pub fn code(&self) -> ErrorCode {
        match self {
            Self::InvalidUrl(_) => ErrorCode::InvalidUrl,
            Self::UnsupportedScheme(_) => ErrorCode::UnsupportedScheme,
            Self::InvalidHeader(_) => ErrorCode::InvalidUrl,
            Self::Http { status } => ErrorCode::Http { status: *status },
            Self::AuthRequired => ErrorCode::AuthRequired,
            Self::ProxyAuthRequired => ErrorCode::ProxyAuthRequired,
            Self::ContentChanged => ErrorCode::ContentChanged,
            Self::Truncated { .. } | Self::Network(_) | Self::Probe(_) => ErrorCode::Network,
            Self::Io(err) if err.kind() == std::io::ErrorKind::StorageFull => ErrorCode::DiskFull,
            Self::Io(_) => ErrorCode::Io,
            Self::Database(_) => ErrorCode::Database,
            Self::DiskFull => ErrorCode::DiskFull,
            Self::ChecksumMismatch { .. } => ErrorCode::ChecksumMismatch,
            Self::Canceled => ErrorCode::Canceled,
            Self::InvalidDestination(_) => ErrorCode::PathTraversal,
            Self::Internal(_) => ErrorCode::Internal,
        }
    }

    /// Wire-safe form: code plus a human-readable message.
    #[must_use]
    pub fn info(&self) -> ErrorInfo {
        ErrorInfo::new(self.code(), self.to_string())
    }

    /// Whether a transfer or probe may be retried without user action.
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::Network(_) | Self::Truncated { .. } => true,
            Self::Http { status } => {
                *status == 408 || *status == 429 || (500..=599).contains(status)
            }
            _ => false,
        }
    }
}
