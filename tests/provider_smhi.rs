// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The SMHI backend, driven by the recorded `SNOW1gv1` response in `tests/fixtures/smhi/`.
//!
//! No test here opens a socket: `StubTransport` answers from the fixtures and `FakeClock` supplies
//! the fetch instant. The expected numbers were computed from the recorded JSON by a separate
//! script applying the documented aggregation rules, so an error in the shared aggregation would
//! have to agree with a wrong expectation *and* the same wrong reading of the fixture.

// Exact comparison is the point in this file: every value comes from the recorded JSON.
#![allow(clippy::float_cmp)]

mod common;

use chrono::NaiveDate;

use cirrocast::cache::CacheMode;
use cirrocast::geo::from_coordinates;
use cirrocast::http::StubReply;
use cirrocast::model::{Condition, DayForecast, DayPart, DayPartKind};
use cirrocast::provider::smhi::{BASE, Smhi};
use common::{ProviderRun, fixture, fixture_location, fixture_reply, provider_clock};

/// A run over one SMHI fixture, at 06:00 UTC on `day`.
fn run(file: &str, day: (i32, u32, u32), mode: CacheMode) -> ProviderRun {
    ProviderRun::new(
        vec![fixture_reply("smhi", file)],
        provider_clock(day.0, day.1, day.2),
        mode,
    )
}

/// The four parts of `day`, in `DayPartKind::ALL` order.
fn part(day: &DayForecast, kind: DayPartKind) -> &DayPart {
    &day.parts[kind.index()]
}

/// `YYYY-MM-DD` as a date.
fn date(year: i32, month: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(year, month, day).expect("a valid date")
}

#[test]
fn the_request_is_the_snow1g_point_path_with_lon_before_lat() {
    let run = run(
        "point_stockholm_2026-09-30.json",
        (2026, 9, 30),
        CacheMode::Normal,
    );
    run.fetch_with(&Smhi, &fixture_location("stockholm"), 3)
        .expect("the fixture parses");

    let calls = run.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0].url(),
        format!("{BASE}/geotype/point/lon/18.0600/lat/59.3300/data.json")
    );
    assert_eq!(
        calls[0].query_pairs(),
        [] as [(String, String); 0],
        "the `parameters` filter is deliberately not sent"
    );
}

#[test]
fn a_partial_local_day_is_skipped_for_the_first_complete_one() {
    let run = run(
        "point_stockholm_2026-09-30.json",
        (2026, 9, 30),
        CacheMode::Normal,
    );
    let report = run
        .fetch_with(&Smhi, &fixture_location("stockholm"), 3)
        .expect("the fixture parses");

    // The series starts at 17:00Z (19:00 local), so the location-local 30th has no morning, noon
    // or night: the first day a `DayPart` can be built for is the 1st.
    let dates: Vec<NaiveDate> = report.days.iter().map(|day| day.date).collect();
    assert_eq!(
        dates,
        vec![date(2026, 10, 1), date(2026, 10, 2), date(2026, 10, 3)]
    );
}

#[test]
fn the_parts_come_from_the_recorded_samples() {
    let run = run(
        "point_stockholm_2026-09-30.json",
        (2026, 9, 30),
        CacheMode::Normal,
    );
    let report = run
        .fetch_with(&Smhi, &fixture_location("stockholm"), 3)
        .expect("the fixture parses");
    let day = &report.days[0];

    // Representative samples: the hour closest to each part's midpoint.
    assert_eq!(part(day, DayPartKind::Morning).temp_c, 14.2);
    assert_eq!(part(day, DayPartKind::Noon).temp_c, 18.3);
    assert_eq!(part(day, DayPartKind::Evening).temp_c, 13.7);
    assert_eq!(part(day, DayPartKind::Night).temp_c, 13.3);
    assert_eq!(part(day, DayPartKind::Morning).wind_kmh, 2.9 * 3.6);
    assert_eq!(part(day, DayPartKind::Morning).wind_dir_deg, Some(113));
    assert_eq!(part(day, DayPartKind::Morning).humidity_pct, Some(98));
    assert_eq!(part(day, DayPartKind::Noon).visibility_km, Some(56.0));

    // Daily extremes come from the day's own samples, since SMHI has no daily block.
    assert_eq!(day.temp_min_c, 12.8);
    assert_eq!(day.temp_max_c, 18.4);
    assert!(day.sunrise.is_none() && day.sunset.is_none());

    // Dominant conditions: `6` (overcast) outranks `4`/`2`/`1` in the morning, the noon samples are
    // all `1` (clear), and the evening is clear as well.
    assert_eq!(
        part(day, DayPartKind::Morning).weather,
        Condition::from_u8(3)
    );
    assert_eq!(part(day, DayPartKind::Noon).weather, Condition::from_u8(0));
    assert_eq!(
        part(day, DayPartKind::Evening).weather,
        Condition::from_u8(0)
    );
    assert_eq!(part(day, DayPartKind::Night).weather, Condition::from_u8(3));
}

#[test]
fn the_current_block_uses_the_latest_started_step() {
    let run = run(
        "point_stockholm_2026-09-30.json",
        (2026, 9, 30),
        CacheMode::Normal,
    );
    let report = run
        .fetch_with(&Smhi, &fixture_location("stockholm"), 3)
        .expect("the fixture parses");

    // The clock sits at 06:00Z, before the series starts, so the first step is the fallback.
    let current = report
        .current
        .expect("the first step builds a current block");
    assert_eq!(
        current.observed_at.to_rfc3339(),
        "2026-09-30T19:00:00+02:00"
    );
    assert_eq!(current.temp_c, 13.9);
    assert_eq!(current.humidity_pct, Some(98));
    assert_eq!(current.pressure_hpa, 1035.1);
    assert_eq!(current.wind_kmh, 4.1 * 3.6);
    assert_eq!(current.wind_gust_kmh, Some(7.4 * 3.6));
    assert_eq!(current.wind_dir_deg, Some(108));
    assert_eq!(current.visibility_km, Some(28.3));
    assert_eq!(current.weather, Condition::from_u8(3));
    // 8 oktas of cloud cover is a fully overcast sky.
    assert_eq!(current.cloud_cover_pct, Some(100));
    // 19:00 local is outside the civil day.
    assert!(!current.is_day);
    // SMHI publishes neither an apparent temperature nor a UV index.
    assert!(current.feels_like_c.is_none());
    assert!(current.uv_index.is_none());
}

#[test]
fn the_sentinel_fixture_keeps_missing_values_missing() {
    let run = run(
        "point_sentinel_2026-10-01.json",
        (2026, 10, 1),
        CacheMode::Normal,
    );
    let report = run
        .fetch_with(&Smhi, &fixture_location("stockholm"), 2)
        .expect("the fixture parses");

    // The payload holds one complete local day, so a two-day request gets one day: fewer than
    // requested is not an error, an incomplete day is never invented.
    assert_eq!(report.days.len(), 1);
    let day = &report.days[0];
    assert_eq!(day.date, date(2026, 10, 1));

    // The 05:00 step carries `9999` for its temperature, so it is dropped: the day's minimum is the
    // midnight sample, not a fabricated zero.
    assert_eq!(day.temp_min_c, 10.0);
    assert_eq!(day.temp_max_c, 10.0 + 23.0 * 0.5);

    // `9999` in an optional field becomes `None` for the part whose representative carries it.
    assert_eq!(part(day, DayPartKind::Night).visibility_km, None);
    assert_eq!(part(day, DayPartKind::Morning).precip_prob_pct, None);
    // ... while a valid value survives.
    assert_eq!(part(day, DayPartKind::Noon).visibility_km, Some(20.0));

    // 18:00 local carries 0.5 mm of light rain, which dominates the clear hours.
    assert_eq!(part(day, DayPartKind::Evening).precip_mm, 0.5);
    assert_eq!(
        part(day, DayPartKind::Evening).weather,
        Condition::from_u8(61)
    );

    // The 09:00 step's interval starts exactly at the clock (06:00Z), so it is the current one.
    let current = report.current.expect("the 09:00 step is complete");
    assert_eq!(current.temp_c, 14.5);
    assert_eq!(
        current.observed_at.to_rfc3339(),
        "2026-10-01T09:00:00+02:00"
    );
    // The same step carries the in-band `9999` for humidity: only that field is nulled, and the
    // rest of the block — the temperature, the sky, the instant — still decodes.
    assert!(
        current.humidity_pct.is_none(),
        "9999 humidity must be null, not a reading: {:?}",
        current.humidity_pct
    );
    // Three oktas is 37.5 % of the sky, rounded.
    assert_eq!(current.cloud_cover_pct, Some(38));
    assert!(current.is_day);
}

#[test]
fn an_out_of_coverage_point_is_an_upstream_error() {
    let run = ProviderRun::new(
        vec![StubReply::status(
            404,
            Vec::new(),
            fixture("smhi/point_out_of_coverage_404.html"),
        )],
        provider_clock(2026, 10, 1),
        CacheMode::Normal,
    );
    // The request goes out before the zone check, so an out-of-coverage point falls through a
    // chain (`Upstream`) instead of stopping it with a usage error.
    let error = run
        .fetch_with(&Smhi, &from_coordinates(39.9, 116.4), 1)
        .expect_err("a 404 is an upstream error");
    assert_eq!(error.exit_code(), 3);
    let text = error.to_string();
    assert!(text.contains("out of coverage"), "{text}");
    assert!(text.contains("39.90,116.40"), "{text}");
    assert!(
        !text.contains("<html>"),
        "the HTML body is not pasted: {text}"
    );
}

#[test]
fn a_provisional_zone_inside_the_coverage_is_a_usage_error() {
    let run = run(
        "point_stockholm_2026-09-30.json",
        (2026, 9, 30),
        CacheMode::Normal,
    );
    let error = run
        .fetch_with(&Smhi, &from_coordinates(59.33, 18.06), 1)
        .expect_err("raw coordinates carry no zone");
    assert_eq!(error.exit_code(), 2);
    let text = error.to_string();
    assert!(text.contains("pass a place name"), "{text}");
    assert!(text.contains("Stockholm"), "{text}");
}

#[test]
fn a_second_fetch_is_served_from_the_cache() {
    let run = run(
        "point_stockholm_2026-09-30.json",
        (2026, 9, 30),
        CacheMode::Normal,
    );
    run.fetch_with(&Smhi, &fixture_location("stockholm"), 3)
        .expect("the fixture parses");
    run.fetch_with(&Smhi, &fixture_location("stockholm"), 3)
        .expect("the cached fixture parses");
    assert_eq!(run.calls().len(), 1, "the cache must not refetch");
}
