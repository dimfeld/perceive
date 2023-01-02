use clap::Args;
use eyre::Result;

use crate::AppState;

#[derive(Debug, Args)]
pub struct HideArgs {
    /// The ID of the item to hide
    id: i64,
    /// Set to unhide the item instead
    #[clap(short, long)]
    unhide: bool,
}

pub fn handle_hide_command(state: &mut AppState, args: HideArgs) -> Result<()> {
    state.database.set_item_hidden(args.id, true)?;
    Ok(())
}
