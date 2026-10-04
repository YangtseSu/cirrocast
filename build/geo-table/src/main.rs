// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Builds the two members cirrocast embeds for its offline geocoder from a `GeoNames`
//! `cities15000.txt` (step 18).
//!
//! ```text
//! cargo run -p geo-table -- <cities15000.txt> [output-dir]   # default: src/geo/data
//! ```
//!
//! Two files land in `output-dir`, plus a [`SNAPSHOT`](Snapshot) record of the input:
//!
//! * `cities.bin.gz` — `magic, u32 row_count, row*`, one row per city:
//!   `u32 geonameid, u16 name, u16 ascii name, u8 country, i32 lat×1e7, i32 lon×1e7,
//!    u32 population, u8 time zone`;
//! * `keys.bin.gz` — `magic, u32 key_count, (u16 folded key, u32 n, geonameid × n)*`, the folded
//!   key index sorted byte-wise, where a key is the fold of the display name, the ascii name or
//!   any alternate name of a row.
//!
//! The output is deterministic: the input order fixes the rows, the keys are sorted, and the gzip
//! members are written with a fixed timestamp — the same input always produces the same bytes.
//! Folding calls the same `cirrocast::geo::fold` the runtime uses, so index and query agree.

use std::collections::BTreeMap;
use std::env;
use std::fs::{self, File};
use std::io::Write as _;
use std::path::Path;
use std::process::ExitCode;

use flate2::Compression;
use flate2::GzBuilder;
use sha2::{Digest as _, Sha256};

use cirrocast::geo::fold::fold;

/// Format version of the row member; the decoder in `src/geo/offline.rs` requires an exact match.
const CITIES_MAGIC: &[u8; 5] = b"CCCT\x01";
/// Format version of the key-index member.
const KEYS_MAGIC: &[u8; 5] = b"CCKY\x01";
/// Where the members are written when the second argument is omitted.
const DEFAULT_OUTPUT: &str = "src/geo/data";
/// The fixed-point scale of stored coordinates; five decimals are the dump's maximum.
const SCALE: f64 = 10_000_000.0;

fn main() -> ExitCode {
    match run() {
        Ok(report) => {
            println!("{report}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn usage() -> String {
    "usage: cargo run -p geo-table -- <cities15000.txt> [output-dir]".to_owned()
}

fn run() -> Result<String, String> {
    let mut args = env::args().skip(1);
    let input = args.next().ok_or_else(usage)?;
    if input == "-h" || input == "--help" {
        return Err(usage());
    }
    let output = args.next().unwrap_or_else(|| DEFAULT_OUTPUT.to_owned());
    if args.next().is_some() {
        return Err(usage());
    }

    let raw = fs::read(&input).map_err(|error| format!("cannot read {input}: {error}"))?;
    let checksum = hex_digest(&raw);
    let text = String::from_utf8(raw).map_err(|error| format!("{input} is not UTF-8: {error}"))?;

    let parsed = parse(&text)?;
    let output = Path::new(&output);
    fs::create_dir_all(output)
        .map_err(|error| format!("cannot create {}: {error}", output.display()))?;

    let cities_bytes = write_cities(&output.join("cities.bin.gz"), &parsed.rows)?;
    let keys_bytes = write_keys(&output.join("keys.bin.gz"), &parsed.keys)?;
    let snapshot = snapshot(&parsed, &checksum);
    let snapshot_path = output.join("SNAPSHOT");
    fs::write(&snapshot_path, snapshot.as_bytes())
        .map_err(|error| format!("cannot write {}: {error}", snapshot_path.display()))?;

    Ok(format!(
        "{} rows ({} skipped), {} keys\ncities.bin.gz {:>10} bytes\nkeys.bin.gz    {:>10} bytes\nSNAPSHOT       {}",
        parsed.rows.len(),
        parsed.skipped,
        parsed.keys.len(),
        cities_bytes,
        keys_bytes,
        snapshot_path.display()
    ))
}

/// The dump's rows, its folded key index and the counters the report prints.
struct Parsed {
    rows: Vec<Row>,
    keys: BTreeMap<String, Vec<u32>>,
    skipped: usize,
    dump_date: String,
}

/// One city row, in the fields the embedded table carries.
struct Row {
    id: u32,
    name: String,
    ascii_name: String,
    country: String,
    lat: i32,
    lon: i32,
    population: u32,
    tz: String,
}

/// Parses the dump, folding every spelling of a row into the key index.
fn parse(text: &str) -> Result<Parsed, String> {
    let mut rows: Vec<Row> = Vec::new();
    let mut keys: BTreeMap<String, Vec<u32>> = BTreeMap::new();
    let mut skipped = 0_usize;
    let mut dump_date = String::new();

    for (index, line) in text.lines().enumerate() {
        let line_number = index + 1;
        if line.trim().is_empty() {
            continue;
        }
        let columns: Vec<&str> = line.split('\t').collect();
        if columns.len() < 18 {
            return Err(format!(
                "line {line_number}: {} tab-separated columns, expected at least 18",
                columns.len()
            ));
        }
        let id = parse_u32(columns[0], "geonameid", line_number)?;
        let name = columns[1];
        let ascii_name = columns[2];
        let alternates = columns[3];
        let lat = parse_coordinate(columns[4], "latitude", 90.0, line_number)?;
        let lon = parse_coordinate(columns[5], "longitude", 180.0, line_number)?;
        let country = columns[8].trim();
        let population = columns[14]
            .parse::<u64>()
            .map_err(|_| {
                format!(
                    "line {line_number}: population `{}` is not a number",
                    columns[14]
                )
            })?
            .min(u64::from(u32::MAX));
        let tz = columns[17].trim();

        if name.is_empty() || tz.is_empty() {
            // A row without a display name or a zone cannot be resolved to; dropping it is the
            // only honest option (the decoder rejects empty zones, and the zone is what day-part
            // aggregation needs).
            skipped += 1;
            continue;
        }
        let modified = columns.get(18).map_or("", |column| column.trim());
        if modified.len() == 10 && modified > dump_date.as_str() {
            modified.clone_into(&mut dump_date);
        }

        // Every spelling the index should answer with.
        let mut spellings = vec![name, ascii_name];
        spellings.extend(alternates.split(',').map(str::trim));
        for spelling in spellings {
            let key = fold(spelling);
            if key.is_empty() {
                continue;
            }
            let entry = keys.entry(key).or_default();
            if entry.last() != Some(&id) {
                entry.push(id);
            }
        }

        rows.push(Row {
            id,
            name: name.to_owned(),
            ascii_name: ascii_name.to_owned(),
            country: country.to_owned(),
            lat: fixed(lat, "latitude")?,
            lon: fixed(lon, "longitude")?,
            population: u32::try_from(population).map_err(|_| "population overflow".to_owned())?,
            tz: tz.to_owned(),
        });
    }

    if rows.is_empty() {
        return Err("no usable rows in the input".to_owned());
    }
    // Row ids are geonameids; the index must stay sorted for the runtime's binary search. BTreeMap
    // already yields the keys in order; sort and deduplicate each key's id list too.
    for ids in keys.values_mut() {
        ids.sort_unstable();
        ids.dedup();
    }
    Ok(Parsed {
        rows,
        keys,
        skipped,
        dump_date,
    })
}

fn parse_u32(text: &str, field: &str, line: usize) -> Result<u32, String> {
    text.trim()
        .parse::<u32>()
        .map_err(|_| format!("line {line}: {field} `{text}` is not a number"))
}

/// A coordinate in the fixed-point form the table stores (1e-7 degrees).
fn fixed(value: f64, field: &str) -> Result<i32, String> {
    let scaled = (value * SCALE).round();
    if scaled < f64::from(i32::MIN) || scaled > f64::from(i32::MAX) {
        return Err(format!("{field} {value} is out of range"));
    }
    // The range check above makes the cast exact; `round` keeps it integral.
    #[allow(clippy::cast_possible_truncation)]
    let fixed = scaled as i32;
    Ok(fixed)
}

fn parse_coordinate(text: &str, field: &str, limit: f64, line: usize) -> Result<f64, String> {
    let value = text
        .trim()
        .parse::<f64>()
        .map_err(|_| format!("line {line}: {field} `{text}` is not a number"))?;
    if !value.is_finite() || value.abs() > limit {
        return Err(format!("line {line}: {field} `{text}` is outside ±{limit}"));
    }
    Ok(value)
}

/// Writes `cities.bin.gz` and returns its byte size.
fn write_cities(path: &Path, rows: &[Row]) -> Result<u64, String> {
    let mut bytes = Vec::with_capacity(rows.len() * 60);
    bytes.extend_from_slice(CITIES_MAGIC);
    push_u32(
        &mut bytes,
        u32::try_from(rows.len()).map_err(|_| "too many rows".to_owned())?,
    );
    for row in rows {
        push_u32(&mut bytes, row.id);
        push_text_u16(&mut bytes, &row.name)?;
        push_text_u16(&mut bytes, &row.ascii_name)?;
        push_text_u8(&mut bytes, &row.country)?;
        push_i32(&mut bytes, row.lat);
        push_i32(&mut bytes, row.lon);
        push_u32(&mut bytes, row.population);
        push_text_u8(&mut bytes, &row.tz)?;
    }
    gzip_to(path, &bytes)
}

/// Writes `keys.bin.gz` and returns its byte size.
fn write_keys(path: &Path, keys: &BTreeMap<String, Vec<u32>>) -> Result<u64, String> {
    let mut bytes = Vec::with_capacity(keys.len() * 32);
    bytes.extend_from_slice(KEYS_MAGIC);
    push_u32(
        &mut bytes,
        u32::try_from(keys.len()).map_err(|_| "too many keys".to_owned())?,
    );
    for (key, ids) in keys {
        push_text_u16(&mut bytes, key)?;
        push_u32(
            &mut bytes,
            u32::try_from(ids.len()).map_err(|_| "too many ids for one key".to_owned())?,
        );
        for id in ids {
            push_u32(&mut bytes, *id);
        }
    }
    gzip_to(path, &bytes)
}

/// The provenance record next to the two members, derived from the input alone (no wall clock, so
/// a rebuild of the same input is byte-identical too).
fn snapshot(parsed: &Parsed, input_sha256: &str) -> String {
    format!(
        "# GeoNames cities15000 — the dataset behind cirrocast's offline geocoder (step 18).\n\
         # Kept verbatim under CC BY 4.0; see REUSE.toml and the README credits section.\n\
         # Rebuild with:\n\
         #   cargo run -p geo-table -- <path to cities15000.txt> src/geo/data\n\
         dump = cities15000\n\
         source = https://download.geonames.org/export/dump/cities15000.zip\n\
         dump-date = {}\n\
         input-sha256 = {input_sha256}\n\
         rows = {}\n\
         keys = {}\n",
        parsed.dump_date,
        parsed.rows.len(),
        parsed.keys.len()
    )
}

fn push_u32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn push_i32(bytes: &mut Vec<u8>, value: i32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn push_text_u8(bytes: &mut Vec<u8>, text: &str) -> Result<(), String> {
    let len = u8::try_from(text.len()).map_err(|_| format!("`{text}` is longer than 255 bytes"))?;
    bytes.push(len);
    bytes.extend_from_slice(text.as_bytes());
    Ok(())
}

fn push_text_u16(bytes: &mut Vec<u8>, text: &str) -> Result<(), String> {
    let len = u16::try_from(text.len()).map_err(|_| {
        format!(
            "a value of {} bytes exceeds the 65535-byte field",
            text.len()
        )
    })?;
    bytes.extend_from_slice(&len.to_le_bytes());
    bytes.extend_from_slice(text.as_bytes());
    Ok(())
}

/// gzips `bytes` into `path` with a fixed header, then returns the file size.
fn gzip_to(path: &Path, bytes: &[u8]) -> Result<u64, String> {
    let file =
        File::create(path).map_err(|error| format!("cannot write {}: {error}", path.display()))?;
    let mut encoder = GzBuilder::new().mtime(0).write(file, Compression::best());
    encoder
        .write_all(bytes)
        .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
    encoder
        .finish()
        .map_err(|error| format!("cannot finish {}: {error}", path.display()))?;
    fs::metadata(path)
        .map(|metadata| metadata.len())
        .map_err(|error| format!("cannot stat {}: {error}", path.display()))
}

fn hex_digest(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    let digest = Sha256::digest(bytes);
    let mut text = String::with_capacity(digest.len() * 2);
    for byte in digest {
        // Writing into the string cannot fail; the result is discarded because `String`'s
        // `Write` never returns an error.
        let _ = write!(text, "{byte:02x}");
    }
    text
}

#[cfg(test)]
mod tests {
    use super::{parse, snapshot, write_cities, write_keys};

    /// Two rows in the dump's column order (only the columns the builder reads are populated).
    const SAMPLE: &str = "\
123\tMünchen\tMunchen\tMuenchen,Munich,Мюнхен\t48.13743\t11.57549\tP\tPPLA\tDE\t\t02\t\t\t\t1260391\t519\t519\tEurope/Berlin\t2026-09-02
456\tSão Paulo\tSao Paulo\tSao Paulo,圣保罗\t-23.5475\t-46.63611\tP\tPPLA\tBR\t\t27\t\t\t\t12400232\t760\t760\tAmerica/Sao_Paulo\t2026-09-02
789\tNo Zone\tNo Zone\t\t1.0\t2.0\tP\tPPL\tXX\t\t\t\t\t\t10\t\t\t\t2026-09-02
";

    #[test]
    fn parsing_folds_every_spelling_and_skips_rows_without_a_zone() {
        let parsed = parse(SAMPLE).expect("the sample parses");
        assert_eq!(parsed.rows.len(), 2);
        assert_eq!(parsed.skipped, 1);
        assert_eq!(parsed.dump_date, "2026-09-02");
        assert!(parsed.keys.contains_key("munchen"));
        assert!(parsed.keys.contains_key("muenchen"));
        assert!(parsed.keys.contains_key("munich"));
        assert!(parsed.keys.contains_key("мюнхен"));
        assert!(parsed.keys.contains_key("saopaulo"));
        assert!(parsed.keys.contains_key("圣保罗"));
        assert_eq!(parsed.keys["munchen"], vec![123]);
    }

    #[test]
    fn the_members_are_byte_identical_across_runs() {
        let parsed = parse(SAMPLE).expect("the sample parses");
        let directory = std::env::temp_dir().join(format!("geo-table-test-{}", std::process::id()));
        std::fs::create_dir_all(&directory).expect("the test directory");
        let first_cities = write_cities(&directory.join("a.bin.gz"), &parsed.rows)
            .expect("the cities member writes");
        let first_keys =
            write_keys(&directory.join("b.bin.gz"), &parsed.keys).expect("the keys member writes");
        let second_cities = write_cities(&directory.join("c.bin.gz"), &parsed.rows)
            .expect("the cities member writes again");
        let second_keys = write_keys(&directory.join("d.bin.gz"), &parsed.keys)
            .expect("the keys member writes again");
        assert_eq!(first_cities, second_cities);
        assert_eq!(first_keys, second_keys);
        let read = |name: &str| std::fs::read(directory.join(name)).expect("the member reads");
        assert_eq!(read("a.bin.gz"), read("c.bin.gz"));
        assert_eq!(read("b.bin.gz"), read("d.bin.gz"));
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn the_snapshot_carries_the_dump_date_and_checksum() {
        let parsed = parse(SAMPLE).expect("the sample parses");
        let text = snapshot(&parsed, "abc123");
        assert!(text.contains("dump-date = 2026-09-02"), "{text}");
        assert!(text.contains("input-sha256 = abc123"), "{text}");
        assert!(text.contains("rows = 2"), "{text}");
    }
}
