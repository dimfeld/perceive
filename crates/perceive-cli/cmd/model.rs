use clap::{Args, Subcommand};
use eyre::eyre;
use perceive_core::model::SentenceEmbeddingsModelType;

use crate::AppState;

#[derive(Debug, Args)]
pub struct ModelArgs {
    #[clap(subcommand)]
    command: ModelCommand,
}

#[derive(Debug, Subcommand)]
pub enum ModelCommand {
    /// Set
    Set(SetArgs),
}

#[derive(Debug, Args)]
pub struct SetArgs {
    model: SentenceEmbeddingsModelType,
}

pub fn handle_model_command(state: &mut AppState, cmd: ModelArgs) -> eyre::Result<()> {
    match cmd.command {
        ModelCommand::Set(args) => set_model(state, args),
    }
}

fn set_model(_state: &mut AppState, _args: SetArgs) -> eyre::Result<()> {
    Err(eyre!("Unimplemented"))
}
