mod chromium_bookmarks;
#[cfg(feature = "browser-history")]
mod chromium_history;
pub mod db;
mod fs;
pub mod parse_html;
pub mod pipeline;

pub use fs::FsSourceConfig;
pub use pipeline::scan_source;
use serde::{Deserialize, Serialize};
use strum::{Display, EnumString};
use time::OffsetDateTime;

use self::pipeline::SourceScanner;
#[cfg(feature = "browser-history")]
pub use self::{
    chromium_bookmarks::ChromiumBookmarksConfig, chromium_history::ChromiumHistoryConfig,
};

#[derive(Debug, Copy, Clone, strum::EnumString)]
#[cfg_attr(feature = "cli", derive(clap::ValueEnum))]
#[strum(serialize_all = "snake_case")]
pub enum SourceTypeTag {
    /// Local sources
    Local,
    /// Sources that fetch data from the web
    Web,
    /// Sources that hold bookmarks
    Bookmarks,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum SourceConfig {
    Fs(FsSourceConfig),
    #[cfg(feature = "browser-history")]
    ChromiumHistory(ChromiumHistoryConfig),
    #[cfg(feature = "browser-history")]
    ChromiumBookmarks(ChromiumBookmarksConfig),
}

impl SourceConfig {
    pub fn matches_tag(&self, tag: SourceTypeTag) -> bool {
        match (self, tag) {
            (Self::Fs(_), SourceTypeTag::Local) => true,
            #[cfg(feature = "browser-history")]
            (Self::ChromiumHistory(_), SourceTypeTag::Web) => true,
            #[cfg(feature = "browser-history")]
            (Self::ChromiumBookmarks(_), SourceTypeTag::Web | SourceTypeTag::Bookmarks) => true,
            _ => false,
        }
    }
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
            #[cfg(feature = "browser-history")]
            SourceConfig::ChromiumBookmarks(config) => {
                Box::new(chromium_bookmarks::ChromiumBookmarksScanner::new(
                    self.id,
                    self.location.clone(),
                    config.clone(),
                )?)
            }
        };

        Ok(scanner)
    }
}
