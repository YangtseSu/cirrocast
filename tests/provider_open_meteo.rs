// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The Open-Meteo backend, driven by the recorded responses in `tests/fixtures/open_meteo/`.
//!
//! No test here opens a socket: `StubTransport` answers from the fixtures and `FakeClock` supplies
//! both the fetch time and the `local_today` the cache key is built from, which is what makes the
//! rollover case testable without waiting for midnight.
//!
//! The expected values are not derived from the implementation: each one was computed from the
//! recorded JSON by a separate script applying the documented aggregation rules, so an error in
//! `aggregate_day` would have to agree with a wrong expectation *and* the same wrong reading of
//! the fixtures to slip through.

// Exact comparison is the point in this file: every value comes from the recorded JSON.
#![allow(clippy::float_cmp)]

mod common;

use std::time::Duration;

use chrono::NaiveDate;

use cirrocast::cache::{CacheKey, CacheMode};
use cirrocast::error::Error;
use cirrocast::http::StubReply;
use cirrocast::model::{Condition, DayForecast, DayPart, DayPartKind, LocationSource};
use common::{ProviderRun, fixture_location, provider_clock, provider_fixture};

/// The four parts of `day`, in `DayPartKind::ALL` order.
fn part(day: &DayForecast, kind: DayPartKind) -> &DayPart {
    &day.parts[kind.index()]
}

#[test]
fn the_request_matches_the_provider_contract() {
    let run = ProviderRun::fixture(
        "forecast_beijing_2026-07-15.json",
        (2026, 7, 15),
        CacheMode::Normal,
    );
    run.fetch(&fixture_location("beijing"), 3)
        .expect("the fixture parses");

    let calls = run.calls();
    assert_eq!(calls.len(), 1);
    let call = &calls[0];
    assert_eq!(call.url(), "https://api.open-meteo.com/v1/forecast");
    let pairs: Vec<(&str, &str)> = call
        .query_pairs()
        .iter()
        .map(|(name, value)| (name.as_str(), value.as_str()))
        .collect();
    assert_eq!(pairs[0], ("latitude", "39.9042"));
    assert_eq!(pairs[1], ("longitude", "116.4074"));
    assert_eq!(
        pairs[2],
        (
            "current",
            "temperature_2m,relative_humidity_2m,apparent_temperature,is_day,precipitation,\
weather_code,cloud_cover,pressure_msl,surface_pressure,wind_speed_10m,wind_direction_10m,\
wind_gusts_10m,visibility,uv_index"
        )
    );
    assert_eq!(
        pairs[3],
        (
            "hourly",
            "temperature_2m,apparent_temperature,precipitation_probability,precipitation,\
weather_code,wind_speed_10m,wind_direction_10m,relative_humidity_2m,visibility"
        )
    );
    assert_eq!(
        pairs[4],
        (
            "daily",
            "weather_code,temperature_2m_max,temperature_2m_min,sunrise,sunset"
        )
    );
    assert_eq!(pairs[5], ("forecast_days", "3"));
    assert_eq!(
        &pairs[6..],
        [
            ("timezone", "auto"),
            ("temperature_unit", "celsius"),
            ("wind_speed_unit", "kmh"),
            ("precipitation_unit", "mm"),
        ]
    );
}

#[test]
fn a_current_only_request_asks_for_neither_hourly_nor_daily() {
    let run = ProviderRun::fixture(
        "forecast_beijing_2026-07-15.json",
        (2026, 7, 15),
        CacheMode::Normal,
    );
    let report = run
        .fetch(&fixture_location("beijing"), 0)
        .expect("the fixture parses");

    let calls = run.calls();
    let names: Vec<&str> = calls[0]
        .query_pairs()
        .iter()
        .map(|(name, _)| name.as_str())
        .collect();
    assert_eq!(
        names,
        [
            "latitude",
            "longitude",
            "current",
            "timezone",
            "temperature_unit",
            "wind_speed_unit",
            "precipitation_unit",
        ]
    );
    assert!(report.days.is_empty());
    let current = report.current.expect("the current block is present");
    assert_eq!(current.temp_c, 18.1);
    assert_eq!(current.weather, Condition::from_u8(0));
    assert_eq!(current.humidity_pct, 11);
    assert_eq!(current.visibility_km, Some(17.28));
    assert_eq!(current.uv_index, Some(0.0));
    assert!(!current.is_day);
}

#[test]
fn the_beijing_fixture_aggregates_to_the_documented_parts() {
    let run = ProviderRun::fixture(
        "forecast_beijing_2026-07-15.json",
        (2026, 7, 15),
        CacheMode::Normal,
    );
    let report = run
        .fetch(&fixture_location("beijing"), 3)
        .expect("the fixture parses");
    assert_eq!(report.days.len(), 3);

    let day = &report.days[0];
    assert_eq!(
        day.date,
        NaiveDate::from_ymd_opt(2026, 7, 15).expect("a date")
    );
    assert_eq!(day.temp_min_c, 25.0);
    assert_eq!(day.temp_max_c, 35.4);
    assert_eq!(
        day.sunrise.expect("a sunrise").to_string(),
        "2026-07-15 04:58:00 +08:00"
    );
    assert_eq!(
        day.sunset.expect("a sunset").to_string(),
        "2026-07-15 19:42:00 +08:00"
    );

    let morning = part(day, DayPartKind::Morning);
    assert_eq!(morning.temp_c, 29.2, "09:00 is the morning's midpoint");
    assert_eq!(morning.weather, Condition::from_u8(0));
    assert_eq!(morning.precip_mm, 0.0);
    assert_eq!(morning.precip_prob_pct, Some(0));
    assert_eq!(morning.wind_kmh, 2.5);
    assert_eq!(
        morning.wind_dir_deg,
        Some(0),
        "upstream reports 360°, which wraps to north"
    );

    let noon = part(day, DayPartKind::Noon);
    assert_eq!(noon.temp_c, 35.4);
    assert_eq!(noon.feels_like_c, Some(39.9));

    let evening = part(day, DayPartKind::Evening);
    assert_eq!(evening.temp_c, 30.2);
    assert_eq!(
        evening.weather,
        Condition::from_u8(3),
        "overcast outranks the clear hours in the part"
    );

    let night = part(day, DayPartKind::Night);
    assert_eq!(night.temp_c, 26.0, "03:00 is the night's midpoint");
    assert_eq!(night.visibility_km, Some(8.92));
}

#[test]
fn the_lisbon_fixture_covers_the_frequency_tiebreak() {
    let run = ProviderRun::fixture(
        "forecast_lisbon_2026-05-04.json",
        (2026, 5, 4),
        CacheMode::Normal,
    );
    let report = run
        .fetch(&fixture_location("lisbon"), 1)
        .expect("the fixture parses");
    let day = &report.days[0];

    // Evening: clear (rank 1) once, mainly clear (rank 1) five times — equal severity, so the
    // higher frequency decides.
    assert_eq!(
        part(day, DayPartKind::Evening).weather,
        Condition::from_u8(1),
        "the tied rank must fall to the more frequent code"
    );
    // Night: mainly clear (rank 1) four times, partly cloudy (rank 2) once — severity wins.
    assert_eq!(part(day, DayPartKind::Night).weather, Condition::from_u8(2));
    // Noon: overcast four times, partly cloudy and mainly clear once each.
    assert_eq!(part(day, DayPartKind::Noon).weather, Condition::from_u8(3));
    assert_eq!(part(day, DayPartKind::Noon).temp_c, 19.0);
    assert!((part(day, DayPartKind::Noon).precip_mm - 0.2).abs() < 1e-3);
    assert_eq!(part(day, DayPartKind::Noon).precip_prob_pct, Some(5));
}

#[test]
fn the_longyearbyen_fixture_has_snow_and_no_sunrise() {
    let run = ProviderRun::fixture(
        "forecast_longyearbyen_2026-01-12.json",
        (2026, 1, 12),
        CacheMode::Normal,
    );
    let report = run
        .fetch(&fixture_location("longyearbyen"), 1)
        .expect("the fixture parses");
    let day = &report.days[0];

    assert_eq!(
        day.date,
        NaiveDate::from_ymd_opt(2026, 1, 12).expect("a date")
    );
    assert_eq!(day.temp_min_c, -8.2);
    assert_eq!(day.temp_max_c, 2.1);
    assert_eq!(
        (day.sunrise, day.sunset),
        (None, None),
        "upstream answers 00:00 for both instants in the polar night"
    );

    let morning = part(day, DayPartKind::Morning);
    assert_eq!(
        morning.weather,
        Condition::from_u8(73),
        "moderate snow (rank 17) outranks the three overcast hours"
    );
    assert!((morning.precip_mm - 0.5).abs() < 1e-3);
    assert_eq!(morning.precip_prob_pct, Some(67));

    let night = part(day, DayPartKind::Night);
    assert_eq!(night.weather, Condition::from_u8(86));
    assert_eq!(night.temp_c, -6.4);
}

#[test]
fn the_berlin_fixture_groups_the_spring_forward_day() {
    let run = ProviderRun::fixture(
        "forecast_berlin_2026-03-29.json",
        (2026, 3, 29),
        CacheMode::Normal,
    );
    let report = run
        .fetch(&fixture_location("berlin"), 1)
        .expect("the fixture parses");
    let day = &report.days[0];

    // Upstream returns a row labelled 02:00 for 2026-03-29 even though that local time does not
    // exist in Europe/Berlin; `resolve_local` moves it forward one hour, so it and the 03:00 row
    // are equidistant from the night's midpoint and the earlier array position wins. The two are
    // consecutive hours of the model's series, so their precipitation is summed, not deduplicated.
    let night = part(day, DayPartKind::Night);
    assert_eq!(night.temp_c, 4.8);
    assert_eq!(night.weather, Condition::from_u8(61));
    assert!((night.precip_mm - 0.9).abs() < 1e-3);
    assert_eq!(night.precip_prob_pct, Some(98));

    assert_eq!(part(day, DayPartKind::Morning).temp_c, 3.7);
    assert_eq!(part(day, DayPartKind::Noon).temp_c, 9.4);
    assert_eq!(part(day, DayPartKind::Evening).temp_c, 6.3);
    assert_eq!(day.temp_min_c, 3.2);
    assert_eq!(day.temp_max_c, 9.5);
    assert_eq!(
        day.sunrise.expect("a sunrise").to_string(),
        "2026-03-29 06:48:00 +02:00"
    );
}

#[test]
fn a_coordinate_location_takes_the_zone_from_the_response() {
    let mut coordinates = fixture_location("beijing");
    coordinates.source = LocationSource::Coordinates;
    coordinates.tz = chrono_tz::Tz::UTC;
    coordinates.name = "39.9042, 116.4074".to_owned();

    let run = ProviderRun::fixture(
        "forecast_beijing_2026-07-15.json",
        (2026, 7, 15),
        CacheMode::Normal,
    );
    let report = run.fetch(&coordinates, 1).expect("the fixture parses");

    assert_eq!(report.location.tz, chrono_tz::Tz::Asia__Shanghai);
    assert!(
        cirrocast::geo::location_line(&report.location).ends_with("Asia/Shanghai"),
        "the header must print the resolved zone"
    );
}

#[test]
fn a_geocoded_location_keeps_the_zone_it_was_resolved_with() {
    let mut geocoded = fixture_location("lisbon");

    let run = ProviderRun::fixture(
        "forecast_beijing_2026-07-15.json",
        (2026, 7, 15),
        CacheMode::Normal,
    );
    let report = run.fetch(&geocoded, 1).expect("the fixture parses");
    geocoded.tz = chrono_tz::Tz::Europe__Lisbon;
    assert_eq!(
        report.location.tz,
        chrono_tz::Tz::Europe__Lisbon,
        "a geocoded zone is authoritative and is not replaced"
    );
}

#[test]
fn a_second_call_is_served_from_the_cache() {
    let run = ProviderRun::fixture(
        "forecast_beijing_2026-07-15.json",
        (2026, 7, 15),
        CacheMode::Normal,
    );
    let first = run
        .fetch(&fixture_location("beijing"), 3)
        .expect("the fixture parses");
    let second = run
        .fetch(&fixture_location("beijing"), 3)
        .expect("the cached body parses");

    assert_eq!(run.calls().len(), 1, "the second call hits the cache");
    assert_eq!(first.days, second.days);
    assert_eq!(first.location, second.location);
}

#[test]
fn a_day_rollover_misses_by_key_instead_of_by_ttl() {
    let clock = provider_clock(2026, 7, 15);
    let run = ProviderRun::new(
        vec![
            provider_fixture("forecast_beijing_2026-07-15.json"),
            provider_fixture("forecast_beijing_2026-07-15.json"),
        ],
        clock.clone(),
        CacheMode::Normal,
    );

    run.fetch(&fixture_location("beijing"), 3)
        .expect("the fixture parses");
    clock.advance(Duration::from_secs(24 * 3600));
    run.fetch(&fixture_location("beijing"), 3)
        .expect("the fixture parses again");

    assert_eq!(run.calls().len(), 2);
    for day in [15, 16] {
        let date = NaiveDate::from_ymd_opt(2026, 7, day).expect("a date");
        let key = CacheKey::weather("open-meteo", 39.9042, 116.4074, 3, date);
        assert!(
            run.cache().entry_path(&key).exists(),
            "{} must be cached",
            key.path().display()
        );
    }
}

#[test]
fn offline_without_an_entry_names_the_key_path() {
    let run = ProviderRun::new(Vec::new(), provider_clock(2026, 7, 15), CacheMode::Offline);

    let error = run
        .fetch(&fixture_location("beijing"), 3)
        .expect_err("nothing is cached yet");
    assert_eq!(error.exit_code(), 3);
    assert!(
        error.to_string().contains(
            "offline mode: no cached entry for weather/open-meteo-39.90-116.41-3-2026-07-15.json"
        ),
        "{error}"
    );
    assert!(run.calls().is_empty(), "offline never asks the transport");
}

#[test]
fn an_upstream_rejection_keeps_the_reason() {
    let body = std::fs::read_to_string(common::fixture_path(
        "http/open_meteo_error_invalid_param.json",
    ))
    .expect("the recorded error fixture is readable");
    let run = ProviderRun::new(
        vec![StubReply::ok(400, body)],
        provider_clock(2026, 7, 15),
        CacheMode::Normal,
    );

    let error = run
        .fetch(&fixture_location("beijing"), 3)
        .expect_err("a 400 is not retried and is not a report");
    assert_eq!(error.exit_code(), 3);
    let Error::Upstream {
        message, status, ..
    } = error
    else {
        panic!("expected an upstream error");
    };
    assert_eq!(status, Some(400));
    assert!(message.contains("Latitude must be in range"), "{message}");
}

#[test]
fn the_report_carries_the_exact_request_as_its_attribution() {
    let run = ProviderRun::fixture(
        "forecast_beijing_2026-07-15.json",
        (2026, 7, 15),
        CacheMode::Normal,
    );
    let report = run
        .fetch(&fixture_location("beijing"), 1)
        .expect("the fixture parses");

    assert_eq!(report.attribution.provider, "open-meteo");
    assert_eq!(report.attribution.url, run.calls()[0].full_url());
    assert_eq!(report.attribution.raw, None, "raw pairs are a -v detail");
}
