//! The single writer thread.
//!
//! One download owns exactly one file handle, owned by one dedicated thread.
//! Network tasks never touch the file; they send `(offset, bytes)` over a
//! bounded channel. This avoids shared file cursors (`try_clone` duplicates
//! the offset on both Unix and Windows), gives natural backpressure, and
//! leaves one place that can fsync.
//!
//! Checkpoint rule: the writer fsyncs the file before the engine commits the
//! returned offset to SQLite, so a durable database offset is never ahead of
//! durable bytes.

use std::fs::File;
use std::io;

use bytes::Bytes;
use tokio::sync::{mpsc, oneshot};

const CHANNEL_CAPACITY: usize = 128;

enum Command {
    Write {
        offset: u64,
        data: Bytes,
    },
    Truncate {
        len: u64,
        ack: oneshot::Sender<io::Result<()>>,
    },
    Sync {
        ack: oneshot::Sender<io::Result<u64>>,
    },
    Finish {
        ack: oneshot::Sender<io::Result<()>>,
    },
}

/// Handle to the writer thread for one download.
pub struct ChunkWriter {
    tx: Option<mpsc::Sender<Command>>,
    join: Option<std::thread::JoinHandle<()>>,
}

impl ChunkWriter {
    /// Spawn the writer thread. `start_offset` is the number of bytes known to
    /// be present in the file already.
    pub fn spawn(file: File, start_offset: u64) -> io::Result<Self> {
        let (tx, rx) = mpsc::channel(CHANNEL_CAPACITY);
        let join = std::thread::Builder::new()
            .name("downpour-writer".to_owned())
            .spawn(move || writer_loop(file, rx, start_offset))?;
        Ok(Self {
            tx: Some(tx),
            join: Some(join),
        })
    }

    /// Queue a chunk at an absolute offset, waiting for channel capacity.
    pub async fn write(&self, offset: u64, data: Bytes) -> io::Result<()> {
        let Some(tx) = self.tx.as_ref() else {
            return Err(closed());
        };
        tx.send(Command::Write { offset, data })
            .await
            .map_err(|_| closed())
    }

    /// Discard everything past `len` (used when a server restarts the body).
    pub async fn truncate(&self, len: u64) -> io::Result<()> {
        let (ack, rx) = oneshot::channel();
        let Some(tx) = self.tx.as_ref() else {
            return Err(closed());
        };
        tx.send(Command::Truncate { len, ack })
            .await
            .map_err(|_| closed())?;
        rx.await.map_err(|_| closed())?
    }

    /// Flush and fsync; returns the highest contiguous written offset.
    pub async fn sync(&self) -> io::Result<u64> {
        let (ack, rx) = oneshot::channel();
        let Some(tx) = self.tx.as_ref() else {
            return Err(closed());
        };
        tx.send(Command::Sync { ack }).await.map_err(|_| closed())?;
        rx.await.map_err(|_| closed())?
    }

    /// Flush, fsync, stop the thread, and wait for it to exit.
    pub async fn finish(mut self) -> io::Result<()> {
        let (ack, rx) = oneshot::channel();
        if let Some(tx) = self.tx.take() {
            let _ = tx.send(Command::Finish { ack }).await;
        }
        let result = rx.await.unwrap_or_else(|_| Err(closed()));

        if let Some(join) = self.join.take() {
            tokio::task::spawn_blocking(move || {
                let _ = join.join();
            })
            .await
            .map_err(|err| io::Error::other(err.to_string()))?;
        }
        result
    }
}

impl Drop for ChunkWriter {
    fn drop(&mut self) {
        // Dropping the sender makes the writer exit its receive loop.
        self.tx.take();
    }
}

fn closed() -> io::Error {
    io::Error::new(io::ErrorKind::BrokenPipe, "writer thread stopped")
}

fn writer_loop(mut file: File, mut rx: mpsc::Receiver<Command>, start_offset: u64) {
    let mut written_end = start_offset;
    let mut failure: Option<io::Error> = None;

    while let Some(command) = rx.blocking_recv() {
        match command {
            Command::Write { offset, data } => {
                if failure.is_some() {
                    continue;
                }
                if let Err(error) = write_all_at(&mut file, offset, &data) {
                    failure = Some(error);
                    continue;
                }
                written_end = written_end.max(offset.saturating_add(data.len() as u64));
            }
            Command::Truncate { len, ack } => {
                let result = match failure.as_ref() {
                    Some(error) => Err(clone_error(error)),
                    None => file.set_len(len).map(|()| {
                        written_end = len;
                    }),
                };
                let _ = ack.send(result);
            }
            Command::Sync { ack } => {
                let result = match failure.as_ref() {
                    Some(error) => Err(clone_error(error)),
                    None => file.sync_data().map(|()| written_end),
                };
                let _ = ack.send(result);
            }
            Command::Finish { ack } => {
                let result = match failure.take() {
                    Some(error) => Err(error),
                    None => file.sync_all(),
                };
                let _ = ack.send(result);
            }
        }
    }
}

fn clone_error(error: &io::Error) -> io::Error {
    io::Error::new(error.kind(), error.to_string())
}

#[cfg(unix)]
fn write_all_at(file: &mut File, mut offset: u64, mut data: &[u8]) -> io::Result<()> {
    use std::os::unix::fs::FileExt;

    while !data.is_empty() {
        match file.write_at(data, offset) {
            Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
            Ok(written) => {
                data = &data[written..];
                offset += written as u64;
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

#[cfg(windows)]
fn write_all_at(file: &mut File, mut offset: u64, mut data: &[u8]) -> io::Result<()> {
    use std::os::windows::fs::FileExt;

    while !data.is_empty() {
        match file.seek_write(data, offset) {
            Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
            Ok(written) => {
                data = &data[written..];
                offset += written as u64;
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

#[cfg(not(any(unix, windows)))]
fn write_all_at(file: &mut File, offset: u64, data: &[u8]) -> io::Result<()> {
    use std::io::{Seek, SeekFrom, Write};

    file.seek(SeekFrom::Start(offset))?;
    file.write_all(data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[tokio::test]
    async fn writes_at_offsets_and_syncs() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.bin");
        let file = File::options()
            .create(true)
            .read(true)
            .write(true)
            .truncate(true)
            .open(&path)
            .unwrap();

        let writer = ChunkWriter::spawn(file, 0).unwrap();
        writer
            .write(0, Bytes::from_static(b"hello "))
            .await
            .unwrap();
        writer.write(6, Bytes::from_static(b"world")).await.unwrap();
        let end = writer.sync().await.unwrap();
        assert_eq!(end, 11);
        writer.finish().await.unwrap();

        let mut contents = Vec::new();
        File::open(&path)
            .unwrap()
            .read_to_end(&mut contents)
            .unwrap();
        assert_eq!(contents, b"hello world");
    }

    #[tokio::test]
    async fn truncate_discards_tail() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.bin");
        let file = File::options()
            .create(true)
            .read(true)
            .write(true)
            .truncate(true)
            .open(&path)
            .unwrap();

        let writer = ChunkWriter::spawn(file, 0).unwrap();
        writer
            .write(0, Bytes::from_static(b"0123456789"))
            .await
            .unwrap();
        writer.sync().await.unwrap();
        writer.truncate(4).await.unwrap();
        assert_eq!(writer.sync().await.unwrap(), 4);
        writer.finish().await.unwrap();

        assert_eq!(std::fs::read(&path).unwrap(), b"0123");
    }
}
