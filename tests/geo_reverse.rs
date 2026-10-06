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

// ---------------------------------------------------------------------------------------------
// `--ip` naming (step 25)
// ---------------------------------------------------------------------------------------------

/// Writes the `ip/ipwho-is.json` entry the binary reads for `--ip`, with `body` as its payload.
fn seed_ip(sandbox: &Sandbox, body: &str) {
    let key = cirrocast::cache::CacheKey::ip("ipwho-is");
    let path = sandbox.cache_dir().join(key.path());
    std::fs::create_dir_all(path.parent().expect("the entry has a parent"))
        .expect("the cache directory");
    let envelope = serde_json::json!({
        "cache_schema_version": cirrocast::cache::CACHE_SCHEMA_VERSION,
        "key": key.normalised(),
        "fetched_at": chrono::Utc::now().to_rfc3339(),
        "ttl_secs": 86_400,
        "status": 200,
        "body": body,
    });
    std::fs::write(
        &path,
        serde_json::to_string_pretty(&envelope).expect("the envelope encodes"),
    )
    .expect("the entry is written");
}

/// An `ipwho.is` answer that names no city: the schema allows it, and it is what the naming path
/// exists for.
const IP_WITHOUT_A_CITY: &str = r#"{"ip":"203.0.113.7","success":true,"region":"Beijing",
    "country":"China","country_code":"CN","latitude":39.907503,"longitude":116.397228,
    "timezone":{"id":"Asia/Shanghai"}}"#;

#[test]
fn an_ip_answer_without_a_city_is_named_from_the_bundled_tables() {
    let sandbox = Sandbox::new();
    seed_ip(&sandbox, IP_WITHOUT_A_CITY);
    let body = std::fs::read_to_string(fixture_path("open_meteo/forecast_beijing_2026-07-15.json"))
        .expect("the recorded forecast is readable");
    // The IP answer carries a real zone, so that is the zone the weather key is built in.
    common::seed_weather(
        &sandbox,
        "open-meteo",
        39.907_503,
        116.397_228,
        3,
        Tz::Asia__Shanghai,
        &body,
    );

    let output = sandbox
        .cirrocast()
        .args(["--ip", "-v", "-f", "plain", "--offline"])
        .output()
        .expect("the binary runs");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout is UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr is UTF-8");

    assert!(
        stderr.contains("ip: located from the public IP via ipwho.is"),
        "{stderr}"
    );
    assert!(
        stderr.contains("location: named by the bundled tables"),
        "{stderr}"
    );
    // The service's own fields survive: its zone, its country and its coordinates.
    assert!(
        stdout.contains("Beijing, China (39.91, 116.40)"),
        "{stdout}"
    );
    assert!(
        stdout.contains("Location data by GeoNames (CC BY 4.0) — https://www.geonames.org/"),
        "{stdout}"
    );
}

#[test]
fn an_ip_answer_nothing_can_name_shows_its_coordinates() {
    let sandbox = Sandbox::new();
    // A point no bundled city is within 25 km of, and `reverse = "off"` so nothing is asked.
    seed_ip(
        &sandbox,
        r#"{"ip":"203.0.113.7","success":true,"region":"","country":"","country_code":"",
            "latitude":0.0,"longitude":-140.0,"timezone":{"id":"Etc/GMT+9"}}"#,
    );

    let output = sandbox
        .cirrocast()
        .env("CIRROCAST_GEO_REVERSE", "off")
        .args(["location", "search", "--ip"])
        .output()
        .expect("the binary runs");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout is UTF-8");
    assert_eq!(stdout, "0, -140 Etc/GMT+9\n", "the pair is the name");
}
