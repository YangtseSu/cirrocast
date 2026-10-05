// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Multi-location runs (step 19): argument order under injected delays, the per-slot failure
//! contract (placeholder on stdout, full error on stderr, largest mapped exit code), and the
//! shape each format takes above one location.
//!
//! The binary tests run the real CLI against a throwaway XDG sandbox whose cache is seeded with a
//! recorded response, so every run is `--offline` and no test touches the network.

mod common;

use std::time::Duration;

use assert_cmd::Command;
use cirrocast::model::Report;
use predicates::prelude::*;

use common::Sandbox;

/// The coordinates the seeded cache entries are keyed by.
const LAT: f64 = 39.9042;
const LON: f64 = 116.4074;

/// The location argument those coordinates spell.
const LOCATION: &str = "@39.9042,116.4074";

/// A second place, so a run can hold two *different* locations.
const LAT2: f64 = 31.2304;
const LON2: f64 = 121.4737;

/// A sandbox whose weather cache holds the 3-day fixture for both coordinate pairs.
fn seeded() -> Sandbox {
    let sandbox = Sandbox::new();
    let body = std::fs::read_to_string(common::fixture_path(
        "open_meteo/forecast_beijing_2026-07-15.json",
    ))
    .expect("the fixture is readable");
    // A coordinate location still carries the provisional UTC zone when the key is built, so the
    // day is today's UTC date — the same rule the CLI follows. The seed spans local midnight so the
    // seed/execute pair cannot race it.
    for (lat, lon) in [(LAT, LON), (LAT2, LON2)] {
        common::seed_weather(
            &sandbox,
            "open-meteo",
            lat,
            lon,
            3,
            chrono_tz::Tz::UTC,
            &body,
        );
    }
    sandbox
}

/// A weather run against the seeded cache, in the given mode.
fn run(sandbox: &Sandbox, args: &[&str]) -> Command {
    let mut command = sandbox.cirrocast();
    command.args(args).arg("--offline");
    command
}

/// The stdout of a run as a string.
fn stdout(assert: &assert_cmd::assert::Assert) -> String {
    String::from_utf8(assert.get_output().stdout.clone()).expect("stdout is UTF-8")
}

/// The stderr of a run as a string.
fn stderr(assert: &assert_cmd::assert::Assert) -> String {
    String::from_utf8(assert.get_output().stderr.clone()).expect("stderr is UTF-8")
}

#[test]
fn ordering_follows_the_argument_order_whatever_the_delays_are() {
    // Distinct, identifiable names, so a slot mix-up names the misplaced place instead of printing
    // two anonymous vectors. Descending delays make the last item finish first: a completion-order
    // collector would reverse this. (`src/parallel.rs` unit-tests the same property on the internal
    // map; this drives the public `fetch_reports` wrapper the CLI uses.)
    let items = [
        "Oslo", "Cairo", "Lima", "Tokyo", "Perth", "Quito", "Dakar", "Riga",
    ];
    let results = cirrocast::fetch_reports(&items, |index, item| {
        let delay = u64::try_from(items.len() - index).unwrap_or(1);
        std::thread::sleep(Duration::from_millis(delay * 20));
        let mut report: Report = common::fixture_report("beijing-1d.json");
        report.location.name = (*item).to_owned();
        Ok(report)
    });
    for (index, (result, expected)) in results.into_iter().zip(items).enumerate() {
        let report =
            result.unwrap_or_else(|error| panic!("slot {index} ({expected}) failed: {error}"));
        assert_eq!(
            report.location.name, expected,
            "slot {index} carries `{}` instead of `{expected}`",
            report.location.name
        );
    }
}

#[test]
fn a_failed_slot_keeps_its_place_and_the_exit_code_is_the_largest_mapped_one() {
    let sandbox = seeded();

    // A location miss (5) next to a success: the placeholder occupies the second slot and the run
    // still prints the first location.
    let assert = run(&sandbox, &[LOCATION, "Nope-9x", "-f", "one-line"])
        .assert()
        .code(5);
    let out = stdout(&assert);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 2, "one line per slot: {out}");
    assert!(lines[0].contains("Clear sky"), "slot 1 renders: {out}");
    assert!(
        lines[1].starts_with("error: Nope-9x: "),
        "slot 2 is the placeholder: {out}"
    );
    let err = stderr(&assert);
    assert!(
        err.contains("error: location not found"),
        "the full error travels on stderr: {err}"
    );

    // A missing key (6) outranks the location miss, and the mixed run exits 6.
    let assert = run(
        &sandbox,
        &[LOCATION, "Nope-9x", "-p", "qweather", "-f", "one-line"],
    )
    .assert()
    .code(6);
    let out = stdout(&assert);
    assert!(out.contains("error: @39.9042,116.4074:"), "{out}");
    assert!(out.contains("error: Nope-9x:"), "{out}");

    // Two healthy locations with a missing key: every slot fails with 6.
    let assert = run(
        &sandbox,
        &[
            LOCATION,
            "@31.2304,121.4737",
            "-p",
            "qweather",
            "-f",
            "one-line",
        ],
    )
    .assert()
    .code(6);
    assert_eq!(stdout(&assert).lines().count(), 2);
    assert_eq!(stderr(&assert).matches("missing API key").count(), 2);
}

#[test]
fn every_format_prints_the_placeholder_in_the_failed_slot() {
    let sandbox = seeded();
    for format in ["one-line", "plain", "art-table", "dumb", "json"] {
        let assert = run(&sandbox, &[LOCATION, "Nope-9x", "-f", format])
            .assert()
            .code(5);
        let out = stdout(&assert);
        if format == "json" {
            assert!(
                out.contains("\"query\": \"Nope-9x\""),
                "the JSON slot names the query: {out}"
            );
        } else {
            assert!(
                out.contains("error: Nope-9x: location not found"),
                "{format} keeps the placeholder: {out}"
            );
        }
    }
}

#[test]
fn the_json_shape_depends_on_the_location_count() {
    let sandbox = seeded();
    // Two arguments that both name the same coordinates, so both slots hit the same cache entry.
    sandbox.write_config("schema_version = 2\n[locations]\ntwin = \"@39.9042,116.4074\"\n");

    let assert = run(&sandbox, &[LOCATION, "-f", "json"]).assert().success();
    let single: serde_json::Value =
        serde_json::from_str(&stdout(&assert)).expect("one location is one object");
    assert!(single.is_object(), "{single}");
    assert_eq!(single["schema_version"], 2);

    let assert = run(&sandbox, &[LOCATION, "@twin", "-f", "json"])
        .assert()
        .success();
    let both: serde_json::Value =
        serde_json::from_str(&stdout(&assert)).expect("two locations are an array");
    let array = both.as_array().expect("an array above one location");
    assert_eq!(array.len(), 2, "{both}");
    for entry in array {
        assert_eq!(entry["schema_version"], 2, "{entry}");
    }

    // A failed slot is an error document of the same version, carrying the mapped exit code.
    let assert = run(&sandbox, &[LOCATION, "Nope-9x", "-f", "json"])
        .assert()
        .code(5);
    let both: serde_json::Value =
        serde_json::from_str(&stdout(&assert)).expect("the mixed document parses");
    let array = both.as_array().expect("an array above one location");
    assert_eq!(array.len(), 2, "{both}");
    assert_eq!(array[1]["query"], "Nope-9x");
    assert_eq!(array[1]["error"]["code"], 5);
    assert_eq!(array[1]["schema_version"], 2);
}

#[test]
fn two_to_four_locations_share_one_art_table_summary() {
    let sandbox = seeded();
    let output = run(
        &sandbox,
        &[LOCATION, "@31.2304,121.4737", "-f", "art-table"],
    )
    .assert()
    .success();
    let out = stdout(&output);
    let lines: Vec<&str> = out.lines().collect();
    // Block 1: header + grid row; one blank line; block 2: header + grid row.
    assert_eq!(lines.len(), 5, "two headers and two rows: {out}");
    assert!(
        lines[0].starts_with("Weather report:") && lines[3].starts_with("Weather report:"),
        "{out}"
    );
    assert!(!lines[1].is_empty() && !lines[4].is_empty(), "{out}");
    assert!(lines[1].contains("Clear sky"), "{out}");
    assert!(lines[1].contains("/+"), "the high/low pair is there: {out}");

    // A terminal too narrow for the grid falls back to the full tables, one per location.
    let output = run(
        &sandbox,
        &[
            LOCATION,
            "@31.2304,121.4737",
            "-f",
            "art-table",
            "--width",
            "20",
        ],
    )
    .assert()
    .success();
    let out = stdout(&output);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[1], "", "the full table leaves a blank line: {out}");
}

#[test]
fn five_locations_take_the_full_tables_and_say_so_once() {
    let sandbox = seeded();
    sandbox.write_config(
        "schema_version = 2\n[locations]\n\
         a = \"@39.9042,116.4074\"\n\
         b = \"@39.9042,116.4074\"\n\
         c = \"@39.9042,116.4074\"\n\
         d = \"@39.9042,116.4074\"\n",
    );
    let assert = run(
        &sandbox,
        &[LOCATION, "@a", "@b", "@c", "@d", "-f", "art-table"],
    )
    .assert()
    .success();
    let out = stdout(&assert);
    assert_eq!(
        out.matches("Weather report:").count(),
        5,
        "each location gets its full table: {out}"
    );
    assert!(
        stderr(&assert).contains("note: art-table summary layout is limited to 4 locations"),
        "the note names the limit: {}",
        stderr(&assert)
    );

    let assert = run(
        &sandbox,
        &[LOCATION, "@a", "@b", "@c", "@d", "-f", "art-table", "-q"],
    )
    .assert()
    .success();
    assert!(
        !stderr(&assert).contains("summary layout"),
        "`-q` silences the note"
    );
}

#[test]
fn single_location_only_flags_are_refused_with_several_locations() {
    let sandbox = seeded();
    for (flag, value) in [
        ("--ip", None),
        ("--station", Some("ZBAA")),
        ("--lat", Some("39.9")),
    ] {
        let mut command = sandbox.cirrocast();
        command.args([LOCATION, "@31.2304,121.4737"]);
        match value {
            Some(value) => command.args([flag, value]),
            None => command.arg(flag),
        };
        if flag == "--lat" {
            command.args(["--lon", "116.4"]);
        }
        command
            .arg("--offline")
            .assert()
            .code(2)
            .stderr(predicate::str::contains("more than one location argument"));
    }
}

#[test]
fn aliases_feed_a_multi_location_run() {
    let sandbox = seeded();
    sandbox.write_config(
        "schema_version = 2\n[locations]\n\
         home = \"@39.9042,116.4074\"\n\
         office = \"@31.2304,121.4737\"\n",
    );
    let assert = run(&sandbox, &["@home", "@office", "-f", "one-line"])
        .assert()
        .success();
    let out = stdout(&assert);
    assert_eq!(out.lines().count(), 2, "{out}");
    assert!(
        out.lines()
            .next()
            .is_some_and(|line| line.contains("Clear sky")),
        "{out}"
    );
}
