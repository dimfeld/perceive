use std::{ops::Deref, path::PathBuf, sync::Arc};

use const_format::concatcp;
use eyre::Result;
use parking_lot::Mutex;
use rusqlite::{params, Connection};
use thiserror::Error;
use time::OffsetDateTime;

use crate::{paths::PROJECT_DIRS, Item, ItemMetadata};

#[derive(Debug, Error)]
pub enum DbError {
    #[error("Failed to open database: {path}: {error}")]
    Open { path: PathBuf, error: eyre::Report },

    #[error("Database connection error: {0}")]
    PoolError(#[from] r2d2::Error),

    #[error("Query error: {0}")]
    Db(#[from] rusqlite::Error),

    #[error("Query error: {0}")]
    Query(#[from] eyre::Report),
}

impl DbError {
    pub fn query(e: impl Into<eyre::Report>) -> DbError {
        DbError::Query(e.into())
    }
}

#[derive(Clone)]
pub struct Database(Arc<DatabaseInner>);

impl Database {
    pub fn new(path: Option<PathBuf>) -> Result<Database, DbError> {
        DatabaseInner::new(path).map(|inner| Database(Arc::new(inner)))
    }
}

impl Deref for Database {
    type Target = DatabaseInner;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

pub struct DatabaseInner {
    pub write_conn: Mutex<Connection>,
    pub read_pool: r2d2::Pool<r2d2_sqlite::SqliteConnectionManager>,
}

impl DatabaseInner {
    pub fn new(path: Option<PathBuf>) -> Result<DatabaseInner, DbError> {
        let path = path.unwrap_or_else(|| PROJECT_DIRS.data_local_dir().join("perceive-search.db"));
        let mut write_conn = rusqlite::Connection::open(&path).map_err(|e| DbError::Open {
            path: path.clone(),
            error: e.into(),
        })?;

        Self::setup_database(&mut write_conn).map_err(|error| DbError::Open {
            path: path.clone(),
            error,
        })?;

        let read_manager = r2d2_sqlite::SqliteConnectionManager::file(&path)
            .with_flags(rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .with_init(|conn| rusqlite::vtab::array::load_module(conn));
        let read_pool = r2d2::Pool::new(read_manager).map_err(|error| DbError::Open {
            path: path.clone(),
            error: error.into(),
        })?;

        Ok(DatabaseInner {
            write_conn: Mutex::new(write_conn),
            read_pool,
        })
    }

    fn setup_database(conn: &mut Connection) -> Result<(), eyre::Report> {
        let migrations = rusqlite_migration::Migrations::new(vec![rusqlite_migration::M::up(
            include_str!("./migrations/00001_init.sql"),
        )]);

        conn.pragma_update(None, "journal", "wal")?;
        conn.pragma_update(None, "synchronous", "normal")?;

        migrations.to_latest(conn)?;
        Ok(())
    }

    pub fn read_item(&self, id: i64) -> Result<Option<Item>> {
        let conn = self.read_pool.get()?;
        let mut stmt = conn.prepare_cached(concatcp!(
            "SELECT ",
            ITEM_COLUMNS,
            " FROM items WHERE id = ?"
        ))?;

        let result = stmt
            .query_and_then([id], deserialize_item_row)?
            .into_iter()
            .next()
            .transpose()?;

        Ok(result)
    }

    pub fn set_item_hidden(&self, id: i64, hidden: bool) -> Result<()> {
        let conn = self.write_conn.lock();
        let mut stmt = conn.prepare_cached("UPDATE items SET hidden_at=? WHERE id=?")?;

        if hidden {
            let now = OffsetDateTime::now_utc().unix_timestamp();
            stmt.execute(params![now, id])?;
        } else {
            stmt.execute(params![None::<i64>, id])?;
        }
        Ok(())
    }
}

/// The standard set of columns in the Item table. Use with [deserialize_item_row]
/// for easy item reading.
pub const ITEM_COLUMNS: &str = r##"id, source_id,
            external_id, hash, content, raw_content, process_version,
            name, author, description,
            modified, last_accessed, skipped"##;

/// Deserialize a row selected by `ITEM_COLUMNS` into an `Item`.
pub fn deserialize_item_row(row: &rusqlite::Row) -> Result<Item> {
    let compressed = row.get_ref(5)?.as_blob_or_null()?;
    let raw_content = compressed.map(zstd::decode_all).transpose()?;

    Ok(Item {
        id: row.get(0)?,
        source_id: row.get(1)?,
        external_id: row.get(2)?,
        hash: row.get(3)?,
        content: row.get(4)?,
        raw_content,
        process_version: row.get(6)?,
        metadata: ItemMetadata {
            name: row.get(7)?,
            author: row.get(8)?,
            description: row.get(9)?,
            mtime: row
                .get::<_, Option<i64>>(10)?
                .map(OffsetDateTime::from_unix_timestamp)
                .transpose()?,
            atime: row
                .get::<_, Option<i64>>(11)?
                .map(OffsetDateTime::from_unix_timestamp)
                .transpose()?,
        },
        skipped: row
            .get_ref(12)?
            .as_str_or_null()?
            .map(|s| s.parse())
            .transpose()?,
    })
}
