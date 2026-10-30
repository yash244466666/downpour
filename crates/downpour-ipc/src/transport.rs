//! Local socket transport: endpoint discovery, token auth, framing.
//!
//! Unix uses a filesystem socket under `$XDG_RUNTIME_DIR/downpour` (0700)
//! with a 0600 token file. Windows uses a namespaced pipe; the token file
//! still gates every client.

use std::fs;
use std::io;
use std::path::PathBuf;

use downpour_types::PROTOCOL_VERSION;
pub use interprocess::local_socket::tokio::Stream;
#[cfg(not(windows))]
use interprocess::local_socket::{GenericFilePath, ToFsName};
#[cfg(windows)]
use interprocess::local_socket::{GenericNamespaced, ToNsName};
use interprocess::local_socket::{ListenerOptions, Name, tokio::prelude::*};
use serde::Serialize;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader};

use crate::protocol::{
    ClientKind, Hello, IpcError, MAX_FRAME_BYTES, Request, Response, encode_frame,
};

pub const SOCKET_FILE: &str = "engine.sock";
pub const TOKEN_FILE: &str = "engine.token";

/// Per-user application data directory.
#[must_use]
pub fn data_dir() -> PathBuf {
    if cfg!(target_os = "macos") {
        return home_dir()
            .map(|home| home.join("Library/Application Support/downpour"))
            .unwrap_or_else(|| PathBuf::from(".downpour"));
    }
    if cfg!(windows) {
        return std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .or_else(|| home_dir().map(|home| home.join("AppData/Local")))
            .map(|base| base.join("downpour"))
            .unwrap_or_else(|| PathBuf::from(".downpour"));
    }
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| home_dir().map(|home| home.join(".local/share")))
        .map(|base| base.join("downpour"))
        .unwrap_or_else(|| PathBuf::from(".downpour"))
}

fn home_dir() -> Option<PathBuf> {
    let variable = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(variable).map(PathBuf::from)
}

/// Directory that holds the socket on Unix.
#[must_use]
pub fn runtime_dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|path| path.is_dir())
        .unwrap_or_else(data_dir)
        .join("downpour")
}

/// Filesystem socket path (Unix).
#[must_use]
pub fn socket_path() -> PathBuf {
    runtime_dir().join(SOCKET_FILE)
}

/// Token file path.
#[must_use]
pub fn token_path() -> PathBuf {
    data_dir().join(TOKEN_FILE)
}

/// Endpoint name for the current platform.
pub fn socket_name() -> io::Result<Name<'static>> {
    #[cfg(windows)]
    {
        Ok("downpour-engine"
            .to_ns_name::<GenericNamespaced>()?
            .into_owned())
    }
    #[cfg(not(windows))]
    {
        let path = socket_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = fs::set_permissions(parent, fs::Permissions::from_mode(0o700));
            }
        }
        Ok(path
            .to_string_lossy()
            .to_string()
            .to_fs_name::<GenericFilePath>()?
            .into_owned())
    }
}

/// A fresh 32-byte token, hex encoded.
pub fn generate_token() -> io::Result<String> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes).map_err(|error| io::Error::other(error.to_string()))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

/// Write the token with owner-only permissions on Unix.
pub fn write_token(token: &str) -> io::Result<()> {
    let path = token_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, token)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

/// Read the current token.
pub fn read_token() -> io::Result<String> {
    Ok(fs::read_to_string(token_path())?.trim().to_owned())
}

/// Listening socket for the engine.
pub struct IpcServer {
    listener: interprocess::local_socket::tokio::Listener,
}

impl IpcServer {
    pub fn bind() -> io::Result<Self> {
        Self::bind_named(socket_name()?)
    }

    /// Bind a specific name (tests use a temporary path).
    pub fn bind_named(name: Name<'static>) -> io::Result<Self> {
        let listener = ListenerOptions::new()
            .name(name)
            .try_overwrite(true)
            .create_tokio()?;
        Ok(Self { listener })
    }

    pub async fn accept(&self) -> io::Result<Stream> {
        self.listener.accept().await
    }
}

/// Read one newline-terminated frame, enforcing the size cap.
pub async fn read_frame<R: AsyncBufRead + Unpin>(
    reader: &mut R,
) -> Result<Option<Vec<u8>>, IpcError> {
    let mut line = Vec::new();
    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            return if line.is_empty() {
                Ok(None)
            } else {
                Ok(Some(line))
            };
        }
        if let Some(position) = available.iter().position(|byte| *byte == b'\n') {
            line.extend_from_slice(&available[..position]);
            reader.consume(position + 1);
            if line.len() > MAX_FRAME_BYTES {
                return Err(IpcError::FrameTooLarge);
            }
            return Ok(Some(line));
        }
        line.extend_from_slice(available);
        let consumed = available.len();
        reader.consume(consumed);
        if line.len() > MAX_FRAME_BYTES {
            return Err(IpcError::FrameTooLarge);
        }
    }
}

/// Write one newline-terminated JSON frame.
pub async fn write_frame<W: AsyncWrite + Unpin, T: Serialize>(
    writer: &mut W,
    value: &T,
) -> Result<(), IpcError> {
    let bytes = encode_frame(value)?;
    writer.write_all(&bytes).await?;
    writer.flush().await?;
    Ok(())
}

/// Verify the first client frame (version and token).
///
/// The caller keeps the same buffered connection for later frames: a fresh
/// `BufReader` per message would discard bytes the client already sent.
pub async fn server_handshake<R: AsyncBufRead + Unpin>(
    reader: &mut R,
    expected_token: &str,
) -> Result<Hello, IpcError> {
    let frame = read_frame(reader).await?;
    let Some(frame) = frame else {
        return Err(IpcError::Io(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "client closed before the handshake",
        )));
    };
    let hello: Hello = serde_json::from_slice(&frame)?;
    if hello.protocol_version != PROTOCOL_VERSION {
        return Err(IpcError::ProtocolMismatch {
            client: hello.protocol_version,
            engine: PROTOCOL_VERSION,
        });
    }
    if !constant_time_eq(hello.token.as_bytes(), expected_token.as_bytes()) {
        return Err(IpcError::Unauthorized);
    }
    Ok(hello)
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut difference = 0_u8;
    for (a, b) in left.iter().zip(right.iter()) {
        difference |= a ^ b;
    }
    difference == 0
}

/// Client for the engine socket.
pub struct IpcClient {
    connection: BufReader<Stream>,
}

impl IpcClient {
    pub async fn connect(kind: ClientKind) -> Result<Self, IpcError> {
        let token = read_token()?;
        Self::connect_named(socket_name()?, &token, kind).await
    }

    pub async fn connect_named(
        name: Name<'static>,
        token: &str,
        kind: ClientKind,
    ) -> Result<Self, IpcError> {
        let mut connection = BufReader::new(Stream::connect(name).await?);
        write_frame(&mut connection, &Hello::new(token, kind)).await?;
        Ok(Self { connection })
    }

    pub async fn call(&mut self, request: &Request) -> Result<Response, IpcError> {
        write_frame(&mut self.connection, request).await?;
        let frame = read_frame(&mut self.connection).await?;
        let Some(frame) = frame else {
            return Err(IpcError::Io(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "engine closed the connection",
            )));
        };
        Ok(serde_json::from_slice(&frame)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use serde_json::json;

    #[cfg(unix)]
    fn test_name(dir: &std::path::Path) -> Name<'static> {
        dir.join("test.sock")
            .to_string_lossy()
            .to_string()
            .to_fs_name::<GenericFilePath>()
            .unwrap()
            .into_owned()
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn round_trips_a_request() {
        let dir = tempfile::tempdir().unwrap();
        let name = test_name(dir.path());
        let server = IpcServer::bind_named(name.clone()).unwrap();
        let token = "test-token".to_owned();

        let server_task = {
            let token = token.clone();
            tokio::spawn(async move {
                let mut connection = BufReader::new(server.accept().await.unwrap());
                let hello = server_handshake(&mut connection, &token).await.unwrap();
                assert_eq!(hello.client, ClientKind::Cli);

                let frame = read_frame(&mut connection).await.unwrap().unwrap();
                let request: Request = serde_json::from_slice(&frame).unwrap();
                write_frame(
                    &mut connection,
                    &Response::ok(request.id, json!({"pong": true})),
                )
                .await
                .unwrap();
            })
        };

        let mut client = IpcClient::connect_named(name, &token, ClientKind::Cli)
            .await
            .unwrap();
        let response = client
            .call(&Request {
                id: 1,
                method: "ping".to_owned(),
                params: json!({}),
            })
            .await
            .unwrap();
        assert_eq!(response.result.unwrap()["pong"], true);
        server_task.await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn rejects_a_bad_token() {
        let dir = tempfile::tempdir().unwrap();
        let name = test_name(dir.path());
        let server = IpcServer::bind_named(name.clone()).unwrap();

        let server_task = tokio::spawn(async move {
            let mut connection = BufReader::new(server.accept().await.unwrap());
            let result = server_handshake(&mut connection, "expected").await;
            assert!(matches!(result, Err(IpcError::Unauthorized)));
        });

        let mut client = IpcClient::connect_named(name, "wrong", ClientKind::Cli)
            .await
            .unwrap();
        let _ = client
            .call(&Request {
                id: 1,
                method: "ping".to_owned(),
                params: json!({}),
            })
            .await;
        server_task.await.unwrap();
    }

    #[test]
    fn tokens_are_hex_and_fresh() {
        let first = generate_token().unwrap();
        let second = generate_token().unwrap();
        assert_eq!(first.len(), 64);
        assert_ne!(first, second);
        assert!(first.bytes().all(|byte| byte.is_ascii_hexdigit()));
    }
}
