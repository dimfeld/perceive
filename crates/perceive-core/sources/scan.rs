use std::rc::Rc;

use ahash::HashMap;
use itertools::Itertools;
use rusqlite::{params, types::Value};

use super::{ItemCompareStrategy, Source};
use crate::{batch_sender::BatchSender, db::Database, Item};

pub trait SourceScanner: Send + Sync {
    /// Scan the sources and output batches of found items
    fn scan(&self, output: flume::Sender<Vec<Item>>) -> Result<(), eyre::Report>;
    /// Read the full content of a single item. If the function reads the file and determines that
    /// it should not be indexed, it should return None.
    fn read(&self, item: Item) -> Result<Option<Item>, eyre::Report>;
}

struct FoundItem {
    id: i64,
    hash: String,
    content: String,
    modified: Option<i64>,
}

enum ScanItemState {
    New,
    Unchanged { id: i64 },
    Found(FoundItem),
    Changed(FoundItem),
}

struct ScanItem {
    state: ScanItemState,
    item: Item,
}

pub fn scan_source(database: &Database, source: &Source) {
    let scanner = source.create_scanner();

    std::thread::scope(|scope| {
        let (item_tx, item_rx) = flume::bounded(8);
        let (matched_tx, matched_rx) = flume::bounded(8);
        let (with_content_tx, with_content_rx) = flume::bounded(8);
        let (with_embeddings_tx, with_embeddings_rx) = flume::bounded(8);

        // Make a pipeline with the following stages:
        // - Scan the file system and send out batches of items
        let scan_task = scope.spawn(|| scanner.scan(item_tx));

        // STAGE 2
        // - Match each item to a database entry.
        // - If one was found, do a preliminary match (i.e. by mtime) to see if it can be skipped.
        let db_lookup_task = scope.spawn(|| {
            match_to_existing_items(
                database,
                source.id,
                source.compare_strategy,
                item_rx,
                matched_tx,
            )
        });

        // STAGE 3
        // - Read the content of the file
        // - Match against the content, if applicable
        // - If the content did not change, skip the item.
        let read_task =
            scope.spawn(|| read_items(source.compare_strategy, matched_rx, with_content_tx));

        // STAGE 4
        // - Calculate embeddings for the items in this batch that we are keeping.
        let embed_task = scope.spawn(|| calculate_embeddings(with_content_rx, with_embeddings_tx));

        // STAGE 5
        // - Update the database for items that will be kept
        let write_db_task = scope.spawn(|| update_db(with_embeddings_rx));

        // TODO - handle errors
        scan_task.join().unwrap();
        db_lookup_task.join().unwrap();
        read_task.join().unwrap();
        embed_task.join().unwrap();
        write_db_task.join().unwrap();
    });

    // AFTER PIPELINE
    // - Delete any items for which the index_version did not get updated to
    //   the latest vesion.
}

struct MatchedItem {
    item: Item,
    existing: Option<Item>,
}

fn match_to_existing_items(
    db: &Database,
    source_id: i64,
    compare_strategy: ItemCompareStrategy,
    rx: flume::Receiver<Vec<Item>>,
    tx: flume::Sender<Vec<ScanItem>>,
) -> Result<(), eyre::Report> {
    let conn = db.read_pool.get()?;

    let mut stmt = conn.prepare_cached(
        r##"
        SELECT external_id, id, hash, modified, content
        FROM items
        WHERE source_id = ? AND external_id IN rarray(?)
    "##,
    )?;

    let compare_mtime = compare_strategy.should_compare_mtime();
    let mtime_is_sufficient = compare_strategy == ItemCompareStrategy::MTime;

    let mut sender = BatchSender::new(64, tx);
    for batch in rx {
        let ids = batch
            .iter()
            .map(|item| Value::from(item.external_id.clone()))
            .collect::<Vec<_>>();

        let mut found = stmt
            .query_map(params![source_id, Rc::new(ids)], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    FoundItem {
                        id: row.get::<_, i64>(1)?,
                        hash: row.get::<_, String>(2)?,
                        modified: row.get::<_, Option<i64>>(3)?,
                        content: if compare_strategy.should_compare_content() {
                            row.get::<_, String>(4)?
                        } else {
                            String::new()
                        },
                    },
                ))
            })?
            .into_iter()
            .collect::<Result<HashMap<_, _>, _>>()?;

        for item in batch {
            let state = found
                .remove(&item.external_id)
                .map(|found| {
                    match (
                        compare_mtime,
                        found.modified,
                        item.metadata.mtime.map(|t| t.unix_timestamp()),
                        mtime_is_sufficient,
                    ) {
                        (false, _, _, _) => ScanItemState::Found(found),
                        // mtime is different so no need to compare the content.
                        (true, Some(a), Some(b), _) if a != b => ScanItemState::Changed(found),
                        // mtime is the same and we're not comparing the content
                        (true, Some(a), Some(b), true) if a == b => {
                            ScanItemState::Unchanged { id: found.id }
                        }
                        // The mtime is the same but we stil have to compare the content.
                        (true, Some(a), Some(b), false) if a == b => ScanItemState::Found(found),
                        // Inconclusive due to lack of mtime info
                        (true, _, _, _) => ScanItemState::Found(found),
                    }
                })
                .unwrap_or(ScanItemState::New);

            sender.add(ScanItem { state, item })?;
        }
    }

    Ok(())
}

fn read_items(
    compare_strategy: ItemCompareStrategy,
    rx: flume::Receiver<Vec<ScanItem>>,
    tx: flume::Sender<Vec<ScanItem>>,
) {
    let sender = BatchSender::new(64, tx);
    for batch in rx {
        for item in batch {}
    }
}

struct ItemWithEmbeddings {
    item: Item,
    embeddings: Vec<f32>,
}

fn calculate_embeddings(
    rx: flume::Receiver<Vec<ScanItem>>,
    tx: flume::Sender<Vec<(ScanItem, Vec<f32>)>>,
) {
    let sender = BatchSender::new(64, tx);
    for batch in rx {
        for item in batch {}
    }
}

fn update_db(rx: flume::Receiver<Vec<(ScanItem, Vec<f32>)>>) {
    for batch in rx {}
}
