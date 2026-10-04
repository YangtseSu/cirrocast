// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The bundled `GeoNames` city table: a name resolves to coordinates, a time zone and a country
//! code with no network at all.
//!
//! Two gzip members are embedded in the binary and decoded lazily, once per process:
//!
//! * `data/keys.bin.gz` — the sorted folded-name index (`folded key → row ids`), decoded by every
//!   search; a miss costs only this member;
//! * `data/cities.bin.gz` — the rows themselves (`geonameid`, name, ascii name, country code,
//!   coordinates, population, IANA zone), decoded only when a search actually matched, and only
//!   the matched rows are materialised into [`City`] values.
//!
//! Both members are produced by the `geo-table` builder from a `GeoNames` `cities15000.txt`; the
//! binary offset/row formats are defined there and read back here through one [`Cursor`], which is
//! what keeps a corrupt blob a typed error naming the rebuild command instead of a panic. Nothing
//! in this module ever writes or opens a file: the data arrives through `include_bytes!`.

use std::str::FromStr as _;
use std::sync::LazyLock;
use std::sync::atomic::{AtomicBool, Ordering};

use chrono_tz::Tz;
use flate2::read::GzDecoder;
use std::io::Read as _;

use crate::error::{Error, Result};
use crate::geo::fold::fold;
use crate::geo::rank::{self, Candidate};
use crate::model::{Location, LocationSource};

/// The two embedded members; the builder writes both under `src/geo/data/`.
const CITIES_GZ: &[u8] = include_bytes!("data/cities.bin.gz");
const KEYS_GZ: &[u8] = include_bytes!("data/keys.bin.gz");

/// Magic plus format version at the front of each decompressed member.
const CITIES_MAGIC: &[u8; 5] = b"CCCT\x01";
const KEYS_MAGIC: &[u8; 5] = b"CCKY\x01";

/// What a corrupt member tells the reader to do about it.
const REBUILD_HINT: &str =
    "rebuild it with `cargo run -p geo-table -- <cities15000.txt> src/geo/data`";

/// How many candidates [`resolve`] considers before picking a winner.
const RESOLVE_LIMIT: u8 = 10;

/// How the folded query is matched against the folded index keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchMode {
    /// Only keys equal to the query: the `:name`/`--exact` spelling.
    Exact,
    /// Keys starting with the query: the default fuzzy spelling.
    Prefix,
}

/// One city row of the bundled table.
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

/// What one key lookup found, split by match tier: rows whose key *is* the query rank above rows
/// whose key merely starts with it, whichever spelling of the row supplied the key.
struct Lookup {
    exact: Vec<u32>,
    prefix: Vec<u32>,
}

/// Searches the bundled table and returns at most `limit` rows in the shared ranking order.
///
/// `mode` selects the exact or prefix key match; both are performed on the folded keys, so
/// `São Paulo`, `Sao Paulo` and `MÜNCHEN` hit the same rows the network geocoder would. An empty
/// folded query (punctuation only) and `limit == 0` are legitimate no-hit answers.
pub fn search(query: &str, mode: MatchMode, limit: u8) -> Result<Vec<City>> {
    let folded = fold(query);
    if folded.is_empty() || limit == 0 {
        return Ok(Vec::new());
    }
    let lookup = index()?.lookup(&folded, mode);
    if lookup.exact.is_empty() && lookup.prefix.is_empty() {
        return Ok(Vec::new());
    }
    let cities = cities()?;
    // Rows whose indexed spelling *is* the query are the exact tier; a row reached only through a
    // prefix is the prefix tier. Each tier is ordered by the shared ranking and the tiers are
    // concatenated, so a city known by an exonym (Vienna for `Wien`) still wins against a place
    // whose name merely begins with the query (`Wiener Neustadt`).
    let mut hits = rank::rank(select(cities, &lookup.exact)?, Some(query), limit);
    let remaining = usize::from(limit).saturating_sub(hits.len());
    if remaining > 0 {
        let remaining = u8::try_from(remaining).unwrap_or(u8::MAX);
        hits.extend(rank::rank(
            select(cities, &lookup.prefix)?,
            Some(query),
            remaining,
        ));
    }
    Ok(hits)
}

/// Materialises the rows named by `ids`, wrapping a decode failure as the corrupt-table error.
fn select(cities: &Cities, ids: &[u32]) -> Result<Vec<City>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    cities
        .select(ids)
        .map_err(|message| corrupt("city table", &message))
}

/// The ranked winner for a name, or [`Error::LocationNotFound`] (exit 5) when nothing matches.
pub fn resolve(query: &str) -> Result<Location> {
    match search(query, MatchMode::Prefix, RESOLVE_LIMIT)?
        .into_iter()
        .next()
    {
        Some(city) => Ok(city.location()),
        None => Err(super::offline_not_found(query)),
    }
}

/// Whether the folded-key index has been decoded in this process.
///
/// The startup assertion uses this to prove `--version` and `--help` never materialise the table;
/// nothing in the crate has to call it.
#[must_use]
pub fn index_loaded() -> bool {
    INDEX_LOADED.load(Ordering::Acquire)
}

/// Set by the index initializer, since a [`LazyLock`] cannot be asked whether it fired.
static INDEX_LOADED: AtomicBool = AtomicBool::new(false);

/// The lazily decoded index; a decode failure is cached too, so a corrupt member is reported once
/// per process instead of being retried on every lookup.
static INDEX: LazyLock<std::result::Result<Index, String>> = LazyLock::new(|| {
    INDEX_LOADED.store(true, Ordering::Release);
    Index::decode(KEYS_GZ)
});

/// The lazily decoded row section, same caching rule.
static CITIES: LazyLock<std::result::Result<Cities, String>> =
    LazyLock::new(|| Cities::decode(CITIES_GZ));

fn index() -> Result<&'static Index> {
    match &*INDEX {
        Ok(index) => Ok(index),
        Err(message) => Err(corrupt("city index", message)),
    }
}

fn cities() -> Result<&'static Cities> {
    match &*CITIES {
        Ok(cities) => Ok(cities),
        Err(message) => Err(corrupt("city table", message)),
    }
}

/// The typed error a member that cannot be decoded reports (exit 1: a broken build artefact, not
/// a user-triggered condition).
fn corrupt(member: &str, message: &str) -> Error {
    Error::Other(format!(
        "the bundled {member} is unusable: {message}; {REBUILD_HINT}"
    ))
}

// ---------------------------------------------------------------------------------------------
// The decoded forms
// ---------------------------------------------------------------------------------------------

/// The sorted folded-key index, in a flat layout: all keys concatenated plus their offsets, and
/// all row ids concatenated plus theirs.
struct Index {
    keys: String,
    /// `keys[key_offsets[i]..key_offsets[i + 1]]` is key `i`; `key_offsets.len() == count + 1`.
    key_offsets: Vec<u32>,
    ids: Vec<u32>,
    /// `ids[id_offsets[i]..id_offsets[i + 1]]` are key `i`'s row ids.
    id_offsets: Vec<u32>,
}

impl Index {
    /// Decodes the `keys.bin.gz` member: `magic, u32 count, (u16 len, key, u32 n, n × u32 id)*`.
    fn decode(compressed: &[u8]) -> std::result::Result<Self, DecodeError> {
        let bytes = gunzip(compressed)?;
        let mut cursor = Cursor::new(&bytes);
        cursor.expect(KEYS_MAGIC)?;
        let count = cursor.u32()?;

        let mut keys = String::new();
        let mut key_offsets = Vec::with_capacity(usize::try_from(count).unwrap_or(0) + 1);
        let mut ids = Vec::new();
        let mut id_offsets = Vec::with_capacity(usize::try_from(count).unwrap_or(0) + 1);
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
    fn len(&self) -> usize {
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
    fn lookup(&self, folded: &str, mode: MatchMode) -> Lookup {
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

/// The decompressed row section plus its header.
struct Cities {
    bytes: Vec<u8>,
}

impl Cities {
    /// Decodes `cities.bin.gz`: `magic, u32 row_count, (row)*`.
    fn decode(compressed: &[u8]) -> std::result::Result<Self, DecodeError> {
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
    /// Every wanted id must exist: the index and the row section are written by one builder run,
    /// so a missing id means the embedded pair does not belong together.
    fn select(&self, wanted: &[u32]) -> std::result::Result<Vec<City>, DecodeError> {
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
            return Err(format!(
                "the index names {} rows the table does not contain",
                wanted.len() - cities.len()
            ));
        }
        Ok(cities)
    }
}

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

/// One row: `u32 id, u16 name, u16 ascii, u8 country, i32 lat, i32 lon, u32 population, u8 tz`.
fn parse_row(cursor: &mut Cursor<'_>) -> std::result::Result<City, DecodeError> {
    let id = cursor.u32()?;
    let name = cursor.text_u16()?;
    let ascii_name = cursor.text_u16()?;
    let country_code = cursor.text_u8()?;
    let lat = f64::from(cursor.i32()?) / 10_000_000.0;
    let lon = f64::from(cursor.i32()?) / 10_000_000.0;
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

/// Why a member could not be decoded; the message is wrapped by [`corrupt`] with the fix.
type DecodeError = String;

/// A bounds-checked little-endian reader over a decompressed member.
struct Cursor<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    fn is_empty(&self) -> bool {
        self.position >= self.bytes.len()
    }

    fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.position)
    }

    /// The next `u32` without consuming it.
    fn peek_u32(&self) -> std::result::Result<u32, DecodeError> {
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

    fn take(&mut self, len: usize) -> std::result::Result<&'a [u8], DecodeError> {
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

    fn expect(&mut self, magic: &[u8]) -> std::result::Result<(), DecodeError> {
        let found = self.take(magic.len())?;
        if found == magic {
            Ok(())
        } else {
            Err(format!(
                "bad magic {found:02x?}; this is not a geo-table member (or the version is newer)"
            ))
        }
    }

    fn u8(&mut self) -> std::result::Result<u8, DecodeError> {
        Ok(*self.take(1)?.first().unwrap_or(&0))
    }

    fn u16(&mut self) -> std::result::Result<u16, DecodeError> {
        let bytes = self.take(2)?;
        Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
    }

    fn u32(&mut self) -> std::result::Result<u32, DecodeError> {
        let bytes = self.take(4)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn i32(&mut self) -> std::result::Result<i32, DecodeError> {
        let bytes = self.take(4)?;
        Ok(i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    /// Advances past a length-prefixed field without materialising it.
    fn skip_text_u8(&mut self) -> std::result::Result<(), DecodeError> {
        let len = usize::from(self.u8()?);
        self.take(len)?;
        Ok(())
    }

    /// Advances past a length-prefixed field without materialising it.
    fn skip_text_u16(&mut self) -> std::result::Result<(), DecodeError> {
        let len = usize::from(self.u16()?);
        self.take(len)?;
        Ok(())
    }

    fn text_u8(&mut self) -> std::result::Result<String, DecodeError> {
        let len = usize::from(self.u8()?);
        self.text(len)
    }

    fn text_u16(&mut self) -> std::result::Result<String, DecodeError> {
        let len = usize::from(self.u16()?);
        self.text(len)
    }

    fn text(&mut self, len: usize) -> std::result::Result<String, DecodeError> {
        let bytes = self.take(len)?;
        String::from_utf8(bytes.to_vec()).map_err(|error| format!("not UTF-8: {error}"))
    }
}

/// Decompresses one gzip member.
fn gunzip(compressed: &[u8]) -> std::result::Result<Vec<u8>, DecodeError> {
    let mut decoder = GzDecoder::new(compressed);
    let mut bytes = Vec::new();
    decoder
        .read_to_end(&mut bytes)
        .map_err(|error| format!("gzip: {error}"))?;
    if bytes.is_empty() {
        return Err("the member decompresses to nothing".to_owned());
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::{MatchMode, REBUILD_HINT, search};

    /// The committed table answers the spellings the step file pins.
    #[test]
    fn folding_gets_a_query_to_the_same_rows() {
        for query in ["São Paulo", "Sao Paulo", "SAO PAULO"] {
            let hits = search(query, MatchMode::Prefix, 5).expect("the bundled table decodes");
            let first = hits.first().expect("São Paulo is in cities15000");
            assert_eq!(first.name, "São Paulo", "{query}");
            assert_eq!(first.country_code, "BR", "{query}");
        }
        for query in ["北京", "Beijing", "Peking"] {
            let hits = search(query, MatchMode::Prefix, 5).expect("the bundled table decodes");
            assert_eq!(hits.first().map(|city| city.id), Some(1_816_670), "{query}");
        }
        for query in ["Wien", "Vienna"] {
            let hits = search(query, MatchMode::Prefix, 5).expect("the bundled table decodes");
            // `Wien` is an alternate spelling, so Vienna is reached by an *exact* key and ranks
            // above Wiener Neustadt, whose display name merely starts with the query.
            assert_eq!(hits.first().map(|city| city.id), Some(2_761_369), "{query}");
        }
        let hits = search("MÜNCHEN", MatchMode::Prefix, 5).expect("the bundled table decodes");
        assert_eq!(
            hits.first().map(|city| city.id),
            Some(2_867_714),
            "{hits:?}"
        );
    }

    #[test]
    fn exact_mode_rejects_a_prefix_and_prefix_mode_finds_it() {
        let exact = search("Springf", MatchMode::Exact, 10).expect("the bundled table decodes");
        assert!(exact.is_empty(), "{exact:?}");
        let prefix = search("Springf", MatchMode::Prefix, 10).expect("the bundled table decodes");
        assert_eq!(
            prefix.first().map(|city| city.name.as_str()),
            Some("Springfield")
        );
        assert!(
            prefix.iter().any(|city| city.name == "Springfield Gardens"),
            "{prefix:?}"
        );
    }

    #[test]
    fn ranking_orders_the_exact_tier_before_the_prefix_tier() {
        // The eight exact Springfields in population order; `Springfield Gardens` is reached by an
        // exact key too (an alternate spelling), but its display name is not an exact match, so it
        // sorts after them by the shared ranking; `Springfield Lakes` only matches by prefix.
        // Pinned against the committed dump, so a data refresh shows up here.
        let exact_expected: Vec<(&str, u64)> = vec![
            ("Springfield", 170_188),
            ("Springfield", 154_341),
            ("Springfield", 114_394),
            ("Springfield", 60_870),
            ("Springfield", 59_680),
            ("Springfield", 30_484),
            ("Springfield", 23_363),
            ("Springfield", 16_808),
            ("Springfield Gardens", 30_515),
        ];
        let hits = search("Springfield", MatchMode::Exact, 10).expect("the bundled table decodes");
        let actual: Vec<(&str, u64)> = hits
            .iter()
            .map(|city| (city.name.as_str(), city.population.unwrap_or(0)))
            .collect();
        assert_eq!(actual, exact_expected, "{hits:?}");

        let hits = search("Springf", MatchMode::Prefix, 10).expect("the bundled table decodes");
        let actual: Vec<(&str, u64)> = hits
            .iter()
            .map(|city| (city.name.as_str(), city.population.unwrap_or(0)))
            .collect();
        // No key *is* `springf`, so every hit is a prefix match and the population order decides
        // among names that all start with the query.
        let prefix_expected: Vec<(&str, u64)> = vec![
            ("Springfield", 170_188),
            ("Springfield", 154_341),
            ("Springfield", 114_394),
            ("Springfield", 60_870),
            ("Springfield", 59_680),
            ("Springfield Gardens", 30_515),
            ("Springfield", 30_484),
            ("Springfield", 23_363),
            ("Springfield", 16_808),
            ("Springfield Lakes", 15_081),
        ];
        assert_eq!(actual, prefix_expected, "{hits:?}");
    }

    #[test]
    fn the_ascii_spelling_carries_matches() {
        let hits = search("MÜNCHEN", MatchMode::Exact, 10).expect("the bundled table decodes");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].name, "Munich");
        assert_eq!(hits[0].ascii_name, "Munich");
        assert_eq!(hits[0].country_code, "DE");
        assert_eq!(
            hits[0].location().source,
            crate::model::LocationSource::Offline
        );
    }

    #[test]
    fn punctuation_only_queries_and_zero_limits_are_empty() {
        assert_eq!(
            search("!!!", MatchMode::Prefix, 10).expect("the bundled table decodes"),
            []
        );
        assert_eq!(
            search("Beijing", MatchMode::Prefix, 0).expect("the bundled table decodes"),
            []
        );
    }

    #[test]
    fn a_missing_city_is_not_found() {
        let error = super::resolve("Nowhereville").expect_err("not in the table");
        assert_eq!(error.exit_code(), 5);
        assert!(error.to_string().contains("(no offline match)"), "{error}");
    }

    #[test]
    fn the_rebuild_hint_names_the_builder() {
        assert!(REBUILD_HINT.contains("cargo run -p geo-table"));
    }
}
