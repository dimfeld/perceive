use std::borrow::Cow;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use super::{
    import::{CountingVecSender, FoundItem, SourceScanner, SourceScannerReadResult},
    ItemCompareStrategy,
};
use crate::{batch_sender::BatchSender, Item, ItemMetadata};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FsSourceConfig {
    pub globs: Vec<String>,
}

pub struct FileScanner {
    pub source_id: i64,
    pub location: String,
    pub config: FsSourceConfig,
}

impl SourceScanner for FileScanner {
    fn scan(&self, output: CountingVecSender<Item>) -> Result<(), eyre::Report> {
        let mut glob_builder = globset::GlobSetBuilder::new();
        if self.config.globs.is_empty() {
            glob_builder.add(globset::Glob::new("*")?);
        } else {
            for glob in &self.config.globs {
                glob_builder.add(globset::Glob::new(glob)?);
            }
        }

        let glob = glob_builder.build()?;

        let mut visitor_builder = FileVisitorBuilder {
            source_id: self.source_id,
            glob,
            output,
        };

        ignore::WalkBuilder::new(std::path::Path::new(&self.location))
            .build_parallel()
            .visit(&mut visitor_builder);
        Ok(())
    }

    fn read(
        &self,
        _existing: Option<&FoundItem>,
        _compare_strategy: ItemCompareStrategy,
        item: &mut Item,
    ) -> Result<SourceScannerReadResult, eyre::Report> {
        let Ok(content) = std::fs::read_to_string(std::path::Path::new(&item.external_id)) else {
            // Currently we just return None if reading fails. This includes where the file
            // is a binary file that can't be converted to UTF-8. In the future we should
            // distinguish between these cases.
            //
            // Future iterations of this will also try to process other file formats that aren't plain
            // text but do contain text (i.e. Word, Pages, etc.).
            return Ok(SourceScannerReadResult::Omit);
        };

        if content.trim().is_empty() {
            return Ok(SourceScannerReadResult::Omit);
        }

        if let Some(doc_content) = process_content(&content, &mut item.metadata) {
            item.content = Some(doc_content);

            let compressed = zstd::encode_all(content.as_bytes(), 3)?;
            item.raw_content = Some(compressed);
        } else {
            item.content = Some(content);
        }

        Ok(SourceScannerReadResult::Found)
    }

    fn reprocess(&self, item: &mut Item) -> Result<SourceScannerReadResult, eyre::Report> {
        let content = match (item.raw_content.as_ref(), item.content.as_ref()) {
            (Some(buffer), _) => {
                let decompressed = zstd::decode_all(buffer.as_slice())?;
                Cow::Owned(String::from_utf8(decompressed)?)
            }
            (None, Some(content)) => Cow::Borrowed(content),
            (None, None) => return Ok(SourceScannerReadResult::Unchanged),
        };

        if let Some(content) = process_content(&content, &mut item.metadata) {
            item.content = Some(content);
            Ok(SourceScannerReadResult::Found)
        } else {
            Ok(SourceScannerReadResult::Unchanged)
        }
    }
}

fn process_content(content: &str, metadata: &mut ItemMetadata) -> Option<String> {
    let parser = gray_matter::Matter::<gray_matter::engine::YAML>::new();
    let Some(parsed) = parser.parse_with_struct::<FileAttributes>(&content) else {
        return None;
    };

    metadata.name = parsed.data.title.or(parsed.data.name);
    metadata.description = parsed.data.description.or(parsed.data.summary);
    metadata.author = parsed.data.author;

    Some(parsed.content)
}

const BATCH_SIZE: usize = 64;

struct FileVisitorBuilder<'a> {
    source_id: i64,
    glob: globset::GlobSet,
    output: CountingVecSender<'a, Item>,
}

impl<'a, 's> ignore::ParallelVisitorBuilder<'s> for FileVisitorBuilder<'a>
where
    'a: 's,
{
    fn build(&mut self) -> Box<dyn ignore::ParallelVisitor + 'a> {
        Box::new(FileVisitor {
            source_id: self.source_id,
            glob: self.glob.clone(),
            sender: BatchSender::new(BATCH_SIZE, self.output.clone()),
        })
    }
}

struct FileVisitor<'a> {
    source_id: i64,
    glob: globset::GlobSet,
    sender: BatchSender<'a, Item>,
}

impl<'a> ignore::ParallelVisitor for FileVisitor<'a> {
    fn visit(&mut self, entry: Result<ignore::DirEntry, ignore::Error>) -> ignore::WalkState {
        if let Ok(entry) = entry {
            let meta = match entry.metadata() {
                Ok(meta) if meta.file_type().is_file() => meta,
                _ => return ignore::WalkState::Continue,
            };

            let path = entry.path();
            let is_match = self.glob.is_match(path);

            if is_match {
                let item = Item {
                    source_id: self.source_id,
                    external_id: entry.path().to_string_lossy().to_string(),
                    hash: None,
                    content: None,
                    raw_content: None,
                    skipped: None,
                    process_version: 0,
                    metadata: crate::ItemMetadata {
                        name: None,
                        author: None,
                        description: None,
                        mtime: meta.modified().ok().map(OffsetDateTime::from),
                        atime: meta.accessed().ok().map(OffsetDateTime::from),
                    },
                };

                let send_result = self.sender.add(item);
                if send_result.is_err() {
                    // Something went wrong downstream, so there's no point in walking the FS
                    // anymore.
                    return ignore::WalkState::Quit;
                }
            }
        }

        ignore::WalkState::Continue
    }
}

#[derive(Debug, Deserialize)]
struct FileAttributes {
    title: Option<String>,
    author: Option<String>,
    name: Option<String>,
    summary: Option<String>,
    description: Option<String>,
}
