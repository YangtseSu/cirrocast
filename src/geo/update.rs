// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! User-installed city tables (step 18b): build one from a dump, install it, compare it.
//!
//! `location update-data` reads a `GeoNames` `cities15000` dump — the official ZIP or a local
//! `.txt`/`.zip` — through the shared [`HttpClient`] (proxy, timeout, retries and the
//! `CIRROCAST_FORBID_NETWORK` guard all apply), extracts it with the minimal ZIP reader below,
//! builds the two members with [`crate::geo::table`], proves them by decoding them back, and
//! installs them atomically under `$XDG_DATA_HOME/cirrocast/geo/`. Nothing in a query ever calls
//! this: the freshness note is the only automatic part, and it never fetches.
//!
//! The ZIP reader is deliberately small: the only producer is `GeoNames`' own dump (a single
//! deflate member), so zip64, encryption and unknown methods are typed errors instead of a
//! dependency. `--from` a local `.txt` is the escape hatch when an archive ever stops matching.

use chrono::{DateTime, NaiveDate, SecondsFormat, Utc};
use serde_json::json;

use crate::cache::Cache;

#[cfg(feature = "offline-geo")]
use std::path::PathBuf;

#[cfg(feature = "offline-geo")]
use crate::error::{Error, Result};
#[cfg(feature = "offline-geo")]
use crate::paths::Paths;
#[cfg(feature = "offline-geo")]
use std::fs;
#[cfg(feature = "offline-geo")]
use std::io::Read as _;

#[cfg(feature = "offline-geo")]
use crate::geo::offline::table_dir;
#[cfg(feature = "offline-geo")]
use crate::geo::table;
#[cfg(feature = "offline-geo")]
use crate::http::{HttpClient, HttpRequest};
#[cfg(feature = "offline-geo")]
use flate2::read::DeflateDecoder;

/// The official dump `location update-data` fetches when nothing else is configured.
pub const OFFICIAL_URL: &str = "https://download.geonames.org/export/dump/cities15000.zip";

/// The cache-relative state file that throttles the freshness note.
pub(crate) const NOTICE_STATE: &str = "geo/update-notice.json";

/// How long a freshness note is not repeated.
const NOTICE_INTERVAL_HOURS: i64 = 24;

/// The largest a dump may decompress to; the real one is ~8.4 MB.
#[cfg(feature = "offline-geo")]
const MAX_DUMP_BYTES: u64 = 64 * 1024 * 1024;

// ---------------------------------------------------------------------------------------------
// The freshness note
// ---------------------------------------------------------------------------------------------

/// Whether the freshness note is due, and how old the table is.
///
/// Pure so the policy is testable without a clock: the note fires when the table's dump date is
/// older than `interval_days` and the previous note is more than a day old.
pub(crate) fn note_due(
    now: DateTime<Utc>,
    dump_date: NaiveDate,
    interval_days: u32,
    last_notice: Option<DateTime<Utc>>,
) -> Option<i64> {
    let age_days = (now.date_naive() - dump_date).num_days();
    if age_days <= i64::from(interval_days) {
        return None;
    }
    if let Some(last) = last_notice
        && (now - last).num_hours() < NOTICE_INTERVAL_HOURS
    {
        return None;
    }
    Some(age_days)
}

/// When the freshness note was last printed, from the cache-dir state file.
pub(crate) fn last_notice(cache: &Cache) -> Option<DateTime<Utc>> {
    let text = cache.read_state(NOTICE_STATE).ok().flatten()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    let stamp = value.get("noticed_at")?.as_str()?;
    DateTime::parse_from_rfc3339(stamp)
        .ok()
        .map(|stamp| stamp.with_timezone(&Utc))
}

/// Records that the freshness note was printed; a failure to write it is ignored (a note must not
/// fail a run, and the next run simply notes again).
pub(crate) fn record_notice(cache: &Cache, now: DateTime<Utc>) {
    let text = json!({
        "noticed_at": now.to_rfc3339_opts(SecondsFormat::Secs, true),
    })
    .to_string();
    let _ = cache.write_state(NOTICE_STATE, &text);
}

// ---------------------------------------------------------------------------------------------
// Building and installing
// ---------------------------------------------------------------------------------------------

/// A dump built into the two members, not yet written anywhere.
#[cfg(feature = "offline-geo")]
pub(crate) struct Candidate {
    pub(crate) cities_gz: Vec<u8>,
    pub(crate) keys_gz: Vec<u8>,
    pub(crate) snapshot: String,
    pub(crate) dump_date: String,
    pub(crate) rows: usize,
    pub(crate) keys: usize,
    pub(crate) input_sha256: String,
}

/// What kind of dump a source names.
#[cfg(feature = "offline-geo")]
#[derive(Debug)]
enum Kind {
    Text,
    Zip,
}

/// Classifies a path or URL by the extension of its path part.
#[cfg(feature = "offline-geo")]
fn source_kind(source: &str) -> Result<Kind> {
    let path = source.split(['?', '#']).next().unwrap_or(source);
    let extension = std::path::Path::new(path)
        .extension()
        .and_then(std::ffi::OsStr::to_str)
        .map(str::to_ascii_lowercase);
    match extension.as_deref() {
        Some("zip") => Ok(Kind::Zip),
        Some("txt") => Ok(Kind::Text),
        _ => Err(Error::Usage(format!(
            "`{source}` is not a `.txt` or `.zip` dump"
        ))),
    }
}

/// Reads a dump from a path or URL. URLs go through the shared client, so the guard, the proxy and
/// the retry policy all apply.
#[cfg(feature = "offline-geo")]
fn read_source(source: &str, http: &HttpClient, verbose: u8) -> Result<Vec<u8>> {
    if source.starts_with("http://") || source.starts_with("https://") {
        if verbose > 0 {
            eprintln!("geo: downloading {source}");
        }
        let response = http.send(&HttpRequest::get(source))?;
        Ok(response.bytes().to_vec())
    } else {
        fs::read(source).map_err(|error| Error::Other(format!("cannot read {source}: {error}")))
    }
}

/// Reads a dump, parses it and builds the two members, proving they decode.
#[cfg(feature = "offline-geo")]
pub(crate) fn build_candidate(source: &str, http: &HttpClient, verbose: u8) -> Result<Candidate> {
    let raw = read_source(source, http, verbose)?;
    // The checksum is taken before the bytes are consumed, so the text path needs no copy.
    let input_sha256 = table::input_sha256(&raw);
    let text = match source_kind(source)? {
        Kind::Zip => extract_cities_txt(&raw)
            .map_err(|message| Error::Other(format!("{source}: {message}")))?,
        Kind::Text => String::from_utf8(raw)
            .map_err(|error| Error::Other(format!("{source} is not UTF-8: {error}")))?,
    };
    let dump =
        table::parse_dump(&text).map_err(|message| Error::Other(format!("{source}: {message}")))?;
    let cities_gz = table::gzip(
        &table::encode_cities(&dump.cities)
            .map_err(|message| Error::Other(format!("{source}: {message}")))?,
    )
    .map_err(|message| Error::Other(format!("{source}: {message}")))?;
    let keys_gz = table::gzip(
        &table::encode_keys(&dump.keys)
            .map_err(|message| Error::Other(format!("{source}: {message}")))?,
    )
    .map_err(|message| Error::Other(format!("{source}: {message}")))?;
    // Prove the pair decodes before it can replace a working table.
    table::validate(&cities_gz, &keys_gz)
        .map_err(|message| Error::Other(format!("the built table does not decode: {message}")))?;
    let snapshot = table::snapshot_text(&dump, &input_sha256);
    Ok(Candidate {
        cities_gz,
        keys_gz,
        snapshot,
        dump_date: dump.dump_date,
        rows: dump.cities.len(),
        keys: dump.keys.len(),
        input_sha256,
    })
}

/// Installs the candidate under `$XDG_DATA_HOME/cirrocast/geo/` (atomic writes, previous table
/// kept until every file is in place) and returns the directory.
#[cfg(feature = "offline-geo")]
pub(crate) fn install(candidate: &Candidate, paths: &Paths) -> Result<PathBuf> {
    let dir = table_dir(paths);
    fs::create_dir_all(&dir)
        .map_err(|error| Error::Config(format!("cannot create {}: {error}", dir.display())))?;
    write_atomic(&dir.join("cities.bin.gz"), &candidate.cities_gz)?;
    write_atomic(&dir.join("keys.bin.gz"), &candidate.keys_gz)?;
    write_atomic(&dir.join("SNAPSHOT"), candidate.snapshot.as_bytes())?;
    Ok(dir)
}

/// One atomic write: the same tmp + rename the cache uses.
#[cfg(feature = "offline-geo")]
fn write_atomic(path: &std::path::Path, bytes: &[u8]) -> Result<()> {
    crate::config::atomic_write(path, bytes, 0o644)
        .map_err(|error| Error::Config(format!("cannot write {}: {error}", path.display())))
}

/// How a candidate compares with the table a run would actually use.
#[cfg(feature = "offline-geo")]
pub(crate) struct Comparison {
    /// Whether all three files match.
    pub(crate) same: bool,
    /// Which table was compared against, for the message.
    pub(crate) against: String,
    /// The names of the files that differ.
    pub(crate) differences: Vec<&'static str>,
}

/// Compares a candidate with the active table: the user's when one is installed, else the bundled
/// one. Byte comparison on the two members plus the `SNAPSHOT` record.
#[cfg(feature = "offline-geo")]
pub(crate) fn compare(candidate: &Candidate, paths: &Paths) -> Result<Comparison> {
    let dir = table_dir(paths);
    let user = dir.join("cities.bin.gz");
    let (cities, keys, snapshot, against) = if user.is_file() {
        (
            fs::read(&user).map_err(|error| {
                Error::Config(format!("cannot read {}: {error}", user.display()))
            })?,
            fs::read(dir.join("keys.bin.gz")).map_err(|error| {
                Error::Config(format!(
                    "cannot read {}: {error}",
                    dir.join("keys.bin.gz").display()
                ))
            })?,
            fs::read_to_string(dir.join("SNAPSHOT")).ok(),
            format!("the user city table in {}", dir.display()),
        )
    } else {
        let (cities, keys, snapshot) = crate::geo::offline::bundled_members();
        (
            cities.to_vec(),
            keys.to_vec(),
            Some(snapshot.to_owned()),
            "the bundled city table".to_owned(),
        )
    };
    let mut differences = Vec::new();
    if cities != candidate.cities_gz {
        differences.push("cities.bin.gz");
    }
    if keys != candidate.keys_gz {
        differences.push("keys.bin.gz");
    }
    if snapshot.as_deref() != Some(candidate.snapshot.as_str()) {
        differences.push("SNAPSHOT");
    }
    Ok(Comparison {
        same: differences.is_empty(),
        against,
        differences,
    })
}

// ---------------------------------------------------------------------------------------------
// The minimal ZIP reader
// ---------------------------------------------------------------------------------------------

/// The end-of-central-directory record's signature.
#[cfg(feature = "offline-geo")]
const EOCD_SIGNATURE: &[u8; 4] = b"PK\x05\x06";
/// A central-directory file header's signature.
#[cfg(feature = "offline-geo")]
const CENTRAL_SIGNATURE: &[u8; 4] = b"PK\x01\x02";
/// A local file header's signature.
#[cfg(feature = "offline-geo")]
const LOCAL_SIGNATURE: &[u8; 4] = b"PK\x03\x04";

/// Extracts `cities15000.txt` from a ZIP archive.
///
/// The reader handles what `GeoNames` publishes: one or more plain entries, stored or deflated,
/// with the sizes in the central directory. zip64, encrypted entries and unknown methods are
/// refused with a message naming the problem.
#[cfg(feature = "offline-geo")]
fn extract_cities_txt(zip: &[u8]) -> std::result::Result<String, String> {
    let entry = choose_entry(zip)?;
    if entry.flags & 0x1 != 0 {
        return Err("encrypted ZIP entries are not supported".to_owned());
    }
    if u64::from(entry.uncompressed) > MAX_DUMP_BYTES {
        return Err(format!(
            "the archive member expands to {} bytes, above the {MAX_DUMP_BYTES} byte cap",
            entry.uncompressed
        ));
    }

    // The local header's own name/extra lengths decide where the data starts; they may differ from
    // the central directory's.
    let local = entry.local_offset as usize;
    let header = zip
        .get(local..local + 30)
        .ok_or_else(|| "a local file header is truncated".to_owned())?;
    if &header[0..4] != LOCAL_SIGNATURE {
        return Err("a local file header is malformed".to_owned());
    }
    let local_name_len = usize::from(u16::from_le_bytes([header[26], header[27]]));
    let local_extra_len = usize::from(u16::from_le_bytes([header[28], header[29]]));
    let data = local + 30 + local_name_len + local_extra_len;
    let compressed = zip
        .get(data..data + entry.compressed as usize)
        .ok_or_else(|| "the archive member is truncated".to_owned())?;

    let mut out = Vec::with_capacity(entry.uncompressed as usize);
    match entry.method {
        0 => out.extend_from_slice(compressed),
        8 => {
            DeflateDecoder::new(compressed)
                .read_to_end(&mut out)
                .map_err(|error| format!("the archive member does not inflate: {error}"))?;
        }
        method => {
            return Err(format!(
                "compression method {method} is not supported (stored and deflate only)"
            ));
        }
    }
    if out.len() != entry.uncompressed as usize {
        return Err(format!(
            "the archive member expanded to {} bytes, expected {}",
            out.len(),
            entry.uncompressed
        ));
    }
    if crc32(&out) != entry.crc {
        return Err("the archive member fails its CRC-32 check".to_owned());
    }
    String::from_utf8(out).map_err(|error| format!("the archive member is not UTF-8: {error}"))
}

/// Walks the central directory and picks the dump member: `cities15000.txt` when it is there,
/// else the first `.txt` member.
#[cfg(feature = "offline-geo")]
fn choose_entry(zip: &[u8]) -> std::result::Result<Entry, String> {
    let eocd = find_eocd(zip).ok_or_else(|| "no ZIP end-of-central-directory record".to_owned())?;
    let disk = read_u16(zip, eocd + 4)?;
    let central_disk = read_u16(zip, eocd + 6)?;
    let entries_here = read_u16(zip, eocd + 8)?;
    let entries = read_u16(zip, eocd + 10)?;
    let central_size = read_u32(zip, eocd + 12)?;
    let central_offset = read_u32(zip, eocd + 16)?;
    if disk != 0 || central_disk != 0 || entries_here != entries {
        return Err("multi-disk ZIP archives are not supported".to_owned());
    }
    if entries == u16::MAX || central_size == u32::MAX || central_offset == u32::MAX {
        return Err(
            "zip64 archives are not supported; extract cities15000.txt and pass the .txt"
                .to_owned(),
        );
    }

    let mut cursor = central_offset as usize;
    let mut chosen: Option<Entry> = None;
    for _ in 0..entries {
        let header = zip
            .get(cursor..cursor + 46)
            .ok_or_else(|| "the central directory is truncated".to_owned())?;
        if &header[0..4] != CENTRAL_SIGNATURE {
            return Err("the central directory is malformed".to_owned());
        }
        let flags = u16::from_le_bytes([header[8], header[9]]);
        let method = u16::from_le_bytes([header[10], header[11]]);
        let crc = u32::from_le_bytes([header[16], header[17], header[18], header[19]]);
        let compressed = u32::from_le_bytes([header[20], header[21], header[22], header[23]]);
        let uncompressed = u32::from_le_bytes([header[24], header[25], header[26], header[27]]);
        let name_len = usize::from(u16::from_le_bytes([header[28], header[29]]));
        let extra_len = usize::from(u16::from_le_bytes([header[30], header[31]]));
        let comment_len = usize::from(u16::from_le_bytes([header[32], header[33]]));
        let local_offset = u32::from_le_bytes([header[42], header[43], header[44], header[45]]);
        let name = zip
            .get(cursor + 46..cursor + 46 + name_len)
            .ok_or_else(|| "the central directory is truncated".to_owned())?;
        let name = String::from_utf8_lossy(name).into_owned();
        let entry = Entry {
            name,
            flags,
            method,
            crc,
            compressed,
            uncompressed,
            local_offset,
        };
        let wanted = chosen.as_ref().is_none_or(|current| {
            entry.name == "cities15000.txt" || !has_extension(&current.name, "txt")
        });
        if has_extension(&entry.name, "txt") && wanted {
            chosen = Some(entry);
        }
        cursor += 46 + name_len + extra_len + comment_len;
    }

    chosen.ok_or_else(|| "the archive has no .txt member".to_owned())
}

/// Whether a member name carries `extension`, case-insensitively.
#[cfg(feature = "offline-geo")]
fn has_extension(name: &str, extension: &str) -> bool {
    std::path::Path::new(name)
        .extension()
        .and_then(std::ffi::OsStr::to_str)
        .is_some_and(|found| found.eq_ignore_ascii_case(extension))
}

/// One central-directory entry, reduced to the fields the extractor needs.
#[cfg(feature = "offline-geo")]
struct Entry {
    name: String,
    flags: u16,
    method: u16,
    crc: u32,
    compressed: u32,
    uncompressed: u32,
    local_offset: u32,
}

/// The last end-of-central-directory record, searched backwards over the largest comment a ZIP
/// may carry.
#[cfg(feature = "offline-geo")]
fn find_eocd(zip: &[u8]) -> Option<usize> {
    if zip.len() < 22 {
        return None;
    }
    let last = zip.len() - 22;
    let first = zip.len().saturating_sub(22 + 65_535);
    (first..=last)
        .rev()
        .find(|&at| zip.get(at..at + 4) == Some(EOCD_SIGNATURE))
}

#[cfg(feature = "offline-geo")]
fn read_u16(zip: &[u8], at: usize) -> std::result::Result<u16, String> {
    let bytes = zip
        .get(at..at + 2)
        .ok_or_else(|| "the archive is truncated".to_owned())?;
    Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
}

#[cfg(feature = "offline-geo")]
fn read_u32(zip: &[u8], at: usize) -> std::result::Result<u32, String> {
    let bytes = zip
        .get(at..at + 4)
        .ok_or_else(|| "the archive is truncated".to_owned())?;
    Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

/// The CRC-32 the ZIP format stores (reflected polynomial 0xEDB88320), bitwise because one
/// download per refresh does not need a table.
#[cfg(feature = "offline-geo")]
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFF_u32;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone as _, Utc};

    use super::note_due;
    #[cfg(feature = "offline-geo")]
    use super::{crc32, extract_cities_txt, find_eocd, source_kind};

    #[test]
    fn the_note_fires_only_for_an_old_table_and_throttles_for_a_day() {
        let now = Utc.with_ymd_and_hms(2026, 10, 4, 12, 0, 0).unwrap();
        let fresh = chrono::NaiveDate::from_ymd_opt(2026, 10, 1).unwrap();
        let old = chrono::NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();

        assert_eq!(note_due(now, fresh, 90, None), None);
        assert_eq!(note_due(now, old, 90, None), Some(276));
        // A note printed an hour ago suppresses the next one; a day-old note does not.
        let hour_ago = now - chrono::TimeDelta::hours(1);
        assert_eq!(note_due(now, old, 90, Some(hour_ago)), None);
        let day_ago = now - chrono::TimeDelta::hours(25);
        assert_eq!(note_due(now, old, 90, Some(day_ago)), Some(276));
    }

    #[cfg(feature = "offline-geo")]
    #[test]
    fn sources_are_classified_by_extension() {
        assert!(matches!(
            source_kind("https://example.org/cities15000.zip?v=2"),
            Ok(super::Kind::Zip)
        ));
        assert!(matches!(
            source_kind("/tmp/cities15000.txt"),
            Ok(super::Kind::Text)
        ));
        let error = source_kind("/tmp/dump.tar.gz").expect_err("an unknown extension");
        assert_eq!(error.exit_code(), 2);
    }

    #[cfg(feature = "offline-geo")]
    #[test]
    fn the_zip_reader_extracts_a_stored_and_a_deflated_member() {
        // Built by `tests/fixtures/geo/make_sample_zip.py`? No: the fixture is committed, and the
        // reader is exercised here against an in-memory archive so the test has no file
        // dependency beyond the fixture used by the CLI suite.
        let stored = stored_zip(
            "cities15000.txt",
            b"1\tX\tX\t\t1.0\t2.0\tP\tPPL\tXX\t\t\t\t\t\t10\t\t\t\t2026-01-01\n",
        );
        let text = extract_cities_txt(&stored).expect("a stored member extracts");
        assert!(text.contains('X'), "{text}");
    }

    #[cfg(feature = "offline-geo")]
    #[test]
    fn the_committed_fixture_zip_extracts() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/geo/cities-sample.zip"
        );
        let zip = std::fs::read(path).expect("the fixture is readable");
        let text = extract_cities_txt(&zip).expect("the fixture extracts");
        assert!(text.contains("Sampleville"), "{text}");
    }

    #[cfg(feature = "offline-geo")]
    #[test]
    fn the_zip_reader_refuses_garbage_and_missing_members() {
        assert!(extract_cities_txt(b"not a zip").is_err());
        let empty = stored_zip("readme.md", b"hello");
        let error = extract_cities_txt(&empty).expect_err("no .txt member");
        assert!(error.contains("no .txt member"), "{error}");
    }

    #[cfg(feature = "offline-geo")]
    #[test]
    fn the_zip_reader_refuses_truncated_zip64_and_encrypted_archives() {
        let archive = stored_zip(
            "cities15000.txt",
            b"1\tX\tX\t\t1.0\t2.0\tP\tPPL\tXX\t\t\t\t\t\t10\t\t\t\t2026-01-01\n",
        );

        // Truncated: the central directory or the member data is cut off.
        let error = extract_cities_txt(&archive[..archive.len() / 2]).expect_err("truncated");
        assert!(!error.is_empty(), "{error}");

        // zip64: the end-of-central-directory record uses a sentinel count.
        let mut zip64 = archive.clone();
        let eocd = find_eocd(&zip64).expect("the archive has an EOCD");
        zip64[eocd + 10..eocd + 12].copy_from_slice(&u16::MAX.to_le_bytes());
        zip64[eocd + 8..eocd + 10].copy_from_slice(&u16::MAX.to_le_bytes());
        let error = extract_cities_txt(&zip64).expect_err("zip64");
        assert!(error.contains("zip64"), "{error}");

        // Encrypted: the general-purpose flag's bit 0 in both headers.
        let mut encrypted = archive.clone();
        let eocd = find_eocd(&encrypted).expect("the archive has an EOCD");
        let central = u32::from_le_bytes([
            encrypted[eocd + 16],
            encrypted[eocd + 17],
            encrypted[eocd + 18],
            encrypted[eocd + 19],
        ]) as usize;
        encrypted[central + 8] |= 0x1;
        let error = extract_cities_txt(&encrypted).expect_err("encrypted");
        assert!(error.contains("encrypted"), "{error}");
    }

    #[cfg(feature = "offline-geo")]
    #[test]
    fn crc32_matches_the_known_check_value() {
        // The standard check value: CRC-32 of "123456789".
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    /// A one-entry ZIP with a stored member, built in memory so the reader's happy path is
    /// exercised without depending on how `zip` tooling happens to write archives.
    #[cfg(feature = "offline-geo")]
    fn stored_zip(name: &str, body: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let crc = crc32(body);
        let name = name.as_bytes();
        // The fixture bodies are tiny; the ZIP fields are 32-bit, so a checked conversion keeps
        // the test helper honest without clippy noise.
        let len = u32::try_from(body.len()).expect("a small test body");
        let name_len = u16::try_from(name.len()).expect("a short test name");
        // Local header.
        out.extend_from_slice(b"PK\x03\x04");
        out.extend_from_slice(&20_u16.to_le_bytes()); // version needed
        out.extend_from_slice(&0_u16.to_le_bytes()); // flags
        out.extend_from_slice(&0_u16.to_le_bytes()); // method: stored
        out.extend_from_slice(&0_u16.to_le_bytes()); // time
        out.extend_from_slice(&0_u16.to_le_bytes()); // date
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&name_len.to_le_bytes());
        out.extend_from_slice(&0_u16.to_le_bytes()); // extra
        out.extend_from_slice(name);
        out.extend_from_slice(body);
        let local_size = out.len();
        // Central directory.
        out.extend_from_slice(b"PK\x01\x02");
        out.extend_from_slice(&20_u16.to_le_bytes()); // version made by
        out.extend_from_slice(&20_u16.to_le_bytes()); // version needed
        out.extend_from_slice(&0_u16.to_le_bytes()); // flags
        out.extend_from_slice(&0_u16.to_le_bytes()); // method
        out.extend_from_slice(&0_u16.to_le_bytes()); // time
        out.extend_from_slice(&0_u16.to_le_bytes()); // date
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&name_len.to_le_bytes());
        out.extend_from_slice(&0_u16.to_le_bytes()); // extra
        out.extend_from_slice(&0_u16.to_le_bytes()); // comment
        out.extend_from_slice(&0_u16.to_le_bytes()); // disk
        out.extend_from_slice(&0_u16.to_le_bytes()); // internal attrs
        out.extend_from_slice(&0_u32.to_le_bytes()); // external attrs
        out.extend_from_slice(&0_u32.to_le_bytes()); // local header offset
        out.extend_from_slice(name);
        let central_size = out.len() - local_size;
        // End of central directory.
        out.extend_from_slice(b"PK\x05\x06");
        out.extend_from_slice(&0_u16.to_le_bytes());
        out.extend_from_slice(&0_u16.to_le_bytes());
        out.extend_from_slice(&1_u16.to_le_bytes());
        out.extend_from_slice(&1_u16.to_le_bytes());
        out.extend_from_slice(
            &u32::try_from(central_size)
                .expect("a small test archive")
                .to_le_bytes(),
        );
        out.extend_from_slice(
            &u32::try_from(local_size)
                .expect("a small test archive")
                .to_le_bytes(),
        );
        out.extend_from_slice(&0_u16.to_le_bytes());
        out
    }
}
