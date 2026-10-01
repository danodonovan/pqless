//! Streams Arrow record batches out as an aligned, plain-text table.
//!
//! Column widths are sized from the header and the first batch, then fixed for
//! the rest of the file so every line stays aligned. Anything wider than its
//! column is truncated with an ellipsis.

use std::fmt::Write as _;
use std::io::Write;

use anyhow::Result;
use arrow::array::RecordBatch;
use arrow::datatypes::Schema;
use arrow::error::ArrowError;
use arrow::util::display::{ArrayFormatter, FormatOptions};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// Widest a column may grow before its values are truncated.
pub const MAX_COL_WIDTH: usize = 40;

/// Separator between columns.
pub const SEP: &str = " │ ";

pub const OPTIONS: FormatOptions<'static> = FormatOptions::new()
    .with_null("null")
    .with_display_error(true);

struct Column {
    width: usize,
    right: bool,
}

/// Writes `batches` to `out` as a table, prefixed by a row-number column.
/// `total_rows` is only used to size that row-number column.
pub fn render<I, W>(schema: &Schema, batches: I, total_rows: usize, out: &mut W) -> Result<()>
where
    I: IntoIterator<Item = Result<RecordBatch, ArrowError>>,
    W: Write,
{
    // Size columns from the first batch that has rows; empty ones print nothing.
    let mut batches = batches.into_iter();
    let first = loop {
        match batches.next().transpose()? {
            Some(b) if b.num_rows() == 0 => continue,
            b => break b.map(|b| format_batch(&b)).transpose()?,
        }
    };

    let index = Column {
        width: total_rows.saturating_sub(1).to_string().len(),
        right: true,
    };
    let columns: Vec<Column> = schema
        .fields()
        .iter()
        .enumerate()
        .map(|(i, field)| {
            let widest = first
                .as_ref()
                .and_then(|cells| cells[i].iter().map(|c| c.width()).max())
                .unwrap_or(0);
            Column {
                width: widest.max(field.name().width()).clamp(1, MAX_COL_WIDTH),
                right: field.data_type().is_numeric(),
            }
        })
        .collect();

    let mut line = String::new();
    let names = schema.fields().iter().map(|f| sanitize(f.name()));
    write_row(&mut line, &index, "#", &columns, names);
    out.write_all(line.as_bytes())?;

    line.clear();
    line.push_str(&"─".repeat(index.width));
    for col in &columns {
        line.push_str("─┼─");
        line.push_str(&"─".repeat(col.width));
    }
    line.push('\n');
    out.write_all(line.as_bytes())?;

    let rest = batches.map(|b| format_batch(&b?));
    let mut row_num = 0usize;
    for cells in first.into_iter().map(Ok).chain(rest) {
        let cells = cells?;
        for r in 0..cells.first().map_or(0, Vec::len) {
            line.clear();
            let values = cells.iter().map(|col| col[r].as_str());
            write_row(&mut line, &index, &row_num.to_string(), &columns, values);
            out.write_all(line.as_bytes())?;
            row_num += 1;
        }
    }
    Ok(())
}

/// Formats every cell of `batch` as sanitised text, column-major.
fn format_batch(batch: &RecordBatch) -> Result<Vec<Vec<String>>> {
    batch
        .columns()
        .iter()
        .map(|array| {
            let fmt = ArrayFormatter::try_new(array.as_ref(), &OPTIONS)?;
            Ok((0..batch.num_rows()).map(|r| cell_text(&fmt, r)).collect())
        })
        .collect()
}

/// The sanitised display text of one value.
pub fn cell_text(fmt: &ArrayFormatter, row: usize) -> String {
    let mut buf = String::new();
    // Errors are rendered inline because display_error is set.
    let _ = write!(buf, "{}", fmt.value(row));
    sanitize(&buf)
}

fn write_row(
    line: &mut String,
    index: &Column,
    row_label: &str,
    columns: &[Column],
    values: impl Iterator<Item = impl AsRef<str>>,
) {
    fit(line, row_label, index.width, index.right);
    for (col, value) in columns.iter().zip(values) {
        line.push_str(SEP);
        fit(line, value.as_ref(), col.width, col.right);
    }
    line.truncate(line.trim_end().len());
    line.push('\n');
}

/// Appends `value` to `line`, padded (on the left if `right` aligned) or
/// truncated to exactly `width` cells.
pub fn fit(line: &mut String, value: &str, width: usize, right: bool) {
    let value_width = value.width();
    if value_width <= width {
        let pad = width - value_width;
        if right {
            line.extend(std::iter::repeat_n(' ', pad));
        }
        line.push_str(value);
        if !right {
            line.extend(std::iter::repeat_n(' ', pad));
        }
        return;
    }
    let budget = width - 1; // leave room for the ellipsis
    let mut used = 0;
    for ch in value.chars() {
        let w = ch.width().unwrap_or(0);
        if used + w > budget {
            break;
        }
        line.push(ch);
        used += w;
    }
    line.push('…');
    line.extend(std::iter::repeat_n(' ', budget - used));
}

/// Makes a value safe to show on one terminal line: escapes common
/// whitespace and replaces other control characters (notably ESC, which a
/// pager in raw mode would otherwise interpret).
pub fn sanitize(s: &str) -> String {
    if !s.chars().any(char::is_control) {
        return s.to_owned();
    }
    let mut out = String::with_capacity(s.len() + 8);
    for ch in s.chars() {
        match ch {
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push(char::REPLACEMENT_CHARACTER),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use arrow::array::{ArrayRef, Float64Array, Int32Array, StringArray};
    use arrow::datatypes::{DataType, Field};

    use super::*;

    fn fitted(value: &str, width: usize, right: bool) -> String {
        let mut s = String::new();
        fit(&mut s, value, width, right);
        s
    }

    #[test]
    fn fit_pads_left_and_right_aligned() {
        assert_eq!(fitted("ab", 4, false), "ab  ");
        assert_eq!(fitted("ab", 4, true), "  ab");
    }

    #[test]
    fn fit_truncates_with_ellipsis() {
        assert_eq!(fitted("abcdef", 4, false), "abc…");
        assert_eq!(fitted("abcdef", 1, false), "…");
    }

    #[test]
    fn fit_respects_wide_characters() {
        // Each CJK char is two cells; only one fits alongside the ellipsis.
        assert_eq!(fitted("日本語", 4, false), "日… ");
        assert_eq!(fitted("日本語", 4, false).width(), 4);
    }

    #[test]
    fn sanitize_escapes_control_characters() {
        assert_eq!(sanitize("a\nb\tc"), "a\\nb\\tc");
        assert_eq!(sanitize("\x1b[31mred"), "\u{fffd}[31mred");
        assert_eq!(sanitize("plain"), "plain");
    }

    fn batch(ids: Vec<Option<i32>>, names: Vec<Option<&str>>) -> RecordBatch {
        RecordBatch::try_from_iter([
            ("id", Arc::new(Int32Array::from(ids)) as ArrayRef),
            ("name", Arc::new(StringArray::from(names)) as ArrayRef),
        ])
        .unwrap()
    }

    fn render_to_string(schema: &Schema, batches: Vec<RecordBatch>, total: usize) -> String {
        let mut out = Vec::new();
        render(schema, batches.into_iter().map(Ok), total, &mut out).unwrap();
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn renders_header_rows_and_nulls_across_batches() {
        let b1 = batch(vec![Some(1), None], vec![Some("alice"), Some("bob")]);
        let b2 = batch(vec![Some(300)], vec![None]);
        let schema = b1.schema();
        let text = render_to_string(&schema, vec![b1, b2], 3);
        assert_eq!(
            text,
            "\
# │   id │ name
──┼──────┼──────
0 │    1 │ alice
1 │ null │ bob
2 │  300 │ null
"
        );
    }

    #[test]
    fn later_wide_values_are_truncated_to_first_batch_width() {
        let b1 = batch(vec![Some(1)], vec![Some("ab")]);
        let b2 = batch(vec![Some(2)], vec![Some("abcdefgh")]);
        let schema = b1.schema();
        let text = render_to_string(&schema, vec![b1, b2], 2);
        // "name" is 4 wide, so the column is 4 wide, not 2.
        assert!(text.ends_with("1 │  2 │ abc…\n"), "{text}");
    }

    #[test]
    fn empty_file_renders_header_only() {
        let schema = Schema::new(vec![Field::new("x", DataType::Float64, true)]);
        let text = render_to_string(&schema, vec![], 0);
        assert_eq!(text, "# │ x\n──┼──\n");
    }

    #[test]
    fn empty_first_batch_does_not_size_columns() {
        let schema = Arc::new(Schema::new(vec![Field::new("x", DataType::Float64, false)]));
        let empty = RecordBatch::try_new(
            schema.clone(),
            vec![Arc::new(Float64Array::from(Vec::<f64>::new()))],
        )
        .unwrap();
        let one = RecordBatch::try_new(
            schema.clone(),
            vec![Arc::new(Float64Array::from(vec![1.5]))],
        )
        .unwrap();
        let text = render_to_string(&schema, vec![empty, one], 1);
        assert!(text.ends_with("0 │ 1.5\n"), "{text}");
    }
}
