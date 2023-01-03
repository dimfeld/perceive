use std::sync::atomic::Ordering;

use const_format::concatcp;
use eyre::{Report, Result};
use itertools::Itertools;
use rayon::prelude::*;

use super::{
    calculate_embeddings::calculate_embeddings, log_thread_error, update_db::update_db,
    wrap_thread, ScanItem, ScanItemState, ScanStats, SourceScannerReadResult, EMBEDDING_BATCH_SIZE,
};
use crate::{
    db::{deserialize_item_row, Database, ITEM_COLUMNS},
    model::Model,
    sources::Source,
    Item,
};

fn read_rows(
    times: &ScanStats,
    database: &Database,
    source: &Source,
    tx: flume::Sender<Vec<Item>>,
) -> Result<()> {
    let conn = database.read_pool.get()?;

    let mut stmt = conn.prepare_cached(concatcp!(
        "SELECT ",
        ITEM_COLUMNS,
        " FROM items WHERE source_id = ? and skipped is null"
    ))?;

    let rows = stmt.query_and_then([source.id], deserialize_item_row)?;

    for batch in &rows.into_iter().chunks(EMBEDDING_BATCH_SIZE) {
        let batch = batch.into_iter().collect::<Result<Vec<_>>>()?;
        times
            .scanned
            .fetch_add(batch.len() as u64, Ordering::Relaxed);
        tx.send(batch)?;
    }

    Ok(())
}

fn reprocess(
    times: &ScanStats,
    source: &Source,
    db_items_rx: flume::Receiver<Vec<Item>>,
    processed_tx: flume::Sender<ScanItem>,
) -> Result<()> {
    // The Rayon threads here often end up waiting on channel sends while
    // the model is encoding, but the model tokenizer also uses Rayon and so this starves
    // them from running and everything deadlocks. So we use a separate pool here.
    let pool = rayon::ThreadPoolBuilder::new().build()?;
    let scanner = source.create_scanner()?;

    pool.install(|| {
        for batch in db_items_rx {
            batch.into_par_iter().try_for_each(|mut item| {
                times
                    .reading
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let result = scanner.reprocess(&mut item)?;
                if matches!(result, SourceScannerReadResult::Found) {
                    times
                        .fetched
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    processed_tx.send(ScanItem {
                        state: ScanItemState::Changed,
                        existing: None,
                        item,
                    })?;
                }

                Ok::<_, Report>(())
            })?;
        }
        Ok::<_, Report>(())
    })?;

    Ok::<_, Report>(())
}

pub fn reprocess_source(
    times: &ScanStats,
    database: &Database,
    model: Model,
    model_id: u32,
    model_version: u32,
    source: &Source,
) -> Result<Model, (Model, eyre::Report)> {
    let returned_model = std::thread::scope(|scope| {
        let (db_items_tx, db_items_rx) = flume::unbounded();
        let (processed_tx, processed_rx) = flume::bounded(EMBEDDING_BATCH_SIZE);
        let (with_embeddings_tx, with_embeddings_rx) = flume::bounded(8);

        let read_task = scope
            .spawn(|| wrap_thread("read_rows", read_rows(times, database, source, db_items_tx)));

        let process_task = scope.spawn(move || {
            wrap_thread(
                "reprocess",
                reprocess(times, source, db_items_rx, processed_tx),
            )
        });

        let embed_task = scope.spawn(move || {
            let result = wrap_thread(
                "embedder",
                calculate_embeddings(times, &model, processed_rx, with_embeddings_tx),
            );
            match result {
                Ok(()) => Ok(model),
                Err(e) => Err((model, e)),
            }
        });

        let write_db_task = scope.spawn(|| {
            wrap_thread(
                "update_db",
                update_db(
                    model_id,
                    model_version,
                    times,
                    database,
                    source.index_version,
                    with_embeddings_rx,
                ),
            )
        });

        let mut errored = false;

        errored = errored || log_thread_error("db_writer", write_db_task.join());
        let returned_model = match embed_task.join().unwrap() {
            Ok(model) => model,
            Err((model, e)) => {
                errored = true;
                eprintln!("Encoding thread error: {e}");
                model
            }
        };

        errored = errored || log_thread_error("processor", process_task.join());
        errored = errored || log_thread_error("reader", read_task.join());

        if errored {
            println!("Reprocessing failed");
        }

        returned_model
    });

    Ok(returned_model)
}
