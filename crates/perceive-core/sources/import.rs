use std::{
    rc::Rc,
    sync::atomic::{AtomicU64, Ordering},
};

use ahash::HashMap;
use rusqlite::{named_params, params, types::Value};

use super::{ItemCompareStrategy, Source};
use crate::{
    batch_sender::BatchSender, db::Database, model::Model, search::serialize_embedding,
    time_tracker::TimeTracker, Item,
};

/// Data returned when the source scanner tries to read an item.
pub enum SourceScannerReadResult {
    /// Found the data.
    Found,
    /// Found the data, but the scanner determined that the data is unchanged from last time.
    /// The pipeline generally determines this, but the scanner may have additional methods,
    /// such as an HTTP request using caching headers to determine if the data is unchanged.
    Unchanged,
    /// Although the data showed up in the scanner, it failed to actually read it. For example,
    /// the scanner may be reading from the browser history but when it comes time to actually read
    /// the page, the server returns a 404 error.
    Skip,
}

pub trait SourceScanner: Send + Sync {
    /// Scan the sources and output batches of found items
    fn scan(&self, output: flume::Sender<Vec<Item>>) -> Result<(), eyre::Report>;
    /// Read the full content of a single item. If the function reads the file and determines that
    /// it should not be indexed, it should return SourceScannerReadResult and the original item.
    fn read(
        &self,
        existing: Option<&FoundItem>,
        item: Item,
    ) -> Result<(SourceScannerReadResult, Item), eyre::Report>;
}

pub struct FoundItem {
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

impl ScanItemState {
    fn item_id(&self) -> Option<i64> {
        match self {
            ScanItemState::New => None,
            ScanItemState::Unchanged { id } => Some(*id),
            ScanItemState::Found(item) => Some(item.id),
            ScanItemState::Changed(item) => Some(item.id),
        }
    }
}

struct ScanItem {
    state: ScanItemState,
    item: Item,
}

#[derive(Default)]
pub struct ScanStats {
    pub scanned: AtomicU64,
    pub encoded: AtomicU64,
    pub added: AtomicU64,
    pub changed: AtomicU64,
    pub unchanged: AtomicU64,

    pub scan_time: TimeTracker,
    pub read_time: TimeTracker,
    pub encode_time: TimeTracker,
    pub write_time: TimeTracker,
}

pub fn scan_source(
    times: &ScanStats,
    database: &Database,
    model: Model,
    model_id: i64,
    model_version: i64,
    source: &Source,
) -> Model {
    let scanner = source.create_scanner();

    #[allow(clippy::let_and_return)] // Much easier to read this way
    let returned_model = std::thread::scope(|scope| {
        let (item_tx, item_rx) = flume::bounded(8);
        let (matched_tx, matched_rx) = flume::bounded(8);
        let (with_content_tx, with_content_rx) = flume::bounded(8);
        let (with_embeddings_tx, with_embeddings_rx) = flume::bounded(8);

        // Make a pipeline with the following stages:
        // - Scan the file system and send out batches of items
        let scan_task = scope.spawn(|| {
            let _track = times.scan_time.begin();
            scanner.scan(item_tx)
        });

        // STAGE 2
        // - Match each item to a database entry.
        // - If one was found, do a preliminary match (i.e. by mtime) to see if it can be skipped.
        let db_lookup_task = scope.spawn(|| {
            match_to_existing_items(
                times,
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
        let read_task = scope.spawn(|| {
            read_items(
                &times.read_time,
                source.compare_strategy,
                scanner.as_ref(),
                matched_rx,
                with_content_tx,
            )
        });

        // STAGE 4
        // - Calculate embeddings for the items in this batch that we are keeping.
        let embed_task =
            scope.spawn(|| calculate_embeddings(times, model, with_content_rx, with_embeddings_tx));

        // STAGE 5
        // - Update the database for items that will be kept
        let write_db_task = scope.spawn(|| {
            update_db(
                model_id,
                model_version,
                times,
                database,
                source.index_version,
                with_embeddings_rx,
            )
        });

        // TODO - handle errors
        write_db_task.join().unwrap().unwrap();
        let returned_model = match embed_task.join().unwrap() {
            Ok(model) => model,
            Err((model, _)) => model,
        };
        read_task.join().unwrap().unwrap();
        db_lookup_task.join().unwrap().unwrap();
        scan_task.join().unwrap().unwrap();

        returned_model
    });

    // AFTER PIPELINE
    // - Delete any items for which the index_version did not get updated to
    //   the latest vesion.

    returned_model
}

fn match_to_existing_items(
    stats: &ScanStats,
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

    let mut sender = BatchSender::new(32, tx);
    for batch in rx {
        stats
            .scanned
            .fetch_add(batch.len() as u64, Ordering::Relaxed);
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
                    let same_time = item
                        .metadata
                        .mtime
                        .map(|t| t.unix_timestamp())
                        .zip(found.modified)
                        .map(|(a, b)| a == b)
                        .filter(|_| compare_mtime);

                    match (same_time, mtime_is_sufficient) {
                        // mtime is different so no need to compare the content.
                        (Some(false), _) => ScanItemState::Changed(found),
                        // mtime is the same and we're not comparing the content
                        (Some(true), true) => ScanItemState::Unchanged { id: found.id },
                        // The mtime is the same but we stil have to compare the content.
                        (Some(true), false) => ScanItemState::Found(found),
                        // Inconclusive due to lack of mtime info, or we are configured to not compare it.
                        (None, _) => ScanItemState::Found(found),
                    }
                })
                .unwrap_or(ScanItemState::New);

            sender.add(ScanItem { state, item })?;
        }
    }

    Ok(())
}

fn read_items(
    times: &TimeTracker,
    compare_strategy: ItemCompareStrategy,
    scanner: &dyn SourceScanner,
    rx: flume::Receiver<Vec<ScanItem>>,
    tx: flume::Sender<Vec<ScanItem>>,
) -> Result<(), eyre::Report> {
    let mut sender = BatchSender::new(64, tx);
    for batch in rx {
        let _track = times.begin();
        for item in batch {
            let ScanItem { item, state } = item;

            let existing = match &state {
                ScanItemState::New => None,
                ScanItemState::Found(found) => Some(found),
                ScanItemState::Changed(found) => Some(found),
                ScanItemState::Unchanged { .. } => {
                    sender.add(ScanItem { item, state })?;
                    continue;
                }
            };

            let (item, state) = match scanner.read(existing, item) {
                Ok((SourceScannerReadResult::Found, item)) => (item, state),
                Ok((SourceScannerReadResult::Unchanged, item)) => {
                    if let Some(id) = state.item_id() {
                        (item, ScanItemState::Unchanged { id })
                    } else {
                        // The scanner said the item was unchanged, but we also don't have an
                        // existing one. Just skip it.
                        continue;
                    }
                }
                Ok((SourceScannerReadResult::Skip, _)) => {
                    continue;
                }
                // TODO - handle errors
                Err(_) => continue,
            };

            let compare_content = compare_strategy.should_compare_content();
            let state = match state {
                ScanItemState::New => ScanItemState::New,
                ScanItemState::Found(found) => {
                    if compare_content
                        && found.content != item.content.as_deref().unwrap_or_default()
                    {
                        ScanItemState::Changed(found)
                    } else {
                        ScanItemState::Unchanged { id: found.id }
                    }
                }
                ScanItemState::Changed(found) => ScanItemState::Changed(found),
                ScanItemState::Unchanged { .. } => unreachable!(),
            };

            sender.add(ScanItem { state, item }).unwrap();
        }
    }

    Ok(())
}

fn calculate_embeddings(
    stats: &ScanStats,
    model: Model,
    rx: flume::Receiver<Vec<ScanItem>>,
    tx: flume::Sender<Vec<(ScanItem, Option<Vec<f32>>)>>,
) -> Result<Model, (Model, eyre::Report)> {
    let mut sender = BatchSender::new(2, tx);
    for batch in rx {
        let _track = stats.encode_time.begin();

        let contents = batch
            .iter()
            .filter(|item| matches!(item.state, ScanItemState::New | ScanItemState::Changed(_)))
            .filter_map(|item| item.item.content.as_deref())
            .collect::<Vec<_>>();

        // TODO Don't unwrap. What is the proper way to handle an error here? Probably just quit
        // the index since it indicates some larger problem.
        let mut embeddings: Vec<Vec<f32>> = if contents.is_empty() {
            // The model doesn't like it if you feed it nothing, so just skip it.
            Vec::new()
        } else {
            model.encode(&contents).unwrap().into()
        };

        stats
            .encoded
            .fetch_add(embeddings.len() as u64, Ordering::Relaxed);

        let mut embedding_index = 0;
        for item in batch {
            let embedding = match (&item.state, item.item.content.is_some()) {
                (ScanItemState::New | ScanItemState::Changed(_), true) => {
                    let embedding = std::mem::take(&mut embeddings[embedding_index]);
                    embedding_index += 1;
                    Some(embedding)
                }
                (ScanItemState::Unchanged { .. }, _) => None,
                (ScanItemState::Found(_), _) => unreachable!(),
                (_, false) => None,
            };

            if let Err(e) = sender.add((item, embedding)) {
                return Err((model, e.into()));
            }
        }
    }

    Ok(model)
}

fn update_db(
    model_id: i64,
    model_version: i64,
    stats: &ScanStats,
    database: &Database,
    index_version: i64,
    rx: flume::Receiver<Vec<(ScanItem, Option<Vec<f32>>)>>,
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
                UPDATE items SET version = ? WHERE id = ?
                "##,
            )?;

            let mut changed_stmt = tx.prepare_cached(
                r##"
                UPDATE items
                SET version=:version, hash=:hash, content=:content, name=:name, author=:author,
                    description=:description, modified=:modified, last_accessed=:last_accessed
                WHERE id=:id
                "##,
            )?;

            let mut new_stmt = tx.prepare_cached(
                r##"
                INSERT INTO items (source_id, external_id, version, hash, content, name, author,
                    description, modified, last_accessed)
                VALUES (:source_id, :external_id, :version, :hash, :content, :name, :author,
                    :description, :modified, :last_accessed);
                "##,
            )?;

            let mut embedding_update_stmt = tx.prepare_cached(
            r##"UPDATE item_embeddings
                    SET embedding = :embedding,
                    item_index_version = :version
                    WHERE item_id = :id AND model_id = :model_id AND model_version = :model_version"##,
        )?;

            let mut embedding_insert_stmt = tx.prepare_cached(
                r##" INSERT INTO item_embeddings
                    (item_id, item_index_version, embedding, model_id, model_version)
                    VALUES (:id, :version, :embedding, :model_id, :model_version) "##,
            )?;

            for (item, embedding) in &batch {
                match (&item.state, embedding) {
                    (ScanItemState::Unchanged { id }, _) => {
                        unchanged_stmt.execute(params![index_version, id])?;
                        unchanged += 1;
                    }
                    (ScanItemState::Changed(found), Some(embedding)) => {
                        changed_stmt.execute(named_params! {
                            ":id": found.id,
                            ":version": index_version,
                            ":hash": item.item.hash.as_deref().unwrap_or_default(),
                            ":content": item.item.content.as_deref().unwrap_or_default(),
                            ":name": item.item.metadata.name.as_deref(),
                            ":author": item.item.metadata.author.as_deref(),
                            ":description": item.item.metadata.description.as_deref(),
                            ":modified": item.item.metadata.mtime.map(|t| t.unix_timestamp()),
                            ":last_accessed": item.item.metadata.atime.map(|t| t.unix_timestamp()),
                        })?;

                        let bytes_vec = serialize_embedding(embedding);
                        embedding_update_stmt.execute(named_params! {
                            ":embedding": &bytes_vec,
                            ":version": index_version,
                            ":id": found.id,
                            ":model_id": model_id,
                            ":model_version": model_version,
                        })?;

                        changed += 1;
                    }
                    (ScanItemState::New, Some(embedding)) => {
                        new_stmt.execute(named_params! {
                            ":source_id": item.item.source_id,
                            ":external_id": item.item.external_id,
                            ":version": index_version,
                            ":hash": item.item.hash.as_deref().unwrap_or_default(),
                            ":content": item.item.content.as_deref().unwrap_or_default(),
                            ":name": item.item.metadata.name.as_deref(),
                            ":author": item.item.metadata.author.as_deref(),
                            ":description": item.item.metadata.description.as_deref(),
                            ":modified": item.item.metadata.mtime.map(|t| t.unix_timestamp()),
                            ":last_accessed": item.item.metadata.atime.map(|t| t.unix_timestamp()),
                        })?;

                        let row_id = tx.last_insert_rowid();

                        let mut bytes_vec =
                            Vec::with_capacity(embedding.len() * std::mem::size_of::<f32>());
                        for value in embedding {
                            bytes_vec.extend(value.to_le_bytes());
                        }

                        embedding_insert_stmt.execute(named_params! {
                            ":embedding": &bytes_vec,
                            ":version": index_version,
                            ":id": row_id,
                            ":model_id": model_id,
                            ":model_version": model_version,
                        })?;
                        new += 1;
                    }
                    (_, None) => unreachable!(),
                    (ScanItemState::Found(_), _) => unreachable!(),
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
