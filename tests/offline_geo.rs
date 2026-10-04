// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The bundled city database end to end: `location search` from the table, the three offline
//! modes, and the ranking the table shares with the network geocoder.
//!
//! Every test runs the real binary in a throwaway XDG sandbox with `CIRROCAST_FORBID_NETWORK=1`,
//! so a run that tried to reach a socket fails loudly instead of passing on the developer's
//! network. The expectations are pinned against the committed `src/geo/data` snapshot: refreshing
//! the dump shows up here, which is the point.

#![cfg(feature = "offline-geo")]

mod common;

use cirrocast::geo::offline::{self, MatchMode};
use cirrocast::geo::rank;
use cirrocast::model::Location;

use common::Sandbox;

/// Runs `location search` in the sandbox and returns the parsed process output.
fn search(sandbox: &Sandbox, args: &[&str]) -> std::process::Output {
    let mut full = vec!["location", "search"];
    full.extend_from_slice(args);
    sandbox
        .cirrocast()
        .args(&full)
        .output()
        .expect("the binary runs")
}

/// Runs `location search` and asserts success, returning stdout.
fn search_ok(sandbox: &Sandbox, args: &[&str]) -> String {
    let output = search(sandbox, args);
    assert!(
        output.status.success(),
        "search {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("stdout is UTF-8")
}

/// The candidate rows of `--all` output, as `(name, population)` in print order.
fn candidates(stdout: &str) -> Vec<(String, u64)> {
    stdout
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let (_, rest) = line.split_once(". ").expect("a numbered row");
            let name = rest.split(',').next().unwrap_or(rest).to_owned();
            let population = rest
                .rsplit_once("(population ")
                .and_then(|(_, tail)| tail.strip_suffix(')'))
                .and_then(|value| value.parse::<u64>().ok())
                .expect("every --all row carries a population");
            (name, population)
        })
        .collect()
}

#[test]
fn springfield_orders_the_exact_tier_by_population_and_prints_ten_rows() {
    let sandbox = Sandbox::new();
    let stdout = search_ok(&sandbox, &["--offline", "--all", "Springfield"]);
    let rows = candidates(&stdout);
    let expected: Vec<(&str, u64)> = vec![
        ("Springfield", 170_188),
        ("Springfield", 154_341),
        ("Springfield", 114_394),
        ("Springfield", 60_870),
        ("Springfield", 59_680),
        ("Springfield", 30_484),
        ("Springfield", 23_363),
        ("Springfield", 16_808),
        // Reached by an exact key (an alternate spelling) but not an exact display name, so it
        // sorts after the eight; then the only prefix-only row.
        ("Springfield Gardens", 30_515),
        ("Springfield Lakes", 15_081),
    ];
    let rows: Vec<(&str, u64)> = rows
        .iter()
        .map(|(name, pop)| (name.as_str(), *pop))
        .collect();
    assert_eq!(rows, expected, "{stdout}");
    assert!(
        stdout
            .starts_with(" 1. Springfield, US (37.22, -93.30) America/Chicago (population 170188)"),
        "{stdout}"
    );
    assert!(
        stdout.lines().count() == 10,
        "ten rows, one per line: {stdout}"
    );
}

#[test]
fn diacritics_fold_to_the_same_winner() {
    let sandbox = Sandbox::new();
    let winner = search_ok(&sandbox, &["--offline", "São Paulo"]);
    assert!(winner.starts_with("São Paulo, BR "), "{winner}");
    for spelling in ["Sao Paulo", "SAO PAULO", "são paulo"] {
        assert_eq!(
            search_ok(&sandbox, &["--offline", spelling]),
            winner,
            "`{spelling}` must fold onto the same city"
        );
    }
}

#[test]
fn han_ascii_and_exonym_spellings_reach_beijing() {
    let sandbox = Sandbox::new();
    let expected = "Beijing, CN (39.91, 116.40) Asia/Shanghai\n";
    for spelling in ["北京", "Beijing", "Peking"] {
        assert_eq!(
            search_ok(&sandbox, &["--offline", spelling]),
            expected,
            "`{spelling}` must reach the same Beijing"
        );
    }
}

#[test]
fn an_exonym_beats_a_longer_prefix_match() {
    // `Wien` is an alternate spelling of Vienna, so Vienna is reached by an exact key and ranks
    // above Wiener Neustadt, whose display name merely starts with the query.
    let sandbox = Sandbox::new();
    let stdout = search_ok(&sandbox, &["--offline", "--all", "Wien"]);
    let rows = candidates(&stdout);
    assert_eq!(rows[0].0, "Vienna", "{stdout}");
    assert_eq!(rows[1].0, "Wiener Neustadt", "{stdout}");
    assert_eq!(rows[2].0, "Vientiane", "{stdout}");
}

#[test]
fn uppercase_ascii_input_matches_the_folded_index() {
    let sandbox = Sandbox::new();
    let winner = search_ok(&sandbox, &["--offline", "MÜNCHEN"]);
    assert!(
        winner.starts_with("Munich, DE (48.14, 11.58) Europe/Berlin"),
        "{winner}"
    );
}

#[test]
fn a_missing_city_exits_five_with_the_offline_marker() {
    let sandbox = Sandbox::new();
    let output = search(&sandbox, &["--offline", "Nowhereville"]);
    assert_eq!(output.status.code(), Some(5), "{output:?}");
    let stderr = String::from_utf8(output.stderr).expect("stderr is UTF-8");
    assert!(stderr.contains("no offline match"), "{stderr}");
    assert!(stderr.contains("Nowhereville"), "{stderr}");
}

#[test]
fn exact_mode_refuses_a_prefix_that_prefix_mode_finds() {
    let sandbox = Sandbox::new();
    let exact = search(&sandbox, &["--offline", "--exact", "Springf"]);
    assert_eq!(exact.status.code(), Some(5), "{exact:?}");

    let prefix = search_ok(&sandbox, &["--offline", "--all", "Springf"]);
    let rows = candidates(&prefix);
    assert_eq!(rows.len(), 10, "{prefix}");
    assert_eq!(rows[0].0, "Springfield", "{prefix}");
}

#[test]
fn the_default_strategy_resolves_from_the_bundle_without_a_socket() {
    // No `--offline`: `geo.strategy = "auto"` asks the bundled table first, and the network guard
    // is on, so a successful run proves the table answered.
    let sandbox = Sandbox::new();
    let default = search_ok(&sandbox, &["Beijing"]);
    let offline = search_ok(&sandbox, &["--offline", "Beijing"]);
    assert_eq!(default, offline);
    let stderr = String::from_utf8(search(&sandbox, &["Beijing"]).stderr).expect("stderr is UTF-8");
    assert!(
        stderr.contains("Location data by GeoNames (CC BY 4.0)"),
        "{stderr}"
    );
}

#[test]
fn the_geo_offline_mode_stops_at_the_bundle_while_weather_mode_still_geocodes() {
    // `Tsinghua` is not a city, so the table has no hit: `--offline=geo` refuses to ask the
    // network and exits 5, while `--offline=weather` keeps the geocoder and dies on the forbidden
    // socket (exit 3) instead — the two scopes are demonstrably different.
    let sandbox = Sandbox::new();
    let geo = search(&sandbox, &["--offline=geo", "Tsinghua"]);
    assert_eq!(geo.status.code(), Some(5), "{geo:?}");
    let geo_stderr = String::from_utf8(geo.stderr).expect("stderr is UTF-8");
    assert!(geo_stderr.contains("no offline match"), "{geo_stderr}");

    let weather = search(&sandbox, &["--offline=weather", "Tsinghua"]);
    assert_eq!(weather.status.code(), Some(3), "{weather:?}");
    let weather_stderr = String::from_utf8(weather.stderr).expect("stderr is UTF-8");
    assert!(
        !weather_stderr.contains("no offline match"),
        "{weather_stderr}"
    );
}

#[test]
fn a_tilde_search_is_refused_offline() {
    let sandbox = Sandbox::new();
    let output = search(&sandbox, &["--offline", "~Tsinghua"]);
    assert_eq!(output.status.code(), Some(3), "{output:?}");
    let stderr = String::from_utf8(output.stderr).expect("stderr is UTF-8");
    assert!(stderr.contains("OpenStreetMap"), "{stderr}");
}

#[test]
fn the_geo_strategy_can_skip_or_force_the_bundle() {
    let sandbox = Sandbox::new();
    sandbox.write_config("[geo]\nstrategy = \"network\"\n");
    // `strategy = "network"` skips the table, so with the network guard on the run must fail with
    // the guard's error rather than resolve from the bundle.
    let output = search(&sandbox, &["Beijing"]);
    assert_eq!(output.status.code(), Some(3), "{output:?}");
    let stderr = String::from_utf8(output.stderr).expect("stderr is UTF-8");
    assert!(
        stderr.contains("outbound network access is disabled"),
        "{stderr}"
    );

    let bundled = Sandbox::new();
    bundled.write_config("[geo]\nstrategy = \"bundled\"\n");
    assert!(search_ok(&bundled, &["Beijing"]).starts_with("Beijing, CN "));
}

// ---------------------------------------------------------------------------------------------
// The ranking the table shares with the network geocoder
// ---------------------------------------------------------------------------------------------

/// The recorded step-04 geocoder fixture, as the `Location`s the network path would rank.
fn fixture_locations(name: &str) -> Vec<Location> {
    let text = common::fixture(name);
    let document: serde_json::Value = serde_json::from_str(&text).expect("the fixture parses");
    document["results"]
        .as_array()
        .expect("the fixture carries results")
        .iter()
        .map(|hit| {
            let tz = hit["timezone"].as_str().expect("a timezone");
            Location {
                name: hit["name"].as_str().expect("a name").to_owned(),
                admin1: hit["admin1"].as_str().map(str::to_owned),
                country: hit["country"].as_str().unwrap_or_default().to_owned(),
                country_code: hit["country_code"].as_str().map(str::to_owned),
                lat: hit["latitude"].as_f64().expect("a latitude"),
                lon: hit["longitude"].as_f64().expect("a longitude"),
                tz: tz.parse().expect("an IANA zone"),
                elevation_m: hit["elevation"].as_f64(),
                population: hit["population"].as_u64(),
                source: cirrocast::model::LocationSource::Geocoder,
                station: None,
            }
        })
        .collect()
}

#[test]
fn the_recorded_geocoder_fixtures_pick_the_same_winner_offline() {
    for (fixture, query) in [
        ("geo/open_meteo_geocode_beijing_ambiguous.json", "Beijing"),
        ("geo/open_meteo_geocode_vienna.json", "Vienna"),
    ] {
        let hits = fixture_locations(fixture);
        let network = rank::rank(hits, Some(query), 10);
        let network_winner = network.first().expect("the fixture has hits");

        let offline = offline::search(query, MatchMode::Prefix, 10).expect("the table decodes");
        let offline_winner = offline
            .first()
            .expect("the table has the winner")
            .location();
        assert_eq!(
            (offline_winner.lat, offline_winner.lon),
            (network_winner.lat, network_winner.lon),
            "{query}: the two sources must pick the same place"
        );
    }
}

#[test]
fn both_candidate_shapes_feed_one_ranking() {
    // The same rows ranked as `City` values (with ascii spellings) and as `Location`s (without)
    // must come out in the same order: the ordering lives in one function, not one per source.
    let cities = offline::search("Springfield", MatchMode::Prefix, 10).expect("the table decodes");
    let locations: Vec<Location> = cities.iter().map(offline::City::location).collect();
    let ranked_cities = rank::rank(cities, Some("Springfield"), 10);
    let ranked_locations = rank::rank(locations, Some("Springfield"), 10);
    let by_city: Vec<(f64, f64)> = ranked_cities
        .iter()
        .map(|city| (city.lat, city.lon))
        .collect();
    let by_location: Vec<(f64, f64)> = ranked_locations
        .iter()
        .map(|location| (location.lat, location.lon))
        .collect();
    assert_eq!(by_city, by_location);
}
