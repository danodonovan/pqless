//! Writes a made-up weather-station dataset for demos:
//! `cargo run --release --example demo_data -- sensors.parquet`
//!
//! The values come from a fixed-seed generator, so every run produces the
//! same file.

use std::fs::File;
use std::sync::Arc;

use arrow::array::{
    ArrayRef, Float64Array, Int32Array, Int64Array, RecordBatch, StringArray,
    TimestampMillisecondArray,
};
use parquet::arrow::ArrowWriter;
use parquet::basic::{Compression, ZstdLevel};
use parquet::file::properties::WriterProperties;

const ROWS: usize = 250_000;
const BATCH: usize = 50_000;
const PROBES: usize = 24;
const STATIONS: usize = 40;

/// 2026-01-01T00:00:00Z
const START_MS: i64 = 1_767_225_600_000;
const STEP_MS: i64 = 10 * 60 * 1000;

const CITIES: &[(&str, f64, f64)] = &[
    ("Reykjavík", 64.15, -21.94),
    ("Edinburgh", 55.95, -3.19),
    ("Cork", 51.90, -8.47),
    ("Bergen", 60.39, 5.32),
    ("Lisbon", 38.72, -9.14),
    ("Kraków", 50.06, 19.94),
    ("Valparaíso", -33.05, -71.62),
    ("Hobart", -42.88, 147.33),
    ("Sapporo", 43.06, 141.35),
    ("Nairobi", -1.29, 36.82),
];

/// xorshift64*: tiny, fast, and deterministic.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    /// Uniform in [0, 1).
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }

    fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.unit()
    }
}

fn round(x: f64, places: i32) -> f64 {
    let m = 10f64.powi(places);
    (x * m).round() / m
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "sensors.parquet".into());
    let mut rng = Rng(0x5eed_1e55_cafe_f00d);

    let mut writer = None;
    for start in (0..ROWS).step_by(BATCH) {
        let batch = make_batch(&mut rng, start, BATCH.min(ROWS - start))?;
        let writer = match &mut writer {
            Some(w) => w,
            None => {
                let props = WriterProperties::builder()
                    .set_compression(Compression::ZSTD(ZstdLevel::default()))
                    .set_max_row_group_row_count(Some(BATCH))
                    .build();
                writer.insert(ArrowWriter::try_new(
                    File::create(&path)?,
                    batch.schema(),
                    Some(props),
                )?)
            }
        };
        writer.write(&batch)?;
    }
    writer.expect("ROWS > 0").close()?;
    eprintln!("wrote {ROWS} rows to {path}");
    Ok(())
}

fn make_batch(
    rng: &mut Rng,
    start: usize,
    len: usize,
) -> Result<RecordBatch, arrow::error::ArrowError> {
    let rows = start..start + len;
    let station = |i: usize| i % STATIONS;
    let city = |i: usize| CITIES[station(i) % CITIES.len()];

    let mut temperature = Vec::with_capacity(len);
    let mut humidity = Vec::with_capacity(len);
    let mut pressure = Vec::with_capacity(len);
    let mut wind_speed = Vec::with_capacity(len);
    let mut wind_dir = Vec::with_capacity(len);
    let mut rainfall = Vec::with_capacity(len);
    let mut status = Vec::with_capacity(len);
    let mut notes = Vec::with_capacity(len);
    for i in rows.clone() {
        let (_, lat, _) = city(i);
        // Colder towards the poles, warmer in the afternoon.
        let hour = ((i / STATIONS) % 144) as f64 / 6.0;
        let daily = (std::f64::consts::TAU * (hour - 9.0) / 24.0).sin();
        let base = 24.0 - lat.abs() * 0.35;
        temperature.push(round(base + 4.0 * daily + rng.range(-1.5, 1.5), 1));
        humidity.push(round(rng.range(35.0, 98.0), 1));
        pressure.push(round(rng.range(985.0, 1035.0), 1));
        wind_speed.push(round(rng.range(0.0, 18.0), 1));
        wind_dir.push((rng.next() % 360) as i32);

        let roll = rng.unit();
        let (state, note) = match roll {
            r if r < 0.004 => ("offline", Some("no response from station")),
            r if r < 0.015 => ("degraded", Some("battery low")),
            r if r < 0.017 => ("ok", Some("maintenance visit")),
            _ => ("ok", None),
        };
        status.push(state);
        notes.push(note);
        // No rain gauge reading while a station is offline.
        rainfall.push((state != "offline").then(|| {
            if rng.unit() < 0.9 {
                0.0
            } else {
                round(rng.range(0.1, 2.5), 1)
            }
        }));
    }

    let mut columns: Vec<(String, ArrayRef)> = vec![
        (
            "reading_id".into(),
            Arc::new(Int64Array::from_iter_values(
                rows.clone().map(|i| i as i64 + 1),
            )),
        ),
        (
            "recorded_at".into(),
            Arc::new(TimestampMillisecondArray::from_iter_values(
                rows.clone()
                    .map(|i| START_MS + (i / STATIONS) as i64 * STEP_MS),
            )),
        ),
        (
            "station".into(),
            Arc::new(StringArray::from_iter_values(
                rows.clone().map(|i| format!("WX-{:03}", station(i) + 1)),
            )),
        ),
        (
            "city".into(),
            Arc::new(StringArray::from_iter_values(
                rows.clone().map(|i| city(i).0),
            )),
        ),
        (
            "latitude".into(),
            Arc::new(Float64Array::from_iter_values(
                rows.clone().map(|i| city(i).1),
            )),
        ),
        (
            "longitude".into(),
            Arc::new(Float64Array::from_iter_values(
                rows.clone().map(|i| city(i).2),
            )),
        ),
        (
            "temperature_c".into(),
            Arc::new(Float64Array::from(temperature)),
        ),
        (
            "humidity_pct".into(),
            Arc::new(Float64Array::from(humidity)),
        ),
        (
            "pressure_hpa".into(),
            Arc::new(Float64Array::from(pressure)),
        ),
        (
            "wind_speed_ms".into(),
            Arc::new(Float64Array::from(wind_speed)),
        ),
        ("wind_dir_deg".into(), Arc::new(Int32Array::from(wind_dir))),
        ("rainfall_mm".into(), Arc::new(Float64Array::from(rainfall))),
        ("status".into(), Arc::new(StringArray::from(status))),
        ("notes".into(), Arc::new(StringArray::from(notes))),
    ];
    for p in 1..=PROBES {
        let values: Vec<f64> = (0..len).map(|_| round(rng.range(-1.0, 1.0), 4)).collect();
        columns.push((
            format!("probe_{p:02}"),
            Arc::new(Float64Array::from(values)),
        ));
    }
    RecordBatch::try_from_iter(columns)
}
