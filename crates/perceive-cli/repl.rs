use std::io::Write;

use clap::{ArgMatches, Command, CommandFactory, FromArgMatches};
use eyre::eyre;
use rustyline::{error::ReadlineError, Editor};
use thiserror::Error;

use crate::{AppState, Args};

#[derive(Error, Debug)]
enum ReplError {
    #[error("Failed to parse mismatched quotes")]
    InvalidQuoting,

    #[error(transparent)]
    ParseError(#[from] clap::Error),

    #[error(transparent)]
    Io(#[from] std::io::Error),
}

fn command() -> Command {
    let base_cmd = crate::Args::command();

    base_cmd
        .arg_required_else_help(true)
        .subcommand_required(true)
        .subcommand(Command::new("exit").alias("quit").about("Exit the REPL"))
    // .subcommand(ModelArgs::augment_args(Command::new("model")))
}

fn to_args<T: FromArgMatches>(matches: &ArgMatches) -> Result<T, clap::Error> {
    <T as FromArgMatches>::from_arg_matches(matches).map_err(|e| {
        let mut cmd = command();
        e.format(&mut cmd)
    })
}

pub fn repl(mut state: AppState) -> Result<(), eyre::Report> {
    let history_path = perceive_core::paths::PROJECT_DIRS
        .data_local_dir()
        .join("repl-history.txt");
    let mut rl = Editor::<()>::new()?;
    rl.load_history(&history_path).ok();

    loop {
        let input = rl.readline("> ");

        let line = match &input {
            Ok(line) => {
                let line = line.trim();
                rl.add_history_entry(line);
                line
            }
            Err(ReadlineError::Interrupted) | Err(ReadlineError::Eof) => {
                rl.save_history(&history_path).ok();
                break;
            }
            Err(e) => return Err(eyre!("{}", e)),
        };

        if line.is_empty() {
            continue;
        }

        rl.add_history_entry(line);

        match parse(line) {
            Ok(matches) => {
                let result = match matches.subcommand() {
                    Some(("quit" | "exit", _)) => {
                        rl.save_history(&history_path).ok();
                        break;
                    }
                    // Some(("model", matches)) => {
                    //     let args = to_args::<crate::cmd::model::ModelArgs>(matches)?;
                    //     crate::cmd::model::model(&mut state, args)
                    // }
                    _ => {
                        let args = to_args::<Args>(&matches)?;

                        if let Some(command) = args.command {
                            crate::cmd::handle_command(&mut state, command)
                        } else {
                            Ok(())
                        }
                    }
                };

                if let Err(e) = result {
                    println!("Error: {e}");
                }
            }
            Err(err) => {
                write!(std::io::stdout(), "{}", err)?;
                std::io::stdout().flush()?;
            }
        }
    }

    Ok(())
}

fn parse(line: &str) -> Result<clap::ArgMatches, ReplError> {
    let mut args = shlex::split(line).ok_or(ReplError::InvalidQuoting)?;

    // This is a dumb way to fulfill the need for clap to have the app name first.
    // There is probably some better solution.
    if args[0] != "perceive" {
        args.insert(0, "perceive".to_string());
    }

    command()
        .try_get_matches_from(args)
        .map_err(ReplError::from)
}
