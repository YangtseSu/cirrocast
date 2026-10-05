// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The NWS backend, driven by the recorded `api.weather.gov` payloads in `tests/fixtures/nws/`.
//!
//! No test here opens a socket: `StubTransport` answers from the fixtures and `FakeClock` supplies
//! both the fetch time and the `local_today` the cache keys are built from.
//!
//! `points.json`, `hourly.json`, `daily.json` and `points_out_of_coverage.json` are the recordings
//! verbatim (fetched with the project's own `User-Agent`). `edge_hourly.json` is **hand-authored**
//! to the same schema for the Fahrenheit/wind-range/null-value/unknown-text traps (its
//! `_provenance` field says so).

// Exact comparison is the point in this file: every value comes from the recorded JSON.
#![allow(clippy::float_cmp)]

mod common;

use std::path::Path;
use std::time::Duration;

use chrono::NaiveDate;
use chrono_tz::Tz;

use cirrocast::cache::{CacheKey, CacheMode};
use cirrocast::geo::from_coordinates;
use cirrocast::http::StubReply;
use cirrocast::model::{Condition, DayPartKind, Location};
use cirrocast::provider::nws::{
    DailyResponse, GRIDPOINTS_BASE, HourlyResponse, Nws, POINTS_BASE, condition_of, icon_condition,
    samples, temperature_c, text_condition, wind_kmh,
};
use common::{ProviderRun, fixture, fixture_reply, provider_clock};

/// The location the recorded fixtures were fetched for (Topeka, Kansas).
fn topeka() -> Location {
    from_coordinates(39.7456, -97.0892)
}

/// The three replies one whole Topeka fetch consumes, in request order.
fn recorded_replies() -> Vec<StubReply> {
    vec![
        fixture_reply("nws", "points.json"),
        fixture_reply("nws", "hourly.json"),
        fixture_reply("nws", "daily.json"),
    ]
}

/// A run over the recorded fixtures, at 06:00 UTC on the fixture's first day.
fn recorded_run() -> ProviderRun {
    ProviderRun::new(
        recorded_replies(),
        provider_clock(2026, 10, 6),
        CacheMode::Normal,
    )
}

/// `YYYY-MM-DD` as a date.
fn date(year: i32, month: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(year, month, day).expect("a valid date")
}

/// Whether the request carries `value` under header `name` (case-insensitively).
fn has_header(request: &cirrocast::http::HttpRequest, name: &str, value: &str) -> bool {
    request
        .headers()
        .iter()
        .any(|(key, held)| key.eq_ignore_ascii_case(name) && held == value)
}

#[test]
fn the_fetch_is_the_two_step_grid_walk() {
    let run = recorded_run();
    let report = run
        .fetch_with(&Nws, &topeka(), 3)
        .expect("the recorded fixtures parse");

    let calls = run.calls();
    assert_eq!(calls.len(), 3, "one point lookup and two grid resources");
    assert_eq!(
        calls[0].url(),
        format!("{POINTS_BASE}/39.7456,-97.0892"),
        "the point goes out at four decimals"
    );
    assert_eq!(
        calls[1].url(),
        format!("{GRIDPOINTS_BASE}/TOP/32,81/forecast/hourly")
    );
    assert_eq!(
        calls[2].url(),
        format!("{GRIDPOINTS_BASE}/TOP/32,81/forecast")
    );
    for call in &calls {
        assert!(
            has_header(call, "Accept", "application/geo+json"),
            "every request asks for the documented media type"
        );
    }

    // The point answer names the zone, which repairs the placeholder a coordinate query carries.
    assert!(
        cirrocast::geo::provisional_zone(&topeka()),
        "the test location must start with the placeholder zone the repair is for"
    );
    assert_eq!(report.location.tz, Tz::America__Chicago);
}

#[test]
fn the_recorded_payload_decodes_to_days_of_four_parts() {
    let run = recorded_run();
    let report = run
        .fetch_with(&Nws, &topeka(), 3)
        .expect("the recorded fixtures parse");

    assert!(
        report.current.is_none(),
        "the forecast payload carries no pressure, so there is no honest current block"
    );

    let dates: Vec<NaiveDate> = report.days.iter().map(|day| day.date).collect();
    assert_eq!(
        dates,
        vec![date(2026, 10, 6), date(2026, 10, 7), date(2026, 10, 8)]
    );
    for day in &report.days {
        for kind in DayPartKind::ALL {
            // Every part is filled: reaching into the array proves aggregation did not panic and
            // the part carries a described condition.
            let part = &day.parts[kind.index()];
            assert!(part.weather.is_known(), "{kind:?} has no condition");
        }
    }

    // Day extremes come from the daily periods, in °C: Tonight's low 54 °F and Tuesday's high
    // 82 °F.
    let first = &report.days[0];
    assert!(
        (first.temp_min_c - 12.222_222).abs() < 0.001,
        "{}",
        first.temp_min_c
    );
    assert!(
        (first.temp_max_c - 27.777_778).abs() < 0.001,
        "{}",
        first.temp_max_c
    );

    let attribution = &report.attribution;
    assert_eq!(attribution.provider, "nws");
    assert_eq!(
        attribution.licence.as_deref(),
        Some("api.weather.gov (NOAA/NWS, public domain)")
    );
}

#[test]
fn the_grid_mapping_is_reused_on_a_second_run() {
    // Enough replies for a second run's two forecast resources; the grid mapping must not ask for
    // a third.
    let run = ProviderRun::new(
        vec![
            fixture_reply("nws", "points.json"),
            fixture_reply("nws", "hourly.json"),
            fixture_reply("nws", "daily.json"),
            fixture_reply("nws", "hourly.json"),
            fixture_reply("nws", "daily.json"),
        ],
        provider_clock(2026, 10, 6),
        CacheMode::Normal,
    );
    let loc = topeka();
    run.fetch_with(&Nws, &loc, 3).expect("the first run parses");
    assert_eq!(run.calls().len(), 3);

    // Past the 600 s weather TTL but well inside the 30-day grid TTL, so only the forecast
    // resources are stale.
    run.clock().advance(Duration::from_secs(601));
    run.fetch_with(&Nws, &loc, 3)
        .expect("the second run parses");

    let calls = run.calls();
    assert_eq!(calls.len(), 5, "the second run re-asked for both resources");
    assert_eq!(
        calls[3].url(),
        format!("{GRIDPOINTS_BASE}/TOP/32,81/forecast/hourly")
    );
    assert_eq!(
        calls[4].url(),
        format!("{GRIDPOINTS_BASE}/TOP/32,81/forecast")
    );
    assert!(
        calls[3..]
            .iter()
            .all(|call| !call.url().starts_with(POINTS_BASE)),
        "the point lookup was never repeated"
    );
}

#[test]
fn an_out_of_coverage_point_is_named_in_the_error() {
    let run = ProviderRun::new(
        vec![StubReply::ok(
            404,
            fixture("nws/points_out_of_coverage.json"),
        )],
        provider_clock(2026, 10, 6),
        CacheMode::Normal,
    );
    let error = run
        .fetch_with(&Nws, &from_coordinates(48.85, 2.35), 1)
        .expect_err("a non-US point has no grid");

    assert_eq!(error.exit_code(), 3, "upstream failures exit 3");
    let text = error.to_string();
    assert!(text.contains("48.8500,2.3500"), "{text}");
    assert!(text.contains("outside the US"), "{text}");
}

#[test]
fn the_edge_fixture_decodes_the_documented_traps() {
    let response: HourlyResponse =
        serde_json::from_str(&fixture("nws/edge_hourly.json")).expect("the edge fixture parses");
    let decoded = samples(&response, Tz::America__Chicago, false).expect("the periods decode");
    assert_eq!(decoded.len(), 2);

    let first = &decoded[0];
    // 76 °F -> °C at decode time.
    assert!(
        (first.temp_c - 24.444_445).abs() < 0.001,
        "{}",
        first.temp_c
    );
    // "5 to 10 mph" keeps the upper bound (10 mph -> 16.09344 km/h).
    assert!(
        (first.wind_kmh - 16.093_44).abs() < 0.001,
        "{}",
        first.wind_kmh
    );
    // SSW is the shared compass table's 9th sector, centred on 203°.
    assert_eq!(first.wind_dir_deg, Some(203));
    // Nullable values stay absent rather than becoming 0.
    assert_eq!(first.precip_prob_pct, None);
    assert_eq!(first.humidity_pct, None);
    // An unknown shortForecast/icon pair is WMO 3.
    assert_eq!(first.weather.code(), 3);

    let second = &decoded[1];
    // Already-metric temperature is left alone; the empty direction is a variable wind.
    assert_eq!(second.temp_c, 20.0);
    assert_eq!(second.wind_dir_deg, None);
    assert_eq!(second.precip_prob_pct, Some(80));
    assert_eq!(second.humidity_pct, Some(90));
    // The icon maps even though the text table would too.
    assert_eq!(second.weather.code(), 80);
}

#[test]
fn a_wind_range_keeps_its_upper_bound_and_notes_itself() {
    let (single, note) = wind_kmh("10 mph");
    assert!((single - 16.093_44).abs() < 0.001);
    assert!(note.is_none());

    let (range, note) = wind_kmh("5 to 10 mph");
    assert!((range - 16.093_44).abs() < 0.001);
    let note = note.expect("a range is reported");
    assert!(note.contains("5 to 10 mph"), "{note}");
}

#[test]
fn fahrenheit_converts_and_celsius_stays() {
    assert!((temperature_c(76.0, "F") - 24.444_445).abs() < 0.001);
    assert_eq!(temperature_c(20.0, "C"), 20.0);
}

#[test]
fn an_unknown_short_forecast_is_wmo_3() {
    assert_eq!(
        condition_of("Volcanic Ash", "", false),
        Condition::from_u8(3)
    );
    // The written-out families do answer.
    assert_eq!(
        condition_of("Partly Cloudy", "", false),
        Condition::from_u8(2)
    );
    assert_eq!(
        condition_of("Chance Rain Showers", "", false),
        Condition::from_u8(80)
    );
    assert_eq!(
        condition_of("Snow Showers", "", false),
        Condition::from_u8(85)
    );
    assert_eq!(
        condition_of(
            "",
            "https://api.weather.gov/icons/land/night/tsra?size=small",
            false
        ),
        Condition::from_u8(95)
    );
}

#[test]
fn every_recorded_condition_is_in_the_tables() {
    // Exhaustiveness over the recordings: no recorded period may fall through to the WMO 3
    // "unknown" fallback.
    let hourly: HourlyResponse =
        serde_json::from_str(&fixture("nws/hourly.json")).expect("the hourly fixture parses");
    let daily: DailyResponse =
        serde_json::from_str(&fixture("nws/daily.json")).expect("the daily fixture parses");
    for period in hourly
        .properties
        .periods
        .iter()
        .chain(&daily.properties.periods)
    {
        assert!(
            icon_condition(&period.icon).is_some()
                || text_condition(&period.short_forecast).is_some(),
            "`{}` / `{}` is not in either table",
            period.icon,
            period.short_forecast
        );
    }
}

#[test]
fn the_cache_keys_have_the_step_24_shapes() {
    assert_eq!(
        CacheKey::grid("nws", 39.7456, -97.0892).path(),
        Path::new("grid/nws-39.746--97.089.json")
    );
    assert_eq!(
        CacheKey::weather_part("nws", "hourly", 39.7456, -97.0892, 3, date(2026, 10, 6)).path(),
        Path::new("weather/nws-hourly-39.75--97.09-3-2026-10-06.json")
    );
    assert_eq!(
        CacheKey::weather_part("nws", "daily", 39.7456, -97.0892, 3, date(2026, 10, 6)).path(),
        Path::new("weather/nws-daily-39.75--97.09-3-2026-10-06.json")
    );
}
