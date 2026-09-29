use std::fs::File;
use std::path::Path;
use std::process::{Command, Output};
use std::sync::Arc;

use arrow::array::{ArrayRef, Int64Array, RecordBatch, StringArray};
use parquet::arrow::ArrowWriter;
use parquet::file::properties::WriterProperties;

fn parq(path: &Path) -> Output {
    // stdout is a pipe here, so parq writes directly instead of paging.
    Command::new(env!("CARGO_BIN_EXE_parq"))
        .arg(path)
        .output()
        .unwrap()
}

fn write_parquet(path: &Path, rows: i64, row_group_size: usize) {
    let ids = Int64Array::from_iter_values(0..rows);
    let names = StringArray::from_iter_values((0..rows).map(|i| format!("row-{i}")));
    let batch = RecordBatch::try_from_iter([
        ("id", Arc::new(ids) as ArrayRef),
        ("name", Arc::new(names) as ArrayRef),
    ])
    .unwrap();
    let props = WriterProperties::builder()
        .set_max_row_group_row_count(Some(row_group_size))
        .build();
    let mut writer =
        ArrowWriter::try_new(File::create(path).unwrap(), batch.schema(), Some(props)).unwrap();
    writer.write(&batch).unwrap();
    writer.close().unwrap();
}

#[test]
fn prints_every_row_across_row_groups() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.parquet");
    write_parquet(&path, 2500, 1000);

    let out = parq(&path);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8(out.stdout).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 2 + 2500);
    assert_eq!(lines[0], "   # │   id │ name");
    assert_eq!(lines[2], "   0 │    0 │ row-0");
    assert_eq!(lines[2501], "2499 │ 2499 │ row-2499");
}

#[test]
fn quitting_early_is_not_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.parquet");
    write_parquet(&path, 200_000, 50_000);

    let out = Command::new("sh")
        .arg("-c")
        .arg(format!(
            "'{}' '{}' | head -n 3",
            env!("CARGO_BIN_EXE_parq"),
            path.display()
        ))
        .output()
        .unwrap();
    assert!(out.status.success());
    assert_eq!(String::from_utf8(out.stdout).unwrap().lines().count(), 3);
    assert_eq!(String::from_utf8_lossy(&out.stderr), "");
}

#[test]
fn reports_non_parquet_input() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("not.parquet");
    std::fs::write(&path, "hello").unwrap();

    let out = parq(&path);
    assert!(!out.status.success());
    let err = String::from_utf8(out.stderr).unwrap();
    assert!(err.starts_with("parq: cannot read"), "{err}");
}
