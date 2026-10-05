// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The MET Norway backend, driven by the recorded `compact` payloads in `tests/fixtures/met-no/`.
//!
//! No test here opens a socket: `StubTransport` answers from the fixtures and `FakeClock` supplies
//! both the fetch time and the `local_today` the cache key is built from.
//!
//! `oslo.json` is the recording verbatim. `polar-twilight.json` is **hand-trimmed** from that same
//! recording (see its `_provenance` field): the record's shape is preserved, but a
//! `*_polartwilight` symbol and a coarse trailing step replace the original values.

// Exact comparison is the point in this file: every value comes from the recorded JSON.
#![allow(clippy::float_cmp)]

mod common;

use std::fs;

use chrono::{NaiveDate, TimeZone as _, Utc};

use cirrocast::cache::{CACHE_SCHEMA_VERSION, CacheEntry, CacheKey, CacheMode};
use cirrocast::geo::from_coordinates;
use cirrocast::http::StubReply;
use cirrocast::model::{Condition, DayForecast, DayPart, DayPartKind};
use cirrocast::provider::met_no::{BASE, CompactResponse, MetNo, SYMBOLS};
use common::{ProviderRun, fixture, fixture_location, fixture_reply, provider_clock};

/// A run over one met-no fixture, at 06:00 UTC on `day`.
fn run(file: &str, day: (i32, u32, u32), mode: CacheMode) -> ProviderRun {
    ProviderRun::new(
        vec![fixture_reply("met-no", file)],
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

/// The request's query pairs, as borrowed strings.
fn query(call: &cirrocast::http::HttpRequest) -> Vec<(&str, &str)> {
    call.query_pairs()
        .iter()
        .map(|(name, value)| (name.as_str(), value.as_str()))
        .collect()
}

#[test]
fn the_request_carries_truncated_four_decimal_coordinates() {
    let run = run("oslo.json", (2026, 10, 5), CacheMode::Normal);
    let mut loc = fixture_location("oslo");
    // More than four decimals: truncation, not rounding, must produce these.
    loc.lat = 59.913_99;
    loc.lon = 10.752_29;
    run.fetch_with(&MetNo, &loc, 1).expect("the fixture parses");

    let calls = run.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].url(), BASE);
    assert_eq!(
        query(&calls[0]),
        [("lat", "59.9139"), ("lon", "10.7522")],
        "the point goes out at four decimals"
    );
}

#[test]
fn altitude_is_sent_only_when_the_location_has_an_elevation() {
    let mut loc = fixture_location("oslo");
    loc.elevation_m = Some(5.0);
    let run = run("oslo.json", (2026, 10, 5), CacheMode::Normal);
    run.fetch_with(&MetNo, &loc, 1).expect("the fixture parses");
    assert_eq!(
        query(&run.calls()[0]),
        [("lat", "59.9139"), ("lon", "10.7522"), ("altitude", "5")]
    );
}

#[test]
fn the_oslo_fixture_decodes_to_nine_days_and_a_current_block() {
    let raw: CompactResponse =
        serde_json::from_str(&fixture("met-no/oslo.json")).expect("the fixture parses");
    assert_eq!(
        raw.properties.timeseries.len(),
        84,
        "the recorded step count"
    );
    assert_eq!(
        raw.properties.meta.units.wind_speed.as_deref(),
        Some("m/s"),
        "the payload declares m/s"
    );

    let run = run("oslo.json", (2026, 10, 5), CacheMode::Normal);
    let report = run
        .fetch_with(&MetNo, &fixture_location("oslo"), 9)
        .expect("the fixture parses");

    let dates: Vec<NaiveDate> = report.days.iter().map(|day| day.date).collect();
    assert_eq!(
        dates,
        vec![
            date(2026, 10, 6),
            date(2026, 10, 7),
            date(2026, 10, 8),
            date(2026, 10, 9),
            date(2026, 10, 10),
            date(2026, 10, 11),
            date(2026, 10, 12),
            date(2026, 10, 13),
            date(2026, 10, 14),
        ]
    );

    let current = report
        .current
        .expect("the first step builds a current block");
    assert_eq!(
        current.observed_at.to_rfc3339(),
        "2026-10-05T22:00:00+02:00"
    );
    assert_eq!(current.temp_c, 15.5);
    assert_eq!(current.humidity_pct, Some(60));
    assert_eq!(current.pressure_hpa, 1002.3);
    assert_eq!(current.cloud_cover_pct, Some(0));
    assert_eq!(current.weather, Condition::from_u8(0));
    assert_eq!(current.precip_mm, 0.0);
    assert_eq!(current.wind_dir_deg, Some(231));
    assert!(!current.is_day, "22:00 local is outside the civil day");
    // m/s → km/h: 5.6 m/s.
    assert_eq!(current.wind_kmh, 5.6_f32 * 3.6);
    // The compact payload carries no apparent temperature, gust, visibility or UV index.
    assert!(current.feels_like_c.is_none());
    assert!(current.wind_gust_kmh.is_none());
    assert!(current.visibility_km.is_none());
    assert!(current.uv_index.is_none());

    // The first day's extremes come from its own samples (there is no daily block).
    let first = &report.days[0];
    assert_eq!(first.temp_min_c, 9.4);
    assert_eq!(first.temp_max_c, 17.7);
    assert!(first.sunrise.is_none() && first.sunset.is_none());
    // 09:00 local is the morning's midpoint and the payload's 6.9 m/s converts to km/h.
    assert_eq!(part(first, DayPartKind::Morning).temp_c, 10.7);
    assert_eq!(part(first, DayPartKind::Morning).wind_kmh, 6.9_f32 * 3.6);
}

#[test]
fn a_rainy_day_aggregates_precipitation_and_the_dominant_condition() {
    let run = run("oslo.json", (2026, 10, 5), CacheMode::Normal);
    let report = run
        .fetch_with(&MetNo, &fixture_location("oslo"), 9)
        .expect("the fixture parses");
    let day = &report.days[2];
    assert_eq!(day.date, date(2026, 10, 8));
    assert_eq!(day.temp_min_c, 7.9);
    assert_eq!(day.temp_max_c, 10.2);

    let morning = part(day, DayPartKind::Morning);
    assert_eq!(morning.temp_c, 7.9);
    assert_eq!(morning.precip_mm, 1.7);
    assert_eq!(morning.weather, Condition::from_u8(63), "`rain` is WMO 63");

    let noon = part(day, DayPartKind::Noon);
    assert_eq!(noon.temp_c, 8.1);
    assert_eq!(noon.precip_mm, 1.4);
    assert_eq!(noon.weather, Condition::from_u8(3));

    let evening = part(day, DayPartKind::Evening);
    assert_eq!(evening.precip_mm, 0.0);
    assert_eq!(
        evening.weather,
        Condition::from_u8(2),
        "`partlycloudy_night` maps to WMO 2"
    );

    // The night mixes overcast and rain hours; rain (rank 12) dominates overcast (rank 3).
    let night = part(day, DayPartKind::Night);
    assert_eq!(night.temp_c, 10.0);
    assert_eq!(night.precip_mm, 1.9);
    assert_eq!(night.weather, Condition::from_u8(63));
}

#[test]
fn the_polar_twilight_fixture_strips_the_suffix_and_reads_a_coarse_tail() {
    let run = run("polar-twilight.json", (2026, 10, 6), CacheMode::Normal);
    // Raw coordinates stay in their provisional UTC, which the fixture's rows are already in.
    let loc = from_coordinates(78.2232, 15.6469);
    let report = run.fetch_with(&MetNo, &loc, 2).expect("the fixture parses");

    assert_eq!(report.days.len(), 2);
    assert_eq!(report.days[0].date, date(2026, 10, 6));
    assert_eq!(report.days[1].date, date(2026, 10, 7));
    for day in &report.days {
        for kind in DayPartKind::ALL {
            assert_eq!(
                part(day, kind).weather,
                Condition::from_u8(2),
                "`partlycloudy_polartwilight` must strip to `partlycloudy`"
            );
        }
    }
    // The trailing day is served 6-hourly (`next_6_hours` only); its accumulation still lands in
    // the part that contains the step.
    assert_eq!(part(&report.days[1], DayPartKind::Evening).precip_mm, 1.2);
}

#[test]
fn an_unknown_symbol_becomes_overcast() {
    fn step(time: &str) -> serde_json::Value {
        serde_json::json!({
            "time": time,
            "data": {
                "instant": {"details": {
                    "air_pressure_at_sea_level": 1010.0,
                    "air_temperature": 10.0,
                    "cloud_area_fraction": 50.0,
                    "relative_humidity": 50.0,
                    "wind_from_direction": 180.0,
                    "wind_speed": 2.0
                }},
                "next_1_hours": {
                    "summary": {"symbol_code": "unknownthing_day"},
                    "details": {"precipitation_amount": 0.0}
                }
            }
        })
    }

    let body = serde_json::json!({
        "properties": {
            "meta": {
                "updated_at": "2026-10-06T00:00:00Z",
                "units": {"wind_speed": "m/s"}
            },
            "timeseries": [
                step("2026-10-06T00:00:00Z"),
                step("2026-10-06T06:00:00Z"),
                step("2026-10-06T12:00:00Z"),
                step("2026-10-06T18:00:00Z"),
            ]
        }
    })
    .to_string();

    let run = ProviderRun::new(
        vec![StubReply::ok(200, body)],
        provider_clock(2026, 10, 6),
        CacheMode::Normal,
    );
    let report = run
        .fetch_with(&MetNo, &from_coordinates(0.0, 0.0), 1)
        .expect("an unknown symbol is not an error");
    assert_eq!(
        part(&report.days[0], DayPartKind::Morning).weather,
        Condition::from_u8(3)
    );
}

#[test]
fn the_symbol_table_lists_every_official_base_code() {
    assert_eq!(SYMBOLS.len(), 41, "the official weathericon/2.0 base list");
    let names: Vec<&str> = SYMBOLS.iter().map(|(name, _)| *name).collect();
    let mut sorted = names.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), names.len(), "no duplicate base code");

    let code = |name: &str| {
        SYMBOLS
            .iter()
            .find(|(candidate, _)| *candidate == name)
            .map(|(_, code)| *code)
    };
    // The step file's sleet split, including the canonical WMO 68/69 pair.
    assert_eq!(code("lightsleetshowers"), Some(68));
    assert_eq!(code("sleetshowers"), Some(68));
    assert_eq!(code("lightsleet"), Some(68));
    assert_eq!(code("sleet"), Some(68));
    assert_eq!(code("heavysleetshowers"), Some(69));
    assert_eq!(code("heavysleet"), Some(69));
    // The provider's own double-`s` typo is reproduced, not corrected.
    assert_eq!(code("lightssleetshowersandthunder"), Some(95));
    assert_eq!(code("lightssnowshowersandthunder"), Some(95));
    // Every `…andthunder` form is the thunderstorm code.
    let thunder: Vec<(&str, u8)> = SYMBOLS
        .iter()
        .copied()
        .filter(|(name, _)| name.ends_with("andthunder"))
        .collect();
    assert_eq!(thunder.len(), 18);
    for (name, code) in thunder {
        assert_eq!(code, 95, "{name}");
    }
}

#[test]
fn a_not_modified_answer_serves_the_stored_body() {
    let last_modified = "Mon, 05 Oct 2026 20:48:38 GMT";
    let run = ProviderRun::new(
        vec![StubReply::status(
            304,
            vec![("Last-Modified".to_owned(), last_modified.to_owned())],
            "",
        )],
        provider_clock(2026, 10, 5),
        CacheMode::Normal,
    );
    let loc = fixture_location("oslo");
    let key = CacheKey::weather("met-no", loc.lat, loc.lon, 9, date(2026, 10, 5));

    // A stored entry that is past its TTL, with the `Last-Modified` the next request echoes.
    let entry = CacheEntry {
        cache_schema_version: CACHE_SCHEMA_VERSION,
        key: key.normalised().to_owned(),
        fetched_at: Utc
            .with_ymd_and_hms(2026, 10, 4, 6, 0, 0)
            .single()
            .expect("a valid instant"),
        ttl_secs: 600,
        status: 200,
        body: fixture("met-no/oslo.json"),
        last_modified: Some(last_modified.to_owned()),
        expires_at: None,
    };
    let path = run.cache().entry_path(&key);
    fs::create_dir_all(path.parent().expect("a parent directory")).expect("the cache directory");
    fs::write(
        &path,
        serde_json::to_string(&entry).expect("the entry encodes"),
    )
    .expect("the entry is written");

    let report = run
        .fetch_with(&MetNo, &loc, 9)
        .expect("a 304 serves the stored body without decoding an empty one");
    assert_eq!(report.days.len(), 9);

    let calls = run.calls();
    assert_eq!(calls.len(), 1, "one conditional revalidation");
    let conditional = calls[0]
        .headers()
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("if-modified-since"));
    assert_eq!(
        conditional.map(|(_, value)| value.as_str()),
        Some(last_modified),
        "the stored Last-Modified must be echoed as If-Modified-Since"
    );
}

#[test]
fn a_second_fetch_is_served_from_the_cache() {
    let run = run("oslo.json", (2026, 10, 5), CacheMode::Normal);
    run.fetch_with(&MetNo, &fixture_location("oslo"), 3)
        .expect("the fixture parses");
    run.fetch_with(&MetNo, &fixture_location("oslo"), 3)
        .expect("the cached fixture parses");
    assert_eq!(run.calls().len(), 1, "the cache must not refetch");
}
