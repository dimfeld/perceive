use std::{borrow::Cow, path::Path};

use ahash::HashMap;
use eyre::{eyre, Context};
use itertools::Itertools;
use reqwest::{blocking::Client, Url};
use serde::{Deserialize, Serialize};
use time::macros::datetime;

use super::{
    parse_html::{fetch_html, reprocess_html_article, should_skip, HTML_PROCESS_VERSION},
    pipeline::{CountingVecSender, FoundItem, SourceScanner, SourceScannerReadResult},
    ItemCompareStrategy,
};
use crate::Item;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChromiumHistoryConfig {
    /// Domains that we should never check
    pub skip: Vec<String>,
}

pub struct ChromiumHistoryScanner {
    pub source_id: i64,
    pub location: String,
    pub config: ChromiumHistoryConfig,
    pub client: Client,
}

impl ChromiumHistoryScanner {
    pub fn new(
        source_id: i64,
        location: String,
        config: ChromiumHistoryConfig,
    ) -> Result<ChromiumHistoryScanner, eyre::Report> {
        Ok(ChromiumHistoryScanner {
            source_id,
            location,
            config,
            client: Client::builder()
                .user_agent("perceive-search")
                .gzip(true)
                .timeout(std::time::Duration::from_secs(30))
                .redirect(reqwest::redirect::Policy::none())
                .build()?,
        })
    }
}

impl SourceScanner for ChromiumHistoryScanner {
    fn scan(&self, tx: CountingVecSender<Item>) -> Result<(), eyre::Report> {
        // Some browsers lock the SQLite history database, so we copy it to be safe.
        let dir = tempfile::tempdir()?;
        let db_path = dir.path().join("History");
        let from_db_path = Path::new(&self.location).join("History");
        std::fs::copy(from_db_path, &db_path)
            .wrap_err_with(|| eyre!("Copying {} to temporary location", self.location))?;

        let conn = rusqlite::Connection::open_with_flags(
            db_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?;
        let mut stmt = conn.prepare(
            r##"SELECT url, MAX(title), MAX(last_visit_time) FROM urls
             -- Skip things like "chrome-extension://"
             WHERE url LIKE 'http%'
             GROUP BY url"##,
        )?;

        let rows = stmt.query_map([], |row| {
            let url: String = row.get(0)?;
            let title: String = row.get(1)?;
            let last_visit_time: i64 = row.get(2)?;

            // Time in in microseconds since 1601/01/01.
            let last_visit_time =
                datetime!(1601 - 01 - 01 0:00 UTC) + time::Duration::microseconds(last_visit_time);

            Ok((url, title, last_visit_time))
        })?;

        let mut output = HashMap::default();

        for row in rows {
            let (mut url_str, title, last_visit_time) = row?;

            // TODO Read cookies as well? This feels intrusive but could help a lot for sources
            // that need login. Probably make it an option

            let Ok(mut url) = Url::parse(&url_str) else {
                    // If it doesn't parse here then there's no point in continuing.
                continue;
            } ;

            // The history contains lots of entries with the same URL but different fragment.
            // Clear it out so that we can dedupe.
            if url.scheme() != "https" || url.fragment().is_some() {
                url.set_scheme("https").ok();
                url.set_fragment(None);
                url_str = url.to_string();
            }

            if should_skip(&self.config.skip, &url) {
                continue;
            }

            let dedupe_path = url.path().trim_end_matches('/');
            let dedupe_key = if dedupe_path != url.path() {
                // Remove trailing slash, but only from the dedupe key, since removing it from
                // the URL that we actually request may cause problems with some sites.
                let mut dedupe_url = url.clone();
                dedupe_url.set_path(dedupe_path);
                Cow::Owned(dedupe_url.to_string())
            } else {
                Cow::Borrowed(&url_str)
            };

            if output.contains_key(dedupe_key.as_ref()) {
                continue;
            }

            output.insert(dedupe_key.into_owned(), (url_str, title, last_visit_time));
        }

        // Rely on the HashMap iteration order being random-ish, since we want to shuffle
        // the URLs as a simple way to avoid hitting the same domain in quick succession.
        for batch in &output.into_values().chunks(64) {
            let batch = batch
                .into_iter()
                .map(|(url_str, title, last_visit_time)| Item {
                    id: -1,
                    source_id: self.source_id,
                    external_id: url_str,
                    hash: None,
                    skipped: None,
                    metadata: crate::ItemMetadata {
                        name: Some(title),
                        atime: Some(last_visit_time),
                        ..Default::default()
                    },
                    content: None,
                    raw_content: None,
                    process_version: HTML_PROCESS_VERSION,
                })
                .collect::<Vec<_>>();

            tx.send(batch)?;
        }

        Ok(())
    }

    fn read(
        &self,
        existing: Option<&FoundItem>,
        compare_strategy: ItemCompareStrategy,
        item: &mut Item,
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

    fn reprocess(&self, item: &mut Item) -> Result<SourceScannerReadResult, eyre::Report> {
        // if item.process_version == PROCESS_VERSION {
        //     return Ok(SourceScannerReadResult::Unchanged);
        // }

        reprocess_html_article(item)
    }
}
