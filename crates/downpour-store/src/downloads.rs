//! Download and segment persistence.
//!
//! Every function takes a `rusqlite::Connection` explicitly: the engine owns
//! the connection (it runs on a dedicated thread in the desktop app), and the
//! store stays free of lifetime and threading policy.

use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

use downpour_types::{
    DownloadId, DownloadSource, DownloadState, ErrorCode, ErrorInfo, SegmentSpec,
};
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde_json::Value;
use uuid::Uuid;

use crate::StoreError;

type Result<T> = std::result::Result<T, StoreError>;

const COLUMNS: &str = "id, url, final_url, file_name, dest_dir, temp_path, final_path, status, \
     total_bytes, downloaded_bytes, mime, etag, last_modified, supports_ranges, connections, \
     speed_limit, checksum_sha256, error_code, error_message, referer, user_agent, source, \
     metadata_json, created_at, started_at, completed_at, updated_at";

/// Current wall-clock time in milliseconds since the Unix epoch.
#[must_use]
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

fn clamp_i64(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

/// A download that has been requested but not yet probed.
#[derive(Debug, Clone)]
pub struct NewDownload {
    pub id: DownloadId,
    pub url: String,
    pub file_name: String,
    pub dest_dir: String,
    pub temp_path: String,
    pub final_path: String,
    pub connections: u16,
    pub source: DownloadSource,
    pub checksum_sha256: Option<String>,
    pub referer: Option<String>,
    pub user_agent: Option<String>,
    pub speed_limit: Option<u64>,
    pub created_at_ms: i64,
    /// True when the caller supplied the name and probing must not replace it.
    pub name_locked: bool,
    /// Extra request headers to replay on resume. Never include secrets here.
    pub headers: Vec<(String, String)>,
}

/// Full persisted state of one download.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DownloadRecord {
    pub id: DownloadId,
    pub url: String,
    pub final_url: Option<String>,
    pub file_name: String,
    pub dest_dir: String,
    pub temp_path: Option<String>,
    pub final_path: Option<String>,
    pub state: DownloadState,
    pub total_bytes: Option<u64>,
    pub downloaded_bytes: u64,
    pub mime: Option<String>,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub supports_ranges: Option<bool>,
    pub connections: u16,
    pub speed_limit: Option<u64>,
    pub checksum_sha256: Option<String>,
    pub error: Option<ErrorInfo>,
    pub referer: Option<String>,
    pub user_agent: Option<String>,
    pub source: DownloadSource,
    pub metadata_json: Option<String>,
    pub created_at_ms: i64,
    pub started_at_ms: Option<i64>,
    pub completed_at_ms: Option<i64>,
    pub updated_at_ms: i64,
}

impl DownloadRecord {
    /// True when the user pinned the file name and probing must keep it.
    #[must_use]
    pub fn name_locked(&self) -> bool {
        let Some(raw) = self.metadata_json.as_deref() else {
            return false;
        };
        serde_json::from_str::<Value>(raw)
            .ok()
            .and_then(|value| value.get("name_locked").and_then(Value::as_bool))
            .unwrap_or(false)
    }

    /// Extra request headers persisted as JSON for resume.
    #[must_use]
    pub fn extra_headers(&self) -> Vec<(String, String)> {
        let Some(raw) = self.metadata_json.as_deref() else {
            return Vec::new();
        };
        let Ok(value) = serde_json::from_str::<Value>(raw) else {
            return Vec::new();
        };
        value
            .get("headers")
            .and_then(Value::as_object)
            .map(|map| {
                map.iter()
                    .filter_map(|(key, value)| {
                        value.as_str().map(|value| (key.clone(), value.to_owned()))
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// Fields learned from the probe request.
#[derive(Debug, Clone, Default)]
pub struct ProbeUpdate {
    pub final_url: String,
    pub total_bytes: Option<u64>,
    pub supports_ranges: bool,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub mime: Option<String>,
}

/// Segment lifecycle state, mirroring the `segments.state` CHECK constraint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SegmentState {
    Pending,
    Active,
    Done,
    Failed,
}

impl SegmentState {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Active => "active",
            Self::Done => "done",
            Self::Failed => "failed",
        }
    }
}

impl FromStr for SegmentState {
    type Err = StoreError;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "pending" => Ok(Self::Pending),
            "active" => Ok(Self::Active),
            "done" => Ok(Self::Done),
            "failed" => Ok(Self::Failed),
            other => Err(StoreError::Data(format!("unknown segment state: {other}"))),
        }
    }
}

/// One persisted segment row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SegmentRecord {
    pub index: u32,
    pub start: u64,
    pub end: u64,
    pub downloaded: u64,
    pub state: SegmentState,
}

impl SegmentRecord {
    #[must_use]
    pub fn to_spec(&self) -> SegmentSpec {
        SegmentSpec {
            index: self.index,
            start: self.start,
            end: self.end,
            downloaded: self.downloaded,
        }
    }

    /// Number of bytes in the segment.
    #[must_use]
    pub fn len(&self) -> u64 {
        self.end.saturating_sub(self.start).saturating_add(1)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.end < self.start
    }

    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.downloaded >= self.len()
    }
}

fn parse_u64(value: i64) -> u64 {
    u64::try_from(value).unwrap_or(0)
}

fn parse_state(value: &str) -> rusqlite::Result<DownloadState> {
    value
        .parse()
        .map_err(|err: downpour_types::UnknownEnumValue| {
            rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(err))
        })
}

fn parse_error_code(raw: Option<&str>, message: Option<String>) -> Option<ErrorInfo> {
    let raw = raw?;
    let code = serde_json::from_str::<ErrorCode>(raw).unwrap_or(ErrorCode::Internal);
    Some(ErrorInfo::new(code, message.unwrap_or_default()))
}

fn row_to_record(row: &Row<'_>) -> rusqlite::Result<DownloadRecord> {
    let state_text: String = row.get(7)?;
    let source_text: String = row.get(21)?;
    let error_code: Option<String> = row.get(17)?;
    let error_message: Option<String> = row.get(18)?;

    Ok(DownloadRecord {
        id: DownloadId(Uuid::parse_str(&row.get::<_, String>(0)?).map_err(|err| {
            rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(err))
        })?),
        url: row.get(1)?,
        final_url: row.get(2)?,
        file_name: row.get(3)?,
        dest_dir: row.get(4)?,
        temp_path: row.get(5)?,
        final_path: row.get(6)?,
        state: parse_state(&state_text)?,
        total_bytes: row.get::<_, Option<i64>>(8)?.map(parse_u64),
        downloaded_bytes: parse_u64(row.get::<_, i64>(9)?),
        mime: row.get(10)?,
        etag: row.get(11)?,
        last_modified: row.get(12)?,
        supports_ranges: row.get::<_, Option<i64>>(13)?.map(|value| value != 0),
        connections: u16::try_from(row.get::<_, i64>(14)?).unwrap_or(8),
        speed_limit: row.get::<_, Option<i64>>(15)?.map(parse_u64),
        checksum_sha256: row.get(16)?,
        error: parse_error_code(error_code.as_deref(), error_message),
        referer: row.get(19)?,
        user_agent: row.get(20)?,
        source: DownloadSource::from_str(&source_text).unwrap_or_default(),
        metadata_json: row.get(22)?,
        created_at_ms: row.get(23)?,
        started_at_ms: row.get(24)?,
        completed_at_ms: row.get(25)?,
        updated_at_ms: row.get(26)?,
    })
}

/// Insert a queued download.
pub fn insert(conn: &Connection, download: &NewDownload) -> Result<()> {
    let mut metadata = serde_json::Map::new();
    if download.name_locked {
        metadata.insert("name_locked".to_owned(), Value::Bool(true));
    }
    if !download.headers.is_empty() {
        let headers: serde_json::Map<String, Value> = download
            .headers
            .iter()
            .map(|(key, value)| (key.clone(), Value::String(value.clone())))
            .collect();
        metadata.insert("headers".to_owned(), Value::Object(headers));
    }
    let metadata = Value::Object(metadata).to_string();

    conn.execute(
        "INSERT INTO downloads (
            id, url, file_name, dest_dir, temp_path, final_path, status,
            total_bytes, downloaded_bytes, supports_ranges, connections,
            checksum_sha256, referer, user_agent, speed_limit, source,
            metadata_json, created_at, updated_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'queued', NULL, 0, NULL, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?14)",
        params![
            download.id.to_string(),
            download.url,
            download.file_name,
            download.dest_dir,
            download.temp_path,
            download.final_path,
            download.connections,
            download.checksum_sha256,
            download.referer,
            download.user_agent,
            download.speed_limit.map(clamp_i64),
            download.source.as_str(),
            metadata,
            download.created_at_ms,
        ],
    )?;
    Ok(())
}

/// Fetch one download.
pub fn get(conn: &Connection, id: DownloadId) -> Result<Option<DownloadRecord>> {
    let sql = format!("SELECT {COLUMNS} FROM downloads WHERE id = ?1");
    let record = conn
        .query_row(&sql, params![id.to_string()], row_to_record)
        .optional()?;
    Ok(record)
}

/// List downloads newest first.
pub fn list(conn: &Connection, limit: usize) -> Result<Vec<DownloadRecord>> {
    let sql = format!("SELECT {COLUMNS} FROM downloads ORDER BY created_at DESC LIMIT ?1");
    let mut statement = conn.prepare(&sql)?;
    let rows = statement.query_map(params![clamp_i64(limit as u64)], row_to_record)?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

/// Downloads that can be picked up by a run loop, oldest first.
pub fn list_runnable(conn: &Connection) -> Result<Vec<DownloadRecord>> {
    let sql = format!(
        "SELECT {COLUMNS} FROM downloads \
         WHERE status IN ('queued', 'paused') ORDER BY created_at ASC"
    );
    let mut statement = conn.prepare(&sql)?;
    let rows = statement.query_map([], row_to_record)?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

/// Update the lifecycle state.
pub fn set_state(conn: &Connection, id: DownloadId, state: DownloadState) -> Result<()> {
    conn.execute(
        "UPDATE downloads SET status = ?1, updated_at = ?2 WHERE id = ?3",
        params![state.as_str(), now_ms(), id.to_string()],
    )?;
    Ok(())
}

/// Store a terminal error and state.
pub fn set_error(conn: &Connection, id: DownloadId, error: &ErrorInfo) -> Result<()> {
    let code = serde_json::to_string(&error.code).unwrap_or_else(|_| "\"internal\"".to_owned());
    conn.execute(
        "UPDATE downloads SET status = 'failed', error_code = ?1, error_message = ?2, \
         updated_at = ?3 WHERE id = ?4",
        params![code, error.message, now_ms(), id.to_string()],
    )?;
    Ok(())
}

/// Clear a previous error without changing state.
pub fn clear_error(conn: &Connection, id: DownloadId) -> Result<()> {
    conn.execute(
        "UPDATE downloads SET error_code = NULL, error_message = NULL, updated_at = ?1 WHERE id = ?2",
        params![now_ms(), id.to_string()],
    )?;
    Ok(())
}

/// Persist what the probe learned.
pub fn record_probe(conn: &Connection, id: DownloadId, probe: &ProbeUpdate) -> Result<()> {
    conn.execute(
        "UPDATE downloads SET final_url = ?1, total_bytes = ?2, supports_ranges = ?3, \
         etag = ?4, last_modified = ?5, mime = ?6, updated_at = ?7 WHERE id = ?8",
        params![
            probe.final_url,
            probe.total_bytes.map(clamp_i64),
            i64::from(probe.supports_ranges),
            probe.etag,
            probe.last_modified,
            probe.mime,
            now_ms(),
            id.to_string(),
        ],
    )?;
    Ok(())
}

/// Persist the resolved file name and paths.
pub fn set_paths(
    conn: &Connection,
    id: DownloadId,
    file_name: &str,
    temp_path: &str,
    final_path: &str,
) -> Result<()> {
    conn.execute(
        "UPDATE downloads SET file_name = ?1, temp_path = ?2, final_path = ?3, updated_at = ?4 \
         WHERE id = ?5",
        params![file_name, temp_path, final_path, now_ms(), id.to_string()],
    )?;
    Ok(())
}

/// Mark the download as started.
pub fn mark_started(conn: &Connection, id: DownloadId) -> Result<()> {
    conn.execute(
        "UPDATE downloads SET status = 'downloading', started_at = COALESCE(started_at, ?1), \
         updated_at = ?1, error_code = NULL, error_message = NULL WHERE id = ?2",
        params![now_ms(), id.to_string()],
    )?;
    Ok(())
}

/// Insert or replace the single segment of a single-stream download.
pub fn upsert_segment(
    conn: &Connection,
    id: DownloadId,
    spec: &SegmentSpec,
    state: SegmentState,
) -> Result<()> {
    conn.execute(
        "INSERT INTO segments (download_id, idx, start_offset, end_offset, downloaded, state, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(download_id, idx) DO UPDATE SET
            start_offset = excluded.start_offset,
            end_offset = excluded.end_offset,
            downloaded = excluded.downloaded,
            state = excluded.state,
            updated_at = excluded.updated_at",
        params![
            id.to_string(),
            i64::from(spec.index),
            clamp_i64(spec.start),
            clamp_i64(spec.end),
            clamp_i64(spec.downloaded),
            state.as_str(),
            now_ms(),
        ],
    )?;
    Ok(())
}

/// Replace the whole segment plan for a download.
pub fn replace_segments(
    conn: &mut Connection,
    id: DownloadId,
    specs: &[SegmentSpec],
) -> Result<()> {
    let tx = conn.transaction()?;
    tx.execute(
        "DELETE FROM segments WHERE download_id = ?1",
        params![id.to_string()],
    )?;
    let now = now_ms();
    for spec in specs {
        tx.execute(
            "INSERT INTO segments (download_id, idx, start_offset, end_offset, downloaded, \
             state, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, 'pending', ?6)",
            params![
                id.to_string(),
                i64::from(spec.index),
                clamp_i64(spec.start),
                clamp_i64(spec.end),
                clamp_i64(spec.downloaded),
                now,
            ],
        )?;
    }
    tx.commit()?;
    Ok(())
}

/// Commit durable progress for every segment in one transaction.
pub fn commit_segments(
    conn: &mut Connection,
    id: DownloadId,
    downloaded_total: u64,
    segments: &[SegmentRecord],
) -> Result<()> {
    let tx = conn.transaction()?;
    let now = now_ms();
    tx.execute(
        "UPDATE downloads SET downloaded_bytes = ?1, updated_at = ?2 WHERE id = ?3",
        params![clamp_i64(downloaded_total), now, id.to_string()],
    )?;
    for segment in segments {
        let state = if segment.is_complete() {
            "done"
        } else {
            "active"
        };
        tx.execute(
            "UPDATE segments SET downloaded = ?1, state = ?2, updated_at = ?3 \
             WHERE download_id = ?4 AND idx = ?5",
            params![
                clamp_i64(segment.downloaded),
                state,
                now,
                id.to_string(),
                i64::from(segment.index),
            ],
        )?;
    }
    tx.commit()?;
    Ok(())
}

/// Load the segment table for a download, ordered by index.
pub fn load_segments(conn: &Connection, id: DownloadId) -> Result<Vec<SegmentRecord>> {
    let mut statement = conn.prepare(
        "SELECT idx, start_offset, end_offset, downloaded, state FROM segments \
         WHERE download_id = ?1 ORDER BY idx",
    )?;
    let rows = statement.query_map(params![id.to_string()], |row| {
        let state_text: String = row.get(4)?;
        let state = state_text.parse::<SegmentState>().map_err(|err| {
            rusqlite::Error::FromSqlConversionFailure(4, rusqlite::types::Type::Text, Box::new(err))
        })?;
        Ok(SegmentRecord {
            index: u32::try_from(row.get::<_, i64>(0)?).unwrap_or(0),
            start: parse_u64(row.get::<_, i64>(1)?),
            end: parse_u64(row.get::<_, i64>(2)?),
            downloaded: parse_u64(row.get::<_, i64>(3)?),
            state,
        })
    })?;

    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

/// Durably record progress: the caller must have synced the file first.
///
/// The file is synced by the writer thread before this transaction runs, so a
/// committed offset never runs ahead of bytes on disk.
pub fn commit_progress(
    conn: &mut Connection,
    id: DownloadId,
    downloaded: u64,
    segment: Option<&SegmentRecord>,
) -> Result<()> {
    let tx = conn.transaction()?;
    let now = now_ms();
    tx.execute(
        "UPDATE downloads SET downloaded_bytes = ?1, updated_at = ?2 WHERE id = ?3",
        params![clamp_i64(downloaded), now, id.to_string()],
    )?;
    if let Some(segment) = segment {
        tx.execute(
            "UPDATE segments SET downloaded = ?1, state = 'active', updated_at = ?2 \
             WHERE download_id = ?3 AND idx = ?4",
            params![
                clamp_i64(segment.downloaded),
                now,
                id.to_string(),
                i64::from(segment.index),
            ],
        )?;
    }
    tx.commit()?;
    Ok(())
}

/// Mark a download complete and append a history row in one transaction.
pub fn mark_completed(
    conn: &mut Connection,
    id: DownloadId,
    bytes: u64,
    checksum_sha256: &str,
    avg_speed_bps: Option<u64>,
) -> Result<()> {
    let tx = conn.transaction()?;
    let now = now_ms();

    let (url, file_name): (String, String) = tx.query_row(
        "SELECT url, file_name FROM downloads WHERE id = ?1",
        params![id.to_string()],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;

    tx.execute(
        "UPDATE downloads SET status = 'completed', downloaded_bytes = ?1, total_bytes = ?1, \
         checksum_sha256 = ?2, completed_at = ?3, updated_at = ?3, error_code = NULL, \
         error_message = NULL WHERE id = ?4",
        params![clamp_i64(bytes), checksum_sha256, now, id.to_string()],
    )?;
    tx.execute(
        "UPDATE segments SET downloaded = end_offset - start_offset + 1, state = 'done', \
         updated_at = ?1 WHERE download_id = ?2",
        params![now, id.to_string()],
    )?;
    tx.execute(
        "INSERT INTO history (id, download_id, url, file_name, bytes, status, avg_speed, \
         checksum_sha256, completed_at) VALUES (?1, ?2, ?3, ?4, ?5, 'completed', ?6, ?7, ?8)",
        params![
            Uuid::now_v7().to_string(),
            id.to_string(),
            url,
            file_name,
            clamp_i64(bytes),
            avg_speed_bps.map(clamp_i64),
            checksum_sha256,
            now,
        ],
    )?;
    tx.commit()?;
    Ok(())
}

/// Reset persisted progress to zero (used when the server content changed).
pub fn reset_progress(conn: &mut Connection, id: DownloadId) -> Result<()> {
    let tx = conn.transaction()?;
    let now = now_ms();
    tx.execute(
        "UPDATE downloads SET downloaded_bytes = 0, updated_at = ?1 WHERE id = ?2",
        params![now, id.to_string()],
    )?;
    tx.execute(
        "UPDATE segments SET downloaded = 0, state = 'pending', updated_at = ?1 \
         WHERE download_id = ?2",
        params![now, id.to_string()],
    )?;
    tx.commit()?;
    Ok(())
}

/// Move downloads interrupted by a crash to `paused` so they can be resumed.
///
/// Returns the number of rows changed.
pub fn recover_interrupted(conn: &Connection) -> Result<usize> {
    let changed = conn.execute(
        "UPDATE downloads SET status = 'paused', updated_at = ?1 \
         WHERE status IN ('probing', 'downloading', 'finalizing')",
        params![now_ms()],
    )?;
    Ok(changed)
}

/// Update the remembered validators after a content change was detected.
pub fn update_validators(
    conn: &Connection,
    id: DownloadId,
    etag: Option<&str>,
    last_modified: Option<&str>,
) -> Result<()> {
    conn.execute(
        "UPDATE downloads SET etag = ?1, last_modified = ?2, updated_at = ?3 WHERE id = ?4",
        params![etag, last_modified, now_ms(), id.to_string()],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Store;

    fn sample(id: DownloadId) -> NewDownload {
        NewDownload {
            id,
            url: "http://127.0.0.1:1/file.bin".to_owned(),
            file_name: "file.bin".to_owned(),
            dest_dir: "/tmp".to_owned(),
            temp_path: "/tmp/file.bin.downpour-part".to_owned(),
            final_path: "/tmp/file.bin".to_owned(),
            connections: 8,
            source: DownloadSource::Api,
            checksum_sha256: None,
            referer: None,
            user_agent: None,
            speed_limit: None,
            created_at_ms: now_ms(),
            name_locked: false,
            headers: vec![("X-Test".to_owned(), "1".to_owned())],
        }
    }

    #[test]
    fn insert_and_get_round_trip() {
        let store = Store::open_in_memory().unwrap();
        let id = DownloadId::new();
        insert(store.connection(), &sample(id)).unwrap();

        let record = get(store.connection(), id).unwrap().expect("record");
        assert_eq!(record.id, id);
        assert_eq!(record.state, DownloadState::Queued);
        assert_eq!(record.connections, 8);
        assert!(!record.name_locked());
        assert_eq!(
            record.extra_headers(),
            vec![("X-Test".to_owned(), "1".to_owned())]
        );
    }

    #[test]
    fn progress_and_completion_are_transactional() {
        let mut store = Store::open_in_memory().unwrap();
        let id = DownloadId::new();
        insert(store.connection(), &sample(id)).unwrap();

        record_probe(
            store.connection(),
            id,
            &ProbeUpdate {
                final_url: "http://127.0.0.1:1/file.bin".to_owned(),
                total_bytes: Some(1000),
                supports_ranges: true,
                etag: Some("\"abc\"".to_owned()),
                last_modified: None,
                mime: Some("application/octet-stream".to_owned()),
            },
        )
        .unwrap();

        let spec = SegmentSpec::new(0, 0, 999);
        upsert_segment(store.connection(), id, &spec, SegmentState::Active).unwrap();

        let mut segment = load_segments(store.connection(), id)
            .unwrap()
            .pop()
            .unwrap();
        segment.downloaded = 400;
        commit_progress(store.connection_mut(), id, 400, Some(&segment)).unwrap();

        let record = get(store.connection(), id).unwrap().unwrap();
        assert_eq!(record.downloaded_bytes, 400);
        assert_eq!(record.total_bytes, Some(1000));

        mark_completed(store.connection_mut(), id, 1000, "deadbeef", Some(2048)).unwrap();
        let record = get(store.connection(), id).unwrap().unwrap();
        assert_eq!(record.state, DownloadState::Completed);
        assert_eq!(record.downloaded_bytes, 1000);

        let history: i64 = store
            .connection()
            .query_row("SELECT count(*) FROM history", [], |row| row.get(0))
            .unwrap();
        assert_eq!(history, 1);
    }

    #[test]
    fn interrupted_downloads_become_paused() {
        let store = Store::open_in_memory().unwrap();
        let id = DownloadId::new();
        insert(store.connection(), &sample(id)).unwrap();
        set_state(store.connection(), id, DownloadState::Downloading).unwrap();

        assert_eq!(recover_interrupted(store.connection()).unwrap(), 1);
        let record = get(store.connection(), id).unwrap().unwrap();
        assert_eq!(record.state, DownloadState::Paused);
    }

    #[test]
    fn errors_round_trip() {
        let store = Store::open_in_memory().unwrap();
        let id = DownloadId::new();
        insert(store.connection(), &sample(id)).unwrap();
        set_error(
            store.connection(),
            id,
            &ErrorInfo::new(ErrorCode::Http { status: 404 }, "not found"),
        )
        .unwrap();

        let record = get(store.connection(), id).unwrap().unwrap();
        assert_eq!(record.state, DownloadState::Failed);
        let error = record.error.expect("error");
        assert_eq!(error.code, ErrorCode::Http { status: 404 });
        assert_eq!(error.message, "not found");
    }
}
