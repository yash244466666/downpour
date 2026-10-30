//! Grabber job and frontier persistence.

use rusqlite::{Connection, params};

use crate::StoreError;
use crate::downloads::now_ms;

type Result<T> = std::result::Result<T, StoreError>;

/// One crawler job.
#[derive(Debug, Clone)]
pub struct NewGrabJob {
    pub id: String,
    pub seed_url: String,
    pub filters_json: Option<String>,
    pub max_depth: u32,
    pub max_pages: u32,
    pub same_origin: bool,
    pub respect_robots: bool,
    pub created_at_ms: i64,
}

pub fn insert_job(conn: &Connection, job: &NewGrabJob) -> Result<()> {
    conn.execute(
        "INSERT INTO grabber_jobs (id, seed_url, filters_json, max_depth, max_pages, \
         same_origin, respect_robots, status, pages_done, found_count, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'running', 0, 0, ?8)",
        params![
            job.id,
            job.seed_url,
            job.filters_json,
            i64::from(job.max_depth),
            i64::from(job.max_pages),
            i64::from(job.same_origin),
            i64::from(job.respect_robots),
            job.created_at_ms,
        ],
    )?;
    Ok(())
}

pub fn update_job(
    conn: &Connection,
    id: &str,
    status: &str,
    pages_done: u32,
    found_count: u32,
) -> Result<()> {
    conn.execute(
        "UPDATE grabber_jobs SET status = ?1, pages_done = ?2, found_count = ?3 WHERE id = ?4",
        params![status, i64::from(pages_done), i64::from(found_count), id],
    )?;
    Ok(())
}

/// Record a discovered URL. `url_hash` stores the canonical URL itself, which
/// is unique per job and avoids a platform-dependent hash.
pub fn upsert_frontier(
    conn: &Connection,
    job_id: &str,
    url: &str,
    depth: u32,
    state: &str,
) -> Result<()> {
    conn.execute(
        "INSERT INTO grabber_frontier (job_id, url_hash, url, depth, state, discovered_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
         ON CONFLICT(job_id, url_hash) DO UPDATE SET depth = excluded.depth, \
         state = excluded.state",
        params![job_id, url, url, i64::from(depth), state, now_ms()],
    )?;
    Ok(())
}

pub fn set_frontier_state(conn: &Connection, job_id: &str, url: &str, state: &str) -> Result<()> {
    conn.execute(
        "UPDATE grabber_frontier SET state = ?1 WHERE job_id = ?2 AND url_hash = ?3",
        params![state, job_id, url],
    )?;
    Ok(())
}
