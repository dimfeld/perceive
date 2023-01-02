use std::sync::atomic::Ordering;

use super::{ScanItem, ScanItemState, ScanStats, SourceScanner, SourceScannerReadResult};
use crate::sources::ItemCompareStrategy;

pub fn read_items(
    stats: &ScanStats,
    compare_strategy: ItemCompareStrategy,
    scanner: &dyn SourceScanner,
    rx: flume::Receiver<ScanItem>,
    tx: flume::Sender<ScanItem>,
) -> Result<(), eyre::Report> {
    for item in rx {
        let _track = stats.read_time.begin();

        let ScanItem { mut item, state } = item;

        let existing = match &state {
            ScanItemState::New => None,
            ScanItemState::Found(found) => Some(found),
            ScanItemState::Changed(found) => Some(found),
            ScanItemState::Unchanged { .. } => {
                tx.send(ScanItem { item, state })?;
                continue;
            }
        };

        let external_id = item.external_id.clone();

        stats.reading.fetch_add(1, Ordering::Relaxed);
        let read_result = scanner.read(existing, compare_strategy, &mut item);
        stats.reading.fetch_sub(1, Ordering::Relaxed);
        stats.fetched.fetch_add(1, Ordering::Relaxed);

        let state = match read_result {
            Ok(SourceScannerReadResult::Found) => state,
            Ok(SourceScannerReadResult::Unchanged) => {
                if let Some(id) = state.item_id() {
                    ScanItemState::Unchanged { id }
                } else {
                    // The scanner said the item was unchanged, but we also don't have an
                    // existing one. Just skip it.
                    continue;
                }
            }
            Ok(SourceScannerReadResult::Omit) => {
                continue;
            }
            Err(e) => {
                // TODO - better error handling
                eprintln!("{}: {e}", external_id);
                continue;
            }
        };

        let compare_content = item.skipped.is_none() && compare_strategy.should_compare_content();
        let state = match state {
            ScanItemState::New | ScanItemState::Unchanged { .. } | ScanItemState::Changed(_) => {
                state
            }
            ScanItemState::Found(found) => {
                if compare_content && found.content != item.content.as_deref().unwrap_or_default() {
                    ScanItemState::Changed(found)
                } else {
                    ScanItemState::Unchanged { id: found.id }
                }
            }
        };

        tx.send(ScanItem { state, item })?;
    }

    Ok(())
}
