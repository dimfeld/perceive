use std::{ops::Deref, path::PathBuf, sync::Arc};

use parking_lot::Mutex;
use rusqlite::Connection;
use thiserror::Error;

use crate::paths::PROJECT_DIRS;

#[derive(Debug, Error)]
pub enum DbError {
    #[error("Failed to open database: {path}: {error}")]
    Open { path: PathBuf, error: eyre::Report },
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
            .with_init(|conn| rusqlite::vtab::array::load_module(&conn));
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

        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchrounous", "NORMAL")?;

        migrations.to_latest(conn)?;
        Ok(())
    }
}
