//! Schema migrations.
//!
//! Every migration is append-only. Never edit a migration that has shipped;
//! add a new one instead.

use rusqlite_migration::{M, Migrations};

pub const MIGRATION_001: &str = r#"
CREATE TABLE settings (
    key         TEXT PRIMARY KEY,
    value_json  TEXT NOT NULL,
    updated_at  INTEGER NOT NULL
) STRICT;

CREATE TABLE queues (
    id                  TEXT PRIMARY KEY,
    name                TEXT NOT NULL,
    max_concurrent      INTEGER NOT NULL DEFAULT 3,
    speed_limit         INTEGER,
    default_connections INTEGER NOT NULL DEFAULT 8,
    priority            INTEGER NOT NULL DEFAULT 0,
    enabled             INTEGER NOT NULL DEFAULT 1,
    schedule_id         TEXT
) STRICT;

CREATE TABLE schedules (
    id           TEXT PRIMARY KEY,
    name         TEXT NOT NULL,
    cron         TEXT,
    timezone     TEXT NOT NULL DEFAULT 'UTC',
    days_mask    INTEGER,
    window_start TEXT,
    window_end   TEXT,
    action       TEXT NOT NULL,
    speed_limit  INTEGER,
    enabled      INTEGER NOT NULL DEFAULT 1,
    next_run_at  INTEGER,
    last_run_at  INTEGER
) STRICT;

CREATE TABLE downloads (
    id               TEXT PRIMARY KEY,
    url              TEXT NOT NULL,
    final_url        TEXT,
    file_name        TEXT NOT NULL,
    dest_dir         TEXT NOT NULL,
    temp_path        TEXT,
    final_path       TEXT,
    total_bytes      INTEGER CHECK (total_bytes IS NULL OR total_bytes >= 0),
    downloaded_bytes INTEGER NOT NULL DEFAULT 0 CHECK (downloaded_bytes >= 0),
    status           TEXT NOT NULL DEFAULT 'queued' CHECK (status IN (
                         'queued','probing','downloading','paused',
                         'finalizing','completed','failed','canceled')),
    mime             TEXT,
    etag             TEXT,
    last_modified    TEXT,
    supports_ranges  INTEGER,
    connections      INTEGER NOT NULL DEFAULT 8 CHECK (connections BETWEEN 1 AND 32),
    speed_limit      INTEGER,
    checksum_sha256  TEXT,
    error_code       TEXT,
    error_message    TEXT,
    referer          TEXT,
    user_agent       TEXT,
    proxy_json       TEXT,
    headers_enc      BLOB,
    queue_id         TEXT REFERENCES queues(id) ON DELETE SET NULL,
    priority         INTEGER NOT NULL DEFAULT 0,
    auto_start       INTEGER NOT NULL DEFAULT 1,
    open_when_done   INTEGER NOT NULL DEFAULT 0,
    source           TEXT NOT NULL DEFAULT 'ui' CHECK (source IN (
                         'ui','browser_extension','clipboard','batch','api')),
    created_at       INTEGER NOT NULL,
    started_at       INTEGER,
    completed_at     INTEGER,
    updated_at       INTEGER NOT NULL,
    metadata_json    TEXT CHECK (metadata_json IS NULL OR json_valid(metadata_json))
) STRICT;

CREATE TABLE segments (
    download_id  TEXT NOT NULL REFERENCES downloads(id) ON DELETE CASCADE,
    idx          INTEGER NOT NULL,
    start_offset INTEGER NOT NULL CHECK (start_offset >= 0),
    end_offset   INTEGER NOT NULL CHECK (end_offset >= start_offset),
    downloaded   INTEGER NOT NULL DEFAULT 0 CHECK (downloaded >= 0),
    state        TEXT NOT NULL DEFAULT 'pending' CHECK (state IN (
                     'pending','active','done','failed')),
    updated_at   INTEGER NOT NULL,
    PRIMARY KEY (download_id, idx)
) STRICT;

CREATE TABLE history (
    id              TEXT PRIMARY KEY,
    download_id     TEXT,
    url             TEXT NOT NULL,
    file_name       TEXT NOT NULL,
    bytes           INTEGER NOT NULL DEFAULT 0,
    status          TEXT NOT NULL,
    avg_speed       INTEGER,
    checksum_sha256 TEXT,
    completed_at    INTEGER NOT NULL
) STRICT;

CREATE TABLE media_jobs (
    id            TEXT PRIMARY KEY,
    page_url      TEXT,
    manifest_url  TEXT NOT NULL,
    kind          TEXT NOT NULL,
    title         TEXT,
    variants_json TEXT CHECK (variants_json IS NULL OR json_valid(variants_json)),
    status        TEXT NOT NULL,
    download_id   TEXT,
    created_at    INTEGER NOT NULL
) STRICT;

CREATE TABLE grabber_jobs (
    id             TEXT PRIMARY KEY,
    seed_url       TEXT NOT NULL,
    filters_json   TEXT CHECK (filters_json IS NULL OR json_valid(filters_json)),
    max_depth      INTEGER NOT NULL DEFAULT 2,
    max_pages      INTEGER NOT NULL DEFAULT 100,
    same_origin    INTEGER NOT NULL DEFAULT 1,
    respect_robots INTEGER NOT NULL DEFAULT 1,
    status         TEXT NOT NULL,
    pages_done     INTEGER NOT NULL DEFAULT 0,
    found_count    INTEGER NOT NULL DEFAULT 0,
    created_at     INTEGER NOT NULL
) STRICT;

CREATE TABLE grabber_frontier (
    job_id        TEXT NOT NULL REFERENCES grabber_jobs(id) ON DELETE CASCADE,
    url_hash      TEXT NOT NULL,
    url           TEXT NOT NULL,
    depth         INTEGER NOT NULL,
    state         TEXT NOT NULL DEFAULT 'pending',
    discovered_at INTEGER NOT NULL,
    PRIMARY KEY (job_id, url_hash)
) STRICT;

CREATE INDEX idx_downloads_status_created ON downloads(status, created_at DESC);
CREATE INDEX idx_segments_download ON segments(download_id, idx);
CREATE INDEX idx_history_completed ON history(completed_at DESC);
CREATE INDEX idx_schedules_due ON schedules(enabled, next_run_at);
CREATE INDEX idx_grabber_frontier_state ON grabber_frontier(job_id, state);

INSERT INTO settings(key, value_json, updated_at) VALUES
    ('downloads.default_connections', '8', 0),
    ('downloads.max_concurrent', '3', 0),
    ('downloads.default_dir', 'null', 0),
    ('downloads.retry_max_attempts', '10', 0),
    ('network.speed_limit', '0', 0),
    ('ui.theme', '"system"', 0),
    ('ui.language', '"en"', 0);
"#;

/// All migrations in order.
#[must_use]
pub fn migrations() -> Migrations<'static> {
    Migrations::new(vec![M::up(MIGRATION_001)])
}
