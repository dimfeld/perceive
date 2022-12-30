use clap::Subcommand;
use eyre::{eyre, Result};

use self::{model::ModelArgs, search::SearchArgs};
use crate::AppState;

pub mod model;
pub mod search;
pub mod source;

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Manage data sources
    Source(source::SourceArgs),
    /// Refresh data
    Refresh,
    /// Do a search
    Search(SearchArgs),
    /// Configure the model
    Model(ModelArgs),
}

pub fn handle_command(state: &mut AppState, cmd: Commands) -> Result<()> {
    match cmd {
        Commands::Refresh => return Err(eyre!("Not implemented yet")),
        Commands::Search(args) => search::search(state, args),
        Commands::Source(args) => source::handle_source_command(state, args),
        Commands::Model(args) => model::handle_model_command(state, args),
    }
}
