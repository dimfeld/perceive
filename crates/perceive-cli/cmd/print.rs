use clap::Args;
use eyre::Result;

use crate::AppState;

#[derive(Debug, Args)]
pub struct PrintArgs {
    /// The ID of the item to print
    id: i64,

    /// Print the raw content instead of the parsed content
    #[clap(short, long)]
    raw: bool,
}

pub fn handle_print_command(state: &mut AppState, args: PrintArgs) -> Result<()> {
    let item = state.database.read_item(args.id)?;
    let Some(item )= item else {
        println!("No item with ID {} found", args.id);
        return Ok(());
    };

    println!("Item {} - {}", args.id, item.external_id);
    if let Some(skipped) = item.skipped.as_ref() {
        println!("Skipped because: {skipped}");
    }

    if let Some(name) = item.metadata.name.as_ref() {
        println!("Name: {name}");
    }

    if let Some(description) = item.metadata.description.as_ref() {
        println!("Description: {description}");
    }

    if let Some(author) = item.metadata.author.as_ref() {
        println!("Author: {author}");
    }

    if let Some(content) = item.content.as_ref() {
        println!("Content:\n{content}");
    }

    if args.raw {
        let raw_content = item.raw_content.map(|compressed| {
            let buf = zstd::decode_all(compressed.as_slice())?;
            let s = String::from_utf8(buf)?;
            Ok::<_, eyre::Report>(s)
        });

        if let Some(Ok(raw_content)) = raw_content {
            println!("\nRaw content:\n{raw_content}");
        }
    }

    Ok(())
}
