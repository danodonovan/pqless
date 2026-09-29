# parq

Browse a Parquet file a screenful at a time, like `less`.

```
$ parq data.parquet
```

Rows are shown as an aligned table with a row-number column. Decoding is
lazy: batches are read only as fast as the pager asks for them, so opening a
multi-GB file is instant and quitting early costs nothing.

## Paging

- Uses `$PARQ_PAGER` verbatim if set, otherwise `$PAGER`, otherwise `less`.
- When the pager is `less`, parq adds `-S` (long rows scroll sideways with
  ←/→ instead of wrapping) and, on less ≥ 600, `--header=2` so column names
  stay pinned while you scroll. If `$LESS` is unset it defaults to `FR`.
- When stdout isn't a terminal, the table is written straight out, so
  `parq f.parquet | grep …` and `parq f.parquet | head` work.
- `PARQ_PAGER=more parq f.parquet` if you really want `more`.

## Display

- Column widths come from the header and the first 1024 rows, capped at 40
  characters; longer values are truncated with `…`.
- Numbers are right-aligned; nulls show as `null`.
- Newlines/tabs inside values are shown as `\n`/`\t`; other control
  characters are replaced so they can't mess with the terminal.

## Build

```
cargo install --path .
```
