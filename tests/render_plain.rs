// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The `plain` renderer over a fixture-backed report.
//!
//! The report comes from the real provider over `StubTransport`, so this covers the whole path a
//! user sees — decoding, aggregation, units — and not just the formatting of a hand-built model.

// The expected lines are built from the recorded fixture, so exact comparison is the assertion.
#![allow(clippy::float_cmp)]

mod common;

use chrono::{TimeZone as _, Utc};

use cirrocast::cache::CacheMode;
use cirrocast::config::UnitOverrides;
use cirrocast::i18n::{I18n, LanguageRequest};
use cirrocast::model::units::UnitSystem;
use cirrocast::model::{LocationSource, Report};
use cirrocast::render::{ColorMode, RenderContext, Renderer, TermCaps, plain::Plain};
use common::{ProviderRun, fixture_location};

/// The English catalog, loaded the way the CLI loads an unconfigured run.
fn english() -> I18n {
    I18n::load(&LanguageRequest::Auto, |_| None)
}

/// A report for one fixture, through the real provider.
fn report() -> Report {
    let run = ProviderRun::fixture(
        "forecast_beijing_2026-07-15.json",
        (2026, 7, 15),
        CacheMode::Normal,
    );
    run.fetch(&fixture_location("beijing"), 1)
        .expect("the fixture parses")
}

/// Renders `report` in `units` at the context width the renderer ignores.
fn render(report: &Report, units: UnitSystem) -> String {
    let i18n = english();
    let ctx = RenderContext {
        units: units
            .resolve(&UnitOverrides::default())
            .expect("the default overrides resolve"),
        color: ColorMode::Never,
        width: 80,
        term: TermCaps::default(),
        lang: i18n.lang(),
        i18n: &i18n,
        alert_credits: &[],
        aqi_index: cirrocast::air::aqi::AqiIndex::Us,
        times: cirrocast::model::LocalTimes::new(
            Utc.with_ymd_and_hms(2026, 7, 15, 12, 0, 0)
                .single()
                .expect("a valid instant"),
            report.location.tz,
        ),
    };
    Plain
        .render(report, &ctx)
        .expect("plain always renders a report")
}

#[test]
fn the_metric_output_is_exactly_these_lines() {
    let text = render(&report(), UnitSystem::Metric);
    assert_eq!(
        text,
        "\
location: Beijing, Beijing (39.90, 116.41) Asia/Shanghai
updated: 2026-09-30T19:30:00+08:00
current: Clear sky 18°C (feels 13°C) wind 13km/h NW humidity 11% precip 0.0mm pressure 1021hPa visibility 17km
day 2026-07-15: Morning Clear sky 29°C 0.0mm (0%) wind 2.5km/h N | Noon Clear sky 35°C 0.0mm (0%) wind 4.7km/h SW | Evening Overcast 30°C 0.0mm (0%) wind 13km/h SW | Night Overcast 26°C 0.0mm (0%) wind 6.0km/h SW
Location data based on GeoNames (CC-BY-4.0) via Open-Meteo — https://open-meteo.com/
Data: Open-Meteo.com (CC BY 4.0) — https://open-meteo.com/
attribution: open-meteo https://api.open-meteo.com/v1/forecast"
    );
}

#[test]
fn the_three_unit_systems_differ_only_where_the_model_converts() {
    let report = report();
    let metric = render(&report, UnitSystem::Metric);
    let us = render(&report, UnitSystem::Us);
    let uk = render(&report, UnitSystem::Uk);

    assert!(
        metric.contains("current: Clear sky 18°C (feels 13°C)"),
        "{metric}"
    );
    assert!(us.contains("current: Clear sky 65°F (feels 55°F)"), "{us}");
    assert!(uk.contains("current: Clear sky 18°C (feels 13°C)"), "{uk}");

    assert!(metric.contains("wind 13km/h NW"), "{metric}");
    assert!(us.contains("wind 8.1mph NW"), "{us}");
    assert!(uk.contains("wind 8.1mph NW"), "{uk}");

    assert!(metric.contains("visibility 17km"), "{metric}");
    assert!(us.contains("visibility 11mi"), "{us}");

    assert!(metric.contains("pressure 1021hPa"), "{metric}");
    assert!(us.contains("pressure 30.14inHg"), "{us}");

    assert!(uk.contains("0.0mm"), "{uk}");
    assert!(us.contains("0.00in"), "{us}");

    for text in [&metric, &us, &uk] {
        assert!(
            !text.contains('\u{1b}'),
            "plain never emits an escape sequence"
        );
        assert!(
            text.ends_with("attribution: open-meteo https://api.open-meteo.com/v1/forecast"),
            "{text}"
        );
    }
}

#[test]
fn a_current_only_report_prints_the_header_the_update_and_the_current_line() {
    let run = ProviderRun::fixture(
        "forecast_beijing_2026-07-15.json",
        (2026, 7, 15),
        CacheMode::Normal,
    );
    let report = run
        .fetch(&fixture_location("beijing"), 0)
        .expect("the fixture parses");

    let text = render(&report, UnitSystem::Metric);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 6, "{text}");
    assert!(lines[0].starts_with("location: "), "{text}");
    assert!(lines[1].starts_with("updated: "), "{text}");
    assert!(lines[2].starts_with("current: "), "{text}");
    assert!(!text.contains("\nday "), "{text}");
}

#[test]
fn an_osm_location_prints_the_odbld_line() {
    let mut report = report();
    report.location.source = LocationSource::Osm;
    let text = render(&report, UnitSystem::Metric);
    assert!(
        text.contains("Location data © OpenStreetMap contributors (ODbL)"),
        "{text}"
    );
    assert!(!text.contains("GeoNames"), "{text}");
}

/// A record format: the width a caller asks for never truncates a line, because a truncated line
/// would silently lose the values the format exists to carry.
#[test]
fn the_width_does_not_change_the_output() {
    let report = report();
    let i18n = english();
    let context = |width| RenderContext {
        units: UnitSystem::Metric
            .resolve(&UnitOverrides::default())
            .expect("the default overrides resolve"),
        color: ColorMode::Never,
        width,
        term: TermCaps::default(),
        lang: i18n.lang(),
        i18n: &i18n,
        alert_credits: &[],
        aqi_index: cirrocast::air::aqi::AqiIndex::Us,
        times: cirrocast::model::LocalTimes::new(
            Utc.with_ymd_and_hms(2026, 7, 15, 12, 0, 0)
                .single()
                .expect("a valid instant"),
            report.location.tz,
        ),
    };
    let narrow = Plain
        .render(&report, &context(20))
        .expect("plain always renders a report");
    assert_eq!(narrow, render(&report, UnitSystem::Metric));
    assert!(
        narrow.lines().any(|line| line.chars().count() > 20),
        "the day summary is longer than 20 columns"
    );
}

/// A value the provider did not report is omitted, never printed as a zero.
///
/// The golden above is the complete fixture, so only the `Some` branches run there: a regression
/// printing `visibility 0km` for a missing value would pass it.
#[test]
fn absent_values_are_omitted_not_zeroed() {
    let mut report = report();
    let current = report
        .current
        .as_mut()
        .expect("the fixture reports current conditions");
    current.feels_like_c = None;
    current.visibility_km = None;
    for day in &mut report.days {
        for part in &mut day.parts {
            part.feels_like_c = None;
            part.visibility_km = None;
            part.precip_prob_pct = None;
            part.wind_dir_deg = None;
        }
    }

    let text = render(&report, UnitSystem::Metric);
    assert!(
        text.contains(
            "current: Clear sky 18°C wind 13km/h NW humidity 11% precip 0.0mm pressure 1021hPa\n"
        ),
        "{text}"
    );
    assert!(
        !text.contains("feels"),
        "an unreported apparent temperature is omitted, not invented:\n{text}"
    );
    assert!(
        !text.contains("visibility"),
        "an unreported visibility is omitted, not a zero:\n{text}"
    );
    assert!(
        !text.contains("(0%)"),
        "an unreported probability is omitted:\n{text}"
    );
    assert!(
        text.contains("Morning Clear sky 29°C 0.0mm wind 2.5km/h |"),
        "the part keeps the values it has:\n{text}"
    );
}
