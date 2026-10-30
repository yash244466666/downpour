//! The single writer thread.
//!
//! One download owns exactly one file handle, owned by one dedicated thread.
//! Network tasks never touch the file; they send `(segment, offset, bytes)`
//! over a bounded channel. This avoids shared file cursors (`try_clone`
//! duplicates the offset on both Unix and Windows), gives natural backpressure,
//! and leaves one place that can fsync.
//!
//! Checkpoint rule: the writer fsyncs the file before the engine commits the
//! returned offsets to SQLite, so a durable database offset is never ahead of
//! durable bytes.

use std::collections::HashMap;
use std::fs::File;
use std::io;

use bytes::Bytes;
use tokio::sync::{mpsc, oneshot};

const CHANNEL_CAPACITY: usize = 256;

type SegmentEnds = Vec<(u32, u64)>;

enum Command {
    Write {
        segment: u32,
        offset: u64,
        data: Bytes,
    },
    Reset {
        len: u64,
        ends: SegmentEnds,
        ack: oneshot::Sender<io::Result<()>>,
    },
    Sync {
        ack: oneshot::Sender<io::Result<SegmentEnds>>,
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
    /// Spawn the writer thread.
    ///
    /// `initial_ends` carries each segment's already-durable end offset
    /// (exclusive), so a resume reports the correct progress before the first
    /// new write.
    pub fn spawn(file: File, initial_ends: SegmentEnds) -> io::Result<Self> {
        let (tx, rx) = mpsc::channel(CHANNEL_CAPACITY);
        let join = std::thread::Builder::new()
            .name("downpour-writer".to_owned())
            .spawn(move || writer_loop(file, rx, initial_ends))?;
        Ok(Self {
            tx: Some(tx),
            join: Some(join),
        })
    }

    /// Queue a chunk at an absolute offset, waiting for channel capacity.
    pub async fn write(&self, segment: u32, offset: u64, data: Bytes) -> io::Result<()> {
        let Some(tx) = self.tx.as_ref() else {
            return Err(closed());
        };
        tx.send(Command::Write {
            segment,
            offset,
            data,
        })
        .await
        .map_err(|_| closed())
    }

    /// Discard everything past `len` and replace the progress map.
    pub async fn reset(&self, len: u64, ends: SegmentEnds) -> io::Result<()> {
        let (ack, rx) = oneshot::channel();
        let Some(tx) = self.tx.as_ref() else {
            return Err(closed());
        };
        tx.send(Command::Reset { len, ends, ack })
            .await
            .map_err(|_| closed())?;
        rx.await.map_err(|_| closed())?
    }

    /// Flush and fsync; returns each segment's highest written end offset.
    pub async fn sync(&self) -> io::Result<SegmentEnds> {
        let (ack, rx) = oneshot::channel();
        let Some(tx) = self.tx.as_ref() else {
            return Err(closed());
        };
        tx.send(Command::Sync { ack }).await.map_err(|_| closed())?;
        rx.await.map_err(|_| closed())?
    }

    /// Sync a single segment, for the single-stream path.
    pub async fn sync_one(&self, segment: u32) -> io::Result<u64> {
        let ends = self.sync().await?;
        Ok(ends
            .iter()
            .find(|(index, _)| *index == segment)
            .map(|(_, end)| *end)
            .unwrap_or(0))
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
        // Dropping the sender makes the writer exit its receive loop. Join it
        // so the file handle is closed before the caller moves or reopens the
        // file: Windows refuses some operations while a handle is open.
        self.tx.take();
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

fn closed() -> io::Error {
    io::Error::new(io::ErrorKind::BrokenPipe, "writer thread stopped")
}

fn writer_loop(mut file: File, mut rx: mpsc::Receiver<Command>, initial_ends: SegmentEnds) {
    let mut ends: HashMap<u32, u64> = initial_ends.into_iter().collect();
    let mut failure: Option<io::Error> = None;

    while let Some(command) = rx.blocking_recv() {
        match command {
            Command::Write {
                segment,
                offset,
                data,
            } => {
                if failure.is_some() {
                    continue;
                }
                if let Err(error) = write_all_at(&mut file, offset, &data) {
                    failure = Some(error);
                    continue;
                }
                let end = offset.saturating_add(data.len() as u64);
                let slot = ends.entry(segment).or_insert(end);
                if end > *slot {
                    *slot = end;
                }
            }
            Command::Reset {
                len,
                ends: new_ends,
                ack,
            } => {
                let result = match failure.as_ref() {
                    Some(error) => Err(clone_error(error)),
                    None => file.set_len(len).map(|()| {
                        ends = new_ends.into_iter().collect();
                    }),
                };
                let _ = ack.send(result);
            }
            Command::Sync { ack } => {
                let result = match failure.as_ref() {
                    Some(error) => Err(clone_error(error)),
                    None => file.sync_data().map(|()| sorted_ends(&ends)),
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

fn sorted_ends(ends: &HashMap<u32, u64>) -> SegmentEnds {
    let mut out: SegmentEnds = ends.iter().map(|(index, end)| (*index, *end)).collect();
    out.sort_unstable_by_key(|(index, _)| *index);
    out
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

    fn open(path: &std::path::Path) -> File {
        File::options()
            .create(true)
            .read(true)
            .write(true)
            .truncate(true)
            .open(path)
            .unwrap()
    }

    #[tokio::test]
    async fn writes_at_offsets_and_syncs() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.bin");
        let writer = ChunkWriter::spawn(open(&path), vec![(0, 0)]).unwrap();

        writer
            .write(0, 0, Bytes::from_static(b"hello "))
            .await
            .unwrap();
        writer
            .write(0, 6, Bytes::from_static(b"world"))
            .await
            .unwrap();
        assert_eq!(writer.sync_one(0).await.unwrap(), 11);
        writer.finish().await.unwrap();

        let mut contents = Vec::new();
        File::open(&path)
            .unwrap()
            .read_to_end(&mut contents)
            .unwrap();
        assert_eq!(contents, b"hello world");
    }

    #[tokio::test]
    async fn tracks_segments_independently() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.bin");
        let writer = ChunkWriter::spawn(open(&path), vec![(0, 0), (1, 0)]).unwrap();

        writer
            .write(1, 4, Bytes::from_static(b"EFGH"))
            .await
            .unwrap();
        writer
            .write(0, 0, Bytes::from_static(b"ABCD"))
            .await
            .unwrap();

        let ends = writer.sync().await.unwrap();
        assert_eq!(ends, vec![(0, 4), (1, 8)]);
        writer.finish().await.unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"ABCDEFGH");
    }

    #[tokio::test]
    async fn reset_discards_the_tail() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.bin");
        let writer = ChunkWriter::spawn(open(&path), vec![(0, 0)]).unwrap();

        writer
            .write(0, 0, Bytes::from_static(b"0123456789"))
            .await
            .unwrap();
        writer.sync_one(0).await.unwrap();
        writer.reset(4, vec![(0, 4)]).await.unwrap();
        assert_eq!(writer.sync_one(0).await.unwrap(), 4);
        writer.finish().await.unwrap();

        assert_eq!(std::fs::read(&path).unwrap(), b"0123");
    }
}
