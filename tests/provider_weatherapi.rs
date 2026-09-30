// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The `WeatherAPI` backend, driven by the recorded response in `tests/fixtures/weatherapi/`.
//!
//! No test here opens a socket: `StubTransport` answers from the fixture, `FakeClock` supplies the
//! fetch instant, and the API key comes from a throwaway `keys.toml` the harness writes. The
//! expected numbers were computed from the recorded JSON by a separate script applying the
//! documented aggregation rules.

// Exact comparison is the point in this file: every value comes from the recorded JSON.
#![allow(clippy::float_cmp)]

mod common;

use cirrocast::cache::CacheMode;
use cirrocast::geo::from_coordinates;
use cirrocast::http::StubReply;
use cirrocast::model::{Condition, DayForecast, DayPart, DayPartKind};
use cirrocast::provider::weatherapi::{FORECAST_URL, WeatherApi};
use common::{ProviderRun, fixture, fixture_location, fixture_reply, provider_clock};

/// The key the tests store; the fixture was scrubbed of the real one.
const KEY: &str = "test-key-0123456789abcdef";

/// A run over the forecast fixture, at 06:00 UTC on 2026-10-01.
fn run(mode: CacheMode) -> ProviderRun {
    let run = ProviderRun::new(
        vec![fixture_reply("weatherapi", "forecast.json")],
        provider_clock(2026, 10, 1),
        mode,
    );
    run.with_key("weatherapi", KEY);
    run
}

/// The four parts of `day`, in `DayPartKind::ALL` order.
fn part(day: &DayForecast, kind: DayPartKind) -> &DayPart {
    &day.parts[kind.index()]
}

#[test]
fn the_request_pins_the_language_and_the_metric_defaults() {
    let run = run(CacheMode::Normal);
    run.fetch_with(&WeatherApi, &fixture_location("beijing"), 3)
        .expect("the fixture parses");

    let calls = run.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].url(), FORECAST_URL);
    let pairs: Vec<(&str, &str)> = calls[0]
        .query_pairs()
        .iter()
        .map(|(name, value)| (name.as_str(), value.as_str()))
        .collect();
    assert_eq!(
        pairs,
        vec![
            ("key", KEY),
            ("q", "39.9042,116.4074"),
            ("days", "3"),
            ("lang", "en"),
        ]
    );
}

#[test]
fn the_current_block_comes_from_the_recorded_response() {
    let run = run(CacheMode::Normal);
    let report = run
        .fetch_with(&WeatherApi, &fixture_location("beijing"), 3)
        .expect("the fixture parses");

    let current = report.current.expect("the fixture has current conditions");
    assert_eq!(
        current.observed_at.to_rfc3339(),
        "2026-10-01T00:15:00+08:00"
    );
    assert_eq!(current.temp_c, 15.1);
    assert_eq!(current.feels_like_c, Some(7.7));
    assert_eq!(current.humidity_pct, 13);
    assert_eq!(current.pressure_hpa, 1023.0);
    assert_eq!(current.wind_kmh, 20.9);
    assert_eq!(current.wind_dir_deg, 302);
    assert_eq!(current.wind_gust_kmh, Some(37.9));
    assert_eq!(current.visibility_km, Some(10.0));
    assert_eq!(current.uv_index, Some(0.0));
    assert_eq!(current.weather, Condition::from_u8(0));
    assert!(!current.is_day);
}

#[test]
fn the_days_use_the_daily_aggregates_and_the_hourly_parts() {
    let run = run(CacheMode::Normal);
    let report = run
        .fetch_with(&WeatherApi, &fixture_location("beijing"), 3)
        .expect("the fixture parses");

    let dates: Vec<String> = report.days.iter().map(|day| day.date.to_string()).collect();
    assert_eq!(dates, vec!["2026-10-01", "2026-10-02", "2026-10-03"]);

    let first = &report.days[0];
    assert_eq!(part(first, DayPartKind::Morning).temp_c, 16.2);
    assert_eq!(part(first, DayPartKind::Noon).temp_c, 21.7);
    assert_eq!(part(first, DayPartKind::Evening).temp_c, 18.5);
    assert_eq!(part(first, DayPartKind::Night).temp_c, 14.8);
    assert_eq!(part(first, DayPartKind::Morning).wind_kmh, 19.4);
    assert_eq!(part(first, DayPartKind::Morning).wind_dir_deg, Some(318));
    assert_eq!(part(first, DayPartKind::Morning).humidity_pct, Some(12));
    assert_eq!(part(first, DayPartKind::Morning).precip_prob_pct, Some(0));
    // 1036 (smoky haze) maps into the atmosphere family.
    assert_eq!(
        part(first, DayPartKind::Evening).weather,
        Condition::from_u8(45)
    );

    // The daily extremes come from the response's own `day` block.
    assert_eq!(first.temp_min_c, 13.5);
    assert_eq!(first.temp_max_c, 21.7);

    // `astro` times are 12-hour clock strings joined to the day's local date.
    assert_eq!(
        first.sunrise.expect("a sunrise").to_rfc3339(),
        "2026-10-01T06:10:00+08:00"
    );
    assert_eq!(
        first.sunset.expect("a sunset").to_rfc3339(),
        "2026-10-01T17:57:00+08:00"
    );
}

#[test]
fn a_provisional_zone_is_repaired_from_the_response() {
    let run = run(CacheMode::Normal);
    let report = run
        .fetch_with(&WeatherApi, &from_coordinates(39.9042, 116.4074), 3)
        .expect("the fixture parses");
    // `tz_id` is an IANA name, so a coordinate location gets a real zone instead of a refusal.
    assert_eq!(report.location.tz.name(), "Asia/Shanghai");
}

#[test]
fn a_second_fetch_is_served_from_the_cache() {
    let run = run(CacheMode::Normal);
    run.fetch_with(&WeatherApi, &fixture_location("beijing"), 3)
        .expect("the fixture parses");
    run.fetch_with(&WeatherApi, &fixture_location("beijing"), 3)
        .expect("the cached fixture parses");
    assert_eq!(run.calls().len(), 1, "the cache must not refetch");
}

#[test]
fn a_rejected_key_is_an_invalid_key_error() {
    let run = ProviderRun::new(
        vec![StubReply::status(
            401,
            Vec::new(),
            fixture("weatherapi/error_401.json"),
        )],
        provider_clock(2026, 10, 1),
        CacheMode::Normal,
    );
    run.with_key("weatherapi", KEY);
    let error = run
        .fetch_with(&WeatherApi, &fixture_location("beijing"), 3)
        .expect_err("a 401 is a rejected key");
    assert_eq!(error.exit_code(), 6);
    let text = error.to_string();
    assert!(text.contains("cirrocast key set weatherapi"), "{text}");
    assert!(!text.contains(KEY), "the key leaked: {text}");
}

#[test]
fn a_403_keeps_the_upstream_taxonomy_and_the_provider_message() {
    let run = ProviderRun::new(
        vec![StubReply::status(
            403,
            Vec::new(),
            r#"{"error":{"code":2007,"message":"API key has exceeded calls per month quota."}}"#,
        )],
        provider_clock(2026, 10, 1),
        CacheMode::Normal,
    );
    run.with_key("weatherapi", KEY);
    let error = run
        .fetch_with(&WeatherApi, &fixture_location("beijing"), 3)
        .expect_err("a quota refusal is an error");
    // A 403 carries quota and plan refusals whose body text is the actionable part; only a 401 is
    // unambiguously "the credential is wrong" (see the provider contract in docs/plans/README.md).
    assert_eq!(error.exit_code(), 3);
    assert!(
        error.to_string().contains("exceeded calls per month quota"),
        "{error}"
    );
}
