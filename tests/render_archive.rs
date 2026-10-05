// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The archive label every format carries (`--date`, `--history`).
//!
//! A historical answer has the same shape as a forecast, so the formats say which one it is: the
//! `art-table` header appends the dates and the archive word, `plain` writes a record, `one-line`
//! prefixes the line and `json` carries `mode`. All four read the one helper in `src/render`, so
//! this test pins the shape once and the formats cannot disagree.
//!
//! No test here opens a socket: the fixtures are hand-written `Report` documents.

mod common;

use cirrocast::config::UnitOverrides;
use cirrocast::i18n::{I18n, LanguageRequest};
use cirrocast::model::Report;
use cirrocast::model::units::UnitSystem;
use cirrocast::render::{ColorMode, Format, RenderContext, TermCaps, renderer_for};

/// The English catalog, loaded the way an unconfigured run loads it.
fn english() -> I18n {
    I18n::load(&LanguageRequest::Auto, |_| None)
}

/// Renders `report` through `format` with the fixed fixture instant.
fn render(report: &Report, format: Format) -> String {
    let i18n = english();
    let caps = TermCaps::read(
        |name| match name {
            "TERM" => Some("xterm-256color".to_owned()),
            "LANG" => Some("en_US.UTF-8".to_owned()),
            _ => None,
        },
        true,
    );
    let ctx = RenderContext {
        units: UnitSystem::Metric
            .resolve(&UnitOverrides::default())
            .expect("the default overrides resolve"),
        color: ColorMode::Never,
        width: 80,
        term: caps,
        times: common::fixture_times(report),
        lang: i18n.lang(),
        i18n: &i18n,
        alert_credits: &[],
        aqi_index: cirrocast::air::aqi::AqiIndex::Us,
    };
    renderer_for(format, &caps, None)
        .expect("the format has a renderer")
        .render(report, &ctx)
        .expect("the fixture renders")
}

/// The archive fixture: `mode: "archive"`, one day dated 2026-09-30.
fn archive() -> Report {
    common::fixture_report("beijing-marine.json")
}

#[test]
fn the_art_table_header_names_the_archive_and_its_date() {
    let text = render(&archive(), Format::ArtTable);
    let header = text.lines().next().expect("the table has a header");
    assert!(
        header.contains("· 2026-09-30 · archive"),
        "the archive is not labelled: {header:?}"
    );
}

#[test]
fn plain_writes_an_archive_record_and_one_line_prefixes_the_date() {
    let plain = render(&archive(), Format::Plain);
    assert!(
        plain.lines().any(|line| line == "archive: 2026-09-30"),
        "plain carries no archive record:\n{plain}"
    );

    let one_line = render(&archive(), Format::OneLine);
    assert!(
        one_line.starts_with("2026-09-30 · archive "),
        "one-line does not lead with the archive label: {one_line:?}"
    );
    assert_eq!(one_line.lines().count(), 1, "one-line stays one line");
}

#[test]
fn json_marks_the_mode_and_a_forecast_says_forecast() {
    let archived: serde_json::Value =
        serde_json::from_str(&render(&archive(), Format::Json)).expect("valid JSON");
    assert_eq!(archived["mode"], "archive");

    let forecast = common::fixture_report("beijing-1d.json");
    let text = render(&forecast, Format::Json);
    assert!(text.contains("\"mode\": \"forecast\""), "{text}");
}

#[test]
fn a_forecast_carries_no_archive_label() {
    let forecast = common::fixture_report("beijing-1d.json");
    let table = render(&forecast, Format::ArtTable);
    assert!(!table.contains("· archive"), "{table}");
    let plain = render(&forecast, Format::Plain);
    assert!(!plain.contains("archive"), "{plain}");
    let one_line = render(&forecast, Format::OneLine);
    assert!(!one_line.contains("archive"), "{one_line}");
}
