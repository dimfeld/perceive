use clap::Subcommand;
use eyre::{eyre, Result};

use self::{hide::HideArgs, model::ModelArgs, print::PrintArgs, search::SearchArgs};
use crate::AppState;

pub mod hide;
pub mod model;
pub mod print;
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
    /// Print the content for an item
    Print(PrintArgs),
    /// Hide an item from the search results
    Hide(HideArgs),
}

pub fn handle_command(state: &mut AppState, cmd: Commands) -> Result<()> {
    match cmd {
        Commands::Refresh => return Err(eyre!("Not implemented yet")),
        Commands::Search(args) => search::search(state, args),
        Commands::Source(args) => source::handle_source_command(state, args),
        Commands::Model(args) => model::handle_model_command(state, args),
        Commands::Print(args) => print::handle_print_command(state, args),
        Commands::Hide(args) => hide::handle_hide_command(state, args),
    }
}
