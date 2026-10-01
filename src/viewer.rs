//! Interactive, `less`-style viewer that decodes only what is on screen.
//!
//! Values are read in blocks of [`BLOCK_ROWS`] rows for just the visible
//! columns, so the first screen of even a very wide file needs only a few
//! pages, and scrolling sideways fetches columns as they come into view.

use std::collections::HashMap;
use std::ops::Range;

use anyhow::Result;
use arrow::array::{Array, ArrayRef};
use arrow::util::display::ArrayFormatter;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};
use ratatui::{DefaultTerminal, Frame};
use unicode_width::UnicodeWidthStr;

use crate::source::Source;
use crate::table::{self, MAX_COL_WIDTH, OPTIONS, SEP};

/// Rows decoded per column per read: quick on any file, yet large enough
/// that scrolling rarely has to wait.
const BLOCK_ROWS: usize = 1024;

/// Column names wider than this are truncated in the header; the full name
/// of the current column is always in the status bar.
const MAX_HEADER_WIDTH: usize = 24;

/// Decoded column blocks kept before the cache is dropped and refilled with
/// just what is on screen.
const CACHE_LIMIT: usize = 4096;

/// Header, rule and status lines around the rows.
const CHROME_LINES: u16 = 3;

const HELP: &[(&str, &str)] = &[
    ("j  ↓  Enter", "down a row"),
    ("k  ↑  y", "up a row"),
    ("space  f  PgDn", "down a page"),
    ("b  PgUp", "up a page"),
    ("d  /  u", "down / up half a page"),
    ("g  Home", "first row"),
    ("G  End", "last row"),
    ("N g", "go to row N"),
    ("l  →  /  h  ←", "next / previous column"),
    ("$  /  0", "last / first column"),
    ("N <key>", "repeat a move N times"),
    ("?", "show / hide this help"),
    ("q  Esc", "quit"),
];

pub fn run(src: &Source, title: String) -> Result<()> {
    let mut terminal = ratatui::try_init()?;
    let result = Viewer::new(src, title).event_loop(&mut terminal);
    ratatui::restore();
    result
}

pub struct Viewer<'a> {
    src: &'a Source,
    title: String,
    /// First row on screen.
    top: usize,
    /// First column on screen.
    left: usize,
    /// The highlighted column.
    col: usize,
    /// A numeric prefix typed so far, as in `less`.
    count: Option<usize>,
    /// Display width of each column; grows as wider values are seen.
    widths: Vec<usize>,
    right_align: Vec<bool>,
    names: Vec<String>,
    /// Decoded values keyed by (row block, column).
    cache: HashMap<(usize, usize), ArrayRef>,
    /// Row lines in the last frame.
    page: usize,
    /// Width available to data columns in the last frame.
    width: usize,
    help: bool,
    error: Option<String>,
}

impl<'a> Viewer<'a> {
    pub fn new(src: &'a Source, title: String) -> Self {
        let fields = src.schema().fields();
        let names: Vec<String> = fields.iter().map(|f| table::sanitize(f.name())).collect();
        Self {
            src,
            title,
            top: 0,
            left: 0,
            col: 0,
            count: None,
            widths: names
                .iter()
                .map(|n| n.width().clamp(1, MAX_HEADER_WIDTH))
                .collect(),
            right_align: fields.iter().map(|f| f.data_type().is_numeric()).collect(),
            names,
            cache: HashMap::new(),
            page: 0,
            width: 0,
            help: false,
            error: None,
        }
    }

    fn event_loop(&mut self, terminal: &mut DefaultTerminal) -> Result<()> {
        loop {
            terminal.draw(|frame| self.draw(frame))?;
            if let Event::Key(key) = event::read()?
                && key.kind == KeyEventKind::Press
                && self.on_key(key)
            {
                return Ok(());
            }
        }
    }

    /// Handles a key press, returning true to quit.
    pub fn on_key(&mut self, key: KeyEvent) -> bool {
        use KeyCode::*;

        if self.help {
            self.help = false;
            return false;
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if let Char(d @ '0'..='9') = key.code
            && !ctrl
            && (d != '0' || self.count.is_some())
        {
            let digit = d.to_digit(10).unwrap() as usize;
            self.count = Some(
                self.count
                    .unwrap_or(0)
                    .saturating_mul(10)
                    .saturating_add(digit),
            );
            return false;
        }
        let count = self.count.take();
        let n = count.unwrap_or(1);
        let page = self.page.max(1);
        let last_col = self.names.len().saturating_sub(1);

        match (key.code, ctrl) {
            (Esc, _) if count.is_some() => {}
            (Char('q'), false) | (Esc, _) | (Char('c'), true) => return true,
            (Char('j' | 'e'), _) | (Down | Enter, _) | (Char('n'), true) => self.down(n),
            (Char('k' | 'y'), _) | (Up, _) | (Char('p'), true) => self.up(n),
            (Char(' ' | 'f' | 'z'), _) | (PageDown, _) | (Char('v'), true) => {
                self.down(n.saturating_mul(page));
            }
            (Char('b' | 'w'), _) | (PageUp, _) => self.up(n.saturating_mul(page)),
            (Char('d'), _) => self.down(n.saturating_mul((page / 2).max(1))),
            (Char('u'), _) => self.up(n.saturating_mul((page / 2).max(1))),
            (Char('g' | '<'), false) | (Home, _) => self.top = count.unwrap_or(0),
            (Char('G' | '>'), false) | (End, _) => self.top = count.unwrap_or(usize::MAX),
            (Char('h'), false) | (Left, _) => self.col = self.col.saturating_sub(n),
            (Char('l'), false) | (Right, _) => self.col = self.col.saturating_add(n).min(last_col),
            (Char('0' | '^'), false) => self.col = 0,
            (Char('$'), false) => self.col = last_col,
            (Char('?'), false) => self.help = true,
            _ => {}
        }
        self.clamp_top();
        self.reveal_col();
        false
    }

    fn down(&mut self, n: usize) {
        self.top = self.top.saturating_add(n);
    }

    fn up(&mut self, n: usize) {
        self.top = self.top.saturating_sub(n);
    }

    fn clamp_top(&mut self) {
        self.top = self.top.min(self.src.num_rows().saturating_sub(self.page));
    }

    fn rows(&self) -> Range<usize> {
        self.top..(self.top + self.page).min(self.src.num_rows())
    }

    /// Columns from `left` that fit on screen, the last possibly cut off.
    fn visible_columns(&self) -> Range<usize> {
        let mut used = 0;
        let mut end = self.left;
        while end < self.widths.len() && used < self.width {
            used += SEP.width() + self.widths[end];
            end += 1;
        }
        self.left..end
    }

    /// Scrolls sideways just enough to show the current column in full.
    fn reveal_col(&mut self) {
        if self.col <= self.left || self.widths.is_empty() {
            self.left = self.col;
            return;
        }
        // Walk back from the current column while columns still fit; only
        // what fits on one screen is ever visited.
        let cost = |c: usize| SEP.width() + self.widths[c];
        let mut used = cost(self.col);
        let mut first = self.col;
        while first > self.left && used + cost(first - 1) <= self.width {
            first -= 1;
            used += cost(first);
        }
        self.left = first;
    }

    pub fn draw(&mut self, frame: &mut Frame) {
        self.prepare(frame.area());
        self.render(frame);
    }

    /// Lays out the screen for `area`, decoding whatever it shows.
    fn prepare(&mut self, area: Rect) {
        self.page = area.height.saturating_sub(CHROME_LINES) as usize;
        self.width = (area.width as usize).saturating_sub(self.index_width());
        self.clamp_top();
        if self.cache.len() > CACHE_LIMIT {
            self.cache.clear();
        }
        // Loading can widen columns, which can scroll new ones into view.
        for _ in 0..8 {
            self.reveal_col();
            if !self.load(self.rows(), self.visible_columns()) {
                break;
            }
        }
    }

    /// Decodes any blocks of `cols` covering `rows` not already cached.
    /// Returns whether anything new was loaded.
    fn load(&mut self, rows: Range<usize>, cols: Range<usize>) -> bool {
        if rows.is_empty() {
            return false;
        }
        let mut loaded = false;
        for block in rows.start / BLOCK_ROWS..=(rows.end - 1) / BLOCK_ROWS {
            let missing: Vec<usize> = cols
                .clone()
                .filter(|c| !self.cache.contains_key(&(block, *c)))
                .collect();
            if missing.is_empty() {
                continue;
            }
            let start = block * BLOCK_ROWS;
            match self.src.read(start..start + BLOCK_ROWS, &missing) {
                Ok(arrays) => {
                    for (c, array) in missing.into_iter().zip(arrays) {
                        self.widths[c] = self.widths[c].max(values_width(&array));
                        self.cache.insert((block, c), array);
                    }
                    loaded = true;
                }
                Err(e) => {
                    self.error = Some(format!("{e:#}"));
                    return false;
                }
            }
        }
        loaded
    }

    fn index_width(&self) -> usize {
        self.src.num_rows().saturating_sub(1).to_string().len()
    }

    fn render(&self, frame: &mut Frame) {
        let [body, status] =
            Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).areas(frame.area());
        let rows = self.rows();
        let cols = self.visible_columns();
        let index_width = self.index_width();
        let dim = Style::new().dim();

        let mut header = vec![Span::raw(fitted("#", index_width, true))];
        let mut rule = "─".repeat(index_width);
        for c in cols.clone() {
            header.push(Span::styled(SEP, dim));
            let name =
                Span::raw(fitted(&self.names[c], self.widths[c], self.right_align[c])).bold();
            header.push(if c == self.col { name.reversed() } else { name });
            rule.push_str("─┼─");
            rule.push_str(&"─".repeat(self.widths[c]));
        }
        let mut lines = vec![Line::from(header), Line::styled(rule, dim)];

        let formatters: HashMap<(usize, usize), ArrayFormatter> = self
            .cache
            .iter()
            .filter(|((block, c), _)| {
                cols.contains(c)
                    && (rows.start / BLOCK_ROWS..=rows.end / BLOCK_ROWS).contains(block)
            })
            .filter_map(|(key, a)| {
                Some((*key, ArrayFormatter::try_new(a.as_ref(), &OPTIONS).ok()?))
            })
            .collect();
        for r in rows.clone() {
            let mut spans = vec![Span::styled(fitted(&r.to_string(), index_width, true), dim)];
            let (block, offset) = (r / BLOCK_ROWS, r % BLOCK_ROWS);
            for c in cols.clone() {
                spans.push(Span::styled(SEP, dim));
                let array = self.cache.get(&(block, c));
                let text = formatters
                    .get(&(block, c))
                    .map_or_else(String::new, |f| table::cell_text(f, offset));
                let cell = Span::raw(fitted(&text, self.widths[c], self.right_align[c]));
                spans.push(match array {
                    Some(a) if a.is_null(offset) => cell.dim(),
                    _ if c == self.col => cell.bold(),
                    _ => cell,
                });
            }
            lines.push(Line::from(spans));
        }
        frame.render_widget(Paragraph::new(lines), body);
        self.render_status(frame, status, rows);
        if self.help {
            render_help(frame);
        }
    }

    fn render_status(&self, frame: &mut Frame, area: Rect, rows: Range<usize>) {
        let total = self.src.num_rows();
        let mut left = format!(" {}  ", self.title);
        left += &match total {
            0 => "no rows".to_owned(),
            _ => format!(
                "rows {}–{} of {}",
                group(rows.start),
                group(rows.end.saturating_sub(1)),
                group(total)
            ),
        };
        if let Some(field) = self.src.schema().fields().get(self.col) {
            left += &format!(
                "  col {}/{}  {}: {}",
                group(self.col + 1),
                group(self.names.len()),
                self.names[self.col],
                field.data_type()
            );
        }
        let right = match (&self.count, &self.error) {
            (Some(n), _) => format!(" {n} "),
            (None, Some(e)) => format!(" error: {e} "),
            (None, None) => " ? help  q quit ".to_owned(),
        };
        let [l, r] = Layout::horizontal([
            Constraint::Fill(1),
            Constraint::Length(right.width() as u16),
        ])
        .areas(area);
        frame.render_widget(Paragraph::new(left).reversed(), l);
        let right = Paragraph::new(right).reversed();
        frame.render_widget(
            if self.error.is_some() {
                right.red()
            } else {
                right
            },
            r,
        );
    }
}

fn render_help(frame: &mut Frame) {
    let keys_width = HELP.iter().map(|(k, _)| k.width()).max().unwrap_or(0);
    let lines: Vec<Line> = HELP
        .iter()
        .map(|(keys, action)| {
            Line::from(vec![
                Span::raw(format!(" {} ", fitted(keys, keys_width, false))).bold(),
                Span::raw(format!(" {action} ")),
            ])
        })
        .collect();
    let width = lines.iter().map(Line::width).max().unwrap_or(0) as u16 + 2;
    let height = lines.len() as u16 + 2;
    let area = frame
        .area()
        .centered(Constraint::Length(width), Constraint::Length(height));
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(lines).block(Block::bordered().title(" keys ")),
        area,
    );
}

/// Widest value in `array` once displayed, capped at [`MAX_COL_WIDTH`].
fn values_width(array: &ArrayRef) -> usize {
    let Ok(fmt) = ArrayFormatter::try_new(array.as_ref(), &OPTIONS) else {
        return 1;
    };
    (0..array.len())
        .map(|i| table::cell_text(&fmt, i).width())
        .max()
        .unwrap_or(1)
        .min(MAX_COL_WIDTH)
}

fn fitted(value: &str, width: usize, right: bool) -> String {
    let mut s = String::with_capacity(width);
    table::fit(&mut s, value, width, right);
    s
}

/// `1234567` → `1,234,567`.
fn group(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

#[cfg(test)]
mod tests {
    use std::fs::File;
    use std::sync::Arc;

    use arrow::array::{Int64Array, RecordBatch, StringArray};
    use parquet::arrow::ArrowWriter;
    use parquet::file::properties::WriterProperties;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;

    const ROWS: i64 = 3000;
    const COLS: usize = 50;

    /// `COLS` int columns `c0..` where `c{j}` at row i is `i * 100 + j`,
    /// plus a trailing string column `s` with nulls on odd rows.
    fn sample() -> (tempfile::TempDir, Source) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("wide.parquet");
        let mut columns: Vec<(String, ArrayRef)> = (0..COLS)
            .map(|j| {
                let values = Int64Array::from_iter_values((0..ROWS).map(|i| i * 100 + j as i64));
                (format!("c{j}"), Arc::new(values) as ArrayRef)
            })
            .collect();
        let strings =
            StringArray::from_iter((0..ROWS).map(|i| (i % 2 == 0).then(|| format!("v{i}"))));
        columns.push(("s".into(), Arc::new(strings)));
        let batch = RecordBatch::try_from_iter(columns).unwrap();
        let props = WriterProperties::builder()
            .set_max_row_group_row_count(Some(1000))
            .build();
        let mut w = ArrowWriter::try_new(File::create(&path).unwrap(), batch.schema(), Some(props))
            .unwrap();
        w.write(&batch).unwrap();
        w.close().unwrap();
        let src = Source::open(&path).unwrap();
        (dir, src)
    }

    struct Harness<'a> {
        viewer: Viewer<'a>,
        terminal: Terminal<TestBackend>,
    }

    impl<'a> Harness<'a> {
        fn new(src: &'a Source, width: u16, height: u16) -> Self {
            let mut h = Self {
                viewer: Viewer::new(src, "wide.parquet".into()),
                terminal: Terminal::new(TestBackend::new(width, height)).unwrap(),
            };
            h.draw();
            h
        }

        fn draw(&mut self) {
            self.terminal.draw(|f| self.viewer.draw(f)).unwrap();
        }

        fn keys(&mut self, keys: &[KeyCode]) {
            for &k in keys {
                assert!(!self.viewer.on_key(KeyEvent::new(k, KeyModifiers::NONE)));
                self.draw();
            }
        }

        fn type_str(&mut self, s: &str) {
            let keys: Vec<KeyCode> = s.chars().map(KeyCode::Char).collect();
            self.keys(&keys);
        }

        fn screen(&self) -> Vec<String> {
            let buf = self.terminal.backend().buffer();
            (0..buf.area.height)
                .map(|y| {
                    let line: String = (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect();
                    line.trim_end().to_owned()
                })
                .collect()
        }

        fn loaded_columns(&self) -> Vec<usize> {
            let mut cols: Vec<usize> = self.viewer.cache.keys().map(|(_, c)| *c).collect();
            cols.sort();
            cols.dedup();
            cols
        }
    }

    #[test]
    fn first_screen_decodes_only_visible_columns() {
        let (_dir, src) = sample();
        let h = Harness::new(&src, 80, 8);
        let screen = h.screen();
        assert_eq!(
            screen[0],
            "   # │     c0 │     c1 │     c2 │     c3 │     c4 │     c5 │     c6 │     c7 │"
        );
        assert_eq!(
            screen[2],
            "   0 │      0 │      1 │      2 │      3 │      4 │      5 │      6 │      7 │"
        );
        assert!(screen[6].starts_with("   4 │    400 │    401 │"));
        assert_eq!(
            screen[7],
            " wide.parquet  rows 0–4 of 3,000  col 1/51  c0: Int64            ? help  q quit"
        );
        // Widths start at the header's, so a few extra columns get decoded
        // before values widen them, but nowhere near all of them.
        let loaded = h.loaded_columns();
        assert!(
            loaded.starts_with(&[0, 1, 2, 3, 4, 5, 6, 7, 8]),
            "{loaded:?}"
        );
        assert!(loaded.len() <= 16, "{loaded:?}");
    }

    #[test]
    fn paging_and_jumping_rows() {
        let (_dir, src) = sample();
        let mut h = Harness::new(&src, 80, 8);
        h.keys(&[KeyCode::Char(' ')]);
        assert_eq!(h.viewer.top, 5);
        h.keys(&[KeyCode::Char('j'), KeyCode::Char('j'), KeyCode::Char('k')]);
        assert_eq!(h.viewer.top, 6);
        h.type_str("1500g");
        assert_eq!(h.viewer.top, 1500);
        assert!(h.screen()[2].starts_with("1500 │"));
        h.keys(&[KeyCode::Char('G')]);
        assert_eq!(h.viewer.top, 2995);
        assert!(h.screen()[6].starts_with("2999 │"));
        assert!(h.screen()[7].contains("rows 2,995–2,999 of 3,000"));
        h.type_str("g");
        assert_eq!(h.viewer.top, 0);
        h.type_str("3b");
        assert_eq!(h.viewer.top, 0);
    }

    #[test]
    fn moving_right_scrolls_and_loads_new_columns() {
        let (_dir, src) = sample();
        let mut h = Harness::new(&src, 80, 8);
        h.type_str("$");
        assert_eq!(h.viewer.col, COLS);
        let screen = h.screen();
        assert!(screen[0].ends_with("│ s"), "{screen:?}");
        assert!(screen[7].contains("col 51/51  s: Utf8"), "{screen:?}");
        // Columns in between were skipped, not decoded.
        let loaded = h.loaded_columns();
        assert!(!loaded.contains(&20), "{loaded:?}");
        h.type_str("0");
        assert_eq!((h.viewer.col, h.viewer.left), (0, 0));
    }

    #[test]
    fn nulls_show_as_null() {
        let (_dir, src) = sample();
        let mut h = Harness::new(&src, 80, 8);
        h.type_str("$");
        let screen = h.screen();
        assert!(screen[2].ends_with("│ v0"), "{screen:?}");
        assert!(screen[3].ends_with("│ null"), "{screen:?}");
    }

    #[test]
    fn counts_repeat_moves_and_esc_cancels_them() {
        let (_dir, src) = sample();
        let mut h = Harness::new(&src, 80, 8);
        h.type_str("3l");
        assert_eq!(h.viewer.col, 3);
        h.type_str("12");
        assert!(h.screen()[7].ends_with(" 12"));
        h.keys(&[KeyCode::Esc]);
        h.type_str("j");
        assert_eq!(h.viewer.top, 1);
    }

    #[test]
    fn quits_on_q_and_help_swallows_one_key() {
        let (_dir, src) = sample();
        let mut h = Harness::new(&src, 80, 20);
        h.type_str("?");
        assert!(h.screen().iter().any(|l| l.contains("keys")));
        let q = KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE);
        assert!(!h.viewer.on_key(q), "q closes help first");
        assert!(h.viewer.on_key(q));
    }

    #[test]
    fn groups_thousands() {
        assert_eq!(group(0), "0");
        assert_eq!(group(999), "999");
        assert_eq!(group(1000), "1,000");
        assert_eq!(group(26542), "26,542");
        assert_eq!(group(1234567), "1,234,567");
    }
}
