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
use cirrocast::i18n::{I18n, LanguageId};
use cirrocast::model::units::UnitSystem;
use cirrocast::model::{LocationSource, Report};
use cirrocast::render::{ColorMode, RenderContext, Renderer, TermCaps, plain::Plain};
use common::{ProviderRun, fixture_location};

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

/// Renders `report` in `units`, clipped to `width`.
fn render(report: &Report, units: UnitSystem, width: usize) -> String {
    let i18n = I18n::new(LanguageId::EN_US);
    let ctx = RenderContext {
        units: units
            .resolve(&UnitOverrides::default())
            .expect("the default overrides resolve"),
        color: ColorMode::Never,
        width,
        term: TermCaps::default(),
        lang: LanguageId::EN_US,
        i18n: &i18n,
        now: Utc
            .with_ymd_and_hms(2026, 7, 15, 12, 0, 0)
            .single()
            .expect("a valid instant")
            .fixed_offset(),
        tz: report.location.tz,
    };
    Plain
        .render(report, &ctx)
        .expect("plain always renders a report")
}

#[test]
fn the_metric_output_is_exactly_these_lines() {
    let text = render(&report(), UnitSystem::Metric, 200);
    assert_eq!(
        text,
        "\
Beijing, Beijing (39.90, 116.41) Asia/Shanghai
Now: 19°C (feels 12°C), Overcast, wind 20 km/h NW, humidity 12%, pressure 1020 hPa, visibility 17 km, 0.0 mm
2026-07-15  min 25°C  max 35°C  sunrise 04:58  sunset 19:42
  Morning  29°C  Clear sky       precip 0.0 mm (0%)   wind 2.5 km/h N
  Noon     35°C  Clear sky       precip 0.0 mm (0%)   wind 4.7 km/h SW
  Evening  30°C  Overcast        precip 0.0 mm (0%)   wind 13 km/h SW
  Night    26°C  Overcast        precip 0.0 mm (0%)   wind 6.0 km/h SW
Location data based on GeoNames (CC-BY-4.0) via Open-Meteo — https://open-meteo.com/
Data: Open-Meteo.com (CC BY 4.0)"
    );
}

#[test]
fn the_three_unit_systems_differ_only_where_the_model_converts() {
    let report = report();
    let metric = render(&report, UnitSystem::Metric, 200);
    let us = render(&report, UnitSystem::Us, 200);
    let uk = render(&report, UnitSystem::Uk, 200);

    assert!(metric.contains("Now: 19°C (feels 12°C)"), "{metric}");
    assert!(us.contains("Now: 65°F (feels 54°F)"), "{us}");
    assert!(uk.contains("Now: 19°C (feels 12°C)"), "{uk}");

    assert!(metric.contains("wind 20 km/h NW"), "{metric}");
    assert!(us.contains("wind 12 mph NW"), "{us}");
    assert!(uk.contains("wind 12 mph NW"), "{uk}");

    assert!(metric.contains("visibility 17 km"), "{metric}");
    assert!(us.contains("visibility 11 mi"), "{us}");

    assert!(metric.contains("min 25°C  max 35°C"), "{metric}");
    assert!(us.contains("min 77°F  max 96°F"), "{us}");

    for text in [&metric, &us, &uk] {
        assert!(
            !text.contains('\u{1b}'),
            "plain never emits an escape sequence"
        );
        assert!(text.ends_with("Data: Open-Meteo.com (CC BY 4.0)"), "{text}");
    }
}

#[test]
fn a_current_only_report_prints_the_header_and_the_now_line() {
    let run = ProviderRun::fixture(
        "forecast_beijing_2026-07-15.json",
        (2026, 7, 15),
        CacheMode::Normal,
    );
    let report = run
        .fetch(&fixture_location("beijing"), 0)
        .expect("the fixture parses");

    let text = render(&report, UnitSystem::Metric, 200);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 4, "{text}");
    assert!(lines[1].starts_with("Now: "));
    assert!(!text.contains("Morning") && !text.contains("2026-07-15  min"));
}

#[test]
fn an_osm_location_prints_the_odbld_line() {
    let mut report = report();
    report.location.source = LocationSource::Osm;
    let text = render(&report, UnitSystem::Metric, 200);
    assert!(
        text.contains("Location data © OpenStreetMap contributors (ODbL)"),
        "{text}"
    );
    assert!(!text.contains("GeoNames"), "{text}");
}

#[test]
fn a_narrow_context_clips_every_line() {
    let text = render(&report(), UnitSystem::Metric, 30);
    for line in text.lines() {
        assert!(line.chars().count() <= 30, "too wide: {line:?}");
    }
    assert_eq!(
        text.lines().count(),
        9,
        "clipping never drops or adds a line: {text}"
    );
}
