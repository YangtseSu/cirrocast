// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The Bright Sky backend, driven by the recorded DWD payloads in `tests/fixtures/brightsky/`.
//!
//! No test here opens a socket: `StubTransport` answers from the fixtures and `FakeClock` supplies
//! both the fetch time and the `local_today` the cache key is built from.
//!
//! `berlin-3day.json` is the recording verbatim (`lat=52.52&lon=13.41&date=2026-10-06` over the
//! request's own span, `tz=UTC`, recorded 2026-10-06 with the app's `User-Agent`).
//! `edge-nulls.json` is **hand-authored** from the same schema (its `_provenance` field says so)
//! to exercise `null` readings and the unknown condition/icon fallback.

// Exact comparison is the point in this file: every value comes from the recorded JSON.
#![allow(clippy::float_cmp)]

mod common;

use chrono::{NaiveDate, Timelike};
use chrono_tz::Tz;

use cirrocast::cache::CacheMode;
use cirrocast::geo::from_coordinates;
use cirrocast::model::{Condition, DayForecast, DayPart, DayPartKind, Location};
use cirrocast::provider::brightsky::{
    BASE, BrightSky, CONDITIONS, ICONS, WeatherResponse, condition_of, raw_summary, samples,
};
use common::{ProviderRun, fixture, fixture_location, fixture_reply, provider_clock};

/// A run over one brightsky fixture, at 06:00 UTC on `day`.
fn run(file: &str, day: (i32, u32, u32)) -> ProviderRun {
    ProviderRun::new(
        vec![fixture_reply("brightsky", file)],
        provider_clock(day.0, day.1, day.2),
        CacheMode::Normal,
    )
}

/// The Berlin location the fixture was recorded for (`Europe/Berlin`, `DE`).
fn berlin() -> Location {
    let mut loc = fixture_location("berlin");
    loc.lon = 13.41;
    "Germany".clone_into(&mut loc.country);
    loc.country_code = Some("DE".to_owned());
    loc
}

/// The four parts of `day`, in `DayPartKind::ALL` order.
fn part(day: &DayForecast, kind: DayPartKind) -> &DayPart {
    &day.parts[kind.index()]
}

/// `YYYY-MM-DD` as a date.
fn date(year: i32, month: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(year, month, day).expect("a valid date")
}

/// The request's query pairs, as borrowed strings.
fn query(call: &cirrocast::http::HttpRequest) -> Vec<(&str, &str)> {
    call.query_pairs()
        .iter()
        .map(|(name, value)| (name.as_str(), value.as_str()))
        .collect()
}

/// The parsed edge fixture.
fn edge() -> WeatherResponse {
    serde_json::from_str(&fixture("brightsky/edge-nulls.json")).expect("the fixture parses")
}

#[test]
fn the_request_carries_the_local_today_rounded_coordinates_and_the_end_of_the_span() {
    let run = run("berlin-3day.json", (2026, 10, 6));
    let mut loc = berlin();
    // More than four decimals: rounding, not truncation, must produce these.
    loc.lat = 52.519_99;
    loc.lon = 13.414_99;
    run.fetch_with(&BrightSky, &loc, 3)
        .expect("the fixture parses");

    let calls = run.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].url(), BASE);
    assert_eq!(
        query(&calls[0]),
        [
            ("lat", "52.5200"),
            ("lon", "13.4150"),
            ("date", "2026-10-06"),
            // `last_date` is an inclusive instant, so the day *after* the last requested one is
            // what serves exactly three whole days (measured: see the provider module note).
            ("last_date", "2026-10-09"),
            ("tz", "UTC"),
        ]
    );
}

#[test]
fn three_days_decode_into_three_days_of_four_parts_each() {
    let run = run("berlin-3day.json", (2026, 10, 6));
    let report = run
        .fetch_with(&BrightSky, &berlin(), 3)
        .expect("the fixture parses");

    let dates: Vec<NaiveDate> = report.days.iter().map(|day| day.date).collect();
    assert_eq!(
        dates,
        vec![date(2026, 10, 6), date(2026, 10, 7), date(2026, 10, 8)]
    );
    for day in &report.days {
        assert_eq!(day.parts.len(), 4, "{} must have four parts", day.date);
        for kind in DayPartKind::ALL {
            // A missing part would have failed the aggregation in `covered_days`.
            assert!(day.parts[kind.index()].temp_c.is_finite());
        }
    }

    // The first day's extremes come from its own samples (there is no daily block).
    let first = &report.days[0];
    assert_eq!(first.temp_min_c, 9.2);
    assert_eq!(first.temp_max_c, 19.5);
    assert_eq!(part(first, DayPartKind::Morning).temp_c, 11.3);
    assert_eq!(part(first, DayPartKind::Morning).wind_kmh, 9.3);
    assert_eq!(part(first, DayPartKind::Morning).wind_dir_deg, Some(241));
    assert!(first.sunrise.is_none() && first.sunset.is_none());
}

#[test]
fn a_rainy_day_aggregates_precipitation_and_the_dominant_condition() {
    let run = run("berlin-3day.json", (2026, 10, 6));
    let report = run
        .fetch_with(&BrightSky, &berlin(), 3)
        .expect("the fixture parses");
    let day = &report.days[2];
    assert_eq!(day.date, date(2026, 10, 8));
    assert_eq!(day.temp_min_c, 9.9);
    assert_eq!(day.temp_max_c, 15.3);

    let morning = part(day, DayPartKind::Morning);
    assert_eq!(morning.weather, Condition::from_u8(63), "`rain` is WMO 63");
    assert!(
        (morning.precip_mm - 2.1).abs() < 0.01,
        "{}",
        morning.precip_mm
    );

    let noon = part(day, DayPartKind::Noon);
    assert_eq!(noon.weather, Condition::from_u8(63));
    assert!((noon.precip_mm - 1.3).abs() < 0.01, "{}", noon.precip_mm);

    // The night mixes `partly-cloudy-night` (WMO 2) with one `cloudy` hour; overcast outranks
    // partly cloudy, so the part does not inherit the day's rain.
    let night = part(day, DayPartKind::Night);
    assert_eq!(night.weather, Condition::from_u8(3));
    assert_eq!(night.precip_mm, 0.0);
}

#[test]
fn the_condition_and_icon_tables_are_total_and_exhaustive() {
    // Every value the API documents maps to the table's code, and nothing in the tables is missing.
    for (name, code) in CONDITIONS {
        assert_eq!(
            condition_of(Some(name), None, false).map(Condition::code),
            Some(code),
            "condition `{name}`"
        );
    }
    for (name, code) in ICONS {
        assert_eq!(
            condition_of(None, Some(name), false).map(Condition::code),
            Some(code),
            "icon `{name}`"
        );
    }
    assert_eq!(CONDITIONS.len(), 7);
    assert_eq!(ICONS.len(), 12);

    // The precedence: a non-dry condition wins, `dry` defers to the icon, an unknown value is WMO 3.
    assert_eq!(
        condition_of(Some("rain"), Some("clear-day"), false),
        Some(Condition::from_u8(63))
    );
    assert_eq!(
        condition_of(Some("dry"), Some("cloudy"), false),
        Some(Condition::from_u8(3))
    );
    assert_eq!(
        condition_of(Some("hailstorm"), Some("hail"), false),
        Some(Condition::from_u8(3)),
        "an unknown condition is overcast, not a guessed neighbour"
    );
    assert_eq!(
        condition_of(Some("dry"), Some("sunny"), false),
        Some(Condition::from_u8(3)),
        "an unknown icon is overcast too"
    );
}

#[test]
fn null_readings_stay_null_and_never_become_zero() {
    let response = edge();
    let decoded = samples(&response, Tz::UTC, false).expect("the fixture parses");

    // 24 rows, one of them (11:00, `precipitation: null`) dropped: a hole must not become 0 mm.
    assert_eq!(decoded.len(), 23);
    assert!(
        !decoded.iter().any(|sample| sample.at.hour() == 11),
        "the null-precipitation hour is dropped, not zero-filled"
    );
    assert!(decoded.iter().all(|sample| sample.precip_mm >= 0.0));

    let at = |hour: u32| {
        decoded
            .iter()
            .find(|sample| sample.at.hour() == hour)
            .unwrap_or_else(|| panic!("no sample at {hour}:00"))
    };
    // A `null` humidity and visibility keep their own nullness (`None`, never `Some(0)`).
    assert_eq!(at(10).humidity_pct, None);
    assert_eq!(at(12).visibility_km, None);
    // The unknown icon (13:00) and the unknown condition (14:00) both fall back to overcast.
    assert_eq!(at(13).weather, Condition::from_u8(3));
    assert_eq!(at(14).weather, Condition::from_u8(3));

    // The report built from the same fixture agrees: a `null` humidity must never surface as 0%.
    let run = ProviderRun::new(
        vec![fixture_reply("brightsky", "edge-nulls.json")],
        provider_clock(2026, 10, 6),
        CacheMode::Normal,
    );
    let report = run
        .fetch_with(&BrightSky, &from_coordinates(52.52, 13.41), 1)
        .expect("the fixture parses");
    assert_eq!(report.days.len(), 1);
    assert_eq!(report.days[0].date, date(2026, 10, 6));
    for kind in DayPartKind::ALL {
        assert_ne!(
            part(&report.days[0], kind).humidity_pct,
            Some(0),
            "a null humidity is not 0%"
        );
    }
}

#[test]
fn the_horizon_is_clamped_to_the_measured_ten_days() {
    let run = run("berlin-3day.json", (2026, 10, 6));
    // 30 requested, 10 served: the request ends at `today + 10`, the last whole day under the
    // forecast feed's measured `last_record`.
    run.fetch_with(&BrightSky, &berlin(), 30)
        .expect("the fixture parses");
    let calls = run.calls();
    let q = query(&calls[0]);
    assert_eq!(q[2], ("date", "2026-10-06"));
    assert_eq!(q[3], ("last_date", "2026-10-16"));

    let report = run
        .fetch_with(&BrightSky, &berlin(), 30)
        .expect("second fetch is cached");
    // The recorded fixture only covers three days, so the report is short and says so.
    assert_eq!(report.days.len(), 3);
}

#[test]
fn the_station_block_reaches_the_verbose_summary() {
    let response: WeatherResponse =
        serde_json::from_str(&fixture("brightsky/berlin-3day.json")).expect("the fixture parses");
    let summary = raw_summary(&response);
    assert!(summary.contains("BERLIN-ALEX."), "{summary}");
    assert!(summary.contains("#2382"), "{summary}");
    assert!(summary.contains("DWD 00399"), "{summary}");
    assert!(summary.contains("WMO 10389"), "{summary}");
    assert!(summary.contains("677 m"), "{summary}");
    assert!(summary.contains("rows 73"), "{summary}");
    assert!(
        summary.contains("2026-10-06T00:00:00+00:00"),
        "the span is named: {summary}"
    );
}

#[test]
fn a_second_fetch_is_served_from_the_cache() {
    let run = run("berlin-3day.json", (2026, 10, 6));
    run.fetch_with(&BrightSky, &berlin(), 3)
        .expect("the fixture parses");
    run.fetch_with(&BrightSky, &berlin(), 3)
        .expect("the cached body parses");
    assert_eq!(run.calls().len(), 1, "the second run hits the cache");
}
