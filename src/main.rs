//! `parq <file>` — browse a Parquet file a screenful at a time, like `less`.

mod source;
mod table;
mod viewer;

use std::io::{self, BufWriter, IsTerminal, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Result;
use clap::Parser;

use crate::source::Source;

/// Rows decoded per step when writing the whole table out.
const BATCH_SIZE: usize = 1024;

/// Browse a Parquet file page by page, like `less`.
///
/// Only the rows and columns on screen are decoded, so even very large or
/// very wide files open instantly. Press ? inside for keys. When stdout is not
/// a terminal the whole table is written out as text instead.
#[derive(Parser)]
#[command(version)]
struct Args {
    /// Parquet file to browse
    file: PathBuf,
}

fn main() -> ExitCode {
    match run(Args::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        // The reader quit before the end of the file: that's normal.
        Err(e) if is_broken_pipe(&e) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("parq: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: Args) -> Result<()> {
    let src = Source::open(&args.file)?;
    if io::stdout().is_terminal() {
        let title = args.file.file_name().unwrap_or(args.file.as_os_str());
        return viewer::run(&src, title.to_string_lossy().into_owned());
    }
    let mut out = BufWriter::new(io::stdout().lock());
    table::render(
        src.schema(),
        src.batches(BATCH_SIZE)?,
        src.num_rows(),
        &mut out,
    )?;
    Ok(out.flush()?)
}

fn is_broken_pipe(e: &anyhow::Error) -> bool {
    e.chain()
        .filter_map(|c| c.downcast_ref::<io::Error>())
        .any(|e| e.kind() == io::ErrorKind::BrokenPipe)
}
