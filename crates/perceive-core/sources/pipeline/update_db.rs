use std::sync::atomic::Ordering;

use rusqlite::{named_params, params};

use super::{EmbeddingsOutput, ScanItemState, ScanStats};
use crate::{db::Database, search::serialize_embedding};

pub fn update_db(
    model_id: u32,
    model_version: u32,
    stats: &ScanStats,
    database: &Database,
    index_version: i64,
    rx: flume::Receiver<EmbeddingsOutput>,
) -> Result<(), eyre::Report> {
    for batch in rx {
        let _track = stats.write_time.begin();

        let mut changed = 0;
        let mut unchanged = 0;
        let mut new = 0;

        let mut write_conn = database.write_conn.lock();
        let tx = write_conn.transaction()?;
        {
            let mut unchanged_stmt = tx.prepare_cached(
                r##"
                UPDATE items SET version = ?, last_accessed = ? WHERE id = ?
                "##,
            )?;

            let mut changed_stmt = tx.prepare_cached(
                r##"
                UPDATE items
                SET version=:version, hash=:hash, content=:content, raw_content=:raw_content,
                    process_version=:process_version,
                    name=:name, author=:author, description=:description,
                    modified=:modified, last_accessed=:last_accessed,
                    skipped=:skipped
                WHERE id=:id
                "##,
            )?;

            let mut new_stmt = tx.prepare_cached(
                r##"
                INSERT INTO items (source_id, external_id, version, hash, content,
                    raw_content, process_version, name, author,
                    description, modified, last_accessed, skipped)
                VALUES (:source_id, :external_id, :version, :hash, :content, :raw_content, :process_version,
                    :name, :author, :description, :modified, :last_accessed, :skipped);
                "##,
            )?;

            let mut embedding_stmt = tx.prepare_cached(
                r##" INSERT INTO item_embeddings
                    (item_id, item_index_version, embedding, model_id, model_version)
                    VALUES (:id, :version, :embedding, :model_id, :model_version)
                    ON CONFLICT (item_id, model_id, model_version) DO UPDATE
                        SET item_index_version=EXCLUDED.item_index_version,
                            embedding=EXCLUDED.embedding"##,
            )?;

            for (item, embedding) in &batch {
                let found_item_id = item.item.id;
                let item_id = match &item.state {
                    ScanItemState::Unchanged => {
                        unchanged_stmt.execute(params![
                            index_version,
                            item.item.metadata.atime.map(|t| t.unix_timestamp()),
                            found_item_id,
                        ])?;
                        unchanged += 1;
                        found_item_id
                    }
                    ScanItemState::Found | ScanItemState::Changed => {
                        changed_stmt.execute(named_params! {
                            ":id": found_item_id,
                            ":version": index_version,
                            ":hash": item.item.hash.as_deref().unwrap_or_default(),
                            ":content": item.item.content.as_deref().unwrap_or_default(),
                            ":raw_content": item.item.raw_content.as_ref(),
                            ":process_version": item.item.process_version,
                            ":name": item.item.metadata.name.as_deref(),
                            ":author": item.item.metadata.author.as_deref(),
                            ":description": item.item.metadata.description.as_deref(),
                            ":modified": item.item.metadata.mtime.map(|t| t.unix_timestamp()),
                            ":last_accessed": item.item.metadata.atime.map(|t| t.unix_timestamp()),
                            ":skipped": item.item.skipped.map(|s| s.to_string()),
                        })?;

                        changed += 1;
                        found_item_id
                    }
                    ScanItemState::New => {
                        new_stmt.execute(named_params! {
                            ":source_id": item.item.source_id,
                            ":external_id": item.item.external_id,
                            ":version": index_version,
                            ":hash": item.item.hash.as_deref().unwrap_or_default(),
                            ":content": item.item.content.as_deref().unwrap_or_default(),
                            ":raw_content": item.item.raw_content.as_ref(),
                            ":process_version": item.item.process_version,
                            ":name": item.item.metadata.name.as_deref(),
                            ":author": item.item.metadata.author.as_deref(),
                            ":description": item.item.metadata.description.as_deref(),
                            ":modified": item.item.metadata.mtime.map(|t| t.unix_timestamp()),
                            ":last_accessed": item.item.metadata.atime.map(|t| t.unix_timestamp()),
                            ":skipped": item.item.skipped.map(|s| s.to_string()),
                        })?;

                        let row_id = tx.last_insert_rowid();

                        new += 1;
                        row_id
                    }
                };

                if let Some(embedding) = embedding {
                    let bytes_vec = serialize_embedding(embedding);
                    embedding_stmt.execute(named_params! {
                        ":embedding": &bytes_vec,
                        ":version": index_version,
                        ":id": item_id,
                        ":model_id": model_id,
                        ":model_version": model_version,
                    })?;
                }
            }
        }

        tx.commit()?;

        stats.added.fetch_add(new, Ordering::Relaxed);
        stats.changed.fetch_add(changed, Ordering::Relaxed);
        stats.unchanged.fetch_add(unchanged, Ordering::Relaxed);
    }

    Ok(())
}
