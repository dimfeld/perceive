use clap::Args;
use eyre::Result;

use crate::AppState;

#[derive(Args, Debug)]
/// Add a term to be matched by the search command
pub struct AddArgs {
    /// The term to add
    pub term: String,
}

pub fn add_term(state: &mut AppState, args: AddArgs) -> Result<()> {
    state.candidates.push(args.term);
    Ok(())
}
