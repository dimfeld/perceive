mod cmd;
mod repl;
mod search;

use clap::Parser;
use cmd::Commands;
use eyre::Result;

#[derive(Parser, Debug)]
#[command(about)]
pub struct Args {
    #[clap(subcommand)]
    pub command: Option<Commands>,
}

fn main() -> Result<()> {
    color_eyre::install().unwrap();

    let args = Args::parse();
    println!("{:?}", args);

    match args.command {
        Some(cmd) => cmd::handle_command(cmd)?,
        None => repl::repl()?,
    };

    Ok(())

    // test()
}
