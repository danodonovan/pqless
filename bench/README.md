# Benchmarks

How long does each tool take to show the first screen of rows, and how much
memory does it use?

## Results

Time to first screen · peak memory. Median of 3 runs, measured 2026-10-07 on
an Apple M2 Pro with 32 GB of RAM, macOS.

| Tool | sensors<br>250k rows × 38 cols, 20 MB | tall<br>20M rows × 4 cols, 237 MB | wide<br>20k rows × 10,000 cols, 1.1 GB |
|---|---|---|---|
| **Interactive viewers** | | | |
| **pqless 0.2.1** | **0.03 s · 12 MB** | **0.03 s · 15 MB** | **0.04 s · 23 MB** |
| parqeye 0.2.0 | 0.07 s · 28 MB | 0.05 s · 34 MB | 7.62 s · 1.3 GB |
| tabiew 0.15.1 | 0.14 s · 185 MB | 0.68 s · 1.7 GB | 2.11 s · 2.0 GB |
| VisiData 3.4 | 0.51 s · 304 MB | 0.60 s · 4.4 GB | 0.96 s · 2.3 GB |
| **Print rows and exit** | | | |
| pqrs 0.3.2 (`head -n 30`) | 0.01 s · 17 MB | 0.01 s · 15 MB | 0.37 s · 1.5 GB |
| DuckDB 1.5.6 (`LIMIT 30`) | 0.03 s · 34 MB | 0.02 s · 28 MB | 2.02 s · 2.5 GB |
| parquet-cli 1.3 (`--head 30`) | 0.34 s · 306 MB | 0.70 s · 2.0 GB | 2.24 s · 3.1 GB |
| parquet-tools 0.2.16 (`show --head 30`) | 0.42 s · 321 MB | 0.78 s · 2.0 GB | 2.86 s · 2.8 GB |

## Reading the results

- **Wide files** are where the approaches differ most. pqless decodes only
  the columns on screen, so it reads a few pages of a 10,000-column file.
  Tools that read whole rows have to decompress every column first.
- **On typical and tall files** the fast tools are all effectively instant.
  Differences under about 50 ms vary from run to run and aren't noticeable.
- **The tools are built for different jobs.** VisiData and tabiew load the
  whole file up front, which is what lets them sort, filter and query it;
  pqless reads only what's on screen and does none of those things. pqrs,
  DuckDB, parquet-cli and parquet-tools print rows and exit rather than
  paging, and DuckDB is a full SQL engine.

## The files

All three are made-up data with a fixed seed, written by pyarrow, the most
widely used Parquet writer.

| File | Shape | Represents |
|---|---|---|
| `sensors` | 250,000 rows × 38 columns (timestamps, text, floats, some nulls); 50,000-row groups | an everyday file |
| `tall` | 20,000,000 rows × 4 columns; 1,000,000-row groups | a long log or event table |
| `wide` | 20,000 rows × 10,000 columns; one row group | a wide feature matrix |

## Method

- Each tool runs in a real pseudo-terminal, 160 × 40, through a terminal
  emulator that answers the terminal's queries as a real one would.
- The clock starts when the tool is launched and stops when a value from the
  file's first row reaches the terminal.
- Interactive viewers are then held open for 5 seconds before quitting, so
  memory used by loading in the background counts. Peak memory is the tool
  process's own maximum resident set size.
- One untimed warm-up run, then the median of 3. The files are in the OS
  cache, so this measures the tools, not the disk.

Not included: parq 0.1.4 (no data view yet) and parquet-viewer 0.2.0 (could
not open files with timestamp columns).

## Reproduce

```
just bench-setup   # installs the tools above into target/bench
just bench         # writes the data files, runs everything, prints the table
```

`just bench pqless,visidata wide` runs a subset. The harness is
[`bench.py`](bench.py) and the data comes from [`gen_data.py`](gen_data.py)
and [`examples/demo_data.rs`](../examples/demo_data.rs).
