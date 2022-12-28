use std::str::FromStr;

use rusqlite::named_params;
use time::OffsetDateTime;

use super::{ItemCompareStrategy, Source};
use crate::db::{Database, DbError};

pub fn list_sources(database: &Database) -> Result<Vec<Source>, DbError> {
    let conn = database.read_pool.get()?;
    let mut stmt = conn.prepare_cached(
        "SELECT id, name, config, location, compare_strategy, status, last_indexed, index_version
        FROM sources
        WHERE deleted_at IS NULL",
    )?;

    let rows = stmt
        .query_and_then([], |row| {
            Ok::<_, DbError>(Source {
                id: row.get(0)?,
                name: row.get(1)?,
                config: serde_json::from_str(row.get_ref(2)?.as_str().map_err(DbError::query)?)
                    .map_err(DbError::query)?,
                location: row.get(3)?,
                compare_strategy: ItemCompareStrategy::from_str(
                    row.get_ref(4)?.as_str().map_err(DbError::query)?,
                )
                .map_err(DbError::query)?,
                status: serde_json::from_str(row.get_ref(5)?.as_str().map_err(DbError::query)?)
                    .map_err(DbError::query)?,
                last_indexed: OffsetDateTime::from_unix_timestamp(row.get(6)?)
                    .unwrap_or_else(|_| OffsetDateTime::now_utc()),
                index_version: row.get(7)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(rows)
}

pub fn add_source(database: &Database, mut source: Source) -> Result<Source, DbError> {
    let conn = database.write_conn.lock();
    let mut stmt = conn.prepare_cached(
        r##"INSERT INTO sources (name, config, location, compare_strategy, status)
            VALUES (:name, :config, :location, :compare_strategy, :status)"##,
    )?;

    stmt.execute(named_params! {
        ":name": source.name,
        ":config": serde_json::to_string(&source.config).map_err(DbError::query)?,
        ":location": source.location,
        ":compare_strategy": source.compare_strategy.to_string(),
        ":status": serde_json::to_string(&source.status).map_err(DbError::query)?,
    })?;

    source.id = conn.last_insert_rowid();

    Ok(source)
}

pub fn update_source(database: &Database, source: &Source) -> Result<(), DbError> {
    let conn = database.write_conn.lock();
    let mut stmt = conn.prepare_cached(
        r##"UPDATE sources SET
            name = :name,
            config = :config,
            location = :location,
            compare_strategy = :compare_strategy,
            status = :status
        WHERE id = :id"##,
    )?;

    stmt.execute(named_params! {
        ":id": source.id,
        ":name": source.name,
        ":config": serde_json::to_string(&source.config).map_err(DbError::query)?,
        ":location": source.location,
        ":compare_strategy": source.compare_strategy.to_string(),
        ":status": serde_json::to_string(&source.status).map_err(DbError::query)?,
    })?;

    Ok(())
}
