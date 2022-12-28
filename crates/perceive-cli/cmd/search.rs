use clap::Args;
use eyre::Result;
use pcore::model::SentenceEmbeddingsModelType;
use perceive_core as pcore;
use tch::IndexOp;

use crate::AppState;

#[derive(Args, Debug)]
pub struct SearchArgs {
    /// The query to search for
    pub query: String,
}

pub fn search(state: &mut AppState, args: SearchArgs) -> Result<()> {
    /*
    let mut sentences = Vec::with_capacity(state.candidates.len() + 1);
    sentences.push(args.query.as_str());
    sentences.extend(state.candidates.iter().map(|s| s.as_str()));

    let model = match state.model.current_model() {
        Some((_, m)) => m,
        None => {
            let default_type = SentenceEmbeddingsModelType::AllMiniLmL6V2;
            println!("No model loaded. Loading {default_type}");
            state.model.model(default_type)?
        }
    };

    let results = model.encode_as_tensor(&sentences)?;

    let query_embedding = results.embeddings.i(0);
    let match_embeddings = results.embeddings.slice(0, 1, None, 1);

    let cos_score = pcore::cosine_similarity_single_query(&query_embedding, &match_embeddings);

    let scores: Vec<f32> = Vec::from(cos_score);

    let mut results = scores
        .into_iter()
        .enumerate()
        .map(|(i, score)| (sentences[i + 1], score))
        .collect::<Vec<_>>();
    results.sort_by_key(|(_, score)| std::cmp::Reverse((*score * 10000.0) as u32));

    for (sentence, score) in results {
        println!("{sentence:<20} {score:.2}");
    }
    */

    Ok(())
}
