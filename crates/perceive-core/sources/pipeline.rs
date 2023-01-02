use std::sync::atomic::{AtomicU64, Ordering};

use smallvec::SmallVec;

use crate::{time_tracker::TimeTracker, Item, SkipReason};

mod calculate_embeddings;
mod import;
mod match_existing_items;
mod read_items;
mod update_db;

pub use import::scan_source;

use super::ItemCompareStrategy;

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

pub trait SourceScanner: Send + Sync {
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

pub struct ScanItem {
    state: ScanItemState,
    item: Item,
}

pub const EMBEDDING_BATCH_SIZE: usize = 64;
pub type EmbeddingsOutput = SmallVec<[(ScanItem, Option<Vec<f32>>); 1]>;

pub struct FoundItem {
    pub id: i64,
    pub hash: String,
    pub content: String,
    pub modified: Option<i64>,
    pub last_accessed: Option<i64>,
    pub skipped: Option<SkipReason>,
    pub has_embedding: bool,
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
    pub(crate) tx: flume::Sender<Vec<T>>,
    pub(crate) count: &'a AtomicU64,
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
