use std::{path::Path, sync::atomic::AtomicBool, time::Duration};

use clap::{Args, Subcommand};
use eyre::{eyre, Result};
use indicatif::ProgressBar;
use perceive_core::sources::{
    db::update_source, pipeline::ScanStats, ChromiumBookmarksConfig, ChromiumHistoryConfig,
    FsSourceConfig, ItemCompareStrategy, Source, SourceConfig,
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
    RebuildSearch(RebuildSearchArgs),
    Reprocess(ReprocessArgs),
    Scan(ScanSourceArgs),
}

#[derive(Debug, Args)]
pub struct AddSourceArgs {
    /// The name of the source
    pub name: String,

    #[clap(subcommand)]
    /// The type of the source, and additional information specific to each source
    pub source_type: SourceTypeArgs,
}

#[derive(Debug, Subcommand)]
pub enum SourceTypeArgs {
    /// A filesystem scanner
    Fs(FsSourceTypeArgs),
    /// Read items from the browser history
    BrowserHistory(BrowserHistorySourceTypeArgs),
    Bookmarks(BookmarksSourceTypeArgs),
}

#[derive(Debug, Args)]
pub struct FsSourceTypeArgs {
    /// The location of the source
    pub location: String,
    /// The globs for files to include
    #[clap(required = true)]
    pub globs: Vec<String>,
}

#[derive(Debug, Args)]
pub struct BrowserHistorySourceTypeArgs {
    /// The directory containing the history database
    ///
    /// In the future this will be expressible as a browser type, from which we'll infer the location.
    pub location: String,

    /// Domains that should be skipped.
    #[clap(long)]
    pub skip: Vec<String>,
}

#[derive(Debug, Args)]
pub struct BookmarksSourceTypeArgs {
    /// The directory containing the bookmarks file
    ///
    /// In the future this will be expressible as a browser type, from which we'll infer the location.
    pub location: String,

    /// Domains that should be skipped.
    #[clap(long)]
    pub skip: Vec<String>,
}

#[derive(Debug, Args)]
pub struct EditSourceArgs {
    /// The name of the source
    pub name: String,
}

#[derive(Debug, Args)]
pub struct RebuildSearchArgs {
    /// The name of the source
    pub name: String,
}

#[derive(Debug, Args)]
pub struct ReprocessArgs {
    /// The name of the source
    pub name: String,
}

#[derive(Debug, Args)]
pub struct ScanSourceArgs {
    /// The name of the source
    pub name: String,
    /// Set to always check by content, even if the dates are the same.
    #[clap(long)]
    pub by_content: bool,
    /// Reindex all items, even if they haven't changed.
    #[clap(short, long, conflicts_with("by_content"))]
    pub force: bool,
}

pub fn handle_source_command(state: &mut AppState, cmd: SourceArgs) -> eyre::Result<()> {
    match cmd.command {
        SourceCommand::Add(args) => add_source(state, args),
        SourceCommand::Edit(args) => Err(eyre!("Not implemented yet")),
        SourceCommand::RebuildSearch(args) => rebuild_search(state, args),
        SourceCommand::Reprocess(args) => reprocess_source(state, args),
        SourceCommand::Scan(args) => scan_source(state, args),
    }
}

fn add_source(state: &mut AppState, args: AddSourceArgs) -> eyre::Result<()> {
    match args.source_type {
        SourceTypeArgs::Fs(cmdargs) => add_fs_source(state, args.name, cmdargs),
        SourceTypeArgs::BrowserHistory(cmdargs) => {
            add_browser_history_source(state, args.name, cmdargs)
        }
        SourceTypeArgs::Bookmarks(cmdargs) => add_bookmarks_source(state, args.name, cmdargs),
    }
}

fn add_fs_source(state: &mut AppState, name: String, args: FsSourceTypeArgs) -> eyre::Result<()> {
    let now = OffsetDateTime::now_utc();
    let location = shellexpand::tilde(&args.location).into_owned();
    let is_dir = std::fs::metadata(Path::new(&location))
        .map(|m| m.is_dir())
        .unwrap_or(false);

    if !is_dir {
        return Err(eyre!("Location must be a directory"));
    }

    let source = Source {
        id: 0, // filled in by add_source
        name,
        location,
        config: SourceConfig::Fs(FsSourceConfig { globs: args.globs }),
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

fn add_browser_history_source(
    state: &mut AppState,
    name: String,
    args: BrowserHistorySourceTypeArgs,
) -> eyre::Result<()> {
    let now = OffsetDateTime::now_utc();
    let location = shellexpand::tilde(&args.location).into_owned();
    let has_history = std::fs::metadata(Path::new(&location).join("History"))
        .map(|m| m.is_file())
        .unwrap_or(false);

    if !has_history {
        return Err(eyre!(
            "Location must be a directory containing a History file"
        ));
    }

    let source = Source {
        id: 0, // filled in by add_source
        name,
        location,
        config: SourceConfig::ChromiumHistory(ChromiumHistoryConfig { skip: args.skip }),
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

fn add_bookmarks_source(
    state: &mut AppState,
    name: String,
    args: BookmarksSourceTypeArgs,
) -> eyre::Result<()> {
    let now = OffsetDateTime::now_utc();
    let location = shellexpand::tilde(&args.location).into_owned();
    let has_history = std::fs::metadata(Path::new(&location).join("Bookmarks"))
        .map(|m| m.is_file())
        .unwrap_or(false);

    if !has_history {
        return Err(eyre!(
            "Location must be a directory containing a Bookmarks file"
        ));
    }

    let source = Source {
        id: 0, // filled in by add_source
        name,
        location,
        config: SourceConfig::ChromiumBookmarks(ChromiumBookmarksConfig { skip: args.skip }),
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
    let start_time = std::time::Instant::now();

    let done = AtomicBool::new(false);
    std::thread::scope(|scope| {
        scope.spawn(|| {
            let scanned_progress = ProgressBar::new_spinner();

            let update_progress = || {
                let scanned = times.scanned.load(std::sync::atomic::Ordering::Relaxed);
                let fetched = times.fetched.load(std::sync::atomic::Ordering::Relaxed);
                let reading = times.reading.load(std::sync::atomic::Ordering::Relaxed);
                let embedding = times.embedding.load(std::sync::atomic::Ordering::Relaxed);
                let encoded = times.encoded.load(std::sync::atomic::Ordering::Relaxed);
                let added = times.added.load(std::sync::atomic::Ordering::Relaxed);
                let changed = times.changed.load(std::sync::atomic::Ordering::Relaxed);
                let unchanged = times.unchanged.load(std::sync::atomic::Ordering::Relaxed);

                scanned_progress.set_message(format!(
                    "Scanned: {scanned} Fetched: {fetched} Fetching: {reading} Encoding: {embedding} Encoded: {encoded} Added: {added} Changed: {changed} Unchanged: {unchanged}",
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

        let compare_strategy = if args.force {
            Some(ItemCompareStrategy::Force)
        } else if args.by_content {
            Some(ItemCompareStrategy::Content)
        } else {
            None
        };

        let result = perceive_core::sources::scan_source(
            &times,
            &state.database,
            &state.model,
            state.model_id,
            state.model_version,
            &state.sources[source_pos],
            compare_strategy,
        );

        done.store(true, std::sync::atomic::Ordering::Relaxed);
        result
    })?;

    let source = &mut state.sources[source_pos];
    source.status = perceive_core::sources::SourceStatus::Ready {
        scanned: times.scanned.load(std::sync::atomic::Ordering::Relaxed) as u32,
        duration: (OffsetDateTime::now_utc().unix_timestamp() - now.unix_timestamp()) as u32,
    };
    update_source(&state.database, source)?;

    println!("Finished in {} seconds", start_time.elapsed().as_secs());

    rebuild_search(state, RebuildSearchArgs { name: args.name })
}

fn rebuild_search(state: &mut AppState, args: RebuildSearchArgs) -> Result<()> {
    let source = state
        .sources
        .iter()
        .find(|s| s.name == args.name)
        .ok_or_else(|| eyre!("Source not found"))?;

    let progress = indicatif::ProgressBar::new_spinner().with_message("Rebuilding search...");
    progress.enable_steady_tick(Duration::from_millis(200));
    let start_time = std::time::Instant::now();

    state.searcher.rebuild_source(
        &state.database,
        source.id,
        state.model_id,
        state.model_version,
    )?;

    let final_msg = format!(
        "Rebuilt source search in {} seconds\n",
        start_time.elapsed().as_secs()
    );
    progress.finish_with_message(final_msg);

    Ok(())
}

fn reprocess_source(state: &mut AppState, args: ReprocessArgs) -> Result<()> {
    let source_pos = state
        .sources
        .iter()
        .position(|s| s.name == args.name)
        .ok_or_else(|| eyre!("Source not found"))?;

    let times = ScanStats::default();

    let done = AtomicBool::new(false);
    std::thread::scope(|scope| {
        scope.spawn(|| {
            let scanned_progress = ProgressBar::new_spinner();

            let update_progress = || {
                let scanned = times.scanned.load(std::sync::atomic::Ordering::Relaxed);
                let reading = times.reading.load(std::sync::atomic::Ordering::Relaxed);
                let fetched = times.fetched.load(std::sync::atomic::Ordering::Relaxed);
                let embedding = times.embedding.load(std::sync::atomic::Ordering::Relaxed);
                let encoded = times.encoded.load(std::sync::atomic::Ordering::Relaxed);
                let added = times.added.load(std::sync::atomic::Ordering::Relaxed);
                let changed = times.changed.load(std::sync::atomic::Ordering::Relaxed);
                let unchanged = times.unchanged.load(std::sync::atomic::Ordering::Relaxed);

                scanned_progress.set_message(format!(
                    "Scanned: {scanned} Processed: {reading} Updated: {fetched}  Encoding: {embedding} Encoded: {encoded} Added: {added} Changed: {changed} Unchanged: {unchanged}",
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

        let reprocess_result = perceive_core::sources::pipeline::reprocess_source(
            &times,
            &state.database,
            &state.model,
            state.model_id,
            state.model_version,
            &state.sources[source_pos],
        );
        done.store(true, std::sync::atomic::Ordering::Relaxed);

        reprocess_result
    })?;

    rebuild_search(state, RebuildSearchArgs { name: args.name })
}
