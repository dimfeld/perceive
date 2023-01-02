use std::rc::Rc;

use ahash::HashMap;
use rusqlite::params;

use super::{FoundItem, ScanItem, ScanItemState};
use crate::{db::Database, sources::ItemCompareStrategy, Item, SkipReason};

pub fn match_to_existing_items(
    db: &Database,
    model_id: u32,
    model_version: u32,
    source_id: i64,
    compare_strategy: ItemCompareStrategy,
    rx: flume::Receiver<Vec<Item>>,
    tx: flume::Sender<ScanItem>,
) -> Result<(), eyre::Report> {
    let conn = db.read_pool.get()?;

    let mut stmt = conn.prepare_cached(
        r##"
        SELECT external_id, id, hash, modified, last_accessed, skipped, content, ie.item_id IS NOT NULL AS has_embedding
        FROM items
        LEFT JOIN item_embeddings ie ON ie.item_id = items.id AND model_id = ? AND model_version = ?
        WHERE source_id = ? AND external_id IN rarray(?)
    "##,
    )?;

    let compare_mtime = compare_strategy.should_compare_mtime();
    let mtime_is_sufficient = compare_strategy == ItemCompareStrategy::MTime;

    for batch in rx {
        let ids = batch
            .iter()
            .map(|item| rusqlite::types::Value::from(item.external_id.clone()))
            .collect::<Vec<_>>();

        let mut found = stmt
            .query_and_then(
                params![model_id, model_version, source_id, Rc::new(ids)],
                |row| {
                    Ok::<_, eyre::Report>((
                        row.get::<_, String>(0)?,
                        FoundItem {
                            id: row.get::<_, i64>(1)?,
                            hash: row.get::<_, String>(2)?,
                            modified: row.get::<_, Option<i64>>(3)?,
                            last_accessed: row.get::<_, Option<i64>>(4)?,
                            skipped: row
                                .get_ref(5)?
                                .as_str_or_null()?
                                .map(|s| s.parse::<SkipReason>())
                                .transpose()?,
                            content: if compare_strategy.should_compare_content() {
                                row.get::<_, String>(6)?
                            } else {
                                String::new()
                            },
                            has_embedding: row.get(7)?,
                        },
                    ))
                },
            )?
            .into_iter()
            .collect::<Result<HashMap<_, _>, _>>()?;

        for item in batch {
            let state = found
                .remove(&item.external_id)
                .map(|found| {
                    let same_time = item
                        .metadata
                        .mtime
                        .map(|t| t.unix_timestamp())
                        .zip(found.modified)
                        .map(|(a, b)| a == b)
                        .filter(|_| compare_mtime);

                    match (
                        compare_strategy == ItemCompareStrategy::Force || !found.has_embedding,
                        same_time,
                        mtime_is_sufficient,
                    ) {
                        // If there's no embedding or we're set to always recalculate, then always call it changed.
                        (true, _, _) => ScanItemState::Changed(found),
                        // mtime is different so no need to compare the content.
                        (false, Some(false), _) => ScanItemState::Changed(found),
                        // mtime is the same and we're not comparing the content
                        (false, Some(true), true) => ScanItemState::Unchanged { id: found.id },
                        // The mtime is the same but we stil have to compare the content.
                        (false, Some(true), false) => ScanItemState::Found(found),
                        // Inconclusive due to lack of mtime info, or we are configured to not compare it.
                        (false, None, _) => ScanItemState::Found(found),
                    }
                })
                .unwrap_or(ScanItemState::New);

            tx.send(ScanItem { state, item })?;
        }
    }

    Ok(())
}
