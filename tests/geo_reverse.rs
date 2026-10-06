// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Naming a coordinate end to end: the real binary, the committed tables, no socket.
//!
//! Every test runs in a throwaway XDG sandbox with `CIRROCAST_FORBID_NETWORK=1` (the shared
//! [`Sandbox`] sets it), so a run that reached for a socket would fail loudly instead of passing on
//! the developer's network — which is what makes "`--offline=geo` names the point" a proof rather
//! than a hope. The expectations are pinned against the committed city table and country layer:
//! refreshing either shows up here, which is the point.

#![cfg(feature = "offline-geo")]

mod common;

use chrono_tz::Tz;
use common::{Sandbox, fixture_path};

/// Runs `location search` in the sandbox and returns the whole process output.
fn search(sandbox: &Sandbox, args: &[&str]) -> std::process::Output {
    let mut full = vec!["location", "search"];
    full.extend_from_slice(args);
    sandbox
        .cirrocast()
        .args(&full)
        .output()
        .expect("the binary runs")
}

/// Runs `location search` and asserts success, returning `(stdout, stderr)`.
fn search_ok(sandbox: &Sandbox, args: &[&str]) -> (String, String) {
    let output = search(sandbox, args);
    assert!(
        output.status.success(),
        "search {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    (
        String::from_utf8(output.stdout).expect("stdout is UTF-8"),
        String::from_utf8(output.stderr).expect("stderr is UTF-8"),
    )
}

/// Beijing's coordinates, whose nearest bundled place is Beijing itself.
const BEIJING: &str = "@39.9042,116.4074";

#[test]
fn a_coordinate_is_named_from_the_bundled_tables() {
    let sandbox = Sandbox::new();
    let (stdout, stderr) = search_ok(&sandbox, &["-v", BEIJING]);

    // The name is a display attribute: the coordinates the user typed stay in the line.
    assert!(
        stdout.starts_with("Beijing, China (39.90, 116.41) "),
        "{stdout}"
    );
    assert!(
        stderr.contains("location: named by the bundled tables, 0.9 km away: Beijing, China"),
        "{stderr}"
    );
    // The name came from GeoNames' table and Natural Earth's layer, so its credit is printed.
    assert!(
        stderr.contains("Location data by GeoNames (CC BY 4.0) — https://www.geonames.org/"),
        "{stderr}"
    );
}

#[test]
fn all_lists_the_nearby_names_nearest_first() {
    let sandbox = Sandbox::new();
    let (stdout, _) = search_ok(&sandbox, &["--all", BEIJING]);

    let rows: Vec<&str> = stdout.lines().collect();
    assert!(rows.len() >= 3, "{stdout}");
    assert!(
        rows[0]
            .starts_with(" 1. Beijing, China (39.91, 116.40) Asia/Shanghai (population 18960744)"),
        "{stdout}"
    );
    assert!(rows[1].starts_with(" 2. Daxing, China "), "{stdout}");
    assert!(rows[2].starts_with(" 3. Tongzhou, China "), "{stdout}");
}

#[test]
fn offline_geo_names_the_point_without_a_socket() {
    let sandbox = Sandbox::new();
    // The sandbox forbids network access, so this run can only succeed from the bundled tables.
    let (stdout, stderr) = search_ok(&sandbox, &["--offline=geo", "-v", BEIJING]);
    assert!(
        stdout.starts_with("Beijing, China (39.90, 116.41) "),
        "{stdout}"
    );
    assert!(stderr.contains("named by the bundled tables"), "{stderr}");
}

#[test]
fn the_open_ocean_is_named_by_nobody() {
    let sandbox = Sandbox::new();
    // `offline` is what `--offline=geo` implies, and the sandbox forbids the socket anyway.
    let (stdout, stderr) = search_ok(&sandbox, &["--offline=geo", "-v", "@0,-140"]);
    assert_eq!(stdout, "0, -140 <timezone resolved at fetch time>\n");
    assert!(
        stderr.contains("no city within 25 km of (0, -140)"),
        "{stderr}"
    );
    assert!(
        !stderr.contains("Location data by"),
        "no name, no credit: {stderr}"
    );
}

#[test]
fn the_off_policy_leaves_the_coordinate_bare() {
    let sandbox = Sandbox::new();
    let output = sandbox
        .cirrocast()
        .env("CIRROCAST_GEO_REVERSE", "off")
        .args(["location", "search", BEIJING])
        .output()
        .expect("the binary runs");
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("stdout is UTF-8");
    assert_eq!(
        stdout,
        "39.9042, 116.4074 <timezone resolved at fetch time>\n"
    );
    assert_eq!(String::from_utf8(output.stderr).expect("stderr"), "");
}

#[test]
fn an_invalid_reverse_policy_is_a_configuration_error() {
    let sandbox = Sandbox::new();
    let output = sandbox
        .cirrocast()
        .env("CIRROCAST_GEO_REVERSE", "sometimes")
        .args(["location", "search", BEIJING])
        .output()
        .expect("the binary runs");
    assert_eq!(output.status.code(), Some(4));
    let stderr = String::from_utf8(output.stderr).expect("stderr is UTF-8");
    assert!(stderr.contains("geo.reverse"), "{stderr}");
}

#[test]
fn a_named_coordinate_carries_its_name_and_credit_into_the_report() {
    let sandbox = Sandbox::new();
    let body = std::fs::read_to_string(fixture_path("open_meteo/forecast_beijing_2026-07-15.json"))
        .expect("the recorded forecast is readable");
    // The binary keys a coordinate's cache entry by the provisional UTC zone it starts from.
    common::seed_weather(&sandbox, "open-meteo", 39.9042, 116.4074, 3, Tz::UTC, &body);

    let output = sandbox
        .cirrocast()
        .args(["--offline", "-f", "plain", BEIJING])
        .output()
        .expect("the binary runs");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout is UTF-8");

    // The report's location is the named coordinate: the request used the coordinates, the
    // document shows the name, and the credit the name's licence asks for is in the document.
    assert!(
        stdout.contains("Beijing, China (39.90, 116.41)"),
        "{stdout}"
    );
    assert!(
        stdout.contains("Location data by GeoNames (CC BY 4.0) — https://www.geonames.org/"),
        "{stdout}"
    );
    assert!(
        stdout.contains("Data: Open-Meteo.com (CC BY 4.0)"),
        "{stdout}"
    );
}
