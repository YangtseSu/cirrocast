// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The METAR backend: the decoder against every recorded report, the station table, and the CLI
//! end to end with a pre-seeded cache.
//!
//! The decoder is checked field by field against the expectation table below, which is written
//! from the raw report in `tests/fixtures/metar/<ICAO>/raw.metar` — the fixture pair (raw report
//! and `format=json` body) is asserted to describe the same observation, so neither can drift.
//! Floating point fields are compared with the tolerance their own conversion has (knots and
//! metres per second are not exactly representable in km/h); every integer, direction, count,
//! condition code and layer is exact.
//!
//! The CLI cases seed the cache instead of stubbing the transport: the binary is a separate
//! process, so `--offline` plus a written entry is the only way to prove the provider path without
//! a network. No test in this file opens a socket; the live check lives in `tests/live.rs`.

mod common;

use std::sync::Arc;
use std::time::Duration;

use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::Value;

use cirrocast::cache::{Cache, CacheKey, CacheMode, SystemClock};
use cirrocast::model::Condition;
use cirrocast::provider::metar::decode::{Cover, decode_metar};
use cirrocast::provider::metar::station_table;
use common::Sandbox;

/// The cache lifetime the observation entries are seeded with.
const OBSERVATION_TTL: Duration = Duration::from_secs(600);

/// The 30-day lifetime the station metadata entries are seeded with.
const STATION_TTL: Duration = Duration::from_hours(30 * 24);

/// The expected wind of one fixture: `(direction, speed in km/h, gust in km/h, VRB, calm)`.
type Wind = (Option<u16>, f32, Option<f32>, bool, bool);

/// The expected clouds of one fixture: `(cover percentage, layers)`.
type Clouds = (u8, &'static [(Cover, Option<f64>, bool)]);

/// One fixture's expected decode.
///
/// Written from the report itself, group by group; the comment above each row names the group the
/// values come from.
struct Expect {
    /// The fixture directory under `tests/fixtures/metar/`.
    icao: &'static str,
    /// `ddHHMMZ`.
    time: (u8, u8, u8),
    /// Wind: direction, speed in km/h, gust in km/h, `VRB`, calm.
    wind: Wind,
    /// The reported direction variation sector.
    sector: Option<(u16, u16)>,
    /// Visibility in km, and whether the report carried `CAVOK`.
    visibility: (Option<f32>, bool),
    /// The present-weather groups, verbatim.
    weather: &'static [&'static str],
    /// The WMO code the weather (or the sky) maps to.
    condition: u8,
    /// Cloud cover percentage and the layers as `(cover, base in metres, convective)`.
    clouds: Clouds,
    /// Temperature and dew point in °C.
    temperatures: (f32, f32),
    /// Altimeter in hPa.
    pressure_hpa: f32,
    /// Precipitation in mm from the remark group, when the report carries one.
    precip_mm: Option<f32>,
    /// RVR groups, verbatim.
    rvr: &'static [&'static str],
}

/// Every fixture, decoded as the expectation table says.
const EXPECTATIONS: &[Expect] = &[
    // `METAR KSMF 302353Z 00000KT 10SM CLR 33/08 A2974 RMK AO2 SLP069 …`
    Expect {
        icao: "KSMF",
        time: (30, 23, 53),
        wind: (Some(0), 0.0, None, false, true),
        sector: None,
        visibility: (Some(10.0 * 1.609_344), false),
        weather: &[],
        condition: 0,
        clouds: (0, &[]),
        temperatures: (33.0, 8.0),
        pressure_hpa: 1007.11,
        precip_mm: None,
        rvr: &[],
    },
    // `METAR ZBAA 010000Z 29005G10MPS 270V330 CAVOK 16/M09 Q1023 NOSIG` — the metric wind variant,
    // a gust, a direction sector and CAVOK.
    Expect {
        icao: "ZBAA",
        time: (1, 0, 0),
        wind: (Some(290), 5.0 * 3.6, Some(10.0 * 3.6), false, false),
        sector: Some((270, 330)),
        visibility: (Some(10.0), true),
        weather: &[],
        condition: 0,
        clouds: (0, &[]),
        temperatures: (16.0, -9.0),
        pressure_hpa: 1023.0,
        precip_mm: None,
        rvr: &[],
    },
    // `METAR LFPG 010000Z VRB02KT 9999 BKN016 18/16 Q1018 NOSIG` — the variable direction the
    // upstream JSON spells `"VRB"`, and a single broken layer.
    Expect {
        icao: "LFPG",
        time: (1, 0, 0),
        wind: (None, 3.7, None, true, false),
        sector: None,
        visibility: (Some(10.0), false),
        weather: &[],
        condition: 3,
        clouds: (75, &[(Cover::Broken, Some(1600.0 * 0.3048), false)]),
        temperatures: (18.0, 16.0),
        pressure_hpa: 1018.0,
        precip_mm: None,
        rvr: &[],
    },
    // `SPECI PASC 010011Z AUTO 25003KT 1/2SM R06/4000VP6000FT SN FZFG FEW005 OVC011 M03/M03
    // A2988 RMK AO2 P0000 …` — a fraction of a statute mile, RVR, snow with freezing fog, two
    // layers and a `P####` remark.
    Expect {
        icao: "PASC",
        time: (1, 0, 11),
        wind: (Some(250), 5.6, None, false, false),
        sector: None,
        visibility: (Some(0.5 * 1.609_344), false),
        weather: &["SN", "FZFG"],
        condition: 73,
        clouds: (
            100,
            &[
                (Cover::Few, Some(500.0 * 0.3048), false),
                (Cover::Overcast, Some(1100.0 * 0.3048), false),
            ],
        ),
        temperatures: (-3.0, -3.0),
        pressure_hpa: 1011.85,
        precip_mm: Some(0.0),
        rvr: &["R06/4000VP6000FT"],
    },
    // `METAR FZAA 010000Z 00000KT 6000 TS BKN014 FEW026CB 23/22 Q1015 NOSIG` — a calm wind, a
    // thunderstorm and a convective layer.
    Expect {
        icao: "FZAA",
        time: (1, 0, 0),
        wind: (Some(0), 0.0, None, false, true),
        sector: None,
        visibility: (Some(6.0), false),
        weather: &["TS"],
        condition: 95,
        clouds: (
            75,
            &[
                (Cover::Broken, Some(1400.0 * 0.3048), false),
                (Cover::Few, Some(2600.0 * 0.3048), true),
            ],
        ),
        temperatures: (23.0, 22.0),
        pressure_hpa: 1015.0,
        precip_mm: None,
        rvr: &[],
    },
    // `METAR YPDN 010000Z AUTO 08007KT 040V100 //// // SCT021 BKN025 28/24 Q1018` — the Australian
    // spelling of "visibility not available", and a sector.
    Expect {
        icao: "YPDN",
        time: (1, 0, 0),
        wind: (Some(80), 13.0, None, false, false),
        sector: Some((40, 100)),
        visibility: (None, false),
        weather: &[],
        condition: 3,
        clouds: (
            75,
            &[
                (Cover::Scattered, Some(2100.0 * 0.3048), false),
                (Cover::Broken, Some(2500.0 * 0.3048), false),
            ],
        ),
        temperatures: (28.0, 24.0),
        pressure_hpa: 1018.0,
        precip_mm: None,
        rvr: &[],
    },
    // `METAR KMDW 302353Z 19007KT 9SM -RA BKN026 OVC095 18/17 A2988 RMK AO2 SLP114 P0002 …`
    Expect {
        icao: "KMDW",
        time: (30, 23, 53),
        wind: (Some(190), 13.0, None, false, false),
        sector: None,
        visibility: (Some(9.0 * 1.609_344), false),
        weather: &["-RA"],
        condition: 61,
        clouds: (
            100,
            &[
                (Cover::Broken, Some(2600.0 * 0.3048), false),
                (Cover::Overcast, Some(9500.0 * 0.3048), false),
            ],
        ),
        temperatures: (18.0, 17.0),
        pressure_hpa: 1011.85,
        precip_mm: Some(0.51),
        rvr: &[],
    },
];

/// The raw report of a fixture, without the trailing newline.
fn raw_report(icao: &str) -> String {
    common::fixture(&format!("metar/{icao}/raw.metar"))
        .trim_end()
        .to_owned()
}

/// Asserts that two converted values agree within the rounding of their own formula.
fn close(left: f32, right: f32, what: &str) {
    assert!(
        (left - right).abs() < 0.005,
        "{what}: decoded {left}, expected {right}"
    );
}

#[test]
fn every_fixture_decodes_as_the_expectation_table_says() {
    for expect in EXPECTATIONS {
        let raw = raw_report(expect.icao);
        let decoded = decode_metar(&raw).unwrap_or_else(|error| panic!("{}: {error}", expect.icao));

        assert_eq!(decoded.icao.as_deref(), Some(expect.icao));
        assert_eq!(
            (decoded.day_of_month, decoded.hour, decoded.minute),
            expect.time,
            "{}",
            expect.icao
        );

        let (direction, speed, gust, variable, calm) = expect.wind;
        assert_eq!(decoded.wind_dir_deg, direction, "{}", expect.icao);
        close(decoded.wind_kmh, speed, expect.icao);
        match (decoded.wind_gust_kmh, gust) {
            (Some(decoded), Some(expected)) => close(decoded, expected, expect.icao),
            (None, None) => {}
            (decoded, expected) => {
                panic!("{}: gust {decoded:?}, expected {expected:?}", expect.icao)
            }
        }
        assert_eq!(decoded.wind_variable, variable, "{}", expect.icao);
        assert_eq!(decoded.calm, calm, "{}", expect.icao);
        assert_eq!(
            decoded.variable_from_deg.zip(decoded.variable_to_deg),
            expect.sector,
            "{}",
            expect.icao
        );

        let (visibility, cavok) = expect.visibility;
        match (decoded.visibility_km, visibility) {
            (Some(decoded), Some(expected)) => close(decoded, expected, expect.icao),
            (None, None) => {}
            (decoded, expected) => {
                panic!(
                    "{}: visibility {decoded:?}, expected {expected:?}",
                    expect.icao
                )
            }
        }
        assert_eq!(decoded.cavok, cavok, "{}", expect.icao);

        assert_eq!(decoded.weather, expect.weather, "{}", expect.icao);
        assert_eq!(
            decoded.condition.code(),
            expect.condition,
            "{}",
            expect.icao
        );

        let (cover_pct, layers) = expect.clouds;
        assert_eq!(decoded.cloud_cover_pct, cover_pct, "{}", expect.icao);
        assert_eq!(decoded.cloud_layers.len(), layers.len(), "{}", expect.icao);
        for (decoded, (cover, base_m, convective)) in decoded.cloud_layers.iter().zip(layers) {
            assert_eq!(decoded.cover, *cover, "{}", expect.icao);
            match (decoded.base_m, base_m) {
                (Some(decoded), Some(expected)) => assert!(
                    (decoded - expected).abs() < 0.01,
                    "{}: base {decoded}, expected {expected}",
                    expect.icao
                ),
                (None, None) => {}
                (decoded, expected) => {
                    panic!("{}: base {decoded:?}, expected {expected:?}", expect.icao)
                }
            }
            assert_eq!(decoded.convective, *convective, "{}", expect.icao);
        }

        let (temp, dewpoint) = expect.temperatures;
        close(decoded.temp_c, temp, expect.icao);
        close(decoded.dewpoint_c, dewpoint, expect.icao);
        close(decoded.pressure_hpa, expect.pressure_hpa, expect.icao);
        match (decoded.precip_mm, expect.precip_mm) {
            (Some(decoded), Some(expected)) => close(decoded, expected, expect.icao),
            (None, None) => {}
            (decoded, expected) => {
                panic!("{}: precip {decoded:?}, expected {expected:?}", expect.icao)
            }
        }
        assert_eq!(decoded.rvr, expect.rvr, "{}", expect.icao);
    }
}

#[test]
fn the_json_fixture_describes_the_same_report_as_the_raw_one() {
    for expect in EXPECTATIONS {
        let body = common::fixture(&format!("metar/{}/current.json", expect.icao));
        let reports: Vec<Value> = serde_json::from_str(&body).expect("the fixture is a JSON array");
        let report = reports.first().expect("the fixture holds one report");
        assert_eq!(
            report["icaoId"].as_str(),
            Some(expect.icao),
            "the fixture names its station"
        );
        assert_eq!(
            report["rawOb"].as_str(),
            Some(raw_report(expect.icao).as_str()),
            "{}: the JSON body and the raw report must be the same observation",
            expect.icao
        );
        assert!(
            report["obsTime"].is_i64(),
            "{}: the observation epoch is what dates the report",
            expect.icao
        );
    }
}

#[test]
fn every_decoded_condition_is_one_the_catalog_names() {
    // A code the renderer would print as "Unknown" is a defect the fixture set has to catch.
    for expect in EXPECTATIONS {
        let condition = Condition::from_u8(expect.condition);
        assert!(
            condition.is_known(),
            "{}: code {} is not described",
            expect.icao,
            expect.condition
        );
    }
}

#[test]
fn the_station_table_answers_the_common_identifiers() {
    let kennedy = station_table::lookup("KJFK").expect("KJFK is in the table");
    assert_eq!(kennedy.name, "New York/JF Kennedy Intl");
    assert_eq!(kennedy.tz, "America/New_York");
    assert!(station_table::lookup("ZZZZ").is_none());

    // A coordinate pair reaches the nearest row; that is how `@lat,lon` finds a station.
    let (station, distance) =
        station_table::nearest_with_distance(51.5, -0.45).expect("a non-empty table");
    assert_eq!(station.icao, "EGLL");
    assert!(distance < 5.0, "{distance} km from Heathrow");
}

// ---------------------------------------------------------------------------------------------
// The CLI, offline, against a seeded cache
// ---------------------------------------------------------------------------------------------

/// A sandbox with the recorded observation of `icao` in the cache, and its station metadata.
fn seeded(icao: &str, with_stationinfo: bool) -> Sandbox {
    let sandbox = Sandbox::new();
    let cache = Cache::with_root(
        sandbox.cache_dir(),
        CacheMode::Normal,
        Arc::new(SystemClock),
        0,
    );
    let observation = common::fixture(&format!("metar/{icao}/current.json"));
    cache
        .write(
            &CacheKey::station_resource("metar", icao, "current"),
            200,
            &observation,
            OBSERVATION_TTL,
        )
        .expect("the observation entry is written");
    if with_stationinfo {
        let info = common::fixture(&format!("stationinfo/{icao}.json"));
        cache
            .write(&CacheKey::station(icao), 200, &info, STATION_TTL)
            .expect("the station metadata entry is written");
    }
    sandbox
}

/// A run of `cirrocast` against a seeded sandbox.
fn run(sandbox: &Sandbox, args: &[&str]) -> Command {
    let mut command = sandbox.cirrocast();
    command.args(args);
    command
}

/// The stdout of a successful run.
fn stdout(sandbox: &Sandbox, args: &[&str]) -> String {
    let assert = run(sandbox, args).assert().success();
    String::from_utf8(assert.get_output().stdout.clone()).expect("stdout is UTF-8")
}

#[test]
fn the_embedded_table_answers_a_station_without_a_metadata_request() {
    // No stationinfo entry is seeded: the table alone has to answer the location.
    let sandbox = seeded("ZBAA", false);
    let output = stdout(&sandbox, &["--station", "ZBAA", "--offline", "-f", "plain"]);

    assert!(output.contains("Beijing Intl, BJ, CN"), "{output}");
    assert!(output.contains("16"), "the decoded temperature: {output}");
    assert!(output.contains("Asia/Shanghai"), "{output}");
}

#[test]
fn cached_station_metadata_answers_a_station_the_table_does_not_know() {
    // KMDW is deliberately not in the embedded table: this proves the cached `stationinfo` path,
    // including the time zone `geo::tz` derives from the row's coordinates.
    let sandbox = seeded("KMDW", true);
    let output = stdout(&sandbox, &["--station", "KMDW", "--offline", "-f", "plain"]);

    assert!(output.contains("Chicago/Midway Intl, IL, US"), "{output}");
    assert!(output.contains("America/Chicago"), "{output}");
    assert!(output.contains("Slight rain"), "{output}");
}

#[test]
fn an_unknown_station_is_a_location_error_naming_it() {
    let sandbox = Sandbox::new();
    let cache = Cache::with_root(
        sandbox.cache_dir(),
        CacheMode::Normal,
        Arc::new(SystemClock),
        0,
    );
    // The recorded upstream answer for a station that does not exist: an empty list.
    cache
        .write(
            &CacheKey::station("ZZZZ"),
            200,
            &common::fixture("stationinfo/ZZZZ.json"),
            STATION_TTL,
        )
        .expect("the empty metadata entry is written");

    run(&sandbox, &["--station", "ZZZZ", "--offline"])
        .assert()
        .code(5)
        .stderr(
            predicate::str::contains("unknown station `ZZZZ`")
                .and(predicate::str::contains("cirrocast location search")),
        );
}

#[test]
fn json_carries_the_capabilities_and_no_days() {
    let sandbox = seeded("ZBAA", false);
    let output = stdout(&sandbox, &["--station", "ZBAA", "--offline", "-f", "json"]);
    let document: Value = serde_json::from_str(&output).expect("the run emits valid JSON");

    assert_eq!(document["days"], Value::Array(Vec::new()));
    assert_eq!(document["capabilities"]["daily"], Value::Bool(false));
    assert_eq!(document["capabilities"]["hourly"], Value::Bool(false));
    assert_eq!(document["capabilities"]["current"], Value::Bool(true));
    assert_eq!(document["capabilities"]["max_days"], Value::from(0));
    assert_eq!(
        document["capabilities"]["locations"]["station"],
        Value::Bool(true)
    );
    assert_eq!(document["location"]["source"], Value::from("station"));
    assert_eq!(document["location"]["station"], Value::from("ZBAA"));
    assert_eq!(document["current"]["temp_c"], Value::from(16.0));
    assert_eq!(
        document["attribution"]["notice"],
        Value::from("aviationweather.gov (NOAA/NWS, public domain)")
    );
}

#[test]
fn the_art_table_states_the_observation_age_and_the_missing_forecast() {
    let sandbox = seeded("ZBAA", false);
    let output = stdout(&sandbox, &["--station", "ZBAA", "--offline"]);

    assert!(output.contains("observed "), "{output}");
    // The age is measured against the wall clock, so the unit depends on when the fixture's
    // observation was taken (minutes just after recording, hours later): assert the shape, not the
    // number. The line is `observed <HH:MM>Z <sep> <n> min|h ago`.
    let observed = output
        .lines()
        .find(|line| line.starts_with("observed "))
        .expect("the observation line");
    assert!(
        observed.ends_with(" min ago") || observed.ends_with(" h ago"),
        "{observed}"
    );
    assert!(
        output.contains("no forecast: METAR is an observation"),
        "{output}"
    );
    // No day table: the box drawing of the columns layout must not appear.
    assert!(!output.contains('\u{250c}'), "{output}");
}

#[test]
fn verbose_prints_the_raw_report_and_the_no_forecast_note() {
    let sandbox = seeded("ZBAA", false);
    let assert = run(&sandbox, &["--station", "ZBAA", "--offline", "-v"])
        .assert()
        .success();
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("UTF-8 stderr");

    assert!(
        stderr.contains("metar: METAR ZBAA 010000Z 29005G10MPS"),
        "{stderr}"
    );
    assert!(
        stderr.contains("reports observations only; for a forecast use a forecast backend"),
        "{stderr}"
    );
    assert!(
        stderr.contains("attribution: aviationweather.gov"),
        "{stderr}"
    );
}

#[test]
fn a_station_from_the_configuration_is_the_default_location() {
    let sandbox = seeded("ZBAA", false);
    sandbox.write_config(
        "schema_version = 1\n[defaults]\nprovider = \"metar\"\n[providers.metar]\nstation = \"ZBAA\"\n",
    );

    let output = stdout(&sandbox, &["--offline", "-f", "plain"]);
    assert!(output.contains("Beijing Intl, BJ, CN"), "{output}");

    // The same configuration with another default backend ignores the station entirely: the run
    // resolves a place (and fails offline, which is the point — it never looked at the station).
    sandbox.write_config(
        "schema_version = 1\n[defaults]\nprovider = \"open-meteo\"\n[providers.metar]\nstation = \"ZBAA\"\n",
    );
    run(&sandbox, &["--offline", "-f", "plain"])
        .assert()
        .code(3)
        .stderr(predicate::str::contains("offline mode"));
}
