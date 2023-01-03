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

        if matches!(item.state, ScanItemState::Unchanged) {
            tx.send(item)?;
            continue;
        }

        let ScanItem {
            mut item,
            existing,
            state,
        } = item;

        let external_id = item.external_id.clone();

        stats.reading.fetch_add(1, Ordering::Relaxed);
        let read_result = scanner.read(existing.as_ref(), compare_strategy, &mut item);
        stats.reading.fetch_sub(1, Ordering::Relaxed);
        stats.fetched.fetch_add(1, Ordering::Relaxed);

        let state = match read_result {
            Ok(SourceScannerReadResult::Found) => state,
            Ok(SourceScannerReadResult::Unchanged) => ScanItemState::Unchanged,
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
        let state = match (state, existing.as_ref()) {
            (ScanItemState::New | ScanItemState::Unchanged | ScanItemState::Changed, _) => state,
            (ScanItemState::Found, None) => ScanItemState::New,
            (ScanItemState::Found, Some(existing)) => {
                if compare_content
                    && existing.content != item.content.as_deref().unwrap_or_default()
                {
                    ScanItemState::Changed
                } else {
                    ScanItemState::Unchanged
                }
            }
        };

        tx.send(ScanItem {
            state,
            existing,
            item,
        })?;
    }

    Ok(())
}
