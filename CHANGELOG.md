# Changelog

## 0.2.0 - 2026-10-05

First release.

- Browse a Parquet file a screenful at a time, with `less`-style keys, a
  pinned header and a status bar showing the current column's name and type.
- Decodes only the rows and columns on screen, so very large or very wide
  files open in a fraction of a second.
- When output is piped, prints the whole file as a plain-text table.
