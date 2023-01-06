use std::sync::atomic::Ordering;

use itertools::Itertools;
use smallvec::{smallvec, SmallVec};

use super::{EmbeddingsOutput, ScanItem, ScanItemState, ScanStats, EMBEDDING_BATCH_SIZE};
use crate::model::Model;

fn calculate_embeddings_batch(
    stats: &ScanStats,
    model: &Model,
    batch: &mut Vec<ScanItem>,
    documents: &Vec<String>,
) -> Result<EmbeddingsOutput, eyre::Report> {
    let _track = stats.encode_time.begin();

    stats
        .embedding
        .fetch_add(documents.len() as u64, Ordering::Relaxed);

    let embeddings: Vec<Vec<f32>> = model.encode(documents)?.into();
    stats
        .embedding
        .fetch_sub(documents.len() as u64, Ordering::Relaxed);

    stats
        .encoded
        .fetch_add(embeddings.len() as u64, Ordering::Relaxed);

    let output = batch
        .drain(..)
        .zip(embeddings.into_iter().map(Some))
        .collect::<SmallVec<_>>();

    Ok(output)
}

pub(super) fn calculate_embeddings(
    stats: &ScanStats,
    model: &Model,
    rx: flume::Receiver<ScanItem>,
    tx: flume::Sender<EmbeddingsOutput>,
) -> Result<(), eyre::Report> {
    let mut batch = Vec::with_capacity(EMBEDDING_BATCH_SIZE);
    let mut documents = Vec::with_capacity(EMBEDDING_BATCH_SIZE);

    for item in rx {
        if matches!(item.state, ScanItemState::Unchanged | ScanItemState::Found)
            || item.item.skipped.is_some()
        {
            tx.send(smallvec![(item, None)])?;
            continue;
        }

        let document =
            if item.item.metadata.name.is_none() && item.item.metadata.description.is_none() {
                item.item.content.as_deref().map(|s| s.trim().to_string())
            } else {
                let document = [
                    item.item.metadata.name.as_deref(),
                    item.item.metadata.description.as_deref(),
                    item.item.content.as_deref(),
                ]
                .into_iter()
                .flatten()
                .filter(|s| !s.trim().is_empty())
                .join("\n");

                if document.is_empty() {
                    None
                } else {
                    Some(document)
                }
            };

        let Some(document) = document else {
            tx.send(smallvec![(item, None)])?;
            continue;
        };

        documents.push(document);
        batch.push(item);
        if batch.len() < EMBEDDING_BATCH_SIZE {
            continue;
        }

        let output = calculate_embeddings_batch(stats, model, &mut batch, &documents)?;
        // batch was cleared by the above, so just clear `documents`
        documents.clear();

        tx.send(output)?;
    }

    if !batch.is_empty() {
        let output = calculate_embeddings_batch(stats, model, &mut batch, &documents)?;
        tx.send(output)?;
    }

    Ok(())
}
