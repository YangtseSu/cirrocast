// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The city-table format: one encoder, one decoder, one byte reader.
//!
//! Both members cirrocast knows — `cities.bin.gz` (the rows) and `keys.bin.gz` (the sorted
//! folded-key index) — are defined here, together with the dump parser that turns a `GeoNames`
//! `cities15000.txt` into rows plus an index, and the `SNAPSHOT` record beside them. Three
//! consumers share it:
//!
//! * `build/geo-table`, the dev-only builder that writes the committed snapshot;
//! * `geo::offline`, which decodes the embedded (or user-installed) members at runtime;
//! * `geo::update`, which builds a user-installed table from a downloaded dump.
//!
//! Keeping the encoder and the decoder in one module is what makes them unable to drift: the
//! round-trip test below encodes a dump and decodes it back in the same process. The module is
//! always compiled (it has no `include_bytes!` and no feature gate) so the builder can use it
//! without enabling the runtime's `offline-geo` feature.
//!
//! Formats, little-endian throughout:
//!
//! ```text
//! cities.bin.gz = "CCCT\x01" u32 row_count
//!                 (u32 geonameid, u16 name, u16 ascii name, u8 country,
//!                  i32 lat×1e7, i32 lon×1e7, u32 population, u8 time zone)*
//! keys.bin.gz   = "CCKY\x01" u32 key_count
//!                 (u16 folded key, u32 n, u32 geonameid × n)*   — keys sorted byte-wise
//! ```

use std::collections::BTreeMap;
use std::io::Write as _;
use std::str::FromStr as _;

use chrono::NaiveDate;
use chrono_tz::Tz;
use flate2::Compression;
use flate2::GzBuilder;
use sha2::{Digest as _, Sha256};

#[cfg(feature = "offline-geo")]
use flate2::read::GzDecoder;
#[cfg(feature = "offline-geo")]
use std::io::Read as _;

use crate::geo::fold::fold;
use crate::geo::rank::Candidate;
use crate::model::{Location, LocationSource};

/// Format version of the row member.
pub(crate) const CITIES_MAGIC: &[u8; 5] = b"CCCT\x01";

/// Format version of the key-index member.
pub(crate) const KEYS_MAGIC: &[u8; 5] = b"CCKY\x01";

/// The fixed-point scale of stored coordinates; five decimals are the dump's maximum.
const SCALE: f64 = 10_000_000.0;

/// The largest a decoded member may be. The bundled pair decompresses to a few MiB; the cap keeps
/// a crafted or bit-rotted member from inflating to a multi-gigabyte allocation, which is what
/// makes a bad member a [`DecodeError`] instead of an OOM kill.
#[cfg(feature = "offline-geo")]
const MAX_MEMBER_BYTES: u64 = 64 * 1024 * 1024;

#[cfg(feature = "offline-geo")]
/// Why a member could not be decoded; the caller wraps it with the fix it can offer.
pub(crate) type DecodeError = String;

/// How the folded query is matched against the folded index keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchMode {
    /// Only keys equal to the query: the `:name`/`--exact` spelling.
    Exact,
    /// Keys starting with the query: the default fuzzy spelling.
    Prefix,
}

/// One city row of a table.
///
/// The display name is `GeoNames`' `name` (already Latin for most places), the ascii name its
/// ASCII/Latin transliteration, and the country is the ISO 3166-1 alpha-2 code — the dump carries
/// no country names, so [`City::location`] fills `country` with the code instead of inventing a
/// name the table does not have.
#[derive(Debug, Clone, PartialEq)]
pub struct City {
    /// `GeoNames` id.
    pub id: u32,
    /// Display name.
    pub name: String,
    /// ASCII/Latin spelling.
    pub ascii_name: String,
    /// ISO 3166-1 alpha-2 country code.
    pub country_code: String,
    /// Latitude in degrees.
    pub lat: f64,
    /// Longitude in degrees.
    pub lon: f64,
    /// Population when the dump reports one.
    pub population: Option<u64>,
    /// IANA time zone.
    pub tz: Tz,
}

impl City {
    /// The [`Location`] the rest of the program speaks.
    #[must_use]
    pub fn location(&self) -> Location {
        Location {
            name: self.name.clone(),
            admin1: None,
            country: self.country_code.clone(),
            country_code: Some(self.country_code.clone()),
            lat: self.lat,
            lon: self.lon,
            tz: self.tz,
            elevation_m: None,
            population: self.population,
            source: LocationSource::Offline,
            station: None,
            named_by: None,
        }
    }
}

impl Candidate for City {
    fn name(&self) -> &str {
        &self.name
    }

    fn ascii_name(&self) -> Option<&str> {
        Some(&self.ascii_name)
    }

    fn population(&self) -> Option<u64> {
        self.population
    }
}

/// A parsed dump: the rows, the folded-key index over every spelling, and the record fields.
pub struct Dump {
    /// The rows, in the dump's own order.
    pub cities: Vec<City>,
    /// Folded key → the ids that answer it, sorted and deduplicated.
    pub keys: BTreeMap<String, Vec<u32>>,
    /// Rows dropped because they carry no display name or no time zone.
    pub skipped: usize,
    /// The newest row modification date in the dump (`YYYY-MM-DD`), empty when it carries none.
    pub dump_date: String,
}

/// Parses a `cities15000.txt` dump and folds every spelling of a row into the key index.
///
/// The parser is strict about the columns it reads and tolerant about the ones it does not: a line
/// with fewer than 18 tab-separated columns is an error naming the line, extra columns are ignored,
/// and a row without a display name or a time zone is counted in [`Dump::skipped`] instead of
/// aborting the parse.
pub fn parse_dump(text: &str) -> std::result::Result<Dump, String> {
    let mut cities: Vec<City> = Vec::new();
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
        let tz = Tz::from_str(tz)
            .map_err(|_| format!("line {line_number}: `{tz}` is not an IANA time zone"))?;
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

        cities.push(City {
            id,
            name: name.to_owned(),
            ascii_name: ascii_name.to_owned(),
            country_code: country.to_owned(),
            lat,
            lon,
            population: (population > 0).then_some(population),
            tz,
        });
    }

    if cities.is_empty() {
        return Err("no usable rows in the input".to_owned());
    }
    // Row ids are geonameids; the index must stay sorted for the runtime's binary search. BTreeMap
    // already yields the keys in order; sort and deduplicate each key's id list too.
    for ids in keys.values_mut() {
        ids.sort_unstable();
        ids.dedup();
    }
    Ok(Dump {
        cities,
        keys,
        skipped,
        dump_date,
    })
}

fn parse_u32(text: &str, field: &str, line: usize) -> std::result::Result<u32, String> {
    text.trim()
        .parse::<u32>()
        .map_err(|_| format!("line {line}: {field} `{text}` is not a number"))
}

fn parse_coordinate(
    text: &str,
    field: &str,
    limit: f64,
    line: usize,
) -> std::result::Result<f64, String> {
    let value = text
        .trim()
        .parse::<f64>()
        .map_err(|_| format!("line {line}: {field} `{text}` is not a number"))?;
    if !value.is_finite() || value.abs() > limit {
        return Err(format!("line {line}: {field} `{text}` is outside ±{limit}"));
    }
    Ok(value)
}

/// A coordinate in the fixed-point form the table stores (1e-7 degrees).
fn fixed(value: f64, field: &str) -> std::result::Result<i32, String> {
    let scaled = (value * SCALE).round();
    if scaled < f64::from(i32::MIN) || scaled > f64::from(i32::MAX) {
        return Err(format!("{field} {value} is out of range"));
    }
    // The range check above makes the cast exact; `round` keeps it integral.
    #[allow(clippy::cast_possible_truncation)]
    let fixed = scaled as i32;
    Ok(fixed)
}

// ---------------------------------------------------------------------------------------------
// Encoding
// ---------------------------------------------------------------------------------------------

/// Encodes the row member (uncompressed): `magic, u32 count, row*`.
pub fn encode_cities(cities: &[City]) -> std::result::Result<Vec<u8>, String> {
    let mut bytes = Vec::with_capacity(cities.len() * 60);
    bytes.extend_from_slice(CITIES_MAGIC);
    push_u32(
        &mut bytes,
        u32::try_from(cities.len()).map_err(|_| "too many rows".to_owned())?,
    );
    for city in cities {
        push_u32(&mut bytes, city.id);
        push_text_u16(&mut bytes, &city.name)?;
        push_text_u16(&mut bytes, &city.ascii_name)?;
        push_text_u8(&mut bytes, &city.country_code)?;
        push_i32(&mut bytes, fixed(city.lat, "latitude")?);
        push_i32(&mut bytes, fixed(city.lon, "longitude")?);
        push_u32(&mut bytes, population_word(city.population));
        push_text_u8(&mut bytes, city.tz.name())?;
    }
    Ok(bytes)
}

/// Encodes the key member (uncompressed): `magic, u32 count, (u16 key, u32 n, id × n)*`.
///
/// The map's iteration order is the key order (`BTreeMap`), which is what the decoder's binary
/// search relies on.
pub fn encode_keys(keys: &BTreeMap<String, Vec<u32>>) -> std::result::Result<Vec<u8>, String> {
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
    Ok(bytes)
}

/// The population word: the dump's zero means "unknown", and the field is 32-bit.
fn population_word(population: Option<u64>) -> u32 {
    let population = population.unwrap_or(0).min(u64::from(u32::MAX));
    #[allow(clippy::cast_possible_truncation)]
    let word = population as u32;
    word
}

fn push_u32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn push_i32(bytes: &mut Vec<u8>, value: i32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn push_text_u8(bytes: &mut Vec<u8>, text: &str) -> std::result::Result<(), String> {
    let len = u8::try_from(text.len()).map_err(|_| format!("`{text}` is longer than 255 bytes"))?;
    bytes.push(len);
    bytes.extend_from_slice(text.as_bytes());
    Ok(())
}

fn push_text_u16(bytes: &mut Vec<u8>, text: &str) -> std::result::Result<(), String> {
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

/// gzips a member with a fixed header (mtime 0), so the same input always produces the same bytes.
pub fn gzip(bytes: &[u8]) -> std::result::Result<Vec<u8>, String> {
    let mut encoder = GzBuilder::new()
        .mtime(0)
        .write(Vec::new(), Compression::best());
    encoder
        .write_all(bytes)
        .map_err(|error| format!("cannot compress: {error}"))?;
    encoder
        .finish()
        .map_err(|error| format!("cannot finish the gzip member: {error}"))
}

/// Decompresses one gzip member, refusing one that inflates past [`MAX_MEMBER_BYTES`].
#[cfg(feature = "offline-geo")]
pub(crate) fn gunzip(compressed: &[u8]) -> std::result::Result<Vec<u8>, DecodeError> {
    gunzip_with_cap(compressed, MAX_MEMBER_BYTES)
}

/// [`gunzip`] with an explicit cap, so the bound itself is testable with a tiny member.
#[cfg(feature = "offline-geo")]
fn gunzip_with_cap(compressed: &[u8], cap: u64) -> std::result::Result<Vec<u8>, DecodeError> {
    // `.take` caps the read itself, so a compression bomb is stopped while it inflates rather than
    // after `read_to_end` has already allocated the whole expansion.
    let mut decoder = GzDecoder::new(compressed).take(cap + 1);
    let mut bytes = Vec::new();
    decoder
        .read_to_end(&mut bytes)
        .map_err(|error| format!("gzip: {error}"))?;
    if bytes.len() as u64 > cap {
        return Err(format!(
            "the member inflates to more than the {cap}-byte cap"
        ));
    }
    if bytes.is_empty() {
        return Err("the member decompresses to nothing".to_owned());
    }
    Ok(bytes)
}

// ---------------------------------------------------------------------------------------------
// The `SNAPSHOT` record
// ---------------------------------------------------------------------------------------------

/// The provenance record written next to the two members, derived from the input alone (no wall
/// clock, so a rebuild of the same input is byte-identical too).
#[must_use]
pub fn snapshot_text(dump: &Dump, input_sha256: &str) -> String {
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
        dump.dump_date,
        dump.cities.len(),
        dump.keys.len()
    )
}

/// Decodes both members without materialising rows, proving a freshly built pair is readable.
#[cfg(feature = "offline-geo")]
pub(crate) fn validate(cities_gz: &[u8], keys_gz: &[u8]) -> std::result::Result<(), DecodeError> {
    Index::decode(keys_gz)?;
    Cities::decode(cities_gz)?;
    Ok(())
}

/// The `dump-date` a `SNAPSHOT` record carries, when it parses as a date.
#[must_use]
pub fn snapshot_dump_date(snapshot: &str) -> Option<NaiveDate> {
    let value = snapshot
        .lines()
        .find_map(|line| line.strip_prefix("dump-date = "))?
        .trim();
    NaiveDate::parse_from_str(value, "%Y-%m-%d").ok()
}

/// The SHA-256 of a dump file, as the `SNAPSHOT` record spells it.
#[must_use]
pub fn input_sha256(bytes: &[u8]) -> String {
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

// ---------------------------------------------------------------------------------------------
// The decoded forms
// ---------------------------------------------------------------------------------------------

#[cfg(feature = "offline-geo")]
/// What one key lookup found, split by match tier: rows whose key *is* the query rank above rows
/// whose key merely starts with it, whichever spelling of the row supplied the key.
pub(crate) struct Lookup {
    /// Rows reached by a key equal to the query.
    pub(crate) exact: Vec<u32>,
    /// Rows reached only by a key starting with the query.
    pub(crate) prefix: Vec<u32>,
}

#[cfg(feature = "offline-geo")]
/// The sorted folded-key index, in a flat layout: all keys concatenated plus their offsets, and
/// all row ids concatenated plus theirs.
#[derive(Debug)]
pub(crate) struct Index {
    keys: String,
    /// `keys[key_offsets[i]..key_offsets[i + 1]]` is key `i`; `key_offsets.len() == count + 1`.
    key_offsets: Vec<u32>,
    ids: Vec<u32>,
    /// `ids[id_offsets[i]..id_offsets[i + 1]]` are key `i`'s row ids.
    id_offsets: Vec<u32>,
}

#[cfg(feature = "offline-geo")]
impl Index {
    /// Decodes the `keys.bin.gz` member: `magic, u32 count, (u16 len, key, u32 n, n × u32 id)*`.
    pub(crate) fn decode(compressed: &[u8]) -> std::result::Result<Self, DecodeError> {
        let bytes = gunzip(compressed)?;
        let mut cursor = Cursor::new(&bytes);
        cursor.expect(KEYS_MAGIC)?;
        let count = cursor.u32()?;
        // Each key costs at least its 2-byte length plus a 4-byte id count, so a count larger than
        // the bytes that remain cannot be honest. Check it before reserving anything sized from it,
        // so a crafted member is a decode error instead of a multi-gigabyte `Vec`.
        let count = usize::try_from(count)
            .map_err(|_| "the index key count does not fit this platform".to_owned())?;
        if count > cursor.remaining() / 6 {
            return Err(format!(
                "the index declares {count} keys, more than the {} remaining bytes can hold",
                cursor.remaining()
            ));
        }

        let mut keys = String::new();
        let mut key_offsets = Vec::with_capacity(count + 1);
        let mut ids = Vec::new();
        let mut id_offsets = Vec::with_capacity(count + 1);
        key_offsets.push(0);
        id_offsets.push(0);
        for _ in 0..count {
            let key = cursor.text_u16()?;
            keys.push_str(&key);
            key_offsets.push(
                u32::try_from(keys.len()).map_err(|_| "the index keys exceed 4 GiB".to_owned())?,
            );
            let n = cursor.u32()?;
            for _ in 0..n {
                ids.push(cursor.u32()?);
            }
            id_offsets.push(
                u32::try_from(ids.len())
                    .map_err(|_| "the index row list exceeds 4 GiB".to_owned())?,
            );
        }
        if !cursor.is_empty() {
            return Err(format!(
                "{} trailing bytes after the index",
                cursor.remaining()
            ));
        }
        Ok(Self {
            keys,
            key_offsets,
            ids,
            id_offsets,
        })
    }

    /// The number of keys.
    pub(crate) fn len(&self) -> usize {
        self.key_offsets.len() - 1
    }

    /// Key `i`.
    fn key(&self, i: usize) -> &str {
        let start = self.key_offsets[i] as usize;
        let end = self.key_offsets[i + 1] as usize;
        self.keys.get(start..end).unwrap_or("")
    }

    /// The rows whose keys match `folded`, split into the exact tier (key == query) and the prefix
    /// tier (key starts with it), each sorted and deduplicated.
    ///
    /// The keys are sorted byte-wise, so a binary search finds the first key not below the query;
    /// `Exact` then walks the equal keys and `Prefix` the keys that start with it. A row may
    /// appear under several matched keys (its name and an alternate spelling both begin with the
    /// query); a row reached by an exact key is removed from the prefix tier so it is not ranked
    /// twice.
    pub(crate) fn lookup(&self, folded: &str, mode: MatchMode) -> Lookup {
        let start = self.lower_bound(folded);
        let mut exact = Vec::new();
        let mut prefix = Vec::new();
        for i in start..self.len() {
            let key = self.key(i);
            if !key.starts_with(folded) {
                break;
            }
            let from = self.id_offsets[i] as usize;
            let to = self.id_offsets[i + 1] as usize;
            let ids = self.ids.get(from..to).unwrap_or(&[]);
            if key == folded {
                exact.extend_from_slice(ids);
            } else if mode == MatchMode::Prefix {
                prefix.extend_from_slice(ids);
            }
        }
        exact.sort_unstable();
        exact.dedup();
        prefix.sort_unstable();
        prefix.dedup();
        if !exact.is_empty() {
            prefix.retain(|id| exact.binary_search(id).is_err());
        }
        Lookup { exact, prefix }
    }

    /// The first key index whose key is `>= folded`.
    fn lower_bound(&self, folded: &str) -> usize {
        let mut low = 0;
        let mut high = self.len();
        while low < high {
            let mid = low + (high - low) / 2;
            if self.key(mid) < folded {
                low = mid + 1;
            } else {
                high = mid;
            }
        }
        low
    }
}

#[cfg(feature = "offline-geo")]
/// The decompressed row section plus its header.
#[derive(Debug)]
pub(crate) struct Cities {
    bytes: Vec<u8>,
}

#[cfg(feature = "offline-geo")]
impl Cities {
    /// Decodes `cities.bin.gz`: `magic, u32 row_count, (row)*`.
    pub(crate) fn decode(compressed: &[u8]) -> std::result::Result<Self, DecodeError> {
        let bytes = gunzip(compressed)?;
        let mut cursor = Cursor::new(&bytes);
        cursor.expect(CITIES_MAGIC)?;
        let count = cursor.u32()?;
        // Walk every row once to prove the section parses before any lookup trusts it; nothing is
        // materialised here, so a decode allocates only the decompressed bytes.
        for _ in 0..count {
            skip_row(&mut cursor)?;
        }
        if !cursor.is_empty() {
            return Err(format!(
                "{} trailing bytes after the rows",
                cursor.remaining()
            ));
        }
        Ok(Self { bytes })
    }

    /// Materialises the rows whose ids are in `wanted` (sorted, unique).
    ///
    /// Every wanted id must exist: the index and the row section are written by one build, so a
    /// missing id means the pair does not belong together.
    pub(crate) fn select(&self, wanted: &[u32]) -> std::result::Result<Vec<City>, DecodeError> {
        let mut cursor = Cursor::new(&self.bytes);
        cursor.expect(CITIES_MAGIC)?;
        let count = cursor.u32()?;
        let mut cities = Vec::with_capacity(wanted.len());
        for _ in 0..count {
            // The id is the first field, so an unwanted row is skipped without allocating.
            if wanted.binary_search(&cursor.peek_u32()?).is_ok() {
                cities.push(parse_row(&mut cursor)?);
            } else {
                skip_row(&mut cursor)?;
            }
        }
        if cities.len() != wanted.len() {
            // A repeated geonameid in the row section can select *more* rows than the index names,
            // so this must not be a subtraction: it is a disagreement in either direction.
            return Err(format!(
                "the index and the row section disagree: {} rows selected, {} named",
                cities.len(),
                wanted.len()
            ));
        }
        Ok(cities)
    }

    /// Materialises the rows within `radius_km` of `(lat, lon)`, nearest first.
    ///
    /// One pass over the row section, reading every row: the coordinates come after two
    /// length-prefixed fields, so a row cannot be dismissed from its id alone, and the walk is
    /// cheap (the whole table is a few tens of thousands of rows). The rows are ordered by
    /// distance, then by population, then by name — a total order, so two places the same distance
    /// away cannot swap between runs — and the caller's limit is applied after the sort.
    pub(crate) fn nearby(
        &self,
        lat: f64,
        lon: f64,
        radius_km: f64,
        limit: u8,
    ) -> std::result::Result<Vec<City>, DecodeError> {
        let mut cursor = Cursor::new(&self.bytes);
        cursor.expect(CITIES_MAGIC)?;
        let count = cursor.u32()?;
        let mut hits: Vec<(f64, City)> = Vec::new();
        for _ in 0..count {
            let city = parse_row(&mut cursor)?;
            let distance = crate::geo::distance_km(lat, lon, city.lat, city.lon);
            if distance <= radius_km {
                hits.push((distance, city));
            }
        }
        hits.sort_by(|(left_distance, left), (right_distance, right)| {
            left_distance
                .total_cmp(right_distance)
                .then_with(|| {
                    right
                        .population
                        .unwrap_or(0)
                        .cmp(&left.population.unwrap_or(0))
                })
                .then_with(|| left.name.cmp(&right.name))
        });
        hits.truncate(usize::from(limit));
        Ok(hits.into_iter().map(|(_, city)| city).collect())
    }
}

#[cfg(feature = "offline-geo")]
/// Validates one row's bounds without materialising it, the decode-time walk's row reader.
fn skip_row(cursor: &mut Cursor<'_>) -> std::result::Result<(), DecodeError> {
    cursor.take(4)?; // geonameid
    cursor.skip_text_u16()?;
    cursor.skip_text_u16()?;
    cursor.skip_text_u8()?;
    cursor.take(8)?; // lat, lon
    cursor.take(4)?; // population
    cursor.skip_text_u8()?;
    Ok(())
}

#[cfg(feature = "offline-geo")]
/// One row: `u32 id, u16 name, u16 ascii, u8 country, i32 lat, i32 lon, u32 population, u8 tz`.
fn parse_row(cursor: &mut Cursor<'_>) -> std::result::Result<City, DecodeError> {
    let id = cursor.u32()?;
    let name = cursor.text_u16()?;
    let ascii_name = cursor.text_u16()?;
    let country_code = cursor.text_u8()?;
    let lat = f64::from(cursor.i32()?) / SCALE;
    let lon = f64::from(cursor.i32()?) / SCALE;
    let population = cursor.u32()?;
    let tz = cursor.text_u8()?;
    let tz = Tz::from_str(&tz).map_err(|_| format!("row {id}: `{tz}` is not an IANA time zone"))?;
    Ok(City {
        id,
        name,
        ascii_name,
        country_code,
        lat,
        lon,
        population: (population > 0).then_some(u64::from(population)),
        tz,
    })
}

// ---------------------------------------------------------------------------------------------
// The byte reader
// ---------------------------------------------------------------------------------------------

#[cfg(feature = "offline-geo")]
/// A bounds-checked little-endian reader over a decompressed member.
pub(crate) struct Cursor<'a> {
    bytes: &'a [u8],
    position: usize,
}

#[cfg(feature = "offline-geo")]
impl<'a> Cursor<'a> {
    pub(crate) fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.position >= self.bytes.len()
    }

    pub(crate) fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.position)
    }

    /// The next `u32` without consuming it.
    pub(crate) fn peek_u32(&self) -> std::result::Result<u32, DecodeError> {
        let end = self
            .position
            .checked_add(4)
            .ok_or_else(|| "offset overflow".to_owned())?;
        let bytes = self
            .bytes
            .get(self.position..end)
            .ok_or_else(|| format!("truncated: wanted 4 bytes at offset {}", self.position))?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    pub(crate) fn take(&mut self, len: usize) -> std::result::Result<&'a [u8], DecodeError> {
        let end = self
            .position
            .checked_add(len)
            .ok_or_else(|| "offset overflow".to_owned())?;
        let slice = self
            .bytes
            .get(self.position..end)
            .ok_or_else(|| format!("truncated: wanted {len} bytes at offset {}", self.position))?;
        self.position = end;
        Ok(slice)
    }

    pub(crate) fn expect(&mut self, magic: &[u8]) -> std::result::Result<(), DecodeError> {
        let found = self.take(magic.len())?;
        if found == magic {
            Ok(())
        } else {
            Err(format!(
                "bad magic {found:02x?}; this is not a geo-table member (or the version is newer)"
            ))
        }
    }

    pub(crate) fn u8(&mut self) -> std::result::Result<u8, DecodeError> {
        Ok(*self.take(1)?.first().unwrap_or(&0))
    }

    pub(crate) fn u16(&mut self) -> std::result::Result<u16, DecodeError> {
        let bytes = self.take(2)?;
        Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
    }

    pub(crate) fn u32(&mut self) -> std::result::Result<u32, DecodeError> {
        let bytes = self.take(4)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    pub(crate) fn i32(&mut self) -> std::result::Result<i32, DecodeError> {
        let bytes = self.take(4)?;
        Ok(i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    /// Advances past a length-prefixed field without materialising it.
    pub(crate) fn skip_text_u8(&mut self) -> std::result::Result<(), DecodeError> {
        let len = usize::from(self.u8()?);
        self.take(len)?;
        Ok(())
    }

    /// Advances past a length-prefixed field without materialising it.
    pub(crate) fn skip_text_u16(&mut self) -> std::result::Result<(), DecodeError> {
        let len = usize::from(self.u16()?);
        self.take(len)?;
        Ok(())
    }

    pub(crate) fn text_u8(&mut self) -> std::result::Result<String, DecodeError> {
        let len = usize::from(self.u8()?);
        self.text(len)
    }

    pub(crate) fn text_u16(&mut self) -> std::result::Result<String, DecodeError> {
        let len = usize::from(self.u16()?);
        self.text(len)
    }

    pub(crate) fn text(&mut self, len: usize) -> std::result::Result<String, DecodeError> {
        let bytes = self.take(len)?;
        String::from_utf8(bytes.to_vec()).map_err(|error| format!("not UTF-8: {error}"))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    #[cfg(feature = "offline-geo")]
    use super::{Cities, City, Index, KEYS_MAGIC, MatchMode, gunzip, gunzip_with_cap};
    use super::{
        encode_cities, encode_keys, gzip, input_sha256, parse_dump, snapshot_dump_date,
        snapshot_text,
    };
    #[cfg(feature = "offline-geo")]
    use crate::geo::fold::fold;

    /// Three rows in the dump's column order (only the columns the parser reads are populated).
    const SAMPLE: &str = "\
123\tMünchen\tMunchen\tMuenchen,Munich,Мюнхен\t48.13743\t11.57549\tP\tPPLA\tDE\t\t02\t\t\t\t1260391\t519\t519\tEurope/Berlin\t2026-09-02
456\tSão Paulo\tSao Paulo\tSao Paulo,圣保罗\t-23.5475\t-46.63611\tP\tPPLA\tBR\t\t27\t\t\t\t12400232\t760\t760\tAmerica/Sao_Paulo\t2026-09-02
789\tNo Zone\tNo Zone\t\t1.0\t2.0\tP\tPPL\tXX\t\t\t\t\t\t10\t\t\t\t2026-09-02
";

    #[test]
    fn parsing_folds_every_spelling_and_skips_rows_without_a_zone() {
        let dump = parse_dump(SAMPLE).expect("the sample parses");
        assert_eq!(dump.cities.len(), 2);
        assert_eq!(dump.skipped, 1);
        assert_eq!(dump.dump_date, "2026-09-02");
        assert!(dump.keys.contains_key("munchen"));
        assert!(dump.keys.contains_key("muenchen"));
        assert!(dump.keys.contains_key("munich"));
        assert!(dump.keys.contains_key("мюнхен"));
        assert!(dump.keys.contains_key("saopaulo"));
        assert!(dump.keys.contains_key("圣保罗"));
        assert_eq!(dump.keys["munchen"], vec![123]);
    }

    #[test]
    fn encoding_is_byte_identical_across_runs() {
        let dump = parse_dump(SAMPLE).expect("the sample parses");
        let cities = gzip(&encode_cities(&dump.cities).expect("the rows encode"))
            .expect("the rows compress");
        let keys =
            gzip(&encode_keys(&dump.keys).expect("the keys encode")).expect("the keys compress");
        let cities_again = gzip(&encode_cities(&dump.cities).expect("the rows encode again"))
            .expect("the rows compress again");
        let keys_again = gzip(&encode_keys(&dump.keys).expect("the keys encode again"))
            .expect("the keys compress again");
        assert_eq!(cities, cities_again);
        assert_eq!(keys, keys_again);
    }

    #[cfg(feature = "offline-geo")]
    #[test]
    fn the_round_trip_returns_the_same_rows_and_keys() {
        let dump = parse_dump(SAMPLE).expect("the sample parses");
        let cities_gz = gzip(&encode_cities(&dump.cities).expect("the rows encode"))
            .expect("the rows compress");
        let keys_gz =
            gzip(&encode_keys(&dump.keys).expect("the keys encode")).expect("the keys compress");

        let index = Index::decode(&keys_gz).expect("the index decodes");
        assert_eq!(index.len(), dump.keys.len());
        let cities = Cities::decode(&cities_gz).expect("the rows decode");

        let all_ids: Vec<u32> = dump.cities.iter().map(|city| city.id).collect();
        let mut decoded = cities.select(&all_ids).expect("every row is present");
        decoded.sort_by_key(|city| city.id);
        let mut expected = dump.cities.clone();
        expected.sort_by_key(|city| city.id);
        assert_eq!(decoded, expected);

        // The decoded index answers the folded spellings the parser indexed.
        for (key, ids) in &dump.keys {
            let lookup = index.lookup(key, MatchMode::Exact);
            assert_eq!(&lookup.exact, ids, "{key}");
            assert!(lookup.prefix.is_empty(), "{key}");
        }
        assert_eq!(
            index.lookup(&fold("München"), MatchMode::Exact).exact,
            vec![123]
        );
    }

    #[test]
    fn the_snapshot_carries_the_dump_date_and_checksum() {
        let dump = parse_dump(SAMPLE).expect("the sample parses");
        let text = snapshot_text(&dump, "abc123");
        assert!(text.contains("dump-date = 2026-09-02"), "{text}");
        assert!(text.contains("input-sha256 = abc123"), "{text}");
        assert!(text.contains("rows = 2"), "{text}");
        assert_eq!(
            snapshot_dump_date(&text),
            Some(chrono::NaiveDate::from_ymd_opt(2026, 9, 2).expect("a valid date"))
        );
    }

    #[test]
    fn a_dump_without_a_usable_row_is_an_error() {
        assert!(parse_dump("").is_err());
        assert!(parse_dump("not\ta\tdump\n").is_err());
        // A single row without a zone is "no usable rows", not an empty table.
        let no_zone = "1\tX\tX\t\t1.0\t2.0\tP\tPPL\tXX\t\t\t\t\t\t10\t\t\t\t2026-09-02\n";
        assert!(parse_dump(no_zone).is_err());
    }

    #[test]
    fn the_input_checksum_is_a_sha256_hex_digest() {
        let digest = input_sha256(b"");
        assert_eq!(digest.len(), 64);
        assert!(digest.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[cfg(feature = "offline-geo")]
    #[test]
    fn a_corrupt_member_is_a_typed_error() {
        let error = gunzip(b"not gzip").expect_err("garbage is not a gzip member");
        assert!(error.contains("gzip"), "{error}");
        let error = Index::decode(b"not gzip").expect_err("garbage is not an index");
        assert!(error.contains("gzip"), "{error}");
        let error = Cities::decode(b"not gzip").expect_err("garbage is not a row section");
        assert!(error.contains("gzip"), "{error}");

        let empty = BTreeMap::new();
        let keys = gzip(&encode_keys(&empty).expect("an empty index encodes"))
            .expect("an empty index compresses");
        let index = Index::decode(&keys).expect("an empty index decodes");
        assert_eq!(index.len(), 0);
    }

    #[cfg(feature = "offline-geo")]
    #[test]
    fn a_declared_count_larger_than_the_bytes_is_refused_before_reserving() {
        // magic + a count of u32::MAX + a little trailing data: an honest count cannot exceed the
        // remaining bytes, so the decoder must reject it instead of reserving ~17 GiB.
        let mut raw = Vec::new();
        raw.extend_from_slice(KEYS_MAGIC);
        raw.extend_from_slice(&u32::MAX.to_le_bytes());
        raw.extend_from_slice(KEYS_MAGIC);
        let member = gzip(&raw).expect("the crafted member compresses");
        let error = Index::decode(&member).expect_err("a bogus count is not decodable");
        assert!(error.contains("keys"), "{error}");
    }

    #[cfg(feature = "offline-geo")]
    #[test]
    fn a_member_that_inflates_past_the_cap_is_refused() {
        let member = gzip(&[0_u8; 64]).expect("the member compresses");
        assert!(
            gunzip_with_cap(&member, 16).is_err(),
            "an expansion past the cap is refused"
        );
        assert_eq!(
            gunzip_with_cap(&member, 4096)
                .expect("a member under the cap decodes")
                .len(),
            64
        );
    }

    #[cfg(feature = "offline-geo")]
    #[test]
    fn a_repeated_geonameid_is_a_disagreement_not_an_underflow() {
        let city = City {
            id: 7,
            name: "Twice".to_owned(),
            ascii_name: "Twice".to_owned(),
            country_code: "ZZ".to_owned(),
            lat: 1.0,
            lon: 2.0,
            population: None,
            tz: chrono_tz::Tz::UTC,
        };
        // Two rows with the same id, while the index names it once.
        let bytes = encode_cities(&[city.clone(), city]).expect("the rows encode");
        let cities = Cities { bytes };
        let error = cities
            .select(&[7])
            .expect_err("a duplicate id is a decode-time disagreement");
        assert!(error.contains("disagree"), "{error}");
    }
}
