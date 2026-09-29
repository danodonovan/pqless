//! `parq <file>` — print a Parquet file as an aligned text table.

mod table;

use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::Parser;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

/// Rows decoded per step. Decoding is lazy, so only as much of the file is
/// read as the reader downstream has consumed.
const BATCH_SIZE: usize = 1024;

/// Print a Parquet file as an aligned text table.
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
    let file =
        File::open(&args.file).with_context(|| format!("cannot open {}", args.file.display()))?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)
        .with_context(|| format!("cannot read {} as Parquet", args.file.display()))?;
    let total_rows = builder.metadata().file_metadata().num_rows().max(0) as usize;
    let schema = builder.schema().clone();
    let batches = builder.with_batch_size(BATCH_SIZE).build()?;

    let mut out = BufWriter::new(io::stdout().lock());
    table::render(&schema, batches, total_rows, &mut out)?;
    Ok(out.flush()?)
}

fn is_broken_pipe(e: &anyhow::Error) -> bool {
    e.chain()
        .filter_map(|c| c.downcast_ref::<io::Error>())
        .any(|e| e.kind() == io::ErrorKind::BrokenPipe)
}
