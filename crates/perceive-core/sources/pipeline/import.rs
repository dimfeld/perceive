use super::{
    calculate_embeddings::calculate_embeddings, match_existing_items::match_to_existing_items,
    read_items::read_items, update_db::update_db, CountingVecSender, ItemCompareStrategy,
    ScanStats, EMBEDDING_BATCH_SIZE,
};
use crate::{
    db::Database,
    model::Model,
    sources::{pipeline::log_thread_error, Source},
};

pub fn scan_source(
    times: &ScanStats,
    database: &Database,
    model: &Model,
    model_id: u32,
    model_version: u32,
    source: &Source,
    override_compare_strategy: Option<ItemCompareStrategy>,
) -> Result<(), eyre::Report> {
    let scanner = source.create_scanner()?;
    let compare_strategy = override_compare_strategy.unwrap_or(source.compare_strategy);

    std::thread::scope(|scope| {
        let (item_tx, item_rx) = flume::unbounded();
        let (matched_tx, matched_rx) = flume::bounded(256);
        let (with_content_tx, with_content_rx) = flume::bounded(EMBEDDING_BATCH_SIZE);
        let (with_embeddings_tx, with_embeddings_rx) = flume::bounded(8);

        let item_tx = CountingVecSender {
            tx: item_tx,
            count: &times.scanned,
        };

        // Make a pipeline with the following stages:
        // - Scan the file system and send out batches of items
        let scan_task = scope.spawn(|| {
            let _track = times.scan_time.begin();
            scanner.scan(item_tx)
        });

        // STAGE 2
        // - Match each item to a database entry.
        // - If one was found, do a preliminary match (i.e. by mtime) to see if it can be skipped.
        let db_lookup_task = scope.spawn(|| {
            match_to_existing_items(
                database,
                model_id,
                model_version,
                source.id,
                compare_strategy,
                item_rx,
                matched_tx,
            )
        });

        // STAGE 3
        // - Read the content of the file
        // - Match against the content, if applicable
        // - If the content did not change, skip the item.
        const READ_PARALLELISM: usize = 8;
        let read_tasks = itertools::repeat_n((matched_rx, with_content_tx), READ_PARALLELISM)
            .into_iter()
            .map(|(matched_rx, with_content_tx)| {
                scope.spawn(|| {
                    read_items(
                        times,
                        compare_strategy,
                        scanner.as_ref(),
                        matched_rx,
                        with_content_tx,
                    )
                })
            })
            .collect::<Vec<_>>();

        // STAGE 4
        // - Calculate embeddings for the items in this batch that we are keeping.
        let embed_task =
            scope.spawn(|| calculate_embeddings(times, model, with_content_rx, with_embeddings_tx));

        // STAGE 5
        // - Update the database for items that will be kept
        let write_db_task = scope.spawn(|| {
            update_db(
                model_id,
                model_version,
                times,
                database,
                source.index_version,
                with_embeddings_rx,
            )
        });

        // TODO - handle errors better
        let mut errored = false;
        errored = errored || log_thread_error("db_writer", write_db_task.join());
        errored = errored || log_thread_error("encoding task", embed_task.join());
        for read_task in read_tasks {
            errored = errored || log_thread_error("read_items", read_task.join());
        }
        errored = errored || log_thread_error("db_lookup", db_lookup_task.join());
        errored = errored || log_thread_error("scanner", scan_task.join());

        if errored {
            println!("Scanning failed");
        }
    });

    // AFTER PIPELINE TODO
    // - For source types that require it, delete any items for which the
    //   index_version did not get updated to the latest vesion. This means
    //   that they were not found in the scan.

    Ok(())
}
