// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The `PirateWeather` backend, driven by the recorded response in `tests/fixtures/pirateweather/`.
//!
//! No test here opens a socket: `StubTransport` answers from the fixture, a fixed clock supplies the
//! fetch instant, and the API key comes from a throwaway `keys.toml` the harness writes. The
//! expected numbers were computed from the recorded JSON by a separate script applying the
//! documented aggregation rules.

// Exact comparison is the point in this file: every value comes from the recorded JSON.
#![allow(clippy::float_cmp)]

mod common;

use std::sync::Arc;
use std::time::SystemTime;

use chrono::{TimeZone as _, Utc};

use cirrocast::cache::{CacheMode, FakeClock};
use cirrocast::geo::from_coordinates;
use cirrocast::http::StubReply;
use cirrocast::model::{Condition, DayForecast, DayPart, DayPartKind};
use cirrocast::provider::pirateweather::{FORECAST_BASE, PirateWeather};
use common::{ProviderRun, fixture, fixture_location, fixture_reply};

/// The key the tests store; the fixture was scrubbed of the real one.
const KEY: &str = "test-key-0123456789abcdef";

/// The instant the fixture was recorded at (`currently.time` is 2026-10-01T00:34+08:00).
fn recording_clock() -> Arc<FakeClock> {
    let start = Utc
        .with_ymd_and_hms(2026, 9, 30, 16, 34, 0)
        .single()
        .expect("a valid instant");
    Arc::new(FakeClock::new(SystemTime::from(start)))
}

/// A run over the recorded response, at the recording instant.
fn run(mode: CacheMode) -> ProviderRun {
    let run = ProviderRun::new(
        vec![fixture_reply("pirateweather", "forecast.json")],
        recording_clock(),
        mode,
    );
    run.with_key("pirateweather", KEY);
    run
}

/// The four parts of `day`, in `DayPartKind::ALL` order.
fn part(day: &DayForecast, kind: DayPartKind) -> &DayPart {
    &day.parts[kind.index()]
}

#[test]
fn the_key_is_a_path_segment_and_the_units_are_pinned() {
    let run = run(CacheMode::Normal);
    run.fetch_with(&PirateWeather, &fixture_location("beijing"), 3)
        .expect("the fixture parses");

    let calls = run.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0].url(),
        format!("{FORECAST_BASE}/{KEY}/39.9042,116.4074")
    );
    let pairs: Vec<(&str, &str)> = calls[0]
        .query_pairs()
        .iter()
        .map(|(name, value)| (name.as_str(), value.as_str()))
        .collect();
    assert_eq!(
        pairs,
        vec![
            ("units", "si"),
            ("exclude", "minutely,alerts"),
            ("lang", "en"),
            ("extend", "hourly"),
        ]
    );
}

#[test]
fn the_current_block_converts_the_si_units_and_the_fractions() {
    let run = run(CacheMode::Normal);
    let report = run
        .fetch_with(&PirateWeather, &fixture_location("beijing"), 3)
        .expect("the fixture parses");

    let current = report.current.expect("the fixture has current conditions");
    assert_eq!(
        current.observed_at.to_rfc3339(),
        "2026-10-01T00:34:00+08:00"
    );
    assert_eq!(current.temp_c, 13.13);
    assert_eq!(current.feels_like_c, Some(9.43));
    // humidity and cloud cover are 0–1 decimals.
    assert_eq!(current.humidity_pct, Some(24));
    assert_eq!(current.cloud_cover_pct, Some(99));
    assert_eq!(current.pressure_hpa, 1021.96);
    // wind is m/s under `units=si`.
    assert_eq!(current.wind_kmh, 1.03 * 3.6);
    assert_eq!(current.wind_gust_kmh, Some(10.38 * 3.6));
    assert_eq!(current.wind_dir_deg, Some(310));
    assert_eq!(current.visibility_km, Some(16.09));
    assert_eq!(current.uv_index, Some(0.0));
    assert_eq!(current.weather, Condition::from_u8(3));
    // The `cloudy` icon carries no day/night suffix, so the local civil day decides.
    assert!(!current.is_day);
}

#[test]
fn the_days_use_the_daily_aggregates_and_the_hourly_parts() {
    let run = run(CacheMode::Normal);
    let report = run
        .fetch_with(&PirateWeather, &fixture_location("beijing"), 3)
        .expect("the fixture parses");

    let dates: Vec<String> = report.days.iter().map(|day| day.date.to_string()).collect();
    assert_eq!(dates, vec!["2026-10-01", "2026-10-02", "2026-10-03"]);

    let first = &report.days[0];
    assert_eq!(part(first, DayPartKind::Morning).temp_c, 16.2);
    assert_eq!(part(first, DayPartKind::Noon).temp_c, 21.7);
    assert_eq!(part(first, DayPartKind::Evening).temp_c, 14.2);
    assert_eq!(part(first, DayPartKind::Night).temp_c, 10.9);
    assert_eq!(part(first, DayPartKind::Morning).wind_kmh, 2.06 * 3.6);
    assert_eq!(part(first, DayPartKind::Morning).wind_dir_deg, Some(312));
    assert_eq!(part(first, DayPartKind::Morning).humidity_pct, Some(25));
    assert_eq!(part(first, DayPartKind::Morning).precip_prob_pct, Some(0));
    // The part's condition is the most severe code present, not the representative's: the morning
    // mixes a partly-cloudy hour into the clear ones, and the evening ends overcast.
    assert_eq!(
        part(first, DayPartKind::Morning).weather,
        Condition::from_u8(2)
    );
    assert_eq!(
        part(first, DayPartKind::Noon).weather,
        Condition::from_u8(0)
    );
    assert_eq!(
        part(first, DayPartKind::Evening).weather,
        Condition::from_u8(3)
    );
    assert_eq!(
        part(first, DayPartKind::Night).weather,
        Condition::from_u8(3)
    );

    // The daily extremes and sun times come from the response's own daily entries.
    assert_eq!(first.temp_min_c, 8.4);
    assert_eq!(first.temp_max_c, 21.7);
    assert_eq!(
        first.sunrise.expect("a sunrise").to_rfc3339(),
        "2026-10-01T06:10:25+08:00"
    );
    assert_eq!(
        first.sunset.expect("a sunset").to_rfc3339(),
        "2026-10-01T17:57:14+08:00"
    );

    // A day with real precipitation probability: the morning of the 3rd peaks at 48 % (the part
    // maximum, not the representative's value), and its overcast hour outranks the partly-cloudy
    // ones.
    let third = &report.days[2];
    assert_eq!(part(third, DayPartKind::Morning).precip_prob_pct, Some(48));
    assert_eq!(
        part(third, DayPartKind::Morning).weather,
        Condition::from_u8(3)
    );
}

#[test]
fn a_provisional_zone_is_repaired_from_the_response() {
    let run = run(CacheMode::Normal);
    let report = run
        .fetch_with(&PirateWeather, &from_coordinates(39.9042, 116.4074), 3)
        .expect("the fixture parses");
    assert_eq!(report.location.tz.name(), "Asia/Shanghai");
}

#[test]
fn a_second_fetch_is_served_from_the_cache() {
    let run = run(CacheMode::Normal);
    run.fetch_with(&PirateWeather, &fixture_location("beijing"), 3)
        .expect("the fixture parses");
    run.fetch_with(&PirateWeather, &fixture_location("beijing"), 3)
        .expect("the cached fixture parses");
    assert_eq!(run.calls().len(), 1, "the cache must not refetch");
}

#[test]
fn a_rejected_key_is_an_invalid_key_error_and_never_leaks_the_key() {
    let run = ProviderRun::new(
        vec![StubReply::status(
            401,
            Vec::new(),
            fixture("pirateweather/error_401.txt"),
        )],
        recording_clock(),
        CacheMode::Normal,
    );
    run.with_key("pirateweather", KEY);
    let error = run
        .fetch_with(&PirateWeather, &fixture_location("beijing"), 3)
        .expect_err("a 401 is a rejected key");
    assert_eq!(error.exit_code(), 6);
    let text = error.to_string();
    assert!(text.contains("cirrocast key set pirateweather"), "{text}");
    assert!(!text.contains(KEY), "the key leaked: {text}");
}
