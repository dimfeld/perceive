use clap::Args;
use eyre::{eyre, Result};
use owo_colors::OwoColorize;
use perceive_core::{
    search::{self, deserialize_embedding},
    sources::SourceTypeTag,
};

use crate::AppState;

#[derive(Args, Debug)]
pub struct SearchArgs {
    /// The query to search for
    #[arg(required_unless_present("like"))]
    pub query: Option<String>,

    /// Find items that are most similar to the item with this ID.
    #[arg(short, long, conflicts_with("query"), required_unless_present("query"))]
    pub like: Option<i64>,

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

    let (query, input_vec) = match (args.query, args.like) {
        (Some(query), _) => {
            let term_embedding = search::encode_query(state.borrow_model(), &query);
            (query, term_embedding)
        }
        (None, Some(like)) => {
            let conn = state.database.read_pool.get()?;
            let mut stmt = conn.prepare_cached(
                "SELECT name, embedding
                FROM items
                JOIN item_embeddings ie ON ie.item_id=items.id
                WHERE id = ? AND model_id = ? AND model_version = ?",
            )?;

            let (name, embedding) = stmt
                .query_and_then(
                    [like, state.model_id as i64, state.model_version as i64],
                    |row| {
                        let name: String = row.get(0)?;
                        let v = row.get_ref(1)?.as_blob()?;
                        Ok::<_, eyre::Report>((name, deserialize_embedding(v)))
                    },
                )?
                .next()
                .ok_or_else(|| eyre!("Item not found"))??;

            (name, embedding)
        }
        (None, None) => {
            return Err(eyre!("No query provided"));
        }
    };

    let results = state.searcher.search_vector_and_retrieve(
        &state.database,
        &sources,
        args.num_results,
        input_vec,
    )?;

    let result_docs = results
        .iter()
        .map(|r| r.0.content.as_deref().unwrap_or_default())
        .collect::<Vec<_>>();

    let highlights = state.highlights_model.highlight(&query, &result_docs)?;

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
