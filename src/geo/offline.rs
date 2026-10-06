// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The local city table: the bundled `GeoNames` snapshot, or the one a user installed (step 18b).
//!
//! Two gzip members make up a table — `keys.bin.gz` (the sorted folded-name index, decoded by
//! every search; a miss costs only this member) and `cities.bin.gz` (the rows, decoded only when a
//! search actually matched, and only the matched rows are materialised). [`OfflineTable::open`]
//! picks the source from `[geo] data`:
//!
//! * `bundled` (and every build without the `offline-geo` feature) reads the members embedded with
//!   `include_bytes!`, lazily;
//! * `auto` (the default) prefers the table installed under `$XDG_DATA_HOME/cirrocast/geo/` and
//!   falls back to the bundled one — with one warning — when that table is missing or corrupt;
//! * `user` requires the installed table and refuses to run without it.
//!
//! A user table is validated eagerly when it is opened (both members decode, rows without
//! materialising them), so a corrupt one is diagnosed once, before any query, and the bundled
//! table stays a working fallback. The format itself lives in [`crate::geo::table`], which the
//! builder and the update command share; nothing here ever writes or opens a socket.

use std::fs;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

use chrono::NaiveDate;

use crate::error::{Error, Result};
use crate::geo::rank;
use crate::geo::table::{Cities, City, Index, MatchMode};
use crate::model::Location;
use crate::paths::Paths;

/// The two embedded members; the builder writes both under `src/geo/data/`.
///
/// `static`, not `const`: a `const` holding an `include_bytes!` value is inlined into every use
/// site, and with more than one codegen unit the linker keeps one anonymous copy per unit — the
/// table was embedded twice (3.4 MB) until this became a `static` with a single address.
static CITIES_GZ: &[u8] = include_bytes!("data/cities.bin.gz");
static KEYS_GZ: &[u8] = include_bytes!("data/keys.bin.gz");

/// The embedded provenance record: the bundled table's dump date comes from it.
static SNAPSHOT: &str = include_str!("data/SNAPSHOT");

/// What a corrupt embedded member tells the reader to do about it.
const REBUILD_HINT: &str =
    "rebuild it with `cargo run -p geo-table -- <cities15000.txt> src/geo/data`";

/// How many candidates [`OfflineTable::resolve`] considers before picking a winner.
const RESOLVE_LIMIT: u8 = 10;

/// Where a user-installed city table lives under the data directory (step 18b).
#[must_use]
pub(crate) fn table_dir(paths: &Paths) -> PathBuf {
    paths.data_dir.join("geo")
}

/// Which table answers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// The snapshot embedded in the binary.
    Bundled,
    /// The table installed under this directory by `location update-data`.
    User(PathBuf),
}

/// A table's members: borrowed from the binary, or read from the user's data directory.
enum Bytes {
    Bundled(&'static [u8]),
    User(Vec<u8>),
}

impl Bytes {
    fn as_slice(&self) -> &[u8] {
        match self {
            Self::Bundled(bytes) => bytes,
            Self::User(bytes) => bytes,
        }
    }
}

/// One opened city table: its source, its bytes and the lazily decoded forms.
pub struct OfflineTable {
    source: Source,
    keys: Bytes,
    cities: Bytes,
    dump_date: Option<NaiveDate>,
    index: OnceLock<std::result::Result<Index, String>>,
    rows: OnceLock<std::result::Result<Cities, String>>,
}

impl OfflineTable {
    /// The bundled table alone (the default, the fallback, and what tests pin).
    #[must_use]
    pub fn bundled() -> Self {
        Self {
            source: Source::Bundled,
            keys: Bytes::Bundled(KEYS_GZ),
            cities: Bytes::Bundled(CITIES_GZ),
            dump_date: crate::geo::table::snapshot_dump_date(SNAPSHOT),
            index: OnceLock::new(),
            rows: OnceLock::new(),
        }
    }

    /// Opens the table `[geo] data` asks for.
    ///
    /// `auto` falls back to the bundled table: silently when no user table is installed (the
    /// documented meaning of "user table when present"), and with a one-line warning — once per
    /// process, since every location of a multi-location run opens the table — when one is
    /// installed but unusable. `quiet` silences the warning; a `user` source that cannot be opened
    /// is an error either way.
    pub fn open(paths: &Paths, data: &str, quiet: bool) -> Result<Self> {
        match data {
            "bundled" => Ok(Self::bundled()),
            "user" => Self::user(paths).map_err(|issue| Error::Config(issue.message().to_owned())),
            _ => match Self::user(paths) {
                Ok(table) => Ok(table),
                Err(UserTableIssue::Missing(_)) => Ok(Self::bundled()),
                Err(issue @ UserTableIssue::Corrupt(_)) => {
                    if !quiet && !WARNED_USER_TABLE.swap(true, Ordering::Relaxed) {
                        eprintln!("warning: {}; using the bundled city table", issue.message());
                    }
                    Ok(Self::bundled())
                }
            },
        }
    }

    /// The table installed under `$XDG_DATA_HOME/cirrocast/geo/`, validated.
    ///
    /// The error distinguishes "nothing is installed" from "what is installed cannot be used": the
    /// two get different handling in `auto` mode (silent fallback versus a warning), and `user`
    /// wraps either message as [`Error::Config`].
    fn user(paths: &Paths) -> std::result::Result<Self, UserTableIssue> {
        let dir = table_dir(paths);
        let installed = dir.is_dir();
        let read = |name: &str| {
            fs::read(dir.join(name)).map_err(|error| {
                let message = format!(
                    "cannot read {}: {error}; run `cirrocast location update-data`",
                    dir.join(name).display()
                );
                if installed {
                    UserTableIssue::Corrupt(message)
                } else {
                    UserTableIssue::Missing(message)
                }
            })
        };
        let keys = read("keys.bin.gz")?;
        let cities = read("cities.bin.gz")?;
        let snapshot = fs::read_to_string(dir.join("SNAPSHOT")).ok();
        let table = Self {
            source: Source::User(dir),
            keys: Bytes::User(keys),
            cities: Bytes::User(cities),
            dump_date: snapshot
                .as_deref()
                .and_then(crate::geo::table::snapshot_dump_date),
            index: OnceLock::new(),
            rows: OnceLock::new(),
        };
        // Validate now, so a corrupt table is diagnosed once here — and, in `auto` mode, before
        // the fallback decision — instead of mid-query.
        if let Err(message) = table.index.get_or_init(|| {
            INDEX_LOADED.store(true, Ordering::Release);
            Index::decode(table.keys.as_slice())
        }) {
            return Err(UserTableIssue::Corrupt(
                table.corrupt_message("city index", message),
            ));
        }
        if let Err(message) = table
            .rows
            .get_or_init(|| Cities::decode(table.cities.as_slice()))
        {
            return Err(UserTableIssue::Corrupt(
                table.corrupt_message("city table", message),
            ));
        }
        Ok(table)
    }

    /// Where this table came from.
    #[must_use]
    pub const fn source(&self) -> &Source {
        &self.source
    }

    /// The table's dump date, when its `SNAPSHOT` record carries one.
    #[must_use]
    pub const fn dump_date(&self) -> Option<NaiveDate> {
        self.dump_date
    }

    /// A phrase naming the table, for `-v` lines and the freshness note.
    #[must_use]
    pub fn describe(&self) -> String {
        let what = match &self.source {
            Source::Bundled => "the bundled city table".to_owned(),
            Source::User(dir) => format!("the user city table in {}", dir.display()),
        };
        match self.dump_date {
            Some(date) => format!("{what} (dump {date})"),
            None => what,
        }
    }

    /// Searches the table and returns at most `limit` rows in the shared ranking order.
    ///
    /// `mode` selects the exact or prefix key match; both are performed on the folded keys, so
    /// `São Paulo`, `Sao Paulo` and `MÜNCHEN` hit the same rows the network geocoder would. An
    /// empty folded query (punctuation only) and `limit == 0` are legitimate no-hit answers.
    pub fn search(&self, query: &str, mode: MatchMode, limit: u8) -> Result<Vec<City>> {
        let folded = crate::geo::fold::fold(query);
        if folded.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }
        let lookup = self.index()?.lookup(&folded, mode);
        if lookup.exact.is_empty() && lookup.prefix.is_empty() {
            return Ok(Vec::new());
        }
        let cities = self.cities()?;
        // Rows whose indexed spelling *is* the query are the exact tier; a row reached only through
        // a prefix is the prefix tier. Each tier is ordered by the shared ranking and the tiers are
        // concatenated, so a city known by an exonym (Vienna for `Wien`) still wins against a place
        // whose name merely begins with the query (`Wiener Neustadt`).
        let mut hits = rank::rank(select(cities, &lookup.exact, self)?, Some(query), limit);
        let remaining = usize::from(limit).saturating_sub(hits.len());
        if remaining > 0 {
            let remaining = u8::try_from(remaining).unwrap_or(u8::MAX);
            hits.extend(rank::rank(
                select(cities, &lookup.prefix, self)?,
                Some(query),
                remaining,
            ));
        }
        Ok(hits)
    }

    /// The ranked winner for a name, or [`Error::LocationNotFound`] (exit 5) when nothing matches.
    pub fn resolve(&self, query: &str) -> Result<Location> {
        match self
            .search(query, MatchMode::Prefix, RESOLVE_LIMIT)?
            .into_iter()
            .next()
        {
            Some(city) => Ok(city.location()),
            None => Err(super::offline_not_found(query)),
        }
    }

    /// The rows within `radius_km` of `(lat, lon)`, nearest first — the offline half of step 25's
    /// coordinate naming.
    ///
    /// `limit == 0` is a legitimate empty answer; a decode failure is the table's own error, with
    /// the rebuild hint the rest of this module reports.
    pub fn nearby(&self, lat: f64, lon: f64, radius_km: f64, limit: u8) -> Result<Vec<City>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        self.cities()?
            .nearby(lat, lon, radius_km, limit)
            .map_err(|message| self.corrupt("city table", &message))
    }

    /// The lazily decoded index; a decode failure is cached too, so a corrupt member is reported
    /// once per table instead of being retried on every lookup.
    fn index(&self) -> Result<&Index> {
        match self.index.get_or_init(|| {
            INDEX_LOADED.store(true, Ordering::Release);
            Index::decode(self.keys.as_slice())
        }) {
            Ok(index) => Ok(index),
            Err(message) => Err(self.corrupt("city index", message)),
        }
    }

    /// The lazily decoded row section, same caching rule.
    fn cities(&self) -> Result<&Cities> {
        match self
            .rows
            .get_or_init(|| Cities::decode(self.cities.as_slice()))
        {
            Ok(cities) => Ok(cities),
            Err(message) => Err(self.corrupt("city table", message)),
        }
    }

    /// The typed error a member that cannot be decoded reports: a broken build artefact (exit 1)
    /// for the embedded pair, the user's own installation (exit 4, with the fix) for theirs.
    fn corrupt(&self, member: &str, message: &str) -> Error {
        let message = self.corrupt_message(member, message);
        match &self.source {
            Source::Bundled => Error::Other(message),
            Source::User(_) => Error::Config(message),
        }
    }

    /// The wording [`OfflineTable::corrupt`] wraps, shared with the eager validation in
    /// [`OfflineTable::user`].
    fn corrupt_message(&self, member: &str, message: &str) -> String {
        match &self.source {
            Source::Bundled => {
                format!("the bundled {member} is unusable: {message}; {REBUILD_HINT}")
            }
            Source::User(dir) => format!(
                "the user city table in {} is unusable: {message}; run `cirrocast location \
                 update-data` or delete that directory",
                dir.display()
            ),
        }
    }
}

/// Materialises the rows named by `ids`, wrapping a decode failure as the table's error.
fn select(cities: &Cities, ids: &[u32], table: &OfflineTable) -> Result<Vec<City>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    cities
        .select(ids)
        .map_err(|message| table.corrupt("city table", &message))
}

/// The bundled members and their `SNAPSHOT`, for the update path's comparisons (step 18b).
#[must_use]
pub(crate) fn bundled_members() -> (&'static [u8], &'static [u8], &'static str) {
    (CITIES_GZ, KEYS_GZ, SNAPSHOT)
}

/// Whether a city table has been decoded in this process.
///
/// The startup assertion uses this to prove `--version` and `--help` never materialise a table;
/// nothing in the crate has to call it.
#[must_use]
pub fn index_loaded() -> bool {
    INDEX_LOADED.load(Ordering::Acquire)
}

/// Set by the index initializer, since a [`OnceLock`] cannot be asked whether it fired.
static INDEX_LOADED: AtomicBool = AtomicBool::new(false);

/// Whether the "user table is unusable" warning has already been printed in this process.
///
/// One run may resolve several locations (step 19), each opening the table: the diagnosis is the
/// same for all of them, so it is printed at most once.
static WARNED_USER_TABLE: AtomicBool = AtomicBool::new(false);

/// Why the user-installed table could not answer.
enum UserTableIssue {
    /// No table is installed; a normal state, and `auto` falls back without a word.
    Missing(String),
    /// A table is installed but unreadable, incomplete or corrupt; worth a warning.
    Corrupt(String),
}

impl UserTableIssue {
    /// The diagnosis, naming the file and the fix.
    fn message(&self) -> &str {
        match self {
            Self::Missing(message) | Self::Corrupt(message) => message,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{OfflineTable, REBUILD_HINT};
    use crate::geo::table::MatchMode;

    fn table() -> OfflineTable {
        OfflineTable::bundled()
    }

    /// The committed table answers the spellings the step file pins.
    #[test]
    fn folding_gets_a_query_to_the_same_rows() {
        let table = table();
        for query in ["São Paulo", "Sao Paulo", "SAO PAULO"] {
            let hits = table
                .search(query, MatchMode::Prefix, 5)
                .expect("the bundled table decodes");
            let first = hits.first().expect("São Paulo is in cities15000");
            assert_eq!(first.name, "São Paulo", "{query}");
            assert_eq!(first.country_code, "BR", "{query}");
        }
        for query in ["北京", "Beijing", "Peking"] {
            let hits = table
                .search(query, MatchMode::Prefix, 5)
                .expect("the bundled table decodes");
            assert_eq!(hits.first().map(|city| city.id), Some(1_816_670), "{query}");
        }
        for query in ["Wien", "Vienna"] {
            let hits = table
                .search(query, MatchMode::Prefix, 5)
                .expect("the bundled table decodes");
            // `Wien` is an alternate spelling, so Vienna is reached by an *exact* key and ranks
            // above Wiener Neustadt, whose display name merely starts with the query.
            assert_eq!(hits.first().map(|city| city.id), Some(2_761_369), "{query}");
        }
        let hits = table
            .search("MÜNCHEN", MatchMode::Prefix, 5)
            .expect("the bundled table decodes");
        assert_eq!(
            hits.first().map(|city| city.id),
            Some(2_867_714),
            "{hits:?}"
        );
    }

    #[test]
    fn exact_mode_rejects_a_prefix_and_prefix_mode_finds_it() {
        let table = table();
        let exact = table
            .search("Springf", MatchMode::Exact, 10)
            .expect("the bundled table decodes");
        assert!(exact.is_empty(), "{exact:?}");
        let prefix = table
            .search("Springf", MatchMode::Prefix, 10)
            .expect("the bundled table decodes");
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
        let table = table();
        let hits = table
            .search("Springfield", MatchMode::Exact, 10)
            .expect("the bundled table decodes");
        let actual: Vec<(&str, u64)> = hits
            .iter()
            .map(|city| (city.name.as_str(), city.population.unwrap_or(0)))
            .collect();
        assert_eq!(actual, exact_expected, "{hits:?}");

        let hits = table
            .search("Springf", MatchMode::Prefix, 10)
            .expect("the bundled table decodes");
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
        let hits = table()
            .search("MÜNCHEN", MatchMode::Exact, 10)
            .expect("the bundled table decodes");
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
        let table = table();
        assert_eq!(
            table
                .search("!!!", MatchMode::Prefix, 10)
                .expect("the bundled table decodes"),
            []
        );
        assert_eq!(
            table
                .search("Beijing", MatchMode::Prefix, 0)
                .expect("the bundled table decodes"),
            []
        );
    }

    #[test]
    fn a_missing_city_is_not_found() {
        let error = table()
            .resolve("Nowhereville")
            .expect_err("not in the table");
        assert_eq!(error.exit_code(), 5);
        assert!(error.to_string().contains("(no offline match)"), "{error}");
    }

    #[test]
    fn the_bundled_table_reports_its_source_and_dump_date() {
        let table = table();
        assert_eq!(table.source(), &super::Source::Bundled);
        assert!(
            table.describe().contains("the bundled city table"),
            "{}",
            table.describe()
        );
        assert!(table.describe().contains("dump 20"), "{}", table.describe());
        assert!(table.dump_date().is_some());
    }

    #[test]
    fn the_rebuild_hint_names_the_builder() {
        assert!(REBUILD_HINT.contains("cargo run -p geo-table"));
    }
}
