use clap::Args;
use eyre::Result;

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

    for result in results {
        println!("{:.2}: {}", result.1.score, result.0.external_id);
    }

    Ok(())
}
