use clap::Subcommand;
use eyre::{eyre, Result};

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Manage data sources
    Source,
    /// Refresh data
    Refresh,
    /// Do a search
    Search,
}

pub fn handle_command(cmd: Commands) -> Result<()> {
    println!("{cmd:?}");
    match cmd {
        Commands::Refresh => return Err(eyre!("Not implemented yet")),
        Commands::Search => crate::search::search(),
        Commands::Source => return Err(eyre!("Not implemented yet")),
    }
}
