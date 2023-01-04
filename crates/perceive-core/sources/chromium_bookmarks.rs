use std::path::Path;

use ahash::HashMap;
use eyre::Result;
use reqwest::{blocking::Client, Url};
use serde::{Deserialize, Serialize};

use super::{
    parse_html::{fetch_html, reprocess_html_article, should_skip, HTML_PROCESS_VERSION},
    pipeline::{self, FoundItem, SourceScanner, SourceScannerReadResult},
    ItemCompareStrategy,
};
use crate::{batch_sender::BatchSender, Item, ItemMetadata};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChromiumBookmarksConfig {
    /// Domains that we should never check
    pub skip: Vec<String>,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum BookmarkEntry {
    Url {
        id: String,
        url: String,
        name: String,
        date_added: String,
        date_last_used: String,
        date_modified: Option<String>,
    },
    Folder {
        id: String,
        name: String,
        children: Vec<BookmarkEntry>,
    },
}

#[derive(Serialize, Deserialize, Debug)]
pub struct BookmarksFile {
    pub roots: HashMap<String, BookmarkEntry>,
}

pub struct ChromiumBookmarksScanner {
    pub source_id: i64,
    pub location: String,
    pub config: ChromiumBookmarksConfig,
    pub client: Client,
}

impl ChromiumBookmarksScanner {
    pub fn new(source_id: i64, location: String, config: ChromiumBookmarksConfig) -> Result<Self> {
        Ok(Self {
            source_id,
            location,
            config,
            client: Client::builder()
                .user_agent("perceive-search")
                .gzip(true)
                .timeout(std::time::Duration::from_secs(30))
                .build()?,
        })
    }

    fn walk_bookmarks(&self, sender: &BatchSender<Item>, entry: &BookmarkEntry) -> Result<()> {
        match entry {
            BookmarkEntry::Url {
                url,
                name,
                date_last_used,
                date_added,
                ..
            } => {
                let chosen_time = if date_last_used == "0" {
                    date_added
                } else {
                    date_last_used
                };

                let atime = chosen_time
                    .parse::<i64>()
                    .ok()
                    .and_then(|t| time::OffsetDateTime::from_unix_timestamp(t).ok());

                let Ok(parsed) = Url::parse(url) else {
                    // Don't exit on a failure to parse the URL, but also don't pass it on.
                    return Ok(());
                };

                if should_skip(&self.config.skip, &parsed) {
                    return Ok(());
                }

                let item = Item {
                    id: -1,
                    external_id: url.clone(),
                    source_id: self.source_id,
                    content: None,
                    raw_content: None,
                    hash: None,
                    process_version: HTML_PROCESS_VERSION,
                    skipped: None,
                    metadata: ItemMetadata {
                        name: Some(name.clone()),
                        atime,
                        ..Default::default()
                    },
                };

                sender.add(item)?;
            }
            BookmarkEntry::Folder { children, .. } => {
                for child in children {
                    self.walk_bookmarks(sender, child)?;
                }
            }
        }

        Ok(())
    }
}

impl SourceScanner for ChromiumBookmarksScanner {
    fn scan(&self, output: pipeline::CountingVecSender<crate::Item>) -> Result<(), eyre::Report> {
        let sender = BatchSender::new(64, output);
        let file_path = Path::new(&self.location).join("Bookmarks");
        let bookmarks: BookmarksFile = serde_json::from_reader(std::fs::File::open(file_path)?)?;

        for root in bookmarks.roots.values() {
            self.walk_bookmarks(&sender, root)?;
        }

        Ok(())
    }

    fn read(
        &self,
        existing: Option<&FoundItem>,
        compare_strategy: super::ItemCompareStrategy,
        item: &mut crate::Item,
    ) -> Result<SourceScannerReadResult, eyre::Report> {
        if compare_strategy != ItemCompareStrategy::Force {
            if let Some(skipped) = existing.and_then(|e| e.skipped) {
                if skipped.permanent() {
                    // We skipped it last time, so continue skipping it.
                    item.skipped = Some(skipped);
                    return Ok(SourceScannerReadResult::Unchanged);
                }
            }

            let existing_atime = existing.and_then(|e| e.last_accessed);
            let new_atime = item.metadata.atime.map(|a| a.unix_timestamp());

            let newer_access = new_atime
                .zip(existing_atime)
                .map(|(n, e)| n > e)
                .unwrap_or(true);
            if !newer_access {
                return Ok(SourceScannerReadResult::Unchanged);
            }
        }

        fetch_html(&self.client, existing, item)
    }

    fn latest_process_version(&self) -> i32 {
        HTML_PROCESS_VERSION
    }

    fn reprocess(&self, item: &mut crate::Item) -> Result<SourceScannerReadResult, eyre::Report> {
        reprocess_html_article(item)
    }
}
