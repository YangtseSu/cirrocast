// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The `OpenWeatherMap` backend, driven by the recorded responses in `tests/fixtures/owm/`.
//!
//! No test here opens a socket: `StubTransport` answers from the fixtures, `FakeClock` supplies the
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
use cirrocast::provider::openweathermap::{CURRENT_URL, FORECAST_URL, OpenWeatherMap};
use common::{ProviderRun, fixture_location, fixture_reply, provider_clock};

/// The key the tests store; the fixtures were scrubbed of the real one.
const KEY: &str = "test-key-0123456789abcdef";

/// A run over the current and forecast fixtures, at 06:00 UTC on 2026-10-01.
fn run(mode: CacheMode) -> ProviderRun {
    let run = ProviderRun::new(
        vec![
            fixture_reply("owm", "current.json"),
            fixture_reply("owm", "forecast.json"),
        ],
        provider_clock(2026, 10, 1),
        mode,
    );
    run.with_key("openweathermap", KEY);
    run
}

/// The four parts of `day`, in `DayPartKind::ALL` order.
fn part(day: &DayForecast, kind: DayPartKind) -> &DayPart {
    &day.parts[kind.index()]
}

#[test]
fn the_two_requests_are_the_documented_endpoints() {
    let run = run(CacheMode::Normal);
    run.fetch_with(&OpenWeatherMap, &fixture_location("beijing"), 3)
        .expect("the fixtures parse");

    let calls = run.calls();
    assert_eq!(
        calls.len(),
        2,
        "one call for the current block, one for the forecast"
    );
    assert_eq!(calls[0].url(), CURRENT_URL);
    assert_eq!(calls[1].url(), FORECAST_URL);

    let pairs: Vec<(&str, &str)> = calls[0]
        .query_pairs()
        .iter()
        .map(|(name, value)| (name.as_str(), value.as_str()))
        .collect();
    assert_eq!(
        pairs,
        vec![
            ("lat", "39.9042"),
            ("lon", "116.4074"),
            ("units", "metric"),
            ("appid", KEY),
        ]
    );
}

#[test]
fn the_current_block_comes_from_the_recorded_response() {
    let run = run(CacheMode::Normal);
    let report = run
        .fetch_with(&OpenWeatherMap, &fixture_location("beijing"), 3)
        .expect("the fixtures parse");

    let current = report.current.expect("the current fixture is complete");
    assert_eq!(
        current.observed_at.to_rfc3339(),
        "2026-10-01T00:14:12+08:00"
    );
    assert_eq!(current.temp_c, 15.92);
    assert_eq!(current.feels_like_c, Some(13.96));
    assert_eq!(current.humidity_pct, Some(15));
    assert_eq!(current.pressure_hpa, 1022.0);
    assert_eq!(current.cloud_cover_pct, Some(0));
    // `units=metric` serves m/s; the model is km/h.
    assert_eq!(current.wind_kmh, 5.41 * 3.6);
    assert_eq!(current.wind_dir_deg, Some(309));
    assert_eq!(current.wind_gust_kmh, Some(10.16 * 3.6));
    assert_eq!(current.visibility_km, Some(10.0));
    assert_eq!(current.weather, Condition::from_u8(0));
    // The icon's trailing `n` is the day/night flag.
    assert!(!current.is_day);
    assert!(current.uv_index.is_none());
}

#[test]
fn the_forecast_slots_become_local_days() {
    let run = run(CacheMode::Normal);
    let report = run
        .fetch_with(&OpenWeatherMap, &fixture_location("beijing"), 3)
        .expect("the fixtures parse");

    let dates: Vec<String> = report.days.iter().map(|day| day.date.to_string()).collect();
    assert_eq!(dates, vec!["2026-10-01", "2026-10-02", "2026-10-03"]);

    let first = &report.days[0];
    assert_eq!(part(first, DayPartKind::Morning).temp_c, 16.23);
    assert_eq!(part(first, DayPartKind::Noon).temp_c, 21.92);
    assert_eq!(part(first, DayPartKind::Evening).temp_c, 18.94);
    assert_eq!(part(first, DayPartKind::Night).temp_c, 11.83);
    assert_eq!(part(first, DayPartKind::Morning).wind_kmh, 4.53 * 3.6);
    assert_eq!(part(first, DayPartKind::Morning).wind_dir_deg, Some(311));
    assert_eq!(part(first, DayPartKind::Morning).humidity_pct, Some(11));
    assert_eq!(part(first, DayPartKind::Morning).precip_mm, 0.0);
    assert_eq!(part(first, DayPartKind::Morning).precip_prob_pct, Some(0));
    assert_eq!(
        part(first, DayPartKind::Morning).weather,
        Condition::from_u8(0)
    );
    // The daily extremes are computed from the slots, not read from `main.temp_min`/`max`.
    assert_eq!(first.temp_min_c, 11.83);
    assert_eq!(first.temp_max_c, 21.92);

    // The third day mixes cloud families: 803 → broken clouds, 804 → overcast.
    let third = &report.days[2];
    assert_eq!(
        part(third, DayPartKind::Evening).weather,
        Condition::from_u8(1)
    );
    assert_eq!(
        part(third, DayPartKind::Night).weather,
        Condition::from_u8(3)
    );
}

#[test]
fn a_second_fetch_is_served_from_the_cache() {
    let run = run(CacheMode::Normal);
    run.fetch_with(&OpenWeatherMap, &fixture_location("beijing"), 3)
        .expect("the fixtures parse");
    run.fetch_with(&OpenWeatherMap, &fixture_location("beijing"), 3)
        .expect("the cached fixtures parse");
    assert_eq!(
        run.calls().len(),
        2,
        "the cache must not refetch either call"
    );
}

#[test]
fn a_missing_key_names_the_environment_variable() {
    let run = ProviderRun::new(
        vec![fixture_reply("owm", "current.json")],
        provider_clock(2026, 10, 1),
        CacheMode::Normal,
    );
    let error = run
        .fetch_with(&OpenWeatherMap, &fixture_location("beijing"), 3)
        .expect_err("no key is stored");
    assert_eq!(error.exit_code(), 6);
    let text = error.to_string();
    assert!(text.contains("openweathermap"), "{text}");
    assert!(text.contains("CIRROCAST_OPENWEATHERMAP_KEY"), "{text}");
}

#[test]
fn a_provisional_zone_is_refused_before_any_request() {
    let run = run(CacheMode::Normal);
    let error = run
        .fetch_with(&OpenWeatherMap, &from_coordinates(39.9042, 116.4074), 3)
        .expect_err("raw coordinates carry no zone");
    assert_eq!(error.exit_code(), 2);
    let text = error.to_string();
    assert!(text.contains("pass a place name"), "{text}");
    assert!(text.contains("--tz Asia/Shanghai"), "{text}");
    assert_eq!(
        run.calls(),
        Vec::<cirrocast::http::HttpRequest>::new(),
        "no request may be sent"
    );
}

#[test]
fn a_rejected_key_is_an_invalid_key_error() {
    let run = ProviderRun::new(
        vec![StubReply::status(
            401,
            Vec::new(),
            r#"{"cod":401,"message":"Invalid API key."}"#,
        )],
        provider_clock(2026, 10, 1),
        CacheMode::Normal,
    );
    run.with_key("openweathermap", KEY);
    let error = run
        .fetch_with(&OpenWeatherMap, &fixture_location("beijing"), 3)
        .expect_err("a 401 is a rejected key");
    assert_eq!(error.exit_code(), 6);
    let text = error.to_string();
    assert!(text.contains("cirrocast key set openweathermap"), "{text}");
    assert!(!text.contains(KEY), "the key leaked: {text}");
}
