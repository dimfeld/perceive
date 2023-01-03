use const_format::concatcp;
use eyre::{Report, Result};
use itertools::Itertools;
use rayon::prelude::*;

use super::{
    calculate_embeddings::calculate_embeddings, log_thread_error, update_db::update_db, ScanItem,
    ScanItemState, ScanStats, SourceScannerReadResult, EMBEDDING_BATCH_SIZE,
};
use crate::{
    db::{deserialize_item_row, Database, ITEM_COLUMNS},
    model::Model,
    sources::Source,
    Item,
};

fn read_rows(
    database: &Database,
    source: &Source,
    process_version: i32,
    tx: flume::Sender<Vec<Item>>,
) -> Result<()> {
    let conn = database.read_pool.get()?;

    let mut stmt = conn.prepare_cached(concatcp!(
        "SELECT ",
        ITEM_COLUMNS,
        " FROM items WHERE source_id = ? AND process_version < ?"
    ))?;

    let rows = stmt.query_and_then([source.id, process_version as i64], deserialize_item_row)?;

    for batch in &rows.into_iter().chunks(EMBEDDING_BATCH_SIZE) {
        let batch = batch.into_iter().collect::<Result<Vec<_>>>()?;
        tx.send(batch)?;
    }

    Ok(())
}

pub fn reprocess_source(
    times: &ScanStats,
    database: &Database,
    model: Model,
    model_id: u32,
    model_version: u32,
    source: &Source,
) -> Result<Model, (Model, eyre::Report)> {
    let scanner = match source.create_scanner() {
        Ok(scanner) => scanner,
        Err(e) => {
            return Err((model, e));
        }
    };
    let process_version = scanner.latest_process_version();

    let (db_items_tx, db_items_rx) = flume::unbounded();
    let (processed_tx, processed_rx) = flume::bounded(EMBEDDING_BATCH_SIZE);
    let (with_embeddings_tx, with_embeddings_rx) = flume::bounded(8);

    let returned_model = std::thread::scope(|scope| {
        let read_task = scope.spawn(|| read_rows(database, source, process_version, db_items_tx));

        let process_task = scope.spawn(|| {
            for batch in db_items_rx {
                batch.into_par_iter().try_for_each(|mut item| {
                    let result = scanner.reprocess(&mut item)?;
                    if matches!(result, SourceScannerReadResult::Found) {
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
        });

        let embed_task = scope.spawn(move || {
            let result = calculate_embeddings(times, &model, processed_rx, with_embeddings_tx);
            match result {
                Ok(()) => Ok(model),
                Err(e) => Err((model, e)),
            }
        });

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
