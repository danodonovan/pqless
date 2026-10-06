# pqless

Browse a Parquet file a screenful at a time, like `less`.

```
$ pqless data.parquet
```

![pqless paging through a 250,000-row file, moving between columns and jumping to rows](site/demo.gif)

More at **[danodonovan.github.io/pqless](https://danodonovan.github.io/pqless/)**.

Rows are shown as an aligned table with a row-number column, a pinned
header, and a status bar giving your position and the current column's full
name and type.

Only what is on screen is decoded: rows are read in blocks of 1024, and only
for the columns in view. Parquet stores columns separately, so a file with
tens of thousands of columns or tens of millions of rows opens in a fraction
of a second, and jumping to any row or column is near instant.

## Install

With [Homebrew](https://brew.sh) on macOS or Linux:

```
brew install danodonovan/tap/pqless
```

Or with the install script (macOS or Linux, x86_64 or arm64):

```
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/danodonovan/pqless/releases/latest/download/pqless-installer.sh | sh
```

This puts `pqless` in `~/.cargo/bin`. Prebuilt archives are also on the
[releases page](https://github.com/danodonovan/pqless/releases). The
binaries are not signed, so on macOS a copy downloaded with a browser needs
`xattr -d com.apple.quarantine pqless` before it will run.

With Rust 1.88 or newer you can build it instead:

```
cargo install --git https://github.com/danodonovan/pqless
```

## Keys

| Keys                | Action                   |
|---------------------|--------------------------|
| `j` `↓` `Enter`     | down a row               |
| `k` `↑` `y`         | up a row                 |
| `space` `f` `PgDn`  | down a page              |
| `b` `PgUp`          | up a page                |
| `d` / `u`           | down / up half a page    |
| `g` `Home`          | first row                |
| `G` `End`           | last row                 |
| *N*`g`              | go to row *N*            |
| `l` `→` / `h` `←`   | next / previous column   |
| `$` / `0`           | last / first column      |
| *N* + key           | repeat a move *N* times  |
| `?`                 | show / hide help         |
| `q` `Esc`           | quit                     |

## Piping

When stdout isn't a terminal the whole table is written out as plain text,
so `pqless f.parquet | grep …` and `pqless f.parquet | head` work. This mode
decodes every column, so it is slow on very wide files.

## Display

- Column names are truncated to 24 characters in the header; the full name
  of the current column is in the status bar.
- Columns grow to fit the widest value seen so far, up to 40 characters;
  longer values are truncated with `…`.
- Numbers are right-aligned; nulls show as `null`.
- Newlines/tabs inside values are shown as `\n`/`\t`; other control
  characters are replaced so they can't mess with the terminal.

## Build

```
cargo build --release
cargo test
```

## License

MIT
