use clap::Args;
use eyre::Result;
use owo_colors::OwoColorize;

use crate::AppState;

#[derive(Args, Debug)]
pub struct SearchArgs {
    /// The query to search for
    pub query: String,

    /// Return this number of search results
    #[clap(short, long, default_value_t = 20)]
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
        let highlight = highlights[index].unwrap_or_default().replace('\n', "•");
        let source_name = state
            .sources
            .iter()
            .find(|s| s.id == result.source_id)
            .map(|s| s.name.as_str())
            .unwrap_or_default();
        println!(
            "{} {} - {} - {highlight}",
            source_name,
            item.id,
            desc.bold()
        );
    }

    Ok(())
}
