// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The `%`-token template engine (step 19): token coverage, width/precision, escapes, presets and
//! the unknown-token policy on both sides of its boundary.
//!
//! Every token is pinned by an explicit expected string taken from the `beijing-1d.json` fixture,
//! and the expectation table's length is asserted against [`TOKENS`], so a token added without a
//! test fails the build. One snapshot covers the concatenation of every token, so a reviewer sees
//! the whole vocabulary on one page.

// The expected values are literals derived from the fixture, so exact comparison is the assertion.
#![allow(clippy::float_cmp)]

mod common;

use std::collections::BTreeMap;

use cirrocast::config::UnitOverrides;
use cirrocast::i18n::{I18n, LanguageRequest};
use cirrocast::model::Report;
use cirrocast::model::units::UnitSystem;
use cirrocast::render::{ColorMode, RenderContext, TermCaps};
use cirrocast::template::{
    self, PRESETS, TOKENS, Token, TokenKind, expand, preset, resolve_template,
};

/// The English catalog, loaded the way the CLI loads an unconfigured run.
fn english() -> I18n {
    I18n::load(&LanguageRequest::Auto, |_| None)
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
fn context<'a>(report: &Report, i18n: &'a I18n) -> RenderContext<'a> {
    RenderContext {
        units: UnitSystem::Metric
            .resolve(&UnitOverrides::default())
            .expect("the default overrides resolve"),
        color: ColorMode::Never,
        width: 80,
        term: capable(),
        times: common::fixture_times(report),
        lang: i18n.lang(),
        i18n,
        alert_credits: &[],
        aqi_index: cirrocast::air::aqi::AqiIndex::Us,
    }
}

/// Expands `template` over the fixture.
fn line(template: &str) -> String {
    let report = report();
    let i18n = english();
    expand(template, &report, &context(&report, &i18n)).expect("the template expands")
}

/// The expected value of every token, one row per [`TOKENS`] entry.
///
/// `beijing-1d.json`: 21.5 °C (feels 22.0, dew point ≈ 11.2), 52 %, 10 km/h from 30°, 1015 hPa,
/// 14 km visibility, code 1 (mainly clear) in daylight, UV 5, sunrise 06:05, sunset 17:58, day
/// high 24 / low 14, crescent moon. No alerts and no air reading in this fixture.
const EXPECTED: &[(char, &str)] = &[
    ('c', "\\o_"),
    ('C', "Mainly clear"),
    ('x', "\\o_"),
    ('t', "+22°C"),
    ('f', "+22°C"),
    ('H', "+24°C"),
    ('L', "+14°C"),
    ('w', "↗ 10km/h NNE"),
    ('h', "52%"),
    ('p', "0.0mm"),
    ('P', "1015hPa"),
    ('e', "+11°C"),
    ('u', "5"),
    ('U', "5 (moderate)"),
    ('m', "◕"),
    ('M', "Waning Gibbous"),
    ('v', "14km"),
    ('l', "Beijing"),
    ('d', "2026-09-30"),
    ('D', "Wed 30 Sep"),
    ('T', "12:15"),
    ('Z', "Asia/Shanghai"),
    ('z', "+0800"),
    ('S', "06:05"),
    ('s', "17:58"),
    ('A', ""),
    ('q', "n/a"),
];

#[test]
fn every_token_renders_the_documented_value() {
    assert_eq!(
        TOKENS.len(),
        EXPECTED.len(),
        "a token without a test row: the tables must be the same length"
    );
    for (letter, expected) in EXPECTED {
        assert_eq!(&line(&format!("%{letter}")), expected, "%{letter}");
    }
    // The table the test walks is the engine's own; a row whose letter does not bind the same
    // token would make the walk meaningless.
    for spec in TOKENS {
        let expected = EXPECTED
            .iter()
            .find(|(letter, _)| *letter == spec.letter)
            .map(|(_, expected)| *expected);
        assert!(expected.is_some(), "%{} has no expectation", spec.letter);
    }
    for (letter, token) in [
        ('c', Token::ConditionArt),
        ('x', Token::ConditionPlain),
        ('H', Token::High),
        ('L', Token::Low),
        ('e', Token::DewPoint),
        ('T', Token::Time),
    ] {
        assert_eq!(template::token(letter), Some(token), "%{letter}");
    }
}

#[test]
fn the_token_kinds_match_the_value_the_token_renders() {
    // The kind decides whether `.prec` rounds or truncates and whether `0` zero-pads; the
    // temperature, humidity, precipitation, pressure, visibility and UV tokens are numbers.
    let numbers: Vec<char> = TOKENS
        .iter()
        .filter(|spec| spec.kind == TokenKind::Number)
        .map(|spec| spec.letter)
        .collect();
    assert_eq!(
        numbers,
        vec!['t', 'f', 'H', 'L', 'h', 'p', 'P', 'e', 'u', 'v']
    );
}

#[test]
fn the_whole_vocabulary_renders_on_one_line() {
    let template: String = TOKENS
        .iter()
        .map(|spec| format!("%{}", spec.letter))
        .collect::<Vec<_>>()
        .join("|");
    insta::with_settings!({ prepend_module_to_snapshot => false }, {
        insta::assert_snapshot!("template_all_tokens", line(&template));
    });
}

#[test]
fn width_pads_and_precision_rounds_or_truncates() {
    for (template, expected) in [
        // Padding: spaces on the left, or on the right with `-`.
        ("%15C", "   Mainly clear"),
        ("%-15C", "Mainly clear   "),
        ("%5l", "Beijing"),
        ("%-5c", "\\o_  "),
        // Zero padding goes between the sign and the digits, only for numeric tokens.
        ("%08.1t", "+021.5°C"),
        ("%08.4C", "    Main"),
        ("%8.1t", " +21.5°C"),
        ("%9.1t", "  +21.5°C"),
        // Precision rounds a numeric token inside its unit suffix.
        ("%.1t", "+21.5°C"),
        ("%.0t", "+22°C"),
        ("%.2p", "0.00mm"),
        ("%.0h", "52%"),
        ("%.1v", "14.0km"),
        // Precision truncates text from the right; the width then pads what is left.
        ("%.4C", "Main"),
        ("%.2l", "Be"),
        ("%8.4C", "    Main"),
        ("%-8.4C", "Main    "),
        // A width smaller than the value never truncates it.
        ("%2t", "+22°C"),
    ] {
        assert_eq!(line(template), expected, "{template}");
    }
}

#[test]
fn escapes_and_braces_behave_as_documented() {
    assert_eq!(line("%%"), "%");
    assert_eq!(line("%l:%%"), "Beijing:%");
    assert_eq!(line("50%"), "50%", "a trailing lone % is literal");
    assert_eq!(
        line("%y %c"),
        "%y \\o_",
        "an unknown token stays literal in the engine"
    );
    assert_eq!(
        line("%{no %c expansion}"),
        "no %c expansion",
        "a braced run is verbatim unless it is one token letter"
    );
    assert_eq!(
        line("%{c}"),
        "\\o_",
        "a braced single letter is the token, for text that would glue onto it"
    );
    assert_eq!(line("%{%}"), "%", "a braced % is the literal percent sign");
    assert_eq!(line("%{}"), "", "an empty braced run is empty text");
    assert_eq!(line("%{a\\}b}"), "a}b", "a backslash escapes the brace");
    assert_eq!(line("a\\nb"), "a\nb");
    assert_eq!(line("a\\tb"), "a\tb");
    assert_eq!(line("a\\\\b"), "a\\b");
    assert_eq!(line("100%"), "100%");
    assert_eq!(
        line("%12"),
        "%12",
        "a specifier without a letter is literal"
    );
}

#[test]
fn an_unknown_token_is_reported_with_its_position_and_specifier() {
    assert_eq!(
        template::warnings("%y %c %12y"),
        vec![
            "unknown template token `%y` at position 1".to_owned(),
            "unknown template token `%y` at position 7".to_owned(),
        ]
    );
    assert_eq!(
        template::warnings("%c %t %% %{x} %{c}"),
        Vec::<String>::new()
    );

    // The CLI gate turns the first report into a usage error naming the table.
    let error = template::validate("%y").expect_err("a typo is refused");
    assert_eq!(error.exit_code(), 2);
    assert!(
        error
            .to_string()
            .contains("unknown template token `%y` at position 1"),
        "{error}"
    );
    assert!(
        error
            .to_string()
            .contains("known tokens: cCxtfHLwhpPeuUmMvldDTZzSsAq"),
        "{error}"
    );
    template::validate("%c %t").expect("the whole table passes the gate");
}

#[test]
fn an_empty_template_is_a_usage_error() {
    let report = report();
    let i18n = english();
    let ctx = context(&report, &i18n);
    for template in ["", "   ", "\t"] {
        let error = expand(template, &report, &ctx).expect_err("never empty");
        assert_eq!(error.exit_code(), 2, "{template:?}");
        assert!(error.to_string().contains("template is empty"), "{error}");
    }
}

#[test]
fn a_preset_is_a_template_and_the_configured_table_extends_it() {
    let none = BTreeMap::new();
    for (name, template) in PRESETS {
        let spec = format!("@{name}");
        assert_eq!(
            resolve_template(Some(&spec), &none).expect("a known preset"),
            template,
            "{spec}"
        );
    }
    assert_eq!(
        resolve_template(None, &none).expect("the default preset"),
        preset("default").expect("the default preset exists")
    );
    assert_eq!(preset("minimal"), Some("%c%t"));

    let error = resolve_template(Some("@nope"), &none).expect_err("never a preset");
    assert_eq!(error.exit_code(), 2);
    let message = error.to_string();
    for (name, template) in PRESETS {
        assert!(message.contains(name), "`{name}` missing from {message}");
        assert!(
            message.contains(template),
            "`{template}` missing from {message}"
        );
    }

    // A `[templates]` entry is addressable through the same `@name` spelling, and a built-in name
    // wins over a configured one of the same name.
    let mut configured = BTreeMap::new();
    configured.insert("compact".to_owned(), "%c%t".to_owned());
    configured.insert("full".to_owned(), "%l".to_owned());
    assert_eq!(
        resolve_template(Some("@compact"), &configured).expect("a configured preset"),
        "%c%t"
    );
    assert_eq!(
        resolve_template(Some("@full"), &configured).expect("the built-in wins"),
        preset("full").expect("the full preset exists")
    );
}

#[test]
fn the_cli_refuses_the_unknown_token_and_accepts_configured_presets() {
    use predicates::prelude::*;

    let sandbox = common::Sandbox::new();
    sandbox
        .cirrocast()
        .args(["@39.9,116.4", "-f", "one-line", "--template", "%y"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("unknown template token `%y`"));

    // `--format` reaches the same three namespaces: a preset renders as `one-line` with its
    // template, and a `[templates]` key is addressable by name.
    sandbox.write_config(
        "schema_version = 2\n\
         [templates]\ncompact = \"%c%t\"\nbad = \"%y\"\n",
    );
    sandbox
        .cirrocast()
        .args(["@39.9,116.4", "-f", "compact", "--offline"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("offline: no cached open-meteo"));

    // A configured preset with a typo is refused like a literal one, through either spelling.
    for args in [
        vec!["@39.9,116.4", "-f", "bad"],
        vec!["@39.9,116.4", "-f", "one-line", "--template", "@bad"],
    ] {
        sandbox
            .cirrocast()
            .args(args)
            .assert()
            .code(2)
            .stderr(predicate::str::contains("unknown template token `%y`"));
    }
}

/// A sandbox whose cache holds the recorded forecast for `@39.9042,116.4074`.
fn seeded() -> common::Sandbox {
    use std::sync::Arc;
    use std::time::Duration;

    use cirrocast::cache::{Cache, CacheKey, CacheMode, SystemClock};

    let sandbox = common::Sandbox::new();
    let body = std::fs::read_to_string(common::fixture_path(
        "open_meteo/forecast_beijing_2026-07-15.json",
    ))
    .expect("the fixture is readable");
    let key = CacheKey::weather(
        "open-meteo",
        39.9042,
        116.4074,
        3,
        chrono::Utc::now().date_naive(),
    );
    let cache = Cache::with_root(
        sandbox.cache_dir(),
        CacheMode::Normal,
        Arc::new(SystemClock),
        0,
    );
    cache
        .write(&key, 200, &body, Duration::from_secs(600))
        .expect("the cache entry is written");
    sandbox
}

#[test]
fn the_full_and_minimal_formats_are_one_line_presets() {
    let sandbox = seeded();
    let run = |args: &[&str]| {
        let assert = sandbox
            .cirrocast()
            .args(args)
            .args(["@39.9042,116.4074", "--offline"])
            .assert()
            .success();
        String::from_utf8(assert.get_output().stdout.clone()).expect("stdout is UTF-8")
    };

    let minimal = run(&["-f", "minimal"]);
    assert_eq!(minimal.trim_end(), "*o*+18°C");
    let full = run(&["-f", "full"]);
    assert!(full.contains("Clear sky"), "{full}");
    assert!(full.starts_with("39.9042, 116.4074: "), "{full}");

    // Both are one-line formats: `--moon` is refused for them exactly as for `one-line`.
    sandbox
        .cirrocast()
        .args(["-f", "full", "--moon", "@39.9042,116.4074", "--offline"])
        .assert()
        .code(2)
        .stderr(predicates::str::contains("--moon"));
}

#[test]
fn a_template_file_or_stdin_supplies_the_template() {
    use predicates::prelude::*;

    let sandbox = seeded();
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = dir.path().join("weather.tmpl");
    std::fs::write(&path, "%l: %c %t\n").expect("the template file is written");

    let assert = sandbox
        .cirrocast()
        .args([
            "-f",
            "one-line",
            "--template-file",
            path.to_str().expect("a UTF-8 path"),
            "@39.9042,116.4074",
            "--offline",
        ])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("stdout is UTF-8");
    assert!(stdout.contains(": *o* +18°C"), "{stdout}");

    // `-` reads standard input.
    let assert = sandbox
        .cirrocast()
        .args([
            "-f",
            "one-line",
            "--template-file",
            "-",
            "@39.9042,116.4074",
            "--offline",
        ])
        .write_stdin("%t")
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("stdout is UTF-8");
    assert_eq!(stdout.trim_end(), "+18°C");

    // The two template flags conflict, and a template flag cannot override a preset-selecting
    // format; both are usage errors before any traffic.
    sandbox
        .cirrocast()
        .args(["-f", "one-line", "--template", "%t", "--template-file", "-"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("cannot be used with"));
    sandbox
        .cirrocast()
        .args(["-f", "full", "--template", "%t"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("already selects a template"));

    // An empty file is the empty-template usage error.
    let empty = dir.path().join("empty.tmpl");
    std::fs::write(&empty, "  \n").expect("the empty file is written");
    sandbox
        .cirrocast()
        .args([
            "-f",
            "one-line",
            "--template-file",
            empty.to_str().expect("a UTF-8 path"),
        ])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("the template is empty"));
}
