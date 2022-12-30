use std::{
    fmt::Display,
    rc::Rc,
    sync::atomic::{AtomicU64, Ordering},
};

use ahash::HashMap;
use itertools::Itertools;
use rusqlite::{named_params, params, types::Value};
use smallvec::{smallvec, SmallVec};

use super::{ItemCompareStrategy, Source};
use crate::{
    db::Database, model::Model, search::serialize_embedding, time_tracker::TimeTracker, Item,
    SkipReason,
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
    Omit,
}

pub(super) trait SourceScanner: Send + Sync {
    /// Scan the sources and output batches of found items
    fn scan(&self, output: CountingVecSender<Item>) -> Result<(), eyre::Report>;
    /// Read the full content of a single item. If the function reads the file and determines that
    /// it should not be indexed, it should return SourceScannerReadResult and the original item.
    fn read(
        &self,
        existing: Option<&FoundItem>,
        compare_strategy: ItemCompareStrategy,
        item: &mut Item,
    ) -> Result<SourceScannerReadResult, eyre::Report>;

    /// For pipelines that do some postprocessing on the data, reprocess that data.
    #[allow(unused_variables)]
    fn reprocess(&self, item: &mut Item) -> Result<SourceScannerReadResult, eyre::Report> {
        Ok(SourceScannerReadResult::Unchanged)
    }
}

pub(super) struct FoundItem {
    pub id: i64,
    pub hash: String,
    pub content: String,
    pub modified: Option<i64>,
    pub last_accessed: Option<i64>,
    pub skipped: Option<SkipReason>,
    pub has_embedding: bool,
}

enum ScanItemState {
    /// We're seeing this item for the first time.
    New,
    /// This item has not changed since the last index.
    Unchanged { id: i64 },
    /// The item exists, and we haven't determined yet if it changed or not.
    Found(FoundItem),
    /// The item exists, and it has changed since last time.
    /// Effectively this means that it will be reencoded.
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
    pub fetched: AtomicU64,
    pub added: AtomicU64,
    pub changed: AtomicU64,
    pub unchanged: AtomicU64,

    pub reading: AtomicU64,
    pub embedding: AtomicU64,

    pub scan_time: TimeTracker,
    pub read_time: TimeTracker,
    pub encode_time: TimeTracker,
    pub write_time: TimeTracker,
}

pub struct CountingVecSender<'a, T> {
    tx: flume::Sender<Vec<T>>,
    count: &'a AtomicU64,
}

impl<'a, T> Clone for CountingVecSender<'a, T> {
    fn clone(&self) -> Self {
        Self {
            tx: self.tx.clone(),
            count: self.count,
        }
    }
}

impl<'a, T> CountingVecSender<'a, T> {
    pub fn send(&self, batch: Vec<T>) -> Result<(), flume::SendError<Vec<T>>> {
        self.count.fetch_add(batch.len() as u64, Ordering::Relaxed);
        self.tx.send(batch)
    }
}

const EMBEDDING_BATCH_SIZE: usize = 64;

pub fn scan_source(
    times: &ScanStats,
    database: &Database,
    model: Model,
    model_id: u32,
    model_version: u32,
    source: &Source,
    override_compare_strategy: Option<ItemCompareStrategy>,
) -> Result<Model, (Model, eyre::Report)> {
    let scanner = match source.create_scanner() {
        Ok(scanner) => scanner,
        Err(e) => return Err((model, e)),
    };
    let compare_strategy = override_compare_strategy.unwrap_or(source.compare_strategy);

    #[allow(clippy::let_and_return)] // Much easier to read this way
    let returned_model = std::thread::scope(|scope| {
        let (item_tx, item_rx) = flume::unbounded();
        let (matched_tx, matched_rx) = flume::bounded(256);
        let (with_content_tx, with_content_rx) = flume::bounded(EMBEDDING_BATCH_SIZE);
        let (with_embeddings_tx, with_embeddings_rx) = flume::bounded(8);

        let item_tx = CountingVecSender {
            tx: item_tx,
            count: &times.scanned,
        };

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
                model_id,
                model_version,
                source.id,
                compare_strategy,
                item_rx,
                matched_tx,
            )
        });

        // STAGE 3
        // - Read the content of the file
        // - Match against the content, if applicable
        // - If the content did not change, skip the item.
        const READ_PARALLELISM: usize = 8;
        let read_tasks = itertools::repeat_n((matched_rx, with_content_tx), READ_PARALLELISM)
            .into_iter()
            .map(|(matched_rx, with_content_tx)| {
                scope.spawn(|| {
                    read_items(
                        times,
                        compare_strategy,
                        scanner.as_ref(),
                        matched_rx,
                        with_content_tx,
                    )
                })
            })
            .collect::<Vec<_>>();

        // STAGE 4
        // - Calculate embeddings for the items in this batch that we are keeping.
        let embed_task = scope.spawn(move || {
            let result = calculate_embeddings(times, &model, with_content_rx, with_embeddings_tx);
            match result {
                Ok(()) => Ok(model),
                Err(e) => Err((model, e)),
            }
        });

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

        fn log_thread_error<T, E: Display>(
            name: &str,
            r: std::thread::Result<Result<T, E>>,
        ) -> bool {
            match r {
                Err(e) => {
                    eprintln!("Thread {name} panicked: {:?}", e);
                    true
                }
                Ok(Err(e)) => {
                    eprintln!("Thread {name} failed: {}", e);
                    true
                }
                Ok(Ok(_)) => false,
            }
        }

        // TODO - handle errors better
        let mut errored = false;
        errored = errored || log_thread_error("db_writer", write_db_task.join());
        let returned_model = match embed_task.join().unwrap() {
            Ok(model) => model,
            Err((model, e)) => {
                errored = true;
                eprintln!("Encoding thread error: {e}");
                model
            }
        };

        for read_task in read_tasks {
            errored = errored || log_thread_error("read_items", read_task.join());
        }
        errored = errored || log_thread_error("db_lookup", db_lookup_task.join());
        errored = errored || log_thread_error("scanner", scan_task.join());

        if errored {
            println!("Scanning failed");
        }

        returned_model
    });

    // AFTER PIPELINE TODO
    // - For source types that require it, delete any items for which the
    //   index_version did not get updated to the latest vesion. This means
    //   that they were not found in the scan.

    Ok(returned_model)
}

fn match_to_existing_items(
    stats: &ScanStats,
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
            .map(|item| Value::from(item.external_id.clone()))
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

fn read_items(
    stats: &ScanStats,
    compare_strategy: ItemCompareStrategy,
    scanner: &dyn SourceScanner,
    rx: flume::Receiver<ScanItem>,
    tx: flume::Sender<ScanItem>,
) -> Result<(), eyre::Report> {
    for item in rx {
        let _track = stats.read_time.begin();

        let ScanItem { mut item, state } = item;

        let existing = match &state {
            ScanItemState::New => None,
            ScanItemState::Found(found) => Some(found),
            ScanItemState::Changed(found) => Some(found),
            ScanItemState::Unchanged { .. } => {
                tx.send(ScanItem { item, state })?;
                continue;
            }
        };

        let external_id = item.external_id.clone();

        stats.reading.fetch_add(1, Ordering::Relaxed);
        let read_result = scanner.read(existing, compare_strategy, &mut item);
        stats.reading.fetch_sub(1, Ordering::Relaxed);
        stats.fetched.fetch_add(1, Ordering::Relaxed);

        let state = match read_result {
            Ok(SourceScannerReadResult::Found) => state,
            Ok(SourceScannerReadResult::Unchanged) => {
                if let Some(id) = state.item_id() {
                    ScanItemState::Unchanged { id }
                } else {
                    // The scanner said the item was unchanged, but we also don't have an
                    // existing one. Just skip it.
                    continue;
                }
            }
            Ok(SourceScannerReadResult::Omit) => {
                continue;
            }
            Err(e) => {
                // TODO - better error handling
                eprintln!("{}: {e}", external_id);
                continue;
            }
        };

        let compare_content = item.skipped.is_none() && compare_strategy.should_compare_content();
        let state = match state {
            ScanItemState::New | ScanItemState::Unchanged { .. } | ScanItemState::Changed(_) => {
                state
            }
            ScanItemState::Found(found) => {
                if compare_content && found.content != item.content.as_deref().unwrap_or_default() {
                    ScanItemState::Changed(found)
                } else {
                    ScanItemState::Unchanged { id: found.id }
                }
            }
        };

        tx.send(ScanItem { state, item })?;
    }

    Ok(())
}

type EmbeddingsOutput = SmallVec<[(ScanItem, Option<Vec<f32>>); 1]>;

fn calculate_embeddings_batch(
    stats: &ScanStats,
    model: &Model,
    batch: &mut Vec<ScanItem>,
    documents: &Vec<String>,
) -> Result<EmbeddingsOutput, eyre::Report> {
    let _track = stats.encode_time.begin();

    stats
        .embedding
        .fetch_add(documents.len() as u64, Ordering::Relaxed);

    // TODO Don't unwrap. What is the proper way to handle an error here? Probably just quit
    // the index job since it indicates some larger problem, but we also might need
    // to reset the model or something (though it seems that Torch just panics
    // in those cases).
    let embeddings: Vec<Vec<f32>> = model.encode(documents).unwrap().into();

    stats
        .embedding
        .fetch_sub(documents.len() as u64, Ordering::Relaxed);

    stats
        .encoded
        .fetch_add(embeddings.len() as u64, Ordering::Relaxed);

    let output = batch
        .drain(..)
        .zip(embeddings.into_iter().map(Some))
        .collect::<SmallVec<_>>();

    Ok(output)
}

fn calculate_embeddings(
    stats: &ScanStats,
    model: &Model,
    rx: flume::Receiver<ScanItem>,
    tx: flume::Sender<EmbeddingsOutput>,
) -> Result<(), eyre::Report> {
    let mut batch = Vec::with_capacity(EMBEDDING_BATCH_SIZE);
    let mut documents = Vec::with_capacity(EMBEDDING_BATCH_SIZE);

    for item in rx {
        if matches!(
            item.state,
            ScanItemState::Unchanged { .. } | ScanItemState::Found(_)
        ) || item.item.skipped.is_some()
        {
            tx.send(smallvec![(item, None)])?;
            continue;
        }

        // We could `std::mem::replace` the batch vector with a new one, but instead
        // we `drain` at the end of the operations.

        let document =
            if item.item.metadata.name.is_none() && item.item.metadata.description.is_none() {
                item.item.content.as_deref().map(|s| s.trim().to_string())
            } else {
                let document = [
                    item.item.metadata.name.as_deref(),
                    item.item.metadata.description.as_deref(),
                    item.item.content.as_deref(),
                ]
                .into_iter()
                .flatten()
                .filter(|s| !s.trim().is_empty())
                .join("\n");

                if document.is_empty() {
                    None
                } else {
                    Some(document)
                }
            };

        let Some(document) = document else {
            tx.send(smallvec![(item, None)])?;
            continue;
        };

        documents.push(document);
        batch.push(item);
        if batch.len() < EMBEDDING_BATCH_SIZE {
            continue;
        }

        let output = calculate_embeddings_batch(stats, model, &mut batch, &documents)?;
        documents.clear();

        tx.send(output)?;
    }

    if !batch.is_empty() {
        let output = calculate_embeddings_batch(stats, model, &mut batch, &documents)?;
        tx.send(output)?;
    }

    Ok(())
}

fn update_db(
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
                let item_id = match &item.state {
                    ScanItemState::Unchanged { id } => {
                        unchanged_stmt.execute(params![
                            index_version,
                            item.item.metadata.atime.map(|t| t.unix_timestamp()),
                            id
                        ])?;
                        unchanged += 1;
                        *id
                    }
                    ScanItemState::Changed(found) => {
                        changed_stmt.execute(named_params! {
                            ":id": found.id,
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
                        found.id
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
                    ScanItemState::Found(_) => unreachable!(),
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
