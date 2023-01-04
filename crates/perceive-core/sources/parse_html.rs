use std::{io::Cursor, time::SystemTime};

use eyre::Context;
use http::{HeaderValue, StatusCode};
use reqwest::{blocking::Client, Url};
use time::OffsetDateTime;

use super::pipeline::{FoundItem, SourceScannerReadResult};
use crate::{Item, SkipReason};

pub const ALWAYS_SKIP: [&str; 5] = [
    // Signin pages. These show up a lot but never contain searchable content.
    "accounts.google.com",
    "ad.doubleclick.net",
    "console.cloud.google.com",
    "console.aws.amazon.com",
    "googleapis.com",
];

/// Skip any domains that end in a skipped domain, to handle both
/// both wildcard domains and specific subdomains.
pub fn should_skip(skip: &[impl AsRef<str>], url: &Url) -> bool {
    let host = url.host_str().unwrap_or("");
    skip.iter()
        .map(|s| s.as_ref())
        .chain(ALWAYS_SKIP.iter().copied())
        .any(|skip| host.ends_with(skip))
}

pub const HTML_PROCESS_VERSION: i32 = 1;

pub fn extract_html_article(
    url: &str,
    raw_content: &[u8],
) -> Result<readability::extractor::Product, eyre::Report> {
    let url = Url::parse(url)?;
    let mut cursor = Cursor::new(raw_content);
    readability::extractor::extract(&mut cursor, &url).map_err(|e| e.into())
}

pub fn reprocess_html_article(item: &mut Item) -> Result<SourceScannerReadResult, eyre::Report> {
    let Some(raw_content) = item.raw_content.as_ref() else {
        return Ok(SourceScannerReadResult::Unchanged);
    };

    let raw_content = zstd::decode_all(raw_content.as_slice())
        .wrap_err_with(|| format!("{} - decompressing content", item.external_id))?;
    let doc = extract_html_article(&item.external_id, &raw_content)
        .wrap_err_with(|| format!("{} - extracting content", item.external_id))?;

    let changed = item
        .metadata
        .name
        .as_ref()
        .map(|n| n != &doc.title)
        .unwrap_or(true)
        || item
            .content
            .as_ref()
            .map(|c| c != &doc.text)
            .unwrap_or(true);

    if !changed {
        return Ok(SourceScannerReadResult::Unchanged);
    }

    item.process_version = HTML_PROCESS_VERSION;
    item.metadata.name = Some(doc.title);
    item.content = Some(doc.text);

    Ok(SourceScannerReadResult::Found)
}

pub fn fetch_html(
    client: &Client,
    existing: Option<&FoundItem>,
    item: &mut Item,
) -> Result<SourceScannerReadResult, eyre::Report> {
    let mut req_headers = reqwest::header::HeaderMap::with_capacity(2);
    if let Some(mtime) = item.metadata.mtime {
        let systime =
            SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(mtime.unix_timestamp() as u64);
        if let Ok(value) = HeaderValue::from_str(&httpdate::fmt_http_date(systime)) {
            req_headers.insert(reqwest::header::IF_MODIFIED_SINCE, value);
        }
    }

    let etag = item
        .hash
        .as_ref()
        .or_else(|| existing.map(|e| &e.hash))
        .and_then(|e| HeaderValue::from_str(e).ok());
    if let Some(etag) = etag {
        req_headers.insert(http::header::IF_NONE_MATCH, etag);
    }

    let response = client.get(&item.external_id).send();
    let response = match response {
        Ok(r) => r,
        Err(_) => {
            item.skipped = Some(SkipReason::FetchError);
            return Ok(SourceScannerReadResult::Found);
        }
    };

    let status = response.status();

    let unchanged = matches!(status, StatusCode::NOT_MODIFIED);
    if unchanged {
        return Ok(SourceScannerReadResult::Unchanged);
    }

    let skip_reason = match status {
        StatusCode::FORBIDDEN | StatusCode::UNAUTHORIZED => Some(SkipReason::Unauthorized),
        StatusCode::NOT_FOUND => Some(SkipReason::NotFound),
        // This is handled later, but we don't want it to fall in the redirect case.
        StatusCode::NOT_MODIFIED => None,
        s if s.is_redirection() => Some(SkipReason::Redirected),
        s if s.is_client_error() || s.is_server_error() => Some(SkipReason::FetchError),
        _ => None,
    };

    if skip_reason.is_some() {
        item.skipped = skip_reason;
        return Ok(SourceScannerReadResult::Found);
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

    let is_html = content_type.starts_with("text/html");

    let raw_content = response.text()?;
    if raw_content.is_empty() {
        item.skipped = Some(SkipReason::NoContent);
        return Ok(SourceScannerReadResult::Found);
    }

    if is_html {
        let (doc, raw_compressed) = rayon::join(
            || extract_html_article(&item.external_id, raw_content.as_bytes()),
            || zstd::encode_all(raw_content.as_bytes(), 3),
        );

        item.raw_content = Some(raw_compressed?);

        let doc = doc?;
        item.metadata.name = Some(doc.title);
        item.content = Some(doc.text);
    } else {
        item.content = Some(raw_content);
    }

    item.process_version = HTML_PROCESS_VERSION;

    Ok(SourceScannerReadResult::Found)
}
