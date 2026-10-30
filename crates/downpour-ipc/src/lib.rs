//! Local IPC protocol for Downpour.
//!
//! Frames are newline-delimited JSON with a hard size cap. The transport is a
//! Unix domain socket or a Windows named pipe; this crate only defines the
//! wire contract and helpers.

mod protocol;

pub use protocol::{
    ClientKind, Event, Hello, IpcError, MAX_FRAME_BYTES, Request, Response, decode_frame,
    encode_frame,
};
