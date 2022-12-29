use clap::Args;
use eyre::Result;
use owo_colors::OwoColorize;

use crate::AppState;

#[derive(Args, Debug)]
pub struct SearchArgs {
    /// The query to search for
    pub query: String,

    #[clap(short, long, default_value_t = 10)]
    pub num_results: usize,
}

pub fn search(state: &mut AppState, args: SearchArgs) -> Result<()> {
    let results = state.searcher.search_and_retrieve(
        &state.database,
        state.borrow_model(),
        args.num_results,
        &args.query,
    )?;

    let result_docs = results
        .iter()
        .map(|r| r.0.content.as_deref().unwrap_or_default())
        .collect::<Vec<_>>();

    let highlights = state
        .highlights_model
        .highlight(&args.query, &result_docs)?;

    for (index, (result, item)) in results.iter().enumerate() {
        let desc = result.metadata.name.as_ref().unwrap_or(&result.external_id);
        let highlight = highlights[index].replace('\n', "•");
        println!("{:.2}: {} - {highlight}", item.score.abs(), desc.bold());
    }

    Ok(())
}
