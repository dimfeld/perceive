use std::{path::Path, sync::atomic::AtomicBool};

use clap::{Args, Subcommand};
use eyre::eyre;
use indicatif::ProgressBar;
use perceive_core::sources::{
    db::update_source, import::ScanStats, FsSourceConfig, Source, SourceConfig,
};
use time::OffsetDateTime;

use crate::AppState;

#[derive(Debug, Args)]
pub struct SourceArgs {
    #[clap(subcommand)]
    pub command: SourceCommand,
}

#[derive(Debug, Subcommand)]
pub enum SourceCommand {
    Add(AddSourceArgs),
    Edit(EditSourceArgs),
    Scan(ScanSourceArgs),
}

#[derive(Debug, Args)]
pub struct AddSourceArgs {
    /// The name of the source
    pub name: String,
    /// The location of the source
    pub location: String,
    /// The globs for files to include
    #[clap(required = true)]
    pub globs: Vec<String>,
}

#[derive(Debug, Args)]
pub struct EditSourceArgs {
    /// The name of the source
    pub name: String,
}

#[derive(Debug, Args)]
pub struct ScanSourceArgs {
    /// The name of the source
    pub name: String,
}

pub fn handle_source_command(state: &mut AppState, cmd: SourceArgs) -> eyre::Result<()> {
    match cmd.command {
        SourceCommand::Add(args) => add_source(state, args),
        SourceCommand::Edit(args) => todo!(),
        SourceCommand::Scan(args) => scan_source(state, args),
    }
}

fn add_source(state: &mut AppState, args: AddSourceArgs) -> eyre::Result<()> {
    let now = OffsetDateTime::now_utc();

    let location = shellexpand::tilde(&args.location).into_owned();
    let is_dir = std::fs::metadata(Path::new(&location))
        .map(|m| m.is_dir())
        .unwrap_or(false);

    if !is_dir {
        return Err(eyre!("Location must be a directory"));
    }

    let source = Source {
        id: 0, // filled in later
        name: args.name,
        location,
        config: SourceConfig::Fs(FsSourceConfig {
            globs: args.globs,
            overrides: Vec::new(),
        }),
        compare_strategy: perceive_core::sources::ItemCompareStrategy::MTimeAndContent,
        status: perceive_core::sources::SourceStatus::Indexing {
            started_at: now.unix_timestamp(),
        },
        last_indexed: now,
        index_version: 0,
    };

    let source = perceive_core::sources::db::add_source(&state.database, source)?;
    state.sources.push(source);
    Ok(())
}

fn scan_source(state: &mut AppState, args: ScanSourceArgs) -> eyre::Result<()> {
    let source_pos = state
        .sources
        .iter_mut()
        .position(|s| s.name == args.name)
        .ok_or_else(|| eyre!("Source not found"))?;

    let now = OffsetDateTime::now_utc();

    {
        let source = &mut state.sources[source_pos];
        source.index_version += 1;
        source.last_indexed = now;
        source.status = perceive_core::sources::SourceStatus::Indexing {
            started_at: now.unix_timestamp(),
        };
    }

    update_source(&state.database, &state.sources[source_pos])?;

    let times = ScanStats::default();

    let done = AtomicBool::new(false);
    std::thread::scope(|scope| {
        scope.spawn(|| {
            let scanned_progress = ProgressBar::new_spinner();

            let update_progress = || {
                let scanned = times.scanned.load(std::sync::atomic::Ordering::Relaxed);
                let encoded = times.encoded.load(std::sync::atomic::Ordering::Relaxed);
                let added = times.added.load(std::sync::atomic::Ordering::Relaxed);
                let changed = times.changed.load(std::sync::atomic::Ordering::Relaxed);
                let unchanged = times.unchanged.load(std::sync::atomic::Ordering::Relaxed);

                scanned_progress.set_message(format!(
                    "Scanned: {scanned} Encoded: {encoded} Added: {added} Changed: {changed} Unchanged: {unchanged}",
                ));
            };

            while !done.load(std::sync::atomic::Ordering::Relaxed) {
                update_progress();
                scanned_progress.tick();
                std::thread::sleep(std::time::Duration::from_millis(100));
            }

            update_progress();
            scanned_progress.finish();
        });

        let model = state.loan_model();

        let model = perceive_core::sources::import::scan_source(
            &times,
            &state.database,
            model,
            0,
            0,
            &state.sources[source_pos],
        );

        state.return_model(model);
        done.store(true, std::sync::atomic::Ordering::Relaxed);
    });

    let source = &mut state.sources[source_pos];
    source.status = perceive_core::sources::SourceStatus::Ready {
        scanned: times.scanned.load(std::sync::atomic::Ordering::Relaxed) as u32,
        duration: (OffsetDateTime::now_utc().unix_timestamp() - now.unix_timestamp()) as u32,
    };
    update_source(&state.database, source)?;

    println!("Done");

    Ok(())
}
