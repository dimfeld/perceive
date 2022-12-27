mod fs;

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use self::fs::FsSourceConfig;
use crate::Item;

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum SourceConfig {
    Fs(FsSourceConfig),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum SourceStatus {
    Indexing { started_at: i64 },
    Ready { num_indexed: u64 },
    Error { error: String },
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub enum ItemCompareStrategy {
    #[default]
    MTimeAndContent,
    MTime,
    Content,
}

impl ItemCompareStrategy {
    pub fn should_compare_mtime(&self) -> bool {
        match self {
            ItemCompareStrategy::MTimeAndContent | ItemCompareStrategy::MTime => true,
            ItemCompareStrategy::Content => false,
        }
    }

    pub fn should_compare_content(&self) -> bool {
        match self {
            ItemCompareStrategy::MTimeAndContent | ItemCompareStrategy::Content => true,
            ItemCompareStrategy::MTime => false,
        }
    }
}

#[derive(Debug)]
pub struct Source {
    pub id: i64,
    pub name: String,
    pub config: SourceConfig,
    pub location: String,
    pub last_indexed: OffsetDateTime,
    pub preferred_model: i64,
}

impl Source {
    fn create_scanner(&self) -> Box<dyn SourceScanner> {
        match &self.config {
            SourceConfig::Fs(config) => Box::new(fs::FileScanner {
                source_id: self.id,
                location: self.location.clone(),
                config: config.clone(),
            }),
        }
    }
}

pub trait SourceScanner: Send + Sync {
    /// Scan the sources and output batches of found items
    fn scan(&self, output: flume::Sender<Vec<Item>>) -> Result<(), eyre::Report>;
    /// Read the full content of a single item. If the function reads the file and determines that
    /// it should not be indexed, it should return None.
    fn read(&self, item: Item) -> Result<Option<Item>, eyre::Report>;
}

pub fn scan_source(source: &Source) {
    let scanner = source.create_scanner();

    std::thread::scope(|scope| {
        let (item_sender, item_receiver) = flume::bounded(8);

        // Make a pipeline with the following stages:
        // - Scan the file system and send out batches of items
        let scan_task = scope.spawn(|| scanner.scan(item_sender));

        // STAGE 2
        // - Match each item to a database entry.
        // - If one was found, do a preliminary match (i.e. by mtime) to see if it can be skipped.

        // STAGE 3
        // - Read the content of the file
        // - Match against the content, if applicable
        // - If the content did not change, skip the item.

        // STAGE 4
        // - Pool the entries into larger batches here since in the general case a lot of
        //   them will have been skipped at this point

        // STAGE 5
        // - Calculate embeddings for the items in this batch that we are keeping.
        // - Update the database for items that will be kept

        // AFTER PIPELINE
        // - Delete any items for which the index_version did not get updated to
        //   the latest vesion.
    });
}
