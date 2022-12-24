use clap::Subcommand;
use eyre::{eyre, Result};

use self::search::SearchArgs;
use crate::AppState;

pub mod add;
pub mod model;
pub mod search;

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Manage data sources
    Source,
    /// Refresh data
    Refresh,
    /// Do a search
    Search(SearchArgs),
}

pub fn handle_command(state: &mut AppState, cmd: Commands) -> Result<()> {
    match cmd {
        Commands::Refresh => return Err(eyre!("Not implemented yet")),
        Commands::Search(args) => search::search(state, args),
        Commands::Source => return Err(eyre!("Not implemented yet")),
    }
}
