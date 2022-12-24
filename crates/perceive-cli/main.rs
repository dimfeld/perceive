mod cmd;
mod repl;
mod state;

use clap::Parser;
use cmd::Commands;
use eyre::Result;
pub use state::AppState;

#[derive(Parser, Debug)]
#[command(about)]
pub struct Args {
    #[clap(subcommand)]
    pub command: Option<Commands>,
}

fn main() -> Result<()> {
    color_eyre::install().unwrap();

    let args = Args::parse();
    let mut state = AppState::new();

    match args.command {
        Some(cmd) => cmd::handle_command(&mut state, cmd)?,
        None => repl::repl(state)?,
    };

    Ok(())
}
