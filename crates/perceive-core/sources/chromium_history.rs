use std::{borrow::Cow, io::Cursor, path::Path, time::SystemTime};

use ahash::HashMap;
use eyre::{eyre, Context};
use http::{HeaderValue, StatusCode};
use itertools::Itertools;
use reqwest::{blocking::Client, Url};
use serde::{Deserialize, Serialize};
use time::{macros::datetime, OffsetDateTime};

use super::{
    import::{CountingVecSender, SourceScanner, SourceScannerReadResult},
    ItemCompareStrategy,
};
use crate::{Item, SkipReason};

const ALWAYS_SKIP: [&str; 5] = [
    // Signin pages. These show up a lot but never contain searchable content.
    "accounts.google.com",
    "ad.doubleclick.net",
    "console.cloud.google.com",
    "console.aws.amazon.com",
    "googleapis.com",
];

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
                .build()?,
        })
    }

    fn extract_html_article(
        url: &Url,
        raw_content: &[u8],
    ) -> Result<readability::extractor::Product, eyre::Report> {
        let mut cursor = Cursor::new(raw_content);
        readability::extractor::extract(&mut cursor, url).map_err(|e| e.into())
    }
}

/// Increment this when the postprocessing pipeline changes.
const PROCESS_VERSION: i32 = 0;

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

            // Skip any domains that end in a skipped domain, to handle both
            // both wildcard domains and specific subdomains.
            let host = url.host_str().unwrap_or("");
            let skip = self
                .config
                .skip
                .iter()
                .map(|s| s.as_str())
                .chain(ALWAYS_SKIP.iter().copied())
                .any(|skip| host.ends_with(skip));
            if skip {
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
                    process_version: PROCESS_VERSION,
                })
                .collect::<Vec<_>>();

            tx.send(batch)?;
        }

        Ok(())
    }

    fn read(
        &self,
        existing: Option<&super::import::FoundItem>,
        compare_strategy: ItemCompareStrategy,
        item: &mut Item,
    ) -> Result<SourceScannerReadResult, eyre::Report> {
        if compare_strategy != ItemCompareStrategy::Force {
            if let Some(skipped) = existing.and_then(|e| e.skipped) {
                // We skipped it last time, so continue skipping it.
                item.skipped = Some(skipped);
                return Ok(SourceScannerReadResult::Unchanged);
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

        let mut req_headers = reqwest::header::HeaderMap::with_capacity(2);
        if let Some(mtime) = item.metadata.mtime {
            let systime = SystemTime::UNIX_EPOCH
                + std::time::Duration::from_secs(mtime.unix_timestamp() as u64);
            if let Ok(value) = HeaderValue::from_str(&httpdate::fmt_http_date(systime)) {
                req_headers.insert(reqwest::header::IF_MODIFIED_SINCE, value);
            }
        }

        let etag = item
            .hash
            .as_ref()
            .and_then(|e| HeaderValue::from_str(e).ok());
        if let Some(etag) = etag {
            req_headers.insert(http::header::IF_NONE_MATCH, etag);
        }

        let response = self.client.get(dbg!(&item.external_id)).send();
        let response = match response {
            Ok(r) => r,
            Err(_) => {
                item.skipped = Some(SkipReason::FetchError);
                return Ok(SourceScannerReadResult::Found);
            }
        };

        let status = response.status();
        let mut skip_reason = match status {
            StatusCode::NOT_FOUND => Some(SkipReason::NotFound),
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => Some(SkipReason::Unauthorized),
            _ => None,
        };

        if skip_reason.is_none() && status.as_u16() >= 400 {
            skip_reason = Some(SkipReason::FetchError);
        }

        if skip_reason.is_some() {
            item.skipped = skip_reason;
            return Ok(SourceScannerReadResult::Found);
        }

        let unchanged = matches!(response.status(), StatusCode::NOT_MODIFIED)
            || response.content_length().map(|l| l == 0).unwrap_or(false);
        if unchanged {
            return Ok(SourceScannerReadResult::Unchanged);
        }

        let headers = response.headers();
        let content_type = headers
            .get(http::header::CONTENT_TYPE)
            .map(|v| {
                let value = v.to_str().unwrap_or_default();
                match value.split_once(';') {
                    Some((mime, _)) => mime.trim(),
                    None => value.trim(),
                }
            })
            .unwrap_or("text/plain");
        item.hash = headers
            .get(http::header::ETAG)
            .and_then(|v| v.to_str().map(String::from).ok());
        item.metadata.mtime = headers
            .get(http::header::LAST_MODIFIED)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| httpdate::parse_http_date(v).ok())
            .map(OffsetDateTime::from);

        if !content_type.starts_with("text/") {
            // Save the item but with empty content. This leaves us with the title, which can be
            // useful for PDFs, and also helps us to store the etag, modified
            // date, etc. so that we aren't doing full fetches over and over again.
            item.content = Some(String::new());
            return Ok(SourceScannerReadResult::Found);
        }

        if content_type.starts_with("text/html") {
            let raw_content = response.text()?;
            let url = Url::parse(&item.external_id)?;

            let (doc, raw_compressed) = rayon::join(
                || Self::extract_html_article(&url, raw_content.as_bytes()),
                || zstd::encode_all(raw_content.as_bytes(), 3),
            );

            item.raw_content = Some(raw_compressed?);

            let doc = doc?;
            item.metadata.name = Some(doc.title);
            item.content = Some(doc.text);
        } else {
            item.content = Some(response.text()?);
        }

        Ok(SourceScannerReadResult::Found)
    }

    fn reprocess(&self, item: &mut Item) -> Result<SourceScannerReadResult, eyre::Report> {
        if item.process_version == PROCESS_VERSION || item.raw_content.is_none() {
            return Ok(SourceScannerReadResult::Unchanged);
        }

        let Some(raw_content) = item.raw_content.as_ref() else {
            return Ok(SourceScannerReadResult::Unchanged);
        };

        let raw_content = zstd::decode_all(raw_content.as_slice())?;
        let url = Url::parse(&item.external_id)?;
        let doc = Self::extract_html_article(&url, &raw_content)?;

        item.metadata.name = Some(doc.title);
        item.content = Some(doc.text);

        Ok(SourceScannerReadResult::Found)
    }
}
