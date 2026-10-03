// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The `one-line` format: one case per token, the escapes, and the five presets.
//!
//! The report is the hand-written `beijing-1d.json` fixture and the clock is the fixture's own
//! observation time, so every expected string below is a literal that a reviewer can check against
//! the fixture without running anything.

// Every expected value is a literal taken from the fixture, so exact comparison is the assertion.
#![allow(clippy::float_cmp)]

mod common;

use cirrocast::config::UnitOverrides;
use cirrocast::i18n::{I18n, LanguageRequest};
use cirrocast::model::Report;
use cirrocast::model::units::UnitSystem;
use cirrocast::render::one_line::{self, OneLine, PRESETS, expand, warnings};
use cirrocast::render::{ColorMode, RenderContext, Renderer, TermCaps};

/// The English catalog, loaded the way the CLI loads an unconfigured run.
fn english() -> I18n {
    I18n::load(&LanguageRequest::Auto, |_| None)
}

/// A catalog for one language tag, for the locale runs.
fn catalog(tag: &str) -> I18n {
    I18n::load(&LanguageRequest::Tag(tag.to_owned()), |_| None)
}

/// The fixture every case renders.
fn report() -> Report {
    common::fixture_report("beijing-1d.json")
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

/// The context for `report`, with the palette and charset injected.
fn context<'a>(
    report: &Report,
    units: UnitSystem,
    caps: TermCaps,
    i18n: &'a I18n,
) -> RenderContext<'a> {
    RenderContext {
        units: units
            .resolve(&UnitOverrides::default())
            .expect("the default overrides resolve"),
        color: ColorMode::Never,
        width: 80,
        term: caps,
        now: common::fixture_now(report),
        tz: report.location.tz,
        lang: i18n.lang(),
        i18n,
        alert_credits: &[],
    }
}

/// Expands `template` over the fixture, at a capable terminal.
fn line(template: &str) -> String {
    let report = report();
    let i18n = english();
    expand(
        template,
        &report,
        &context(&report, UnitSystem::Metric, capable(), &i18n),
    )
    .expect("the template expands")
}

/// The rendered `one-line` output for a template and a unit system.
fn rendered(template: &str, units: UnitSystem) -> String {
    rendered_in(template, units, &english())
}

/// The same, in a given language.
fn rendered_in(template: &str, units: UnitSystem, i18n: &I18n) -> String {
    let report = report();
    OneLine::new(one_line::resolve_template(Some(template)).expect("a template"))
        .render(&report, &context(&report, units, capable(), i18n))
        .expect("the renderer renders")
}

#[test]
fn every_token_renders_the_documented_value() {
    // `beijing-1d.json`: 21.5 °C (feels 22.0), 10 km/h from 30°, 52 %, 1015 hPa, 14 km visibility,
    // code 1 (mainly clear) in daylight, UV 5, sunrise 06:05, sunset 17:58.
    for (template, expected) in [
        ("%c", "\\o_"),
        ("%C", "Mainly clear"),
        ("%t", "+22°C"),
        ("%f", "+22°C"),
        ("%w", "↗ 10km/h NNE"),
        ("%h", "52%"),
        ("%p", "0.0mm"),
        ("%P", "1015hPa"),
        ("%v", "14km"),
        ("%u", "5"),
        ("%U", "5 (moderate)"),
        ("%d", "2026-09-30"),
        ("%D", "Wed 30 Sep"),
        ("%Z", "Asia/Shanghai"),
        ("%z", "+0800"),
        ("%S", "06:05"),
        ("%s", "17:58"),
        ("%l", "Beijing"),
        ("%L", "39.90,116.41"),
        ("%m", "n/a"),
    ] {
        assert_eq!(line(template), expected, "{template}");
    }
}

#[test]
fn the_night_variant_and_the_ascii_charset_follow_the_terminal() {
    // The night fixture is the same place after dark: the sky art switches to its night sibling.
    let night = common::fixture_report("beijing-night.json");
    let i18n = english();
    let ctx = context(&night, UnitSystem::Metric, capable(), &i18n);
    assert_eq!(expand("%c", &night, &ctx).expect("expands"), "*o*");

    // A bare terminal gets the 7-bit arrow, which is why the arrow is borrowed from `art`.
    let report = report();
    let ctx = context(&report, UnitSystem::Metric, TermCaps::default(), &i18n);
    assert_eq!(
        expand("%w", &report, &ctx).expect("expands"),
        "/ 10km/h NNE",
        "TERM=dumb or a non-UTF-8 locale falls back to the ASCII arrows"
    );
}

#[test]
fn the_unit_systems_convert_the_tokens_not_the_report() {
    assert_eq!(
        rendered("%t %f %w %h %p %P %v", UnitSystem::Metric),
        "+22°C +22°C ↗ 10km/h NNE 52% 0.0mm 1015hPa 14km"
    );
    assert_eq!(
        rendered("%t %f %w %h %p %P %v", UnitSystem::Us),
        "+71°F +72°F ↗ 6.2mph NNE 52% 0.00in 29.97inHg 8.7mi"
    );
    assert_eq!(
        rendered("%t %f %w %h %p %P %v", UnitSystem::Uk),
        "+22°C +22°C ↗ 6.2mph NNE 52% 0.0mm 1015hPa 8.7mi"
    );

    let report = report();
    assert_eq!(
        report.current.expect("a current block").temp_c,
        21.5,
        "the report itself is never converted"
    );
}

#[test]
fn escapes_are_unwrapped_before_the_tokens_are_read() {
    assert_eq!(line("%%"), "%");
    assert_eq!(line("%l:%%"), "Beijing:%");
    assert_eq!(line("50%"), "50%", "a trailing lone % is literal");
    assert_eq!(line("%q %c"), "%q \\o_", "an unknown token stays literal");
    assert_eq!(line("%q").len(), 2, "`%q` is two characters, not one");
    assert_eq!(
        line("%{no %c expansion}"),
        "no %c expansion",
        "a braced run is verbatim"
    );
    assert_eq!(line("%{a\\}b}"), "a}b", "a backslash escapes the brace");
    assert_eq!(line("a\\nb"), "a\nb");
    assert_eq!(line("a\\tb"), "a\tb");
    assert_eq!(line("a\\\\b"), "a\\b");
}

#[test]
fn an_unknown_token_is_reported_once_per_occurrence() {
    assert_eq!(
        warnings("%q %c %q"),
        vec![
            "note: unknown one-line token `%q` at position 1 is printed literally".to_owned(),
            "note: unknown one-line token `%q` at position 7 is printed literally".to_owned(),
        ]
    );
    assert_eq!(warnings("%c %t %% %{x}"), Vec::<String>::new());
}

#[test]
fn an_empty_template_is_a_usage_error() {
    let report = report();
    let i18n = english();
    let ctx = context(&report, UnitSystem::Metric, capable(), &i18n);
    for template in ["", "   ", "\t"] {
        let error = expand(template, &report, &ctx).expect_err("never empty");
        assert_eq!(error.exit_code(), 2, "{template:?}");
        assert!(error.to_string().contains("template is empty"), "{error}");
    }
}

#[test]
fn a_preset_is_a_template_and_an_unknown_one_lists_them() {
    for (name, template) in PRESETS {
        let spec = format!("@{name}");
        assert_eq!(
            one_line::resolve_template(Some(&spec)).expect("a known preset"),
            template,
            "{spec}"
        );
    }
    assert_eq!(
        one_line::resolve_template(None).expect("the default preset"),
        one_line::preset("default").expect("the default preset exists")
    );

    let error = one_line::resolve_template(Some("@nope")).expect_err("never a preset");
    assert_eq!(error.exit_code(), 2);
    let message = error.to_string();
    for (name, template) in PRESETS {
        assert!(message.contains(name), "`{name}` missing from {message}");
        assert!(
            message.contains(template),
            "`{template}` missing from {message}"
        );
    }
}

/// Snapshots one preset under `name` in the shared `tests/snapshots` directory.
macro_rules! snapshot {
    ($name:literal, $template:literal) => {
        insta::with_settings!({ prepend_module_to_snapshot => false }, {
            insta::assert_snapshot!($name, rendered($template, UnitSystem::Metric));
        });
    };
}

#[test]
fn the_presets_render_the_documented_lines() {
    snapshot!("one_line_preset_default", "@default");
    snapshot!("one_line_preset_short", "@short");
    snapshot!("one_line_preset_full", "@full");
    snapshot!("one_line_preset_uv", "@uv");
    snapshot!("one_line_preset_sun", "@sun");
}

/// The full preset in Chinese: same line, every word from the `zh-CN` catalog.
///
/// The tokens carry the vocabulary (`晴间多云`, `东南风`, `紫外线 5（中等）`, `06:05`) and the
/// separators stay the template's, because a one-line template is a template in every language.
#[test]
fn the_full_preset_reads_in_chinese() {
    insta::with_settings!({ prepend_module_to_snapshot => false }, {
        insta::assert_snapshot!(
            "one_line_preset_full_zh",
            rendered_in("@full", UnitSystem::Metric, &catalog("zh-CN"))
        );
    });
}

/// Every token that has a value renders it in Chinese too.
#[test]
fn the_chinese_tokens_read_the_catalog() {
    let chinese = catalog("zh-CN");
    for (template, expected) in [
        ("%C", "晴间多云"),
        ("%w", "↗ 10km/h 北东北风"),
        ("%U", "5（中等）"),
        ("%D", "9月30日 周三"),
        ("%d", "2026-09-30"),
    ] {
        assert_eq!(
            rendered_in(template, UnitSystem::Metric, &chinese),
            expected,
            "{template}"
        );
    }
}

/// A template is one line: the width a caller asks for never truncates it, because truncation would
/// silently drop the tokens the format exists to print.
#[test]
fn the_width_does_not_change_the_output() {
    let report = report();
    let i18n = english();
    let caps = capable();
    let context = |width| RenderContext {
        units: UnitSystem::Metric
            .resolve(&UnitOverrides::default())
            .expect("the default overrides resolve"),
        color: ColorMode::Never,
        width,
        term: caps,
        now: common::fixture_now(&report),
        tz: report.location.tz,
        lang: i18n.lang(),
        i18n: &i18n,
        alert_credits: &[],
    };

    let template = one_line::preset("full").expect("the full preset exists");
    let narrow = expand(template, &report, &context(20)).expect("the template expands");
    assert_eq!(
        narrow,
        expand(template, &report, &context(200)).expect("the template expands")
    );
    assert!(
        narrow.chars().count() > 20,
        "the full preset is wider than 20 columns: {narrow:?}"
    );
}

/// The direction is genuinely optional upstream — a calm `OpenWeatherMap` reading and a METAR
/// `VRB` both leave it out — so a known speed must not become `n/a` for want of an arrow.
#[test]
fn the_wind_token_prints_a_known_speed_without_a_direction() {
    let mut report = report();
    report.current = None;
    for day in &mut report.days {
        for part in &mut day.parts {
            part.wind_dir_deg = None;
        }
    }
    let i18n = english();
    let text = expand(
        "%w",
        &report,
        &context(&report, UnitSystem::Metric, capable(), &i18n),
    )
    .expect("the template expands");
    assert_eq!(
        text, "12km/h",
        "the noon part's known speed prints without a direction"
    );
    assert!(!text.contains("n/a"), "{text}");
}
