// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Renderer-level regression tests for the variable-wind fix (§3.12).
//!
//! The fixture `report/vrb-wind.json` carries a `null` `wind_dir_deg` — the shape a METAR `VRB`
//! observation now produces — and every format is asserted to leave the direction out instead of
//! drawing the due-north label the old `unwrap_or(0)` produced. The instant is the fixture's own
//! observation time, so nothing here depends on the network or the clock.

mod common;

use cirrocast::config::UnitOverrides;
use cirrocast::i18n::{I18n, LanguageRequest};
use cirrocast::model::Report;
use cirrocast::model::units::UnitSystem;
use cirrocast::render::{Format, RenderContext, TermCaps, renderer_for};
use cirrocast::template::expand;

/// The English catalog, loaded the way the CLI loads an unconfigured run.
fn english() -> I18n {
    I18n::load(&LanguageRequest::Auto, |_| None)
}

/// A terminal that can do everything: UTF-8, a tty, 256 colours.
fn capable() -> TermCaps {
    TermCaps::read(
        |name| match name {
            "TERM" => Some("xterm-256color".to_owned()),
            "LANG" => Some("en_US.UTF-8".to_owned()),
            _ => None,
        },
        true,
    )
}

/// The fixture the section renders.
fn report() -> Report {
    common::fixture_report("vrb-wind.json")
}

/// Renders `format` over the fixture at a capable terminal.
fn rendered(format: Format) -> String {
    let report = report();
    let i18n = english();
    let caps = capable();
    let context = RenderContext {
        units: UnitSystem::Metric
            .resolve(&UnitOverrides::default())
            .expect("the default overrides resolve"),
        color: cirrocast::render::ColorMode::Never,
        width: 80,
        term: caps,
        times: common::fixture_times(&report),
        lang: i18n.lang(),
        i18n: &i18n,
        alert_credits: &[],
        aqi_index: cirrocast::air::aqi::AqiIndex::Us,
    };
    renderer_for(format, &caps, None)
        .expect("the format has a renderer")
        .render(&report, &context)
        .expect("the fixture renders")
}

#[test]
fn a_variable_wind_serialises_as_null_in_json() {
    let document: serde_json::Value =
        serde_json::from_str(&rendered(Format::Json)).expect("json is valid");
    let direction = &document["current"]["wind_dir_deg"];
    assert!(
        direction.is_null(),
        "a VRB wind must serialise as null, got {direction}"
    );
}

#[test]
fn a_variable_wind_has_no_cardinal_in_the_wind_token() {
    let report = report();
    let i18n = english();
    let caps = capable();
    let context = RenderContext {
        units: UnitSystem::Metric
            .resolve(&UnitOverrides::default())
            .expect("the default overrides resolve"),
        color: cirrocast::render::ColorMode::Never,
        width: 80,
        term: caps,
        times: common::fixture_times(&report),
        lang: i18n.lang(),
        i18n: &i18n,
        alert_credits: &[],
        aqi_index: cirrocast::air::aqi::AqiIndex::Us,
    };
    let wind = expand("%w", &report, &context).expect("%w expands");
    assert_eq!(wind, "11km/h", "the speed alone, with no direction");
    assert!(!wind.contains('N'), "no due-north label: {wind}");
}

#[test]
fn a_variable_wind_has_no_cardinal_in_the_art_table() {
    let output = rendered(Format::ArtTable);
    assert!(output.contains("11km/h"), "the speed is drawn:\n{output}");
    assert!(
        !output.contains("11km/h N"),
        "the due-north label must not appear:\n{output}"
    );
}
