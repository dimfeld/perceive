mod fs;
mod scan;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use self::{fs::FsSourceConfig, scan::SourceScanner};
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

#[derive(Debug, Default, Copy, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
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
    pub compare_strategy: ItemCompareStrategy,
    pub status: SourceStatus,
    pub last_indexed: OffsetDateTime,
    pub index_version: i64,
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
