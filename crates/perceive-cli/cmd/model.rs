use clap::Args;
use dialoguer::FuzzySelect;
use perceive_core::SentenceEmbeddingsModelType;
use strum::IntoEnumIterator;

use crate::AppState;

#[derive(Args, Debug)]
pub struct ModelArgs {
    /// The type of model to load. If not specified, a list of available models will be shown.
    pub model_type: Option<SentenceEmbeddingsModelType>,
}

fn choose_model(
    currently_loaded: Option<SentenceEmbeddingsModelType>,
) -> Option<SentenceEmbeddingsModelType> {
    let items = SentenceEmbeddingsModelType::iter().collect::<Vec<_>>();

    let default = currently_loaded
        .and_then(|currently_loaded| items.iter().position(|&r| r == currently_loaded))
        .unwrap_or(0);
    FuzzySelect::new()
        .with_prompt("Choose a model")
        .items(&items)
        .default(default)
        .interact()
        .ok()
        .map(|s| items[s])
}

pub fn model(state: &mut AppState, args: ModelArgs) -> Result<(), eyre::Report> {
    let model_type = match args
        .model_type
        .or_else(|| choose_model(state.model.current_model().map(|m| m.0)))
    {
        Some(t) => t,
        None => return Ok(()),
    };

    if state.model.needs_load(model_type) {
        println!("Loading model {model_type}");
        state.model.model(model_type)?;
    }

    Ok(())
}
