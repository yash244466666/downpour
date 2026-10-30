//! SQLite persistence for Downpour.
//!
//! `rusqlite` is used deliberately: the database is local and single-writer, so
//! async SQL adds complexity without concurrency. One writer connection lives on
//! a dedicated thread (see the engine); readers may use a second connection.

mod migrations;

use std::path::Path;

use rusqlite::Connection;
use thiserror::Error;

pub use migrations::migrations;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("migration error: {0}")]
    Migration(#[from] rusqlite_migration::Error),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// A migration-checked SQLite connection.
pub struct Store {
    conn: Connection,
}

impl Store {
    /// Open (or create) a database file and run all migrations.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let mut conn = Connection::open(path)?;
        Self::configure(&conn)?;
        migrations().to_latest(&mut conn)?;
        Ok(Self { conn })
    }

    /// Open an in-memory database, primarily for tests.
    pub fn open_in_memory() -> Result<Self, StoreError> {
        let mut conn = Connection::open_in_memory()?;
        Self::configure(&conn)?;
        migrations().to_latest(&mut conn)?;
        Ok(Self { conn })
    }

    fn configure(conn: &Connection) -> Result<(), StoreError> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "busy_timeout", 5000)?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.pragma_update(None, "temp_store", "MEMORY")?;
        Ok(())
    }

    #[must_use]
    pub fn connection(&self) -> &Connection {
        &self.conn
    }

    #[must_use]
    pub fn connection_mut(&mut self) -> &mut Connection {
        &mut self.conn
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_and_migrates_in_memory() {
        let store = Store::open_in_memory().expect("open");
        let count: i64 = store
            .connection()
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = 'downloads'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn migrations_are_idempotent() {
        let store = Store::open_in_memory().expect("open");
        let mut conn = store.conn;
        migrations()
            .to_latest(&mut conn)
            .expect("re-run migrations");
    }
}
