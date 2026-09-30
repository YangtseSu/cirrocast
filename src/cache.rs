// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The on-disk response cache, its cache keys and the injectable clock.
//!
//! One JSON envelope per entry, written through the same atomic writer the configuration uses, so
//! a concurrent reader never sees a half-written file. Entries store the response body as **raw
//! text**: the cache never re-serialises a provider payload, which keeps `cat` on an entry
//! meaningful, lets `--offline` serve entries written by another build, and removes a whole class
//! of "we parsed and re-encoded it, and lost a field" bugs.
//!
//! Modes come from the mutually exclusive `--no-cache`/`--refresh`/`--offline` flags:
//!
//! | mode | reads | writes |
//! |------|-------|--------|
//! | `Normal` | yes | yes |
//! | `NoCache` | no | no |
//! | `Refresh` | no | yes |
//! | `Offline` | yes | no |
//!
//! Time is never read directly: everything that needs "now" asks the [`Clock`], so TTL boundaries
//! and the Nominatim throttle are tested by asserting the requested waits instead of sleeping.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime};

use chrono::{DateTime, NaiveDate, Utc};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::config::atomic_write;
use crate::error::{Error, Result};
use crate::paths::Paths;

/// The envelope version, independent of `Config::schema_version`: changing the configuration must
/// not invalidate the cache, and changing the envelope must not force a configuration migration.
pub const CACHE_SCHEMA_VERSION: u32 = 1;

/// The namespaces `cache stat` reports, always in this order.
pub const NAMESPACES: [&str; 3] = ["weather", "geocode", "ip"];

/// File mode of a freshly written cache entry.
const ENTRY_MODE: u32 = 0o644;

/// File mode of the directories this module creates.
const DIRECTORY_MODE: u32 = 0o700;

/// Where "now" comes from.
pub trait Clock: Send + Sync {
    /// The current wall clock time.
    fn now(&self) -> SystemTime;

    /// Waits for `duration`; in tests this records the request instead of blocking.
    fn sleep(&self, duration: Duration);
}

/// The real clock.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> SystemTime {
        SystemTime::now()
    }

    fn sleep(&self, duration: Duration) {
        std::thread::sleep(duration);
    }
}

/// A clock that only moves when a test moves it, and that records every requested wait.
///
/// `sleep` advances its own time, so a caller that sleeps and then asks for `now()` sees the
/// advanced value — which is what the Nominatim throttle relies on.
#[derive(Debug)]
pub struct FakeClock {
    state: Mutex<FakeState>,
}

#[derive(Debug)]
struct FakeState {
    now: SystemTime,
    sleeps: Vec<Duration>,
}

impl FakeClock {
    /// A clock stopped at `start`.
    pub fn new(start: SystemTime) -> Self {
        Self {
            state: Mutex::new(FakeState {
                now: start,
                sleeps: Vec::new(),
            }),
        }
    }

    /// Moves the clock forward without recording a sleep.
    pub fn advance(&self, duration: Duration) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.now += duration;
    }

    /// Every wait a caller asked for, in order.
    pub fn sleeps(&self) -> Vec<Duration> {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .sleeps
            .clone()
    }

    /// The sum of every requested wait.
    pub fn total_slept(&self) -> Duration {
        self.sleeps().iter().sum()
    }
}

impl Clock for FakeClock {
    fn now(&self) -> SystemTime {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .now
    }

    fn sleep(&self, duration: Duration) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.sleeps.push(duration);
        state.now += duration;
    }
}

/// What the cache is allowed to do for this run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheMode {
    /// Read and write.
    Normal,
    /// Neither read nor write; every request goes upstream.
    NoCache,
    /// Never read, always fetch, then write.
    Refresh,
    /// Read, never write; a miss is a hard failure.
    Offline,
}

impl CacheMode {
    /// Builds the mode from the three flags; clap already rejects a combination, this is the
    /// defensive twin for callers that assemble flags themselves.
    pub fn from_flags(no_cache: bool, refresh: bool, offline: bool) -> Result<Self> {
        match (no_cache, refresh, offline) {
            (false, false, false) => Ok(Self::Normal),
            (true, false, false) => Ok(Self::NoCache),
            (false, true, false) => Ok(Self::Refresh),
            (false, false, true) => Ok(Self::Offline),
            _ => Err(Error::Usage(
                "--no-cache, --refresh and --offline are mutually exclusive".to_owned(),
            )),
        }
    }

    /// Whether reads are served from disk.
    #[must_use]
    pub const fn reads(self) -> bool {
        matches!(self, Self::Normal | Self::Offline)
    }

    /// Whether a fetch is stored.
    #[must_use]
    pub const fn writes(self) -> bool {
        matches!(self, Self::Normal | Self::Refresh)
    }

    /// The flag spelling, for `-v` lines.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::NoCache => "no-cache",
            Self::Refresh => "refresh",
            Self::Offline => "offline",
        }
    }
}

/// Where one entry lives, relative to the cache root, plus the request text it belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheKey {
    path: PathBuf,
    normalised: String,
}

impl CacheKey {
    /// A hashed key: `sha256(normalised_request)` under `namespace/`.
    ///
    /// Hashing rather than naming the request matters wherever it can contain a credential (an
    /// API key in a query parameter must never appear in a filename); the original text is kept in
    /// the entry envelope so `-v` and `cache stat` stay debuggable.
    #[must_use]
    pub fn hash(namespace: &str, normalised_request: &str) -> Self {
        let digest = Sha256::digest(normalised_request.as_bytes());
        Self {
            path: PathBuf::from(namespace).join(format!("{}.json", hex(&digest))),
            normalised: normalised_request.to_owned(),
        }
    }

    /// A key per IP-location service, so a fallback result never masquerades as the primary's.
    #[must_use]
    pub fn ip(service: &str) -> Self {
        Self {
            path: PathBuf::from("ip").join(format!("{service}.json")),
            normalised: format!("ip|{service}"),
        }
    }

    /// A weather key: readable, because it holds nothing but provider, coordinates, day count and
    /// the location-local date.
    #[must_use]
    pub fn weather(provider: &str, lat: f64, lon: f64, days: u8, date: NaiveDate) -> Self {
        let name = format!("{provider}-{lat:.2}-{lon:.2}-{days}-{date}.json");
        Self {
            path: PathBuf::from("weather").join(&name),
            normalised: format!("weather|{provider}|{lat:.2}|{lon:.2}|{days}|{date}"),
        }
    }

    /// The path below the cache root.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The request text this key was derived from.
    #[must_use]
    pub fn normalised(&self) -> &str {
        &self.normalised
    }
}

/// One cached response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheEntry {
    /// The envelope version; a mismatch is a miss, never an error.
    pub cache_schema_version: u32,
    /// The normalised request, kept for debugging.
    pub key: String,
    /// When the response was fetched.
    pub fetched_at: DateTime<Utc>,
    /// How long the entry stays fresh.
    pub ttl_secs: u64,
    /// The status the upstream answered with.
    pub status: u16,
    /// The response body, verbatim.
    pub body: String,
}

impl CacheEntry {
    /// Whether the entry is still fresh at `now`.
    ///
    /// A clock that appears to run backwards counts as fresh: a stale-looking entry is a miss, but
    /// a wrong wall clock must not make every cache read fail.
    #[must_use]
    fn is_fresh(&self, now: SystemTime) -> bool {
        let fetched: SystemTime = self.fetched_at.into();
        now.duration_since(fetched)
            .map_or(true, |age| age < Duration::from_secs(self.ttl_secs))
    }
}

/// What one namespace holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamespaceStat {
    /// The namespace, e.g. `weather`.
    pub name: &'static str,
    /// Number of entry files.
    pub entries: u64,
    /// Total size of those files.
    pub bytes: u64,
    /// Oldest `fetched_at` among the entries that parse.
    pub oldest: Option<DateTime<Utc>>,
    /// Newest `fetched_at` among the entries that parse.
    pub newest: Option<DateTime<Utc>>,
}

/// The result of `cache stat`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheStat {
    /// One row per known namespace, in [`NAMESPACES`] order.
    pub namespaces: Vec<NamespaceStat>,
}

/// The result of `cache clean`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CleanReport {
    /// How many entry files were removed.
    pub removed: u64,
}

/// The cache under `$XDG_CACHE_HOME/cirrocast`.
pub struct Cache {
    root: PathBuf,
    mode: CacheMode,
    clock: Arc<dyn Clock>,
    verbose: u8,
}

impl Cache {
    /// Opens the cache of this user.
    #[must_use]
    pub fn open(paths: &Paths, mode: CacheMode, clock: Arc<dyn Clock>, verbose: u8) -> Self {
        Self {
            root: paths.cache_dir.clone(),
            mode,
            clock,
            verbose,
        }
    }

    /// Opens a cache rooted anywhere — how tests get a private directory.
    #[must_use]
    pub fn with_root(
        root: impl Into<PathBuf>,
        mode: CacheMode,
        clock: Arc<dyn Clock>,
        verbose: u8,
    ) -> Self {
        Self {
            root: root.into(),
            mode,
            clock,
            verbose,
        }
    }

    /// The cache root.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The mode this cache was opened with.
    #[must_use]
    pub const fn mode(&self) -> CacheMode {
        self.mode
    }

    /// The shared clock — what the Nominatim throttle sleeps on.
    #[must_use]
    pub fn clock(&self) -> &dyn Clock {
        self.clock.as_ref()
    }

    /// Where an entry lives.
    #[must_use]
    pub fn entry_path(&self, key: &CacheKey) -> PathBuf {
        self.root.join(key.path())
    }

    /// Reads a fresh entry, or `None` when there is none (missing, expired, unreadable or written
    /// with another envelope version). Modes that never read answer `None` without touching disk.
    pub fn read(&self, key: &CacheKey) -> Result<Option<CacheEntry>> {
        if !self.mode.reads() {
            self.log(&format!(
                "{}: not reading {}",
                self.mode.name(),
                key.path().display()
            ));
            return Ok(None);
        }
        let path = self.entry_path(key);
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == ErrorKind::NotFound => {
                self.log(&format!("{}: miss", path.display()));
                return Ok(None);
            }
            Err(error) => {
                return Err(Error::Config(format!(
                    "cannot read the cache entry {}: {error}",
                    path.display()
                )));
            }
        };

        let entry = match serde_json::from_str::<CacheEntry>(&text) {
            Ok(entry) if entry.cache_schema_version == CACHE_SCHEMA_VERSION => entry,
            Ok(entry) => {
                self.log(&format!(
                    "{}: written with cache schema {}, ignoring",
                    path.display(),
                    entry.cache_schema_version
                ));
                return Ok(None);
            }
            Err(error) => {
                self.log(&format!(
                    "{}: unreadable ({error}), ignoring",
                    path.display()
                ));
                return Ok(None);
            }
        };

        if !entry.is_fresh(self.clock.now()) {
            self.log(&format!("{}: expired", path.display()));
            return Ok(None);
        }
        self.log(&format!("{}: hit", path.display()));
        Ok(Some(entry))
    }

    /// Stores a response body. Modes that never write return without touching disk.
    pub fn write(&self, key: &CacheKey, status: u16, body: &str, ttl: Duration) -> Result<()> {
        if !self.mode.writes() {
            self.log(&format!(
                "{}: not writing {}",
                self.mode.name(),
                key.path().display()
            ));
            return Ok(());
        }
        let entry = CacheEntry {
            cache_schema_version: CACHE_SCHEMA_VERSION,
            key: key.normalised.clone(),
            fetched_at: self.clock.now().into(),
            ttl_secs: ttl.as_secs(),
            status,
            body: body.to_owned(),
        };
        let text = serde_json::to_string_pretty(&entry)
            .map_err(|error| Error::Other(format!("cannot encode a cache entry: {error}")))?;
        let path = self.entry_path(key);
        create_directories(&path)?;
        atomic_write(&path, text.as_bytes(), ENTRY_MODE)?;
        self.log(&format!("{}: wrote {} bytes", path.display(), text.len()));
        Ok(())
    }

    /// Serves `T` from the cache when possible, otherwise calls `fetch`, stores the body as raw
    /// text and parses it.
    ///
    /// A cached body that no longer parses counts as a miss and is fetched again (so a provider
    /// schema change heals itself instead of failing forever); a miss in [`CacheMode::Offline`] is
    /// [`Error::Network`] naming the exact key path.
    pub fn read_or_fetch_json<T: DeserializeOwned>(
        &self,
        key: &CacheKey,
        ttl: Duration,
        fetch: impl FnOnce() -> Result<(u16, String)>,
    ) -> Result<T> {
        if let Some(entry) = self.read(key)? {
            match serde_json::from_str(&entry.body) {
                Ok(value) => return Ok(value),
                Err(error) => self.log(&format!(
                    "{}: body no longer parses ({error}), fetching again",
                    key.path().display()
                )),
            }
        }
        if self.mode == CacheMode::Offline {
            return Err(Error::Network(format!(
                "offline mode: no cached entry for {}",
                key.path().display()
            )));
        }

        let (status, body) = fetch()?;
        self.write(key, status, &body, ttl)?;
        serde_json::from_str(&body).map_err(|error| Error::Upstream {
            provider: "cache".to_owned(),
            status: Some(status),
            message: format!(
                "the response stored at {} does not parse as JSON: {error}",
                key.path().display()
            ),
        })
    }

    /// Reads a state file under the cache root (the Nominatim throttle lives in one).
    ///
    /// State files are not entries: they have no TTL and are never served to a caller as a
    /// response.
    pub fn read_state(&self, relative: &str) -> Result<Option<String>> {
        let path = self.root.join(relative);
        match fs::read_to_string(&path) {
            Ok(text) => Ok(Some(text)),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
            Err(error) => Err(Error::Config(format!(
                "cannot read {}: {error}",
                path.display()
            ))),
        }
    }

    /// Writes a state file under the cache root, atomically. Deliberately not gated on the cache
    /// mode: state is not a cached response.
    pub fn write_state(&self, relative: &str, text: &str) -> Result<()> {
        let path = self.root.join(relative);
        create_directories(&path)?;
        atomic_write(&path, text.as_bytes(), ENTRY_MODE)
    }

    /// Counts and sizes the known namespaces.
    pub fn stat(&self) -> Result<CacheStat> {
        let mut namespaces = Vec::with_capacity(NAMESPACES.len());
        for name in NAMESPACES {
            let directory = self.root.join(name);
            let mut namespace = NamespaceStat {
                name,
                entries: 0,
                bytes: 0,
                oldest: None,
                newest: None,
            };
            for path in entry_files(&directory)? {
                let Ok(metadata) = fs::metadata(&path) else {
                    continue;
                };
                if !metadata.is_file() {
                    continue;
                }
                namespace.entries += 1;
                namespace.bytes += metadata.len();
                if let Ok(text) = fs::read_to_string(&path)
                    && let Ok(entry) = serde_json::from_str::<CacheEntry>(&text)
                {
                    namespace.oldest = Some(
                        namespace
                            .oldest
                            .map_or(entry.fetched_at, |oldest| oldest.min(entry.fetched_at)),
                    );
                    namespace.newest = Some(
                        namespace
                            .newest
                            .map_or(entry.fetched_at, |newest| newest.max(entry.fetched_at)),
                    );
                }
            }
            namespaces.push(namespace);
        }
        Ok(CacheStat { namespaces })
    }

    /// Removes expired entries, or the whole tree with `all`.
    ///
    /// Offline mode refuses: the user asked for no writes, and a deletion is one.
    pub fn clean(&self, all: bool) -> Result<CleanReport> {
        if self.mode == CacheMode::Offline {
            return Err(Error::Usage(
                "offline mode: cache writes are disabled".to_owned(),
            ));
        }
        let mut removed = 0;
        if all {
            for name in NAMESPACES {
                for path in entry_files(&self.root.join(name))? {
                    if fs::remove_file(&path).is_ok() {
                        removed += 1;
                    }
                }
            }
            let throttle = self.root.join("ratelimit");
            if let Ok(entries) = fs::read_dir(&throttle) {
                for entry in entries.flatten() {
                    if entry.path().is_file() {
                        // State files (the Nominatim throttle) are cleaned too, but they are not
                        // cache entries and are not counted as such in the report.
                        let _ = fs::remove_file(entry.path());
                    }
                }
            }
        } else {
            let now = self.clock.now();
            for name in NAMESPACES {
                for path in entry_files(&self.root.join(name))? {
                    let Ok(text) = fs::read_to_string(&path) else {
                        continue;
                    };
                    let Ok(entry) = serde_json::from_str::<CacheEntry>(&text) else {
                        continue;
                    };
                    if !entry.is_fresh(now) && fs::remove_file(&path).is_ok() {
                        removed += 1;
                    }
                }
            }
        }
        self.log(&format!("removed {removed} entries"));
        Ok(CleanReport { removed })
    }

    /// A `-v` line.
    fn log(&self, message: &str) {
        if self.verbose > 0 {
            eprintln!("cache: {message}");
        }
    }
}

/// Creates the directory chain of `path` with mode `0700`.
fn create_directories(path: &Path) -> Result<()> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    if parent.as_os_str().is_empty() {
        return Ok(());
    }
    create_dir_all_private(parent)
        .map_err(|error| Error::Config(format!("cannot create {}: {error}", parent.display())))
}

/// `create_dir_all` with `0700`, so the cache tree is not world readable.
#[cfg(unix)]
fn create_dir_all_private(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt as _;

    fs::DirBuilder::new()
        .recursive(true)
        .mode(DIRECTORY_MODE)
        .create(path)
}

/// Non-Unix shim: file modes are not a concept there.
#[cfg(not(unix))]
fn create_dir_all_private(path: &Path) -> std::io::Result<()> {
    fs::create_dir_all(path)
}

/// Every regular file directly inside `directory`; a missing directory is simply empty.
fn entry_files(directory: &Path) -> Result<Vec<PathBuf>> {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(Error::Config(format!(
                "cannot list {}: {error}",
                directory.display()
            )));
        }
    };
    let mut files = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| {
            Error::Config(format!("cannot list {}: {error}", directory.display()))
        })?;
        let path = entry.path();
        if path.is_file() {
            files.push(path);
        }
    }
    files.sort();
    Ok(files)
}

/// Lower-case hex, the spelling cache file names use.
fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(text, "{byte:02x}");
    }
    text
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime};

    use super::{CacheEntry, CacheKey, CacheMode, hex};

    #[test]
    fn cache_modes_cover_reads_and_writes() {
        let mode = |no_cache, refresh, offline| {
            CacheMode::from_flags(no_cache, refresh, offline).expect("valid flags")
        };
        assert_eq!(mode(false, false, false), CacheMode::Normal);
        assert_eq!(mode(true, false, false), CacheMode::NoCache);
        assert_eq!(mode(false, true, false), CacheMode::Refresh);
        assert_eq!(mode(false, false, true), CacheMode::Offline);
        assert!(CacheMode::from_flags(true, false, true).is_err());

        assert!(CacheMode::Normal.reads() && CacheMode::Normal.writes());
        assert!(!CacheMode::NoCache.reads() && !CacheMode::NoCache.writes());
        assert!(!CacheMode::Refresh.reads() && CacheMode::Refresh.writes());
        assert!(CacheMode::Offline.reads() && !CacheMode::Offline.writes());
    }

    #[test]
    fn keys_are_stable_and_namespaced() {
        let first = CacheKey::hash("geocode", "open-meteo|beijing|10|en");
        let again = CacheKey::hash("geocode", "open-meteo|beijing|10|en");
        let other = CacheKey::hash("geocode", "open-meteo|shanghai|10|en");
        assert_eq!(first, again);
        assert_ne!(first, other);
        assert_eq!(
            first.path().parent().and_then(|parent| parent.to_str()),
            Some("geocode")
        );

        let weather = CacheKey::weather(
            "open-meteo",
            39.9075,
            116.39723,
            3,
            chrono::NaiveDate::from_ymd_opt(2026, 9, 30).expect("a valid date"),
        );
        assert_eq!(
            weather.path().to_string_lossy(),
            "weather/open-meteo-39.91-116.40-3-2026-09-30.json"
        );
        assert_eq!(
            CacheKey::ip("ipwho-is").path().to_string_lossy(),
            "ip/ipwho-is.json"
        );
    }

    #[test]
    fn hex_is_lower_case_and_fixed_width() {
        assert_eq!(hex(&[0x00, 0x0f, 0xff]), "000fff");
    }

    #[test]
    fn freshness_is_strictly_before_the_ttl() {
        let fetched = chrono::DateTime::from_timestamp(1_700_000_000, 0).expect("a valid instant");
        let entry = CacheEntry {
            cache_schema_version: 1,
            key: "k".to_owned(),
            fetched_at: fetched,
            ttl_secs: 600,
            status: 200,
            body: String::new(),
        };
        let at = |offset: u64| SystemTime::from(fetched) + Duration::from_secs(offset);
        assert!(entry.is_fresh(at(599)));
        assert!(!entry.is_fresh(at(600)));
        assert!(entry.is_fresh(SystemTime::from(fetched) - Duration::from_secs(60)));
    }
}
