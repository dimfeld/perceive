#[cfg(feature = "browser-history")]
pub mod chromium_history;
pub mod db;
mod fs;
pub mod import;

pub use fs::FsSourceConfig;
use serde::{Deserialize, Serialize};
use strum::{Display, EnumString};
use time::OffsetDateTime;

#[cfg(feature = "browser-history")]
pub use self::chromium_history::ChromiumHistoryConfig;
use self::import::SourceScanner;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum SourceConfig {
    Fs(FsSourceConfig),
    #[cfg(feature = "browser-history")]
    ChromiumHistory(ChromiumHistoryConfig),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum SourceStatus {
    Indexing { started_at: i64 },
    Ready { scanned: u32, duration: u32 },
    Error { error: String },
}

#[derive(
    Debug, Default, Display, EnumString, Copy, Clone, Serialize, Deserialize, PartialEq, Eq,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum ItemCompareStrategy {
    /// Compare the modified time and the content of the item.
    #[default]
    MTimeAndContent,
    /// Compare only the modified time of the item
    MTime,
    /// Compare the content of the item
    Content,
    /// Always consider the item changed
    Force,
}

impl ItemCompareStrategy {
    pub fn should_compare_mtime(&self) -> bool {
        match self {
            ItemCompareStrategy::MTimeAndContent | ItemCompareStrategy::MTime => true,
            ItemCompareStrategy::Content | ItemCompareStrategy::Force => false,
        }
    }

    pub fn should_compare_content(&self) -> bool {
        match self {
            ItemCompareStrategy::MTimeAndContent | ItemCompareStrategy::Content => true,
            ItemCompareStrategy::MTime | ItemCompareStrategy::Force => false,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Source {
    pub id: i64,
    pub name: String,
    pub config: SourceConfig,
    pub location: String,
    pub compare_strategy: ItemCompareStrategy,
    pub status: SourceStatus,
    pub last_indexed: OffsetDateTime,
    pub index_version: i64,
}

impl Source {
    fn create_scanner(&self) -> Result<Box<dyn SourceScanner>, eyre::Report> {
        let scanner: Box<dyn SourceScanner> = match &self.config {
            SourceConfig::Fs(config) => Box::new(fs::FileScanner {
                source_id: self.id,
                location: self.location.clone(),
                config: config.clone(),
            }),
            #[cfg(feature = "browser-history")]
            SourceConfig::ChromiumHistory(config) => {
                Box::new(chromium_history::ChromiumHistoryScanner::new(
                    self.id,
                    self.location.clone(),
                    config.clone(),
                )?)
            }
        };

        Ok(scanner)
    }
}
