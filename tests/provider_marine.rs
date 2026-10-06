// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The Open-Meteo Marine backend, driven by the recorded responses in `tests/fixtures/marine/`.
//!
//! No test here opens a socket: `StubTransport` answers from the fixtures and `FakeClock` supplies
//! the fetch instant and the location-local date the cache key is built from. The Sylt distance is
//! the real great-circle offset between the requested coordinate and the recorded cell, computed
//! independently of the implementation.

// Exact comparison is the point in this file: every value comes from the recorded JSON.
#![allow(clippy::float_cmp)]

mod common;

use chrono::NaiveDate;
use chrono_tz::Tz;

use cirrocast::cache::{CacheEntry, CacheKey, CacheMode};
use cirrocast::model::{Location, LocationSource, MarineSource};
use cirrocast::provider::ProviderId;
use cirrocast::provider::open_meteo_marine::{
    BASE, CURRENT_VARIABLES, DAILY_VARIABLES, OpenMeteoMarine, far_cell_note, fetch,
};
use common::{ProviderRun, fixture_reply, provider_clock};

/// A location for a point, in `tz`.
fn location(name: &str, lat: f64, lon: f64) -> Location {
    Location {
        name: name.to_owned(),
        admin1: None,
        country: String::new(),
        country_code: None,
        lat,
        lon,
        tz: Tz::Europe__Berlin,
        elevation_m: None,
        population: None,
        source: LocationSource::Geocoder,
        station: None,
        named_by: None,
    }
}

/// A run over one marine fixture, at 06:00 UTC on 2026-10-05 (08:00 in Berlin).
fn run(file: &str) -> ProviderRun {
    ProviderRun::new(
        vec![fixture_reply("marine", file)],
        provider_clock(2026, 10, 5),
        CacheMode::Normal,
    )
}

fn date(year: i32, month: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(year, month, day).expect("a valid date")
}

#[test]
fn the_request_matches_the_provider_contract() {
    let run = run("sylt.json");
    run.fetch_with(&OpenMeteoMarine, &location("Sylt", 54.54, 10.23), 0)
        .expect("the fixture parses");

    let calls = run.calls();
    assert_eq!(calls.len(), 1);
    let call = &calls[0];
    assert_eq!(call.url(), BASE);
    let pairs: Vec<(&str, &str)> = call
        .query_pairs()
        .iter()
        .map(|(name, value)| (name.as_str(), value.as_str()))
        .collect();
    assert_eq!(
        pairs,
        [
            ("latitude", "54.5400"),
            ("longitude", "10.2300"),
            ("current", CURRENT_VARIABLES),
            ("daily", DAILY_VARIABLES),
            ("cell_selection", "sea"),
            ("timezone", "auto"),
        ]
    );
}

#[test]
fn the_sylt_fixture_decodes_to_the_recorded_reading() {
    let run = run("sylt.json");
    let report = run
        .fetch_with(&OpenMeteoMarine, &location("Sylt", 54.54, 10.23), 0)
        .expect("the fixture parses");

    assert!(
        report.current.is_none(),
        "the marine source carries no current block"
    );
    assert_eq!(report.days, Vec::new());
    let marine = report.marine.expect("the marine panel is present");
    assert_eq!(marine.source, MarineSource::OpenMeteoMarine);
    assert_eq!(marine.time.to_string(), "2026-10-05 23:00:00 +02:00");
    assert_eq!(marine.wave_height_m, Some(0.42));
    assert_eq!(marine.wave_direction_deg, Some(253));
    assert_eq!(marine.wave_period_s, Some(2.7));
    assert_eq!(marine.swell_wave_height_m, Some(0.04));
    assert_eq!(marine.sea_surface_temp_c, Some(16.1));

    assert_eq!(marine.sampled_lat, 54.541_664);
    assert_eq!(marine.sampled_lon, 10.208_343_5);
    assert!(
        (marine.distance_km - 1.409).abs() < 0.01,
        "the sampled cell sits {:.3} km away",
        marine.distance_km
    );
    assert!(!marine.sampled_cell_is_far());

    assert_eq!(marine.days.len(), 7);
    assert_eq!(marine.days[0].date, date(2026, 10, 5));
    assert_eq!(marine.days[0].wave_height_max_m, Some(0.46));
    assert_eq!(marine.days[0].wave_period_max_s, Some(2.75));
    assert_eq!(marine.days[0].wave_direction_dominant_deg, Some(241));
    assert_eq!(marine.days[6].date, date(2026, 10, 11));
    assert_eq!(marine.days[6].wave_height_max_m, Some(0.48));
    assert_eq!(marine.days[6].wave_direction_dominant_deg, Some(250));
}

#[test]
fn the_free_fetch_entry_point_answers_the_same_marine_reading() {
    let run = run("sylt.json");
    let env = run.env();
    let marine = fetch(&location("Sylt", 54.54, 10.23), &env).expect("the fixture parses");
    assert_eq!(marine.wave_height_m, Some(0.42));
    assert_eq!(marine.sampled_lat, 54.541_664);
}

#[test]
fn a_land_point_gets_a_far_cell_note_naming_the_sampled_coordinate() {
    let run = run("berlin-land.json");
    let report = run
        .fetch_with(&OpenMeteoMarine, &location("Berlin", 52.52, 13.405), 0)
        .expect("the fixture parses");
    let marine = report.marine.expect("the marine panel is present");
    assert!(marine.sampled_cell_is_far());
    assert!(
        (marine.distance_km - 308.44).abs() < 0.1,
        "the sampled cell sits {:.3} km away",
        marine.distance_km
    );

    let note = far_cell_note(&marine, true, false).expect("a far cell is named under -v");
    assert!(note.contains("54.5417"), "{note}");
    assert!(note.contains("10.2083"), "{note}");
    assert!(note.contains("308.4 km"), "{note}");
    assert!(note.starts_with("open-meteo-marine"), "{note}");

    assert_eq!(
        far_cell_note(&marine, false, false),
        None,
        "no note without -v"
    );
    assert_eq!(
        far_cell_note(&marine, true, true),
        None,
        "-q silences the note"
    );
}

#[test]
fn a_response_without_a_current_block_is_an_upstream_error_naming_the_place() {
    let run = run("no-current.json");
    let error = run
        .fetch_with(&OpenMeteoMarine, &location("Berlin", 52.52, 13.405), 0)
        .expect_err("no current block");
    assert_eq!(error.exit_code(), 3);
    let text = error.to_string();
    assert!(text.contains("no marine reading"), "{text}");
    assert!(text.contains("Berlin"), "{text}");
    assert!(text.contains("52.52"), "{text}");
}

#[test]
fn the_registry_licence_is_the_marine_source_credit() {
    assert_eq!(
        ProviderId::OpenMeteoMarine.metadata().licence,
        Some(MarineSource::OpenMeteoMarine.licence())
    );

    let run = run("sylt.json");
    let report = run
        .fetch_with(&OpenMeteoMarine, &location("Sylt", 54.54, 10.23), 0)
        .expect("the fixture parses");
    assert_eq!(
        report.attribution.licence.as_deref(),
        Some(MarineSource::OpenMeteoMarine.licence())
    );
    assert_eq!(report.attribution.provider, "open-meteo-marine");
}

#[test]
fn the_cache_key_carries_zero_days_and_the_weather_ttl() {
    let run = run("sylt.json");
    run.fetch_with(&OpenMeteoMarine, &location("Sylt", 54.54, 10.23), 0)
        .expect("the fixture parses");

    let key = CacheKey::weather_part(
        "open-meteo-marine",
        "marine",
        54.54,
        10.23,
        0,
        date(2026, 10, 5),
    );
    assert_eq!(
        key.normalised(),
        "weather|open-meteo-marine|marine|54.54|10.23|0|2026-10-05"
    );
    assert!(
        key.path()
            .to_string_lossy()
            .ends_with("weather/open-meteo-marine-marine-54.54-10.23-0-2026-10-05.json"),
        "{}",
        key.path().display()
    );

    let path = run.cache().entry_path(&key);
    let text = std::fs::read_to_string(&path).expect("the entry is written under that key");
    let entry: CacheEntry = serde_json::from_str(&text).expect("a cache envelope");
    assert_eq!(entry.ttl_secs, 600, "the weather TTL");
    assert_eq!(entry.status, 200);
}

#[test]
fn a_second_fetch_is_served_from_the_cache() {
    let run = run("sylt.json");
    run.fetch_with(&OpenMeteoMarine, &location("Sylt", 54.54, 10.23), 0)
        .expect("the fixture parses");
    run.fetch_with(&OpenMeteoMarine, &location("Sylt", 54.54, 10.23), 0)
        .expect("the cached fixture parses");
    assert_eq!(run.calls().len(), 1, "the cache must not refetch");
}

#[test]
fn the_request_forwards_no_requested_days() {
    // The marine panel is current conditions plus the daily wave summary, so a `--days` value in
    // a hand-built request must not change the URL (there is no `forecast_days` parameter).
    let run = run("sylt.json");
    run.fetch_with(&OpenMeteoMarine, &location("Sylt", 54.54, 10.23), 8)
        .expect("the fixture parses");
    let names: Vec<String> = run.calls()[0]
        .query_pairs()
        .iter()
        .map(|(name, _)| name.clone())
        .collect();
    assert!(!names.iter().any(|name| name == "forecast_days"));
    assert!(!names.iter().any(|name| name == "hourly"));
}
