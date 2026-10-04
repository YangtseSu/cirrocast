// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Builds the two members cirrocast embeds for its offline geocoder from a `GeoNames`
//! `cities15000.txt` (step 18).
//!
//! ```text
//! cargo run -p geo-table -- <cities15000.txt> [output-dir]   # default: src/geo/data
//! ```
//!
//! Two files land in `output-dir`, plus a `SNAPSHOT` record of the input: `cities.bin.gz` (the
//! rows), `keys.bin.gz` (the folded-key index) and `SNAPSHOT` (dump date, input SHA-256, row and
//! key counts). The format, the dump parser and the gzip writer live in
//! `cirrocast::geo::table`, which the runtime's user-facing update path shares, so this binary is
//! only the command line and the file writing.
//!
//! The output is deterministic: the input order fixes the rows, the keys are sorted, and the gzip
//! members are written with a fixed timestamp — the same input always produces the same bytes.

use std::env;
use std::fs;
use std::path::Path;
use std::process::ExitCode;

use cirrocast::geo::table;

/// Where the members are written when the second argument is omitted.
const DEFAULT_OUTPUT: &str = "src/geo/data";

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
    build(Path::new(&input), Path::new(&output))
}

/// Reads `input`, writes the three files under `output` and returns the report line.
fn build(input: &Path, output: &Path) -> Result<String, String> {
    let raw =
        fs::read(input).map_err(|error| format!("cannot read {}: {error}", input.display()))?;
    let checksum = table::input_sha256(&raw);
    let text = String::from_utf8(raw)
        .map_err(|error| format!("{} is not UTF-8: {error}", input.display()))?;

    let dump = table::parse_dump(&text)?;
    fs::create_dir_all(output)
        .map_err(|error| format!("cannot create {}: {error}", output.display()))?;

    let cities = table::gzip(&table::encode_cities(&dump.cities)?)?;
    let keys = table::gzip(&table::encode_keys(&dump.keys)?)?;
    let snapshot = table::snapshot_text(&dump, &checksum);

    let cities_path = output.join("cities.bin.gz");
    let keys_path = output.join("keys.bin.gz");
    let snapshot_path = output.join("SNAPSHOT");
    fs::write(&cities_path, &cities)
        .map_err(|error| format!("cannot write {}: {error}", cities_path.display()))?;
    fs::write(&keys_path, &keys)
        .map_err(|error| format!("cannot write {}: {error}", keys_path.display()))?;
    fs::write(&snapshot_path, snapshot.as_bytes())
        .map_err(|error| format!("cannot write {}: {error}", snapshot_path.display()))?;

    Ok(format!(
        "{} rows ({} skipped), {} keys\ncities.bin.gz {:>10} bytes\nkeys.bin.gz    {:>10} bytes\nSNAPSHOT       {}",
        dump.cities.len(),
        dump.skipped,
        dump.keys.len(),
        cities.len(),
        keys.len(),
        snapshot_path.display()
    ))
}

#[cfg(test)]
mod tests {
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
        let report = build(&input, &first).expect("the build succeeds");
        assert!(report.contains("1 rows (1 skipped), 3 keys"), "{report}");
        build(&input, &second).expect("the second build succeeds");
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
}
