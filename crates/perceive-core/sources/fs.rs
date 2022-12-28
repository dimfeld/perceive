use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use super::import::{FoundItem, SourceScanner, SourceScannerReadResult};
use crate::{batch_sender::BatchSender, Item};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FsSourceConfig {
    pub globs: Vec<String>,
    pub overrides: Vec<String>,
}

pub struct FileScanner {
    pub source_id: i64,
    pub location: String,
    pub config: FsSourceConfig,
}

impl SourceScanner for FileScanner {
    fn scan(&self, output: flume::Sender<Vec<Item>>) -> Result<(), eyre::Report> {
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
        mut item: Item,
    ) -> Result<(SourceScannerReadResult, Item), eyre::Report> {
        let Ok(content) = std::fs::read_to_string(std::path::Path::new(&item.external_id)) else {
            // Currently we just return None if reading fails. This includes where the file
            // is a binary file that can't be converted to UTF-8. In the future we should
            // distinguish between these cases.
            //
            // Future iterations of this will also try to process other file formats that aren't plain
            // text but do contain text (i.e. Word, Pages, etc.).
            return Ok((SourceScannerReadResult::Skip, item));
        };

        if content.trim().is_empty() {
            return Ok((SourceScannerReadResult::Skip, item));
        }

        item.content = Some(content);
        Ok((SourceScannerReadResult::Found, item))
    }
}

const BATCH_SIZE: usize = 64;

struct FileVisitorBuilder {
    source_id: i64,
    glob: globset::GlobSet,
    output: flume::Sender<Vec<Item>>,
}

impl<'s> ignore::ParallelVisitorBuilder<'s> for FileVisitorBuilder {
    fn build(&mut self) -> Box<dyn ignore::ParallelVisitor + 's> {
        Box::new(FileVisitor {
            source_id: self.source_id,
            glob: self.glob.clone(),
            sender: BatchSender::new(BATCH_SIZE, self.output.clone()),
        })
    }
}

struct FileVisitor {
    source_id: i64,
    glob: globset::GlobSet,
    sender: BatchSender<Item>,
}

impl ignore::ParallelVisitor for FileVisitor {
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

impl Drop for FileVisitor {
    fn drop(&mut self) {
        self.sender.flush().ok();
    }
}
