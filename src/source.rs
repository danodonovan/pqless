//! Random access to a Parquet file by row range and column subset.
//!
//! Parquet stores each column separately, so reading a few columns touches
//! only their pages. That matters for wide files, where decoding every column
//! just to show the first screen can mean reading most of the file.

use std::fs::File;
use std::ops::Range;
use std::path::Path;

use anyhow::{Context, Result};
use arrow::array::{ArrayRef, RecordBatch, RecordBatchReader};
use arrow::compute::concat_batches;
use arrow::datatypes::SchemaRef;
use parquet::arrow::ProjectionMask;
use parquet::arrow::arrow_reader::{
    ArrowReaderMetadata, ArrowReaderOptions, ParquetRecordBatchReader,
    ParquetRecordBatchReaderBuilder,
};
use parquet::file::metadata::PageIndexPolicy;

pub struct Source {
    file: File,
    meta: ArrowReaderMetadata,
    /// First row of each row group, followed by the total row count.
    row_group_starts: Vec<usize>,
}

impl Source {
    pub fn open(path: &Path) -> Result<Self> {
        let file = File::open(path).with_context(|| format!("cannot open {}", path.display()))?;
        // The offset index, when present, lets jumps skip pages without
        // decompressing them.
        let options = ArrowReaderOptions::new().with_page_index_policy(PageIndexPolicy::Optional);
        let meta = ArrowReaderMetadata::load(&file, options)
            .with_context(|| format!("cannot read {} as Parquet", path.display()))?;
        let mut row_group_starts = vec![0];
        for rg in meta.metadata().row_groups() {
            let last = *row_group_starts.last().unwrap();
            row_group_starts.push(last + rg.num_rows().max(0) as usize);
        }
        Ok(Self {
            file,
            meta,
            row_group_starts,
        })
    }

    pub fn schema(&self) -> &SchemaRef {
        self.meta.schema()
    }

    pub fn num_rows(&self) -> usize {
        *self.row_group_starts.last().unwrap()
    }

    /// Every row of every column, in batches of `batch_size`.
    pub fn batches(&self, batch_size: usize) -> Result<ParquetRecordBatchReader> {
        Ok(self.builder()?.with_batch_size(batch_size).build()?)
    }

    /// Reads `rows` of the top-level `columns` (ascending schema indices),
    /// returning one array per requested column, in order.
    pub fn read(&self, rows: Range<usize>, columns: &[usize]) -> Result<Vec<ArrayRef>> {
        debug_assert!(columns.is_sorted());
        let rows = rows.start..rows.end.min(self.num_rows());
        if rows.is_empty() || columns.is_empty() {
            return Ok(Vec::new());
        }
        let starts = &self.row_group_starts;
        let first = starts.partition_point(|&s| s <= rows.start) - 1;
        let last = starts.partition_point(|&s| s < rows.end) - 1;

        let mask = ProjectionMask::roots(self.meta.parquet_schema(), columns.iter().copied());
        let reader = self
            .builder()?
            .with_projection(mask)
            .with_row_groups((first..=last).collect())
            .with_offset(rows.start - starts[first])
            .with_limit(rows.len())
            .with_batch_size(rows.len())
            .build()?;
        let schema = reader.schema();
        let batches = reader.collect::<Result<Vec<RecordBatch>, _>>()?;
        Ok(concat_batches(&schema, &batches)?.columns().to_vec())
    }

    fn builder(&self) -> Result<ParquetRecordBatchReaderBuilder<File>> {
        Ok(ParquetRecordBatchReaderBuilder::new_with_metadata(
            self.file.try_clone()?,
            self.meta.clone(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use arrow::array::{Array, Int64Array, StringArray};
    use parquet::arrow::ArrowWriter;
    use parquet::file::properties::WriterProperties;

    use super::*;

    /// 25 rows in row groups of 10, with columns a = i, b = "s{i}", c = -i.
    fn sample() -> (tempfile::TempDir, Source) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.parquet");
        let batch = RecordBatch::try_from_iter([
            (
                "a",
                Arc::new(Int64Array::from_iter_values(0..25)) as ArrayRef,
            ),
            (
                "b",
                Arc::new(StringArray::from_iter_values(
                    (0..25).map(|i| format!("s{i}")),
                )),
            ),
            (
                "c",
                Arc::new(Int64Array::from_iter_values((0..25).map(|i| -i))),
            ),
        ])
        .unwrap();
        let props = WriterProperties::builder()
            .set_max_row_group_row_count(Some(10))
            .build();
        let mut w = ArrowWriter::try_new(File::create(&path).unwrap(), batch.schema(), Some(props))
            .unwrap();
        w.write(&batch).unwrap();
        w.close().unwrap();
        let source = Source::open(&path).unwrap();
        (dir, source)
    }

    fn ints(a: &ArrayRef) -> Vec<i64> {
        a.as_any()
            .downcast_ref::<Int64Array>()
            .unwrap()
            .values()
            .to_vec()
    }

    #[test]
    fn counts_rows_across_row_groups() {
        let (_dir, src) = sample();
        assert_eq!(src.num_rows(), 25);
        assert_eq!(src.row_group_starts, vec![0, 10, 20, 25]);
    }

    #[test]
    fn reads_a_window_spanning_row_groups() {
        let (_dir, src) = sample();
        let cols = src.read(8..13, &[0, 2]).unwrap();
        assert_eq!(cols.len(), 2);
        assert_eq!(ints(&cols[0]), vec![8, 9, 10, 11, 12]);
        assert_eq!(ints(&cols[1]), vec![-8, -9, -10, -11, -12]);
    }

    #[test]
    fn reads_within_one_row_group_and_clamps_at_the_end() {
        let (_dir, src) = sample();
        assert_eq!(ints(&src.read(20..21, &[0]).unwrap()[0]), vec![20]);
        let tail = src.read(23..100, &[1]).unwrap();
        assert_eq!(tail[0].len(), 2);
        assert!(src.read(30..40, &[0]).unwrap().is_empty());
    }
}
