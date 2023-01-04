use clap::Args;
use eyre::{eyre, Result};
use owo_colors::OwoColorize;
use perceive_core::sources::SourceTypeTag;

use crate::AppState;

#[derive(Args, Debug)]
pub struct SearchArgs {
    /// The query to search for
    pub query: String,

    /// Search only in the specified source
    #[arg(short, long)]
    pub source: Option<String>,

    /// Search only sources of a particular type
    #[arg(
        rename_all = "kebab-case",
        short = 't',
        long = "type",
        conflicts_with("source")
    )]
    pub source_type: Option<SourceTypeTag>,

    /// Return this number of search results
    #[arg(short, long, default_value_t = 20)]
    pub num_results: usize,
}

pub fn search(state: &mut AppState, args: SearchArgs) -> Result<()> {
    let sources = match (args.source, args.source_type) {
        (Some(name), _) => {
            let source = state
                .sources
                .iter()
                .find(|s| s.name == name)
                .ok_or_else(|| eyre!("Source not found"))?;

            vec![source.id]
        }
        (None, Some(source_type)) => state
            .sources
            .iter()
            .filter(|s| s.config.matches_tag(source_type))
            .map(|s| s.id)
            .collect(),
        (None, None) => state.sources.iter().map(|s| s.id).collect(),
    };

    let results = state.searcher.search_and_retrieve(
        &state.database,
        state.borrow_model(),
        &sources,
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
