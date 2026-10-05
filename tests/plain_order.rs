// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The `plain` document's **order**, frozen.
//!
//! `plain` is the format a script reads a field at a time from (`grep '^current:'`, `awk -F': '`),
//! so the sequence of its records — and the sequence of the fields inside the `current:` record —
//! is part of the output contract, not incidental formatting. [`tests/render_plain.rs`] pins the
//! full text of one document; this file pins the *shape* of every document the model can produce,
//! including the panel records (`air`, `moon`, `alert`) that only a report with those panels
//! carries, so a rename, a removal or a reordering fails here even if no value changed.
//!
//! A failure here is a breaking output change: it needs a minor version bump, one release of dual
//! emission where feasible, and a `CHANGELOG.md` entry naming both shapes (`docs/ecosystem.md`,
//! "Output contracts").

mod common;

use chrono::{TimeZone as _, Utc};

use cirrocast::config::UnitOverrides;
use cirrocast::i18n::{I18n, LanguageRequest};
use cirrocast::model::Report;
use cirrocast::model::units::UnitSystem;
use cirrocast::render::{ColorMode, RenderContext, Renderer, TermCaps, plain::Plain};
use common::fixture_report;

/// The English catalog, loaded the way the CLI loads an unconfigured run.
fn english() -> I18n {
    I18n::load(&LanguageRequest::Auto, |_| None)
}

/// One fixture report rendered by `plain`, in metric units at a width the format ignores.
fn render(file: &str) -> String {
    render_with(file, &[])
}

/// [`render`] with the credit lines a run resolved (the alert sources' licences).
fn render_with(file: &str, credits: &[String]) -> String {
    let report = fixture_report(file);
    let i18n = english();
    let ctx = RenderContext {
        units: UnitSystem::Metric
            .resolve(&UnitOverrides::default())
            .expect("the default overrides resolve"),
        color: ColorMode::Never,
        width: 80,
        term: TermCaps::default(),
        lang: i18n.lang(),
        i18n: &i18n,
        alert_credits: credits,
        aqi_index: cirrocast::air::aqi::AqiIndex::Us,
        times: common::fixture_times(&report),
    };
    Plain
        .render(&report, &ctx)
        .expect("plain always renders a report")
}

/// The ordered record keys of a `plain` document.
///
/// A record key is the lower-case token a record starts with (`location`, `updated`, `current`,
/// `air`, `moon`, `sun`, `attribution`); a `day` record keeps its date, because there is one per
/// forecast day. A line that is a bare credit sentence carries no key and is reported as `credit`,
/// which is itself part of the order: the credits sit between the day records and `attribution`.
fn record_keys(text: &str) -> Vec<String> {
    /// The keys a record can start with, plus `day`, which carries its date. The air panel's title
    /// (`Air quality` → `air_quality`) and its pollutant labels are catalog spellings; a rename in
    /// the catalog is exactly the kind of change this file is here to catch.
    const KEYS: [&str; 17] = [
        "location",
        "alert",
        "updated",
        "current",
        "day",
        "air_quality",
        "pm2.5",
        "pm10",
        "o3",
        "no2",
        "so2",
        "co",
        "pollen",
        "uv",
        "moon",
        "sun",
        "attribution",
    ];
    text.lines()
        .map(|line| {
            let head = line.split_once(':').map_or(line, |(head, _)| head);
            let (word, rest) = head.split_once(' ').unwrap_or((head, ""));
            // `day 2026-09-30` is a key with its date; a bare `day` would not be a record.
            let is_key = (KEYS.contains(&word) && (word != "day" || !rest.is_empty()))
                || KEYS.contains(&head);
            if is_key {
                head.to_owned()
            } else {
                "credit".to_owned()
            }
        })
        .collect()
}

#[test]
fn the_records_appear_in_this_order() {
    // A forecast document: the place, when the reading was taken, the reading, the days, then the
    // two credit lines and the machine-readable attribution record.
    assert_eq!(
        record_keys(&render("beijing-3d-day.json")),
        vec![
            "location",
            "updated",
            "current",
            "day 2026-09-30",
            "day 2026-10-01",
            "day 2026-10-02",
            "credit",
            "credit",
            "attribution",
        ]
    );

    // A document with the alert panel: each warning is a record of its own, directly after the
    // location and before the reading it accompanies. The credit a warning source requires lands
    // with the other credits, after the data licence and before `attribution`.
    assert_eq!(
        record_keys(&render_with(
            "beijing-alerts.json",
            &["Alerts: example CAP feed".to_owned()]
        )),
        vec![
            "location",
            "alert",
            "updated",
            "current",
            "day 2026-09-30",
            "credit",
            "credit",
            "credit",
            "attribution",
        ]
    );

    // A document with the air panel: the panel's title with its indices, then one record per
    // pollutant that has a value, the pollen block, the UV reading, and the panel's own credit —
    // before the location credit and the data licence.
    assert_eq!(
        record_keys(&render("beijing-air.json")),
        vec![
            "location",
            "updated",
            "current",
            "day 2026-09-30",
            "air_quality",
            "pm2.5",
            "pm10",
            "o3",
            "no2",
            "so2",
            "co",
            "pollen",
            "uv",
            "credit",
            "credit",
            "credit",
            "attribution",
        ]
    );

    // A document with the astro panel: the moon block, then the sun block, before the credits.
    assert_eq!(
        record_keys(&render("beijing-astro.json")),
        vec![
            "location",
            "updated",
            "current",
            "day 2026-09-30",
            "moon",
            "sun",
            "credit",
            "credit",
            "attribution",
        ]
    );
}

#[test]
fn the_current_record_fields_are_in_this_order() {
    let text = render("beijing-3d-day.json");
    let line = text
        .lines()
        .find(|line| line.starts_with("current:"))
        .expect("the fixture carries current conditions");

    // The condition and its temperature lead the record (they are the values a one-line reader
    // takes first), then every labelled field in the order the module documents.
    let needles = [
        "current: ",
        "(feels ",
        " wind ",
        " humidity ",
        " precip ",
        " pressure ",
        " visibility ",
    ];
    let mut last = 0;
    for needle in needles {
        let at = line
            .find(needle)
            .unwrap_or_else(|| panic!("`{needle}` is missing from the record: {line}"));
        assert!(
            at >= last,
            "`{needle}` moved before an earlier field: {line}"
        );
        last = at;
    }
}

#[test]
fn the_day_record_parts_are_in_display_order() {
    let text = render("beijing-3d-day.json");
    let line = text
        .lines()
        .find(|line| line.starts_with("day "))
        .expect("the fixture carries a day record");

    // Morning, Noon, Evening, Night — the four canonical day parts, left to right.
    let mut last = 0;
    for part in ["Morning ", "Noon ", "Evening ", "Night "] {
        let at = line
            .find(part)
            .unwrap_or_else(|| panic!("`{part}` is missing from the record: {line}"));
        assert!(at >= last, "`{part}` moved before an earlier part: {line}");
        last = at;
    }
}

/// The fixture must be the shape this file claims to pin; a report that lost its panels would make
/// the assertions above pass for the wrong reason.
#[test]
fn the_fixtures_carry_the_panels_this_test_pins() {
    let panels = |file: &str| {
        let report: Report = fixture_report(file);
        (
            report.days.len(),
            !report.alerts.is_empty(),
            report.air.is_some(),
            report.astro.is_some(),
        )
    };
    assert_eq!(panels("beijing-3d-day.json"), (3, false, false, false));
    assert_eq!(panels("beijing-alerts.json"), (1, true, false, false));
    assert_eq!(panels("beijing-air.json"), (1, false, true, false));
    assert_eq!(panels("beijing-astro.json"), (1, false, false, true));
    let _ = Utc.with_ymd_and_hms(2026, 9, 30, 12, 0, 0).single();
}
