"""Writes the benchmark files into DIR (made-up data, fixed seed).

    python gen_data.py DIR

sensors.parquet must already exist in DIR (from `cargo run --example
demo_data`); it is rewritten with pyarrow so that every file comes from the
most widely used Parquet writer.
"""

import sys

import numpy as np
import pyarrow as pa
import pyarrow.parquet as pq


def main(out):
    rng = np.random.default_rng(42)

    sensors = pq.read_table(f"{out}/sensors.parquet")
    pq.write_table(sensors, f"{out}/sensors.parquet", row_group_size=50_000, compression="zstd")
    print(f"sensors: {sensors.num_rows:,} rows x {sensors.num_columns} cols")

    n = 20_000_000
    labels = np.array([f"category-{i:04d}" for i in range(1000)])
    tall = pa.table({
        "id": np.arange(n, dtype=np.int64),
        "ts": pa.array(np.datetime64("2026-01-01") + np.arange(n).astype("timedelta64[s]"), pa.timestamp("ms")),
        "value": rng.random(n),
        "label": pa.DictionaryArray.from_arrays(rng.integers(0, 1000, n).astype(np.int32), labels).dictionary_decode(),
    })
    pq.write_table(tall, f"{out}/tall.parquet", row_group_size=1_000_000, compression="zstd")
    print(f"tall: {n:,} rows x {tall.num_columns} cols")

    # One row group with one page per column: showing even the first row
    # means reading every column chunk, unless a reader projects columns.
    rows, cols = 20_000, 10_000
    data = rng.random((rows, cols - 1), dtype=np.float32)
    columns = {"row_id": np.arange(7_000_001, 7_000_001 + rows, dtype=np.int64)}
    columns.update({f"feature_{j:05d}": data[:, j] for j in range(cols - 1)})
    pq.write_table(pa.table(columns), f"{out}/wide.parquet", row_group_size=rows, compression="snappy")
    print(f"wide: {rows:,} rows x {cols:,} cols, one row group")


if __name__ == "__main__":
    main(sys.argv[1])
