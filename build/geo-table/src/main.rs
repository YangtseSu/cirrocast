// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Builds the two members cirrocast embeds for its offline geocoder from a `GeoNames`
//! `cities15000` dump (step 18), or checks the committed snapshot against a dump (step 18b).
//!
//! ```text
//! cargo run -p geo-table -- <path-or-url> [output-dir]   # default: src/geo/data
//! cargo run -p geo-table -- <path-or-url> --check        # compare, write nothing
//! ```
//!
//! The source may be a local `cities15000.txt`, a local `.zip`, or an `http(s)` URL of either.
//! Reading/fetching, extracting and encoding are `cirrocast::geo::update::build_candidate` — the
//! same code `cirrocast location update-data` runs — so the maintainer path that writes the
//! committed snapshot and the user path that installs a table under `$XDG_DATA_HOME` cannot drift;
//! this binary only decides *where* the three files go (or whether they would change) and prints
//! the report.
//!
//! A build writes `cities.bin.gz` (the rows), `keys.bin.gz` (the folded-key index) and `SNAPSHOT`
//! (dump date, input SHA-256, row and key counts) into `output-dir`. The output is deterministic —
//! the same input always produces the same bytes — so a refresh is a reviewable diff; `--check`
//! makes that diff visible without writing anything and exits 1 when the dump differs.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use cirrocast::cache::SystemClock;
use cirrocast::config::{Config, Network};
use cirrocast::geo::country;
use cirrocast::geo::update::{self, Candidate};
use cirrocast::http::{HttpClient, UreqTransport};
use cirrocast::paths::Paths;

/// Where the members are written when the second argument is omitted.
const DEFAULT_OUTPUT: &str = "src/geo/data";

/// The per-attempt timeout for a URL source; a dev tool can afford a slow link.
const TIMEOUT: Duration = Duration::from_secs(60);

/// The three files a table is made of, in the order every report lists them.
const MEMBERS: [&str; 3] = ["cities.bin.gz", "keys.bin.gz", "SNAPSHOT"];

/// What one run did, and the report to print.
enum Outcome {
    /// The members were written; exit 0.
    Built(String),
    /// `--check` found the committed snapshot current; exit 0.
    Current(String),
    /// `-h`/`--help` was passed; exit 0.
    Help(String),
    /// `--check` found a difference; exit 1.
    Differs(String),
}

fn main() -> ExitCode {
    match run() {
        Ok(Outcome::Built(report) | Outcome::Current(report) | Outcome::Help(report)) => {
            println!("{report}");
            ExitCode::SUCCESS
        }
        Ok(Outcome::Differs(report)) => {
            println!("{report}");
            eprintln!(
                "the dump differs; run without `--check` to refresh, then run `cargo test \
                 --workspace` (the canaries pin rows of the committed snapshot) and re-record the \
                 size/timing numbers in docs/plans/21-perf-and-resource-budget.md."
            );
            ExitCode::FAILURE
        }
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn usage() -> String {
    "usage: cargo run -p geo-table -- <path-or-url> [output-dir] [--check]\n\
           cargo run -p geo-table -- --countries <path-or-url> [output-dir] [--check]"
        .to_owned()
}

/// The `-h`/`--help` text: the synopsis plus one line on what the tool does.
fn help() -> String {
    format!(
        "{}\n\nBuilds the offline city table (`cities.bin.gz`, `keys.bin.gz`, `SNAPSHOT`) from a\n\
         GeoNames cities15000 dump — a local `.txt`/`.zip` or an `http(s)` URL — or, with\n\
         `--check`, compares the committed snapshot against a dump and exits 1 when it differs.\n\
         With `--countries`, builds the offline country layer (`countries.bin.gz`, `COUNTRIES`)\n\
         from a Natural Earth `admin_0_countries` GeoJSON (1:50m; the 1:10m dataset is the\n\
         documented fallback when the compressed member would exceed the 1 MiB budget).",
        usage()
    )
}

/// Whether the arguments ask for the help text rather than a build.
fn wants_help(args: &[String]) -> bool {
    args.iter()
        .any(|argument| argument == "-h" || argument == "--help")
}

/// What a run builds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// The city table from a `GeoNames` `cities15000` dump (step 18).
    Cities,
    /// The country layer from a Natural Earth `admin_0_countries` `GeoJSON` (step 25).
    Countries,
}

/// The parsed command line.
struct Args {
    /// The dump to read: a `.txt`/`.zip` path or an `http(s)` URL of either (cities), or a
    /// `.geojson` path or URL of one (countries).
    source: String,
    /// What to build.
    mode: Mode,
    /// Where the members live (or would be written).
    output: PathBuf,
    /// Compare instead of writing.
    check: bool,
}

/// Parses the arguments; `--check` may appear before or after the positionals, and `--countries`
/// (with its value) selects the country layer.
fn parse_args(args: impl IntoIterator<Item = String>) -> Result<Args, String> {
    let mut check = false;
    let mut countries: Option<String> = None;
    let mut positional: Vec<String> = Vec::new();
    let mut arguments = args.into_iter();
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "-h" | "--help" => return Err(usage()),
            "--check" => check = true,
            "--countries" => {
                let value = arguments
                    .next()
                    .ok_or_else(|| format!("--countries needs a path or URL\n{}", usage()))?;
                countries = Some(value);
            }
            other if other.starts_with("--countries=") => {
                let value = other.trim_start_matches("--countries=").to_owned();
                if value.is_empty() {
                    return Err(format!("--countries needs a path or URL\n{}", usage()));
                }
                countries = Some(value);
            }
            _ => positional.push(argument),
        }
    }

    let (source, mode, output_position) = match countries {
        // `--countries` supplies the source itself, so the one optional positional is the output.
        Some(source) => (source, Mode::Countries, 0),
        None => (
            positional.first().cloned().ok_or_else(usage)?,
            Mode::Cities,
            1,
        ),
    };
    if positional.len() > output_position + 1 {
        return Err(usage());
    }
    let output = positional
        .get(output_position)
        .map_or_else(|| DEFAULT_OUTPUT.to_owned(), Clone::clone);
    Ok(Args {
        source,
        mode,
        output: PathBuf::from(output),
        check,
    })
}

fn run() -> Result<Outcome, String> {
    let raw: Vec<String> = env::args().skip(1).collect();
    if wants_help(&raw) {
        return Ok(Outcome::Help(help()));
    }
    let args = parse_args(raw)?;
    match (args.mode, args.check) {
        (Mode::Cities, true) => check_snapshot(&args.source, &args.output),
        (Mode::Cities, false) => build(&args.source, &args.output).map(Outcome::Built),
        (Mode::Countries, true) => check_countries(&args.source, &args.output),
        (Mode::Countries, false) => build_countries(&args.source, &args.output).map(Outcome::Built),
    }
}

/// Builds the country layer from `source` and writes its two files under `output`.
fn build_countries(source: &str, output: &Path) -> Result<String, String> {
    let candidate = country_candidate(source)?;
    fs::create_dir_all(output)
        .map_err(|error| format!("cannot create {}: {error}", output.display()))?;
    for (name, bytes) in [
        (country::MEMBER, candidate.member.as_slice()),
        (country::RECORD, candidate.record.as_bytes()),
    ] {
        let path = output.join(name);
        fs::write(&path, bytes)
            .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
    }
    Ok(format!(
        "{}\nNext: run `cargo test --workspace` (the layer's canary pins coordinates and the size \
         budget), re-record the size in docs/plans/25-location-sources-2.md, then commit.",
        country_summary(&candidate, output)
    ))
}

/// Builds the candidate and compares it with the files already in `output`; writes nothing.
fn check_countries(source: &str, output: &Path) -> Result<Outcome, String> {
    use std::fmt::Write as _;

    let candidate = country_candidate(source)?;
    let mut report = String::from("== committed country layer vs this source ==\n");
    let mut differs = false;
    for name in [country::MEMBER, country::RECORD] {
        let path = output.join(name);
        let committed =
            fs::read(&path).map_err(|error| format!("cannot read {}: {error}", path.display()))?;
        let fresh = if name == country::MEMBER {
            &candidate.member
        } else {
            candidate.record.as_bytes()
        };
        let same = committed == fresh;
        differs |= !same;
        // Writing into the string cannot fail; the result is discarded because `String`'s `Write`
        // never returns an error.
        let _ = writeln!(
            report,
            "  {:<16} {}",
            name,
            if same { "unchanged" } else { "CHANGED" }
        );
    }
    if differs {
        Ok(Outcome::Differs(report))
    } else {
        let committed = fs::read_to_string(output.join(country::RECORD)).ok();
        let _ = write!(
            report,
            "\ncommitted: {}\nthis source: {}\nthe committed country layer is current.",
            describe_record(committed.as_deref()),
            describe_record(Some(&candidate.record))
        );
        Ok(Outcome::Current(report))
    }
}

/// The country build path itself: fetch or read, parse, encode, validate.
fn country_candidate(source: &str) -> Result<country::CountryCandidate, String> {
    let network = network_config();
    let transport = UreqTransport::new(&network, TIMEOUT)
        .map_err(|error| format!("cannot build the HTTP client: {error}"))?;
    let http = HttpClient::new(
        Box::new(transport),
        network.retries,
        Arc::new(SystemClock),
        0,
    );
    country::build_candidate(source, &http, 1).map_err(|error| error.to_string())
}

/// The country build report: counts, sizes and where the record went.
fn country_summary(candidate: &country::CountryCandidate, output: &Path) -> String {
    let kib = candidate.member.len().div_ceil(1024);
    format!(
        "{} countries ({} skipped), {} points\n{:<16}{:>10} bytes ({kib} KiB)\n{:<16}{}",
        candidate.countries,
        candidate.skipped,
        candidate.points,
        country::MEMBER,
        candidate.member.len(),
        country::RECORD,
        output.join(country::RECORD).display()
    )
}

/// One `COUNTRIES` record as a single line, for the `--check` report.
fn describe_record(record: Option<&str>) -> String {
    let Some(record) = record else {
        return "no COUNTRIES record".to_owned();
    };
    let field = |key: &str| {
        record
            .lines()
            .find_map(|line| line.strip_prefix(&format!("{key} = ")))
            .map_or("?", str::trim)
    };
    format!(
        "{} countries, {} points, scale {} (input sha256 {})",
        field("countries"),
        field("points"),
        field("scale"),
        field("input-sha256")
    )
}

/// Builds the table from `source` and writes the three files under `output`.
fn build(source: &str, output: &Path) -> Result<String, String> {
    let candidate = candidate(source)?;
    fs::create_dir_all(output)
        .map_err(|error| format!("cannot create {}: {error}", output.display()))?;
    for (name, bytes) in [
        ("cities.bin.gz", candidate.cities_gz.as_slice()),
        ("keys.bin.gz", candidate.keys_gz.as_slice()),
        ("SNAPSHOT", candidate.snapshot.as_bytes()),
    ] {
        let path = output.join(name);
        fs::write(&path, bytes)
            .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
    }
    Ok(format!(
        "{}\nNext: run `cargo test --workspace` (the canaries pin rows of the committed snapshot), \
         re-record the size/timing numbers in docs/plans/21-perf-and-resource-budget.md, then \
         commit.",
        summary(&candidate, output)
    ))
}

/// Builds the candidate and compares it with the files already in `output`; writes nothing.
fn check_snapshot(source: &str, output: &Path) -> Result<Outcome, String> {
    use std::fmt::Write as _;

    let candidate = candidate(source)?;
    let mut report = String::from("== committed snapshot vs this dump ==\n");
    let mut differs = false;
    for name in MEMBERS {
        let path = output.join(name);
        let committed =
            fs::read(&path).map_err(|error| format!("cannot read {}: {error}", path.display()))?;
        let fresh = match name {
            "cities.bin.gz" => &candidate.cities_gz,
            "keys.bin.gz" => &candidate.keys_gz,
            _ => candidate.snapshot.as_bytes(),
        };
        let same = committed == fresh;
        differs |= !same;
        // Writing into the string cannot fail; the result is discarded because `String`'s `Write`
        // never returns an error.
        let _ = writeln!(
            report,
            "  {:<12} {}",
            name,
            if same { "unchanged" } else { "CHANGED" }
        );
    }
    let committed = fs::read_to_string(output.join("SNAPSHOT")).ok();
    let _ = write!(
        report,
        "\ncommitted: {}\nthis dump: {}\n",
        describe(committed.as_deref()),
        describe(Some(&candidate.snapshot))
    );
    if differs {
        Ok(Outcome::Differs(report))
    } else {
        let dump = if candidate.dump_date.is_empty() {
            "date unknown"
        } else {
            &candidate.dump_date
        };
        let _ = write!(
            report,
            "the committed snapshot is current for this dump (dump {dump})."
        );
        Ok(Outcome::Current(report))
    }
}

/// The build path itself: fetch or read, extract, parse, encode, validate.
fn candidate(source: &str) -> Result<Candidate, String> {
    let network = network_config();
    let transport = UreqTransport::new(&network, TIMEOUT)
        .map_err(|error| format!("cannot build the HTTP client: {error}"))?;
    let http = HttpClient::new(
        Box::new(transport),
        network.retries,
        Arc::new(SystemClock),
        0,
    );
    update::build_candidate(source, &http, 1).map_err(|error| error.to_string())
}

/// The `[network]` table the maintainer's configuration holds, so the builder dials through the
/// same proxy `cirrocast location update-data` would.
///
/// A missing or unreadable configuration falls back to [`Network::default`], which still honours
/// `HTTPS_PROXY`/`ALL_PROXY`/`NO_PROXY` from the environment.
fn network_config() -> Network {
    let Ok(paths) = Paths::resolve() else {
        return Network::default();
    };
    Config::load(&paths).map_or_else(|_| Network::default(), |config| config.network)
}

/// The build report: counts, sizes and where the `SNAPSHOT` went.
fn summary(candidate: &Candidate, output: &Path) -> String {
    format!(
        "{} rows ({} skipped), {} keys\ncities.bin.gz {:>10} bytes\nkeys.bin.gz    {:>10} bytes\nSNAPSHOT       {}",
        candidate.rows,
        candidate.skipped,
        candidate.keys,
        candidate.cities_gz.len(),
        candidate.keys_gz.len(),
        output.join("SNAPSHOT").display()
    )
}

/// One `SNAPSHOT` record as a single line, for the `--check` report.
fn describe(snapshot: Option<&str>) -> String {
    let Some(snapshot) = snapshot else {
        return "no SNAPSHOT record".to_owned();
    };
    let field = |key: &str| {
        snapshot
            .lines()
            .find_map(|line| line.strip_prefix(&format!("{key} = ")))
            .map_or("?", str::trim)
    };
    format!(
        "dump {}, {} rows, {} keys (input sha256 {})",
        field("dump-date"),
        field("rows"),
        field("keys"),
        field("input-sha256")
    )
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{
        Mode, Outcome, build, build_countries, check_countries, check_snapshot, help, parse_args,
        wants_help,
    };
    use cirrocast::geo::country;

    /// A dump with one usable row and one without a zone.
    const SAMPLE: &str = "\
123\tMünchen\tMunchen\tMuenchen,Munich\t48.13743\t11.57549\tP\tPPLA\tDE\t\t02\t\t\t\t1260391\t519\t519\tEurope/Berlin\t2026-09-02
789\tNo Zone\tNo Zone\t\t1.0\t2.0\tP\tPPL\tXX\t\t\t\t\t\t10\t\t\t\t2026-09-02
";

    /// A miniature Natural Earth document: one square, one triangle, one feature without geometry.
    const COUNTRY_SAMPLE: &str = r#"{"type":"FeatureCollection","features":[
        {"type":"Feature","properties":{"NAME":"Square","ISO_A2":"SQ","ISO_A2_EH":"SQ"},
         "geometry":{"type":"Polygon","coordinates":[[[0,0],[2,0],[2,2],[0,2],[0,0]]]}},
        {"type":"Feature","properties":{"NAME":"Triangle","ISO_A2":"-99","ISO_A2_EH":"-99"},
         "geometry":{"type":"Polygon","coordinates":[[[10,10],[12,10],[11,12],[10,10]]]}},
        {"type":"Feature","properties":{"NAME":"Nowhere","ISO_A2":"-99"},"geometry":null}
    ]}"#;

    /// A test directory named after the calling test.
    fn scratch(name: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!("geo-table-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("the test directory");
        root
    }

    #[test]
    fn the_build_writes_all_three_files_and_is_deterministic() {
        let root = scratch("build");
        let input = root.join("cities15000.txt");
        std::fs::write(&input, SAMPLE).expect("the sample dump");

        let first = root.join("first");
        let second = root.join("second");
        let report = build(&input.to_string_lossy(), &first).expect("the build succeeds");
        assert!(report.contains("1 rows (1 skipped), 3 keys"), "{report}");
        build(&input.to_string_lossy(), &second).expect("the second build succeeds");
        for name in ["cities.bin.gz", "keys.bin.gz", "SNAPSHOT"] {
            let a = std::fs::read(first.join(name)).expect("the first member");
            let b = std::fs::read(second.join(name)).expect("the second member");
            assert_eq!(a, b, "{name} differs between runs");
        }
        let snapshot = std::fs::read_to_string(first.join("SNAPSHOT")).expect("the snapshot");
        assert!(snapshot.contains("dump-date = 2026-09-02"), "{snapshot}");
        assert!(snapshot.contains("rows = 1"), "{snapshot}");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_zip_source_builds_the_same_table_as_the_text() {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/geo");
        let root = scratch("zip");
        let from_txt = root.join("txt");
        let from_zip = root.join("zip");

        let text = build(
            &fixture.join("cities-sample.txt").to_string_lossy(),
            &from_txt,
        )
        .expect("the .txt builds");
        assert!(text.contains("2 rows (0 skipped), 3 keys"), "{text}");
        build(
            &fixture.join("cities-sample.zip").to_string_lossy(),
            &from_zip,
        )
        .expect("the .zip builds");
        for name in ["cities.bin.gz", "keys.bin.gz", "SNAPSHOT"] {
            let a = std::fs::read(from_txt.join(name)).expect("the text member");
            let b = std::fs::read(from_zip.join(name)).expect("the zip member");
            assert_eq!(a, b, "{name} differs between the .txt and the .zip");
        }

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn check_reports_unchanged_current_and_changed() {
        let root = scratch("check");
        let input = root.join("cities15000.txt");
        std::fs::write(&input, SAMPLE).expect("the sample dump");
        let output = root.join("data");
        build(&input.to_string_lossy(), &output).expect("the first build");

        // The same dump: current, exit 0, nothing written.
        let outcome = check_snapshot(&input.to_string_lossy(), &output).expect("the check runs");
        let Outcome::Current(report) = outcome else {
            panic!("expected the snapshot to be current");
        };
        assert!(report.contains("unchanged"), "{report}");
        assert!(report.contains("dump 2026-09-02"), "{report}");

        // A dump with one more row: differs, exit 1.
        let changed = root.join("changed.txt");
        std::fs::write(&changed, format!("{SAMPLE}9900001\tNew\tNew\t\t1.0\t2.0\tP\tPPL\tZZ\t\t\t\t\t\t10\t\t\t\t2026-09-02\n"))
            .expect("the changed dump");
        let outcome = check_snapshot(&changed.to_string_lossy(), &output).expect("the check runs");
        let Outcome::Differs(report) = outcome else {
            panic!("expected the snapshot to differ");
        };
        assert!(report.contains("CHANGED"), "{report}");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_unknown_source_kind_is_refused() {
        let error =
            build("/tmp/dump.tar.gz", Path::new("/tmp/unused")).expect_err("an unknown extension");
        assert!(error.contains(".txt") && error.contains(".zip"), "{error}");
    }

    #[test]
    fn the_argument_parser_accepts_the_documented_forms() {
        let args = parse_args(["--check".to_owned(), "a.zip".to_owned()]).expect("parses");
        assert!(args.check, "--check is seen before the positionals");
        assert_eq!(args.source, "a.zip");
        assert_eq!(args.mode, Mode::Cities);
        assert_eq!(args.output, Path::new("src/geo/data"));

        let args = parse_args(["a.zip".to_owned(), "out".to_owned(), "--check".to_owned()])
            .expect("parses");
        assert!(args.check, "--check is seen after the positionals");
        assert_eq!(args.output, Path::new("out"));

        let args = parse_args(["a.txt".to_owned()]).expect("parses");
        assert!(!args.check);
        assert!(
            parse_args(Vec::new()).is_err(),
            "no source is a usage error"
        );
        assert!(
            parse_args(["a".to_owned(), "b".to_owned(), "c".to_owned()]).is_err(),
            "a third positional is a usage error"
        );
        assert!(parse_args(["--help".to_owned()]).is_err());

        // The country form takes its source from the flag, so the one positional is the output.
        let args = parse_args([
            "--countries".to_owned(),
            "ne.geojson".to_owned(),
            "out".to_owned(),
        ])
        .expect("parses");
        assert_eq!(args.mode, Mode::Countries);
        assert_eq!(args.source, "ne.geojson");
        assert_eq!(args.output, Path::new("out"));
        let args = parse_args(["--countries=ne.geojson".to_owned()]).expect("parses");
        assert_eq!(args.mode, Mode::Countries);
        assert_eq!(args.source, "ne.geojson");
        assert_eq!(args.output, Path::new("src/geo/data"));
        assert!(
            parse_args(["--countries".to_owned()]).is_err(),
            "--countries without a value is a usage error"
        );
        assert!(
            parse_args([
                "--countries".to_owned(),
                "ne.geojson".to_owned(),
                "out".to_owned(),
                "extra".to_owned()
            ])
            .is_err(),
            "a second positional in the country form is a usage error"
        );
    }

    #[test]
    fn the_country_build_writes_both_files_and_is_deterministic() {
        let root = scratch("countries");
        let input = root.join("ne_50m_admin_0_countries.geojson");
        std::fs::write(&input, COUNTRY_SAMPLE).expect("the sample GeoJSON");

        let first = root.join("first");
        let second = root.join("second");
        let report = build_countries(&input.to_string_lossy(), &first).expect("the build succeeds");
        assert!(
            report.contains("2 countries (1 skipped), 9 points"),
            "{report}"
        );
        build_countries(&input.to_string_lossy(), &second).expect("the second build succeeds");
        for name in [country::MEMBER, country::RECORD] {
            let a = std::fs::read(first.join(name)).expect("the first member");
            let b = std::fs::read(second.join(name)).expect("the second member");
            assert_eq!(a, b, "{name} differs between runs");
        }
        let record = std::fs::read_to_string(first.join(country::RECORD)).expect("the record");
        assert!(record.contains("countries = 2"), "{record}");
        assert!(record.contains("points = 9"), "{record}");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_country_check_reports_unchanged_and_changed() {
        let root = scratch("countries-check");
        let input = root.join("ne.geojson");
        std::fs::write(&input, COUNTRY_SAMPLE).expect("the sample GeoJSON");
        let output = root.join("data");
        build_countries(&input.to_string_lossy(), &output).expect("the first build");

        let outcome = check_countries(&input.to_string_lossy(), &output).expect("the check runs");
        let Outcome::Current(report) = outcome else {
            panic!("expected the layer to be current");
        };
        assert!(report.contains("unchanged"), "{report}");
        assert!(report.contains("2 countries"), "{report}");

        let changed = root.join("changed.geojson");
        std::fs::write(&changed, COUNTRY_SAMPLE.replace("Square", "Oblong"))
            .expect("the changed GeoJSON");
        let outcome = check_countries(&changed.to_string_lossy(), &output).expect("the check runs");
        let Outcome::Differs(report) = outcome else {
            panic!("expected the layer to differ");
        };
        assert!(report.contains("CHANGED"), "{report}");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn help_is_recognised_before_a_build_is_attempted() {
        assert!(wants_help(&["--help".to_owned()]));
        assert!(wants_help(&["-h".to_owned()]));
        assert!(wants_help(&["a.zip".to_owned(), "--help".to_owned()]));
        assert!(!wants_help(&["a.zip".to_owned()]));
        assert!(help().contains("usage:"), "{}", help());
        assert!(help().contains("--countries"), "{}", help());
    }
}
