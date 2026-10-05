// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The Open-Meteo archive backend, driven by the recorded reanalysis response in
//! `tests/fixtures/archive/`.
//!
//! No test here opens a socket: `StubTransport` answers from the fixture and `FakeClock` supplies
//! the fetch instant, which is also the `local_today` the latency seam and the window validation
//! are judged against. The expected numbers were computed from the recorded JSON by a separate
//! script applying the documented aggregation rules, so an error in the shared aggregation would
//! have to agree with a wrong expectation *and* the same wrong reading of the fixture.
//!
//! The recorded span is the one the acceptance criterion names: Berlin (52.52/13.41) on
//! `2026-09-14`, requested with exactly the provider's parameter list.

// Exact comparison is the point in this file: every value comes from the recorded JSON.
#![allow(clippy::float_cmp)]

mod common;

use chrono::NaiveDate;

use cirrocast::cache::{CacheKey, CacheMode};
use cirrocast::http::StubReply;
use cirrocast::model::{Condition, DayForecast, DayPart, DayPartKind, Report, ReportMode};
use cirrocast::provider::open_meteo;
use cirrocast::provider::open_meteo_archive::{BASE, OpenMeteoArchive};
use cirrocast::provider::{DateWindow, FetchRequest, HourlyResolution, Provider};
use common::{ProviderRun, fixture, fixture_location, fixture_reply, provider_clock};

/// `YYYY-MM-DD` as a date.
fn date(year: i32, month: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(year, month, day).expect("a valid date")
}

/// The window covering exactly `2026-09-14`, the fixture's own span.
fn recorded_day() -> DateWindow {
    DateWindow::day(date(2026, 9, 14))
}

/// A run with the recorded archive body, at 06:00 UTC on `day`.
fn run(day: (i32, u32, u32), mode: CacheMode) -> ProviderRun {
    ProviderRun::new(
        vec![fixture_reply("archive", "berlin-2026-09-14.json")],
        provider_clock(day.0, day.1, day.2),
        mode,
    )
}

/// One fetch of `window` through the archive provider.
fn fetch(run: &ProviderRun, window: DateWindow) -> cirrocast::error::Result<Report> {
    OpenMeteoArchive.fetch(
        &fixture_location("berlin"),
        &FetchRequest::for_window(window, HourlyResolution::Hourly),
        &run.env(),
    )
}

/// The four parts of `day`, in `DayPartKind::ALL` order.
fn part(day: &DayForecast, kind: DayPartKind) -> &DayPart {
    &day.parts[kind.index()]
}

#[test]
fn the_request_is_the_archive_path_with_the_explicit_span_and_no_current_block() {
    let run = run((2026, 10, 6), CacheMode::Normal);
    fetch(&run, recorded_day()).expect("the fixture parses");

    let calls = run.calls();
    assert_eq!(calls.len(), 1);
    let call = &calls[0];
    assert_eq!(call.url(), BASE);
    let pairs: Vec<(&str, &str)> = call
        .query_pairs()
        .iter()
        .map(|(name, value)| (name.as_str(), value.as_str()))
        .collect();
    assert_eq!(pairs[0], ("latitude", "52.5200"));
    assert_eq!(pairs[1], ("longitude", "13.4050"));
    assert_eq!(pairs[2], ("start_date", "2026-09-14"));
    assert_eq!(pairs[3], ("end_date", "2026-09-14"));
    assert_eq!(
        pairs[4],
        (
            "hourly",
            "temperature_2m,apparent_temperature,precipitation,weather_code,wind_speed_10m,\
wind_direction_10m,relative_humidity_2m,visibility"
        )
    );
    assert_eq!(
        pairs[5],
        (
            "daily",
            "weather_code,temperature_2m_max,temperature_2m_min,sunrise,sunset"
        )
    );
    assert_eq!(
        &pairs[6..],
        [
            ("timezone", "auto"),
            ("temperature_unit", "celsius"),
            ("wind_speed_unit", "kmh"),
            ("precipitation_unit", "mm"),
        ]
    );
    assert!(
        !pairs.iter().any(|(name, _)| *name == "current"),
        "the archive endpoint answers no current block"
    );
    assert!(
        !pairs.iter().any(|(name, _)| *name == "forecast_days"),
        "a window replaces the day count"
    );
}

#[test]
fn the_fixture_decodes_into_one_day_with_four_parts() {
    let run = run((2026, 10, 6), CacheMode::Normal);
    let report = fetch(&run, recorded_day()).expect("the fixture parses");

    assert_eq!(report.days.len(), 1);
    let day = &report.days[0];
    assert_eq!(day.date, date(2026, 9, 14));

    // The daily block carries the extremes and the sun times; the parts come from the hours.
    assert_eq!(day.temp_min_c, 14.2);
    assert_eq!(day.temp_max_c, 18.8);
    assert_eq!(
        day.sunrise.expect("a sunrise").to_string(),
        "2026-09-14 06:38:00 +02:00"
    );
    assert_eq!(
        day.sunset.expect("a sunset").to_string(),
        "2026-09-14 19:24:00 +02:00"
    );

    // Representative samples: the hour closest to each part's midpoint (09:00/15:00/21:00/03:00).
    let morning = part(day, DayPartKind::Morning);
    assert_eq!(morning.temp_c, 14.7);
    assert_eq!(morning.feels_like_c, Some(13.6));
    assert_eq!(morning.precip_mm, 0.0);
    assert_eq!(morning.wind_kmh, 11.3);
    assert_eq!(morning.wind_dir_deg, Some(290));
    assert_eq!(morning.humidity_pct, Some(83));

    let noon = part(day, DayPartKind::Noon);
    assert_eq!(noon.temp_c, 18.6);
    assert_eq!(noon.feels_like_c, Some(16.5));

    let evening = part(day, DayPartKind::Evening);
    assert_eq!(evening.temp_c, 15.4);
    assert_eq!(evening.wind_kmh, 0.8);

    let night = part(day, DayPartKind::Night);
    assert_eq!(night.temp_c, 15.7, "03:00 is the night's midpoint");
    assert_eq!(night.humidity_pct, Some(96));

    // Precipitation is the part sum: two drizzle hours carry 0.1 mm each in the night.
    assert!((night.precip_mm - 0.2).abs() < 1e-6);
    assert_eq!(morning.precip_mm, 0.0);

    // The condition is the most severe code of the part: light drizzle (rank 6) beats the
    // overcast hours in the night, overcast (rank 3) beats partly cloudy in the morning and the
    // noon, and the evening is mainly clear throughout.
    assert_eq!(night.weather, Condition::from_u8(51));
    assert_eq!(morning.weather, Condition::from_u8(3));
    assert_eq!(noon.weather, Condition::from_u8(3));
    assert_eq!(evening.weather, Condition::from_u8(1));

    // The reanalysis has no probability of precipitation and answers no visibility for this point;
    // both stay absent rather than becoming a zero.
    for kind in DayPartKind::ALL {
        let part = part(day, kind);
        assert_eq!(part.precip_prob_pct, None, "{kind:?}");
        assert_eq!(part.visibility_km, None, "{kind:?}");
    }
}

#[test]
fn the_report_is_an_archive_report_with_no_current_block() {
    let run = run((2026, 10, 6), CacheMode::Normal);
    let report = fetch(&run, recorded_day()).expect("the fixture parses");

    assert_eq!(report.mode, ReportMode::Archive);
    assert!(
        report.current.is_none(),
        "the archive endpoint answers no current block"
    );
    assert_eq!(report.attribution.provider, "open-meteo-archive");
    assert_eq!(report.attribution.display_name, "Open-Meteo Archive");
    assert_eq!(report.attribution.url, run.calls()[0].full_url());
    assert_eq!(report.attribution.raw, None, "raw pairs are a -v detail");
    let licence = report
        .attribution
        .licence
        .as_deref()
        .expect("the registry row carries the archive's credit");
    assert!(licence.contains("CC BY 4.0"), "{licence}");
    assert!(licence.contains("Open-Meteo.com"), "{licence}");
}

#[test]
fn a_verbose_run_carries_the_recorded_time_and_code_pairs() {
    let run = run((2026, 10, 6), CacheMode::Normal);
    let mut env = run.env();
    env.verbose = 1;
    let report = OpenMeteoArchive
        .fetch(
            &fixture_location("berlin"),
            &FetchRequest::for_window(recorded_day(), HourlyResolution::Hourly),
            &env,
        )
        .expect("the fixture parses");

    let raw = report
        .attribution
        .raw
        .expect("a verbose run keeps the raw pairs");
    assert!(
        raw.contains("\"2026-09-14T03:00\",51]"),
        "the drizzle hour's code is 51: {raw}"
    );
}

#[test]
fn the_cache_entry_is_keyed_by_the_provider_and_the_window_end() {
    let run = run((2026, 10, 6), CacheMode::Normal);
    fetch(&run, recorded_day()).expect("the fixture parses");
    fetch(&run, recorded_day()).expect("the cached body parses");

    assert_eq!(run.calls().len(), 1, "the second fetch hits the cache");
    let key = CacheKey::weather("open-meteo-archive", 52.52, 13.405, 1, date(2026, 9, 14));
    assert_eq!(
        key.path().to_string_lossy(),
        "weather/open-meteo-archive-52.52-13.40-1-2026-09-14.json",
        "the key carries the provider, the window's length and the window's end"
    );
    assert!(
        run.cache().entry_path(&key).exists(),
        "{} must be cached",
        key.path().display()
    );
}

#[test]
fn the_earliest_reanalysis_date_is_accepted() {
    // The recorded body, re-dated: the request shape and the acceptance of 1940-01-01 are what the
    // test checks, and re-dating keeps the answer coherent for the decoder.
    let body = fixture("archive/berlin-2026-09-14.json").replace("2026-09-14", "1940-01-01");
    let run = ProviderRun::new(
        vec![StubReply::ok(200, body)],
        provider_clock(2026, 10, 6),
        CacheMode::Normal,
    );
    let report = fetch(&run, DateWindow::day(date(1940, 1, 1))).expect("1940-01-01 is served");

    let calls = run.calls();
    let names: Vec<(&str, &str)> = calls[0]
        .query_pairs()
        .iter()
        .map(|(name, value)| (name.as_str(), value.as_str()))
        .collect();
    assert_eq!(names[2], ("start_date", "1940-01-01"));
    assert_eq!(names[3], ("end_date", "1940-01-01"));
    assert_eq!(report.days.len(), 1);
    assert_eq!(report.days[0].date, date(1940, 1, 1));
}

#[test]
fn a_date_before_1940_is_refused() {
    let run = run((2026, 10, 6), CacheMode::Normal);
    let error = fetch(&run, DateWindow::day(date(1939, 12, 31)))
        .expect_err("the reanalysis starts in 1940");

    assert_eq!(error.exit_code(), 2);
    let text = error.to_string();
    assert!(text.contains("1940-01-01"), "{text}");
    assert!(text.contains("1939-12-31"), "{text}");
    assert_eq!(
        run.calls(),
        Vec::<cirrocast::http::HttpRequest>::new(),
        "a refused window never reaches the transport"
    );
}

#[test]
fn a_window_that_reaches_today_is_refused() {
    let run = run((2026, 10, 6), CacheMode::Normal);
    let error = fetch(&run, DateWindow::day(date(2026, 10, 6))).expect_err("today is not history");

    assert_eq!(error.exit_code(), 2);
    let text = error.to_string();
    assert!(text.contains("archive only"), "{text}");
    assert!(text.contains("2026-10-06"), "{text}");
    assert_eq!(run.calls(), Vec::<cirrocast::http::HttpRequest>::new());
}

#[test]
fn a_window_inside_the_latency_band_is_served_by_the_forecast_api() {
    // 2026-09-16 is the location-local today, so `today - 5` is 2026-09-11 and the window's end
    // (2026-09-14) is inside ERA5's latency band: the archive cannot answer, and the forecast API
    // does. The clock is what makes the seam deterministic.
    let run = run((2026, 9, 16), CacheMode::Normal);
    let report = fetch(&run, recorded_day()).expect("the forecast API answers");

    let calls = run.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0].url(),
        open_meteo::BASE,
        "the recent window goes to the forecast host, not the archive host"
    );
    let pairs: Vec<(&str, &str)> = calls[0]
        .query_pairs()
        .iter()
        .map(|(name, value)| (name.as_str(), value.as_str()))
        .collect();
    assert_eq!(pairs[2], ("hourly", open_meteo_hourly()));
    assert_eq!(pairs[3], ("daily", open_meteo_daily()));
    assert_eq!(pairs[4], ("start_date", "2026-09-14"));
    assert_eq!(pairs[5], ("end_date", "2026-09-14"));
    assert!(
        !pairs.iter().any(|(name, _)| *name == "forecast_days"),
        "the window is an explicit span"
    );
    assert!(
        !pairs.iter().any(|(name, _)| *name == "current"),
        "a window that has already happened has no `now`"
    );

    // The delegated answer decodes through the shared path, and the window is still an archive
    // report: its end (the 14th) is in the past, so `--date` keeps the dated archive header.
    assert_eq!(report.mode, ReportMode::Archive);
    assert!(report.current.is_none());
    assert_eq!(report.days.len(), 1);
    assert_eq!(report.days[0].date, date(2026, 9, 14));
    assert_eq!(report.attribution.provider, "open-meteo");
    assert_eq!(
        report.location.tz,
        chrono_tz::Tz::Europe__Berlin,
        "the decoded answer keeps the geocoded zone"
    );

    // A window fetch is keyed under the forecast provider by the span it covered, so it can never
    // be mistaken for a forecast keyed on the day the run happened to fetch it.
    let window_key = CacheKey::weather("open-meteo", 52.52, 13.405, 1, date(2026, 9, 14));
    assert!(run.cache().entry_path(&window_key).exists());
    let plain_key = CacheKey::weather("open-meteo", 52.52, 13.405, 1, date(2026, 9, 16));
    assert!(
        !run.cache().entry_path(&plain_key).exists(),
        "a window is not cached as a plain forecast for the local today"
    );
}

/// The forecast endpoint's hourly parameter list, as the shared request builds it.
fn open_meteo_hourly() -> &'static str {
    "temperature_2m,apparent_temperature,precipitation_probability,precipitation,weather_code,\
wind_speed_10m,wind_direction_10m,relative_humidity_2m,visibility"
}

/// The forecast endpoint's daily parameter list, shared with the archive.
fn open_meteo_daily() -> &'static str {
    "weather_code,temperature_2m_max,temperature_2m_min,sunrise,sunset"
}

#[test]
fn a_request_without_a_window_is_a_usage_error() {
    let run = run((2026, 10, 6), CacheMode::Normal);
    let error = OpenMeteoArchive
        .fetch(
            &fixture_location("berlin"),
            &FetchRequest::new(3, HourlyResolution::Hourly),
            &run.env(),
        )
        .expect_err("the archive is not a forecast backend");

    assert_eq!(error.exit_code(), 2);
    let text = error.to_string();
    assert!(text.contains("archive only"), "{text}");
    assert!(text.contains("--date <YYYY-MM-DD>"), "{text}");
    assert!(text.contains("--history <N>d"), "{text}");
    assert_eq!(run.calls(), Vec::<cirrocast::http::HttpRequest>::new());
}

#[test]
fn a_window_ending_in_the_latency_band_but_starting_long_ago_still_delegates() {
    // A multi-day window whose tail is recent: the whole span is asked of the forecast API, which
    // is the one service that has every day in it.
    let run = run((2026, 9, 16), CacheMode::Normal);
    let window = DateWindow {
        start: date(2026, 9, 10),
        end: date(2026, 9, 14),
    };
    fetch(&run, window).expect("the forecast API answers");

    let calls = run.calls();
    let pairs: Vec<(&str, &str)> = calls[0]
        .query_pairs()
        .iter()
        .map(|(name, value)| (name.as_str(), value.as_str()))
        .collect();
    assert_eq!(pairs[4], ("start_date", "2026-09-10"));
    assert_eq!(pairs[5], ("end_date", "2026-09-14"));
    let key = CacheKey::weather("open-meteo", 52.52, 13.405, 5, date(2026, 9, 14));
    assert!(run.cache().entry_path(&key).exists());
}

#[test]
fn a_second_call_is_served_from_the_cache() {
    let run = run((2026, 10, 6), CacheMode::Normal);
    let first = fetch(&run, recorded_day()).expect("the fixture parses");
    let second = fetch(&run, recorded_day()).expect("the cached body parses");

    assert_eq!(
        run.calls().len(),
        1,
        "the window is cached under its own end date"
    );
    assert_eq!(first.days, second.days);
    assert_eq!(first.location, second.location);
}
