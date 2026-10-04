// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Builds the two members cirrocast embeds for its offline geocoder from a `GeoNames`
//! `cities15000` dump (step 18).
//!
//! ```text
//! cargo run -p geo-table -- <path-or-url> [output-dir]   # default: src/geo/data
//! ```
//!
//! The source may be a local `cities15000.txt`, a local `.zip`, or an `http(s)` URL of either.
//! Reading/fetching, extracting and encoding are `cirrocast::geo::update::build_candidate` — the
//! same code `cirrocast location update-data` runs — so the maintainer path that writes the
//! committed snapshot and the user path that installs a table under `$XDG_DATA_HOME` cannot drift;
//! this binary only decides *where* the three files go and prints the report.
//!
//! Two files land in `output-dir`, plus a `SNAPSHOT` record of the input: `cities.bin.gz` (the
//! rows), `keys.bin.gz` (the folded-key index) and `SNAPSHOT` (dump date, input SHA-256, row and
//! key counts). The output is deterministic — the same input always produces the same bytes — so a
//! refresh is a reviewable diff.

use std::env;
use std::fs;
use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use cirrocast::cache::SystemClock;
use cirrocast::config::Network;
use cirrocast::geo::update;
use cirrocast::http::{HttpClient, UreqTransport};

/// Where the members are written when the second argument is omitted.
const DEFAULT_OUTPUT: &str = "src/geo/data";

/// The per-attempt timeout for a URL source; a dev tool can afford a slow link.
const TIMEOUT: Duration = Duration::from_secs(60);

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
    "usage: cargo run -p geo-table -- <path-or-url> [output-dir]".to_owned()
}

fn run() -> Result<String, String> {
    let mut args = env::args().skip(1);
    let source = args.next().ok_or_else(usage)?;
    if source == "-h" || source == "--help" {
        return Err(usage());
    }
    let output = args.next().unwrap_or_else(|| DEFAULT_OUTPUT.to_owned());
    if args.next().is_some() {
        return Err(usage());
    }
    build(&source, Path::new(&output))
}

/// Builds the table from `source` and writes the three files under `output`.
fn build(source: &str, output: &Path) -> Result<String, String> {
    let network = Network::default();
    let transport = UreqTransport::new(&network, TIMEOUT)
        .map_err(|error| format!("cannot build the HTTP client: {error}"))?;
    let http = HttpClient::new(
        Box::new(transport),
        network.retries,
        Arc::new(SystemClock),
        0,
    );
    let candidate = update::build_candidate(source, &http, 1).map_err(|error| error.to_string())?;

    fs::create_dir_all(output)
        .map_err(|error| format!("cannot create {}: {error}", output.display()))?;
    let cities_path = output.join("cities.bin.gz");
    let keys_path = output.join("keys.bin.gz");
    let snapshot_path = output.join("SNAPSHOT");
    fs::write(&cities_path, &candidate.cities_gz)
        .map_err(|error| format!("cannot write {}: {error}", cities_path.display()))?;
    fs::write(&keys_path, &candidate.keys_gz)
        .map_err(|error| format!("cannot write {}: {error}", keys_path.display()))?;
    fs::write(&snapshot_path, candidate.snapshot.as_bytes())
        .map_err(|error| format!("cannot write {}: {error}", snapshot_path.display()))?;

    Ok(format!(
        "{} rows ({} skipped), {} keys\ncities.bin.gz {:>10} bytes\nkeys.bin.gz    {:>10} bytes\nSNAPSHOT       {}",
        candidate.rows,
        candidate.skipped,
        candidate.keys,
        candidate.cities_gz.len(),
        candidate.keys_gz.len(),
        snapshot_path.display()
    ))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::build;

    /// A dump with one usable row and one without a zone.
    const SAMPLE: &str = "\
123\tMünchen\tMunchen\tMuenchen,Munich\t48.13743\t11.57549\tP\tPPLA\tDE\t\t02\t\t\t\t1260391\t519\t519\tEurope/Berlin\t2026-09-02
789\tNo Zone\tNo Zone\t\t1.0\t2.0\tP\tPPL\tXX\t\t\t\t\t\t10\t\t\t\t2026-09-02
";

    #[test]
    fn the_build_writes_all_three_files_and_is_deterministic() {
        let root = std::env::temp_dir().join(format!("geo-table-test-{}", std::process::id()));
        let input = root.join("cities15000.txt");
        std::fs::create_dir_all(&root).expect("the test directory");
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
        let root = std::env::temp_dir().join(format!("geo-table-zip-test-{}", std::process::id()));
        let from_txt = root.join("txt");
        let from_zip = root.join("zip");
        std::fs::create_dir_all(&root).expect("the test directory");

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
        let snapshot = std::fs::read_to_string(from_zip.join("SNAPSHOT")).expect("the snapshot");
        assert!(snapshot.contains("dump-date = 2020-01-01"), "{snapshot}");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_unknown_source_kind_is_refused() {
        let error =
            build("/tmp/dump.tar.gz", Path::new("/tmp/unused")).expect_err("an unknown extension");
        assert!(error.contains(".txt") && error.contains(".zip"), "{error}");
    }
}
