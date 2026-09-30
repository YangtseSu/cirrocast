// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Localization, end to end: the completeness of every shipped catalog, the negotiation between
//! `--lang`, the environment and the configuration, and the shape of what the user is told when a
//! request cannot be honoured.
//!
//! The catalogs are checked twice, because they can fail in two different ways. Their *text* is
//! parsed to compare key sets — a catalog that forgot a translation, or that invented one, fails
//! there. Their *bundles* are loaded through the public path and every key the renderers may ask
//! for is resolved — a catalog whose syntax broke, or whose fallback order stopped working, fails
//! there instead.
//!
//! Nothing here inherits the ambient locale: [`Sandbox::cirrocast`] pins `LC_ALL=C.UTF-8`, and each
//! case sets the variables it is about. That is what lets the suite pass under `LANG=zh_CN.UTF-8`
//! and under an empty environment alike.

mod common;

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use assert_cmd::Command;
use cirrocast::cache::{Cache, CacheKey, CacheMode, SystemClock};
use cirrocast::i18n::{CATALOGS, I18n, LanguageRequest, MessageKey, RENDERER_KEYS, condition_key};
use cirrocast::model::condition::Condition;
use common::Sandbox;

/// The coordinates the seeded cache entry is keyed by.
const LOCATION: &str = "@39.9042,116.4074";
/// The same pair, as the cache key wants it.
const LAT: f64 = 39.9042;
/// The longitude half of the pair.
const LON: f64 = 116.4074;

/// A sandbox whose weather cache holds the three-day fixture for [`LOCATION`].
fn seeded() -> Sandbox {
    let sandbox = Sandbox::new();
    let body = std::fs::read_to_string(common::fixture_path(
        "open_meteo/forecast_beijing_2026-07-15.json",
    ))
    .expect("the fixture is readable");
    let key = CacheKey::weather("open-meteo", LAT, LON, 3, chrono::Utc::now().date_naive());
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

/// A run against the seeded cache, offline.
fn run(sandbox: &Sandbox, args: &[&str]) -> Command {
    let mut command = sandbox.cirrocast();
    command.args(args).arg("--offline").arg(LOCATION);
    command
}

/// The stdout of a successful run.
fn stdout(sandbox: &Sandbox, args: &[&str]) -> String {
    let assert = run(sandbox, args).assert().success();
    String::from_utf8(assert.get_output().stdout.clone()).expect("stdout is UTF-8")
}

/// The message ids of one catalog, as the FTL syntax defines them: a key at the start of a line,
/// then `=`. The catalogs are flat, so this is the whole grammar it needs.
fn key_set(source: &str) -> BTreeSet<String> {
    source
        .lines()
        .filter(|line| !line.starts_with([' ', '#', '\t']) && !line.trim().is_empty())
        .filter_map(|line| line.split_once('=').map(|(key, _)| key.trim().to_owned()))
        .collect()
}

/// Every key the renderers, the tokens and the CLI can ask for.
///
/// [`RENDERER_KEYS`] covers the static keys; the hundred condition keys are derived from the code,
/// so they are enumerated here instead — that enumeration is what makes a provider adding a new
/// code safe, because a code without a translation fails this file.
fn every_renderer_key() -> Vec<MessageKey> {
    let mut wanted: Vec<MessageKey> = RENDERER_KEYS.to_vec();
    for code in 0..=99_u8 {
        wanted.push(condition_key(Condition::from_u8(code)));
    }
    wanted
}

/// The catalog spelling of the keys above, for the text-level comparison.
fn every_renderer_key_name() -> Vec<String> {
    every_renderer_key()
        .iter()
        .map(|key| key.as_str().into_owned())
        .collect()
}

/// One argument for every variable any message uses.
///
/// A message that takes no argument ignores the extras, so a single call shape can exercise every
/// key: a catalog entry whose syntax is broken renders its key back and fails the assertion, and a
/// message that needs an argument the caller forgot fails here instead of printing `{$value}` in
/// the middle of a forecast.
fn arguments() -> Vec<(&'static str, fluent_bundle::FluentValue<'static>)> {
    [
        ("value", "1"),
        ("band", "low"),
        ("weekday", "Mon"),
        ("month", "Sep"),
        ("month-number", "09"),
        ("day", "01"),
        ("day-plain", "1"),
        ("year", "2026"),
    ]
    .into_iter()
    .map(|(name, value)| (name, fluent_bundle::FluentValue::from(value)))
    .collect()
}

#[test]
fn every_catalog_carries_every_key_the_renderers_ask_for() {
    let english = key_set(CATALOGS[0].1);
    assert!(
        CATALOGS.len() >= 2,
        "the shipped locales are en-US and zh-CN"
    );

    for (tag, source) in CATALOGS {
        let catalog = key_set(source);
        let mut missing: Vec<&String> = english.difference(&catalog).collect();
        missing.sort();
        let mut orphan: Vec<&String> = catalog.difference(&english).collect();
        orphan.sort();
        assert!(missing.is_empty(), "{tag} is missing {missing:?}");
        assert!(orphan.is_empty(), "{tag} has unknown keys {orphan:?}");

        for key in every_renderer_key_name() {
            assert!(catalog.contains(&key), "{tag} has no message for `{key}`");
        }
    }
}

#[test]
fn every_catalog_resolves_every_key_through_the_bundle() {
    for (tag, _) in CATALOGS {
        let request = LanguageRequest::Tag((*tag).to_owned());
        let i18n = I18n::load(&request, |_| None);
        assert_eq!(i18n.lang().tag(), *tag, "{tag}");
        let arguments = arguments();
        for key in every_renderer_key() {
            let rendered = i18n.format(&key, &arguments);
            assert_ne!(rendered, key.as_str(), "{tag} is missing `{key}`");
            assert!(!rendered.is_empty(), "{tag}: `{key}` renders empty");
            assert!(
                !rendered.contains(['{', '}']),
                "{tag}: `{key}` left an unformatted variable: {rendered}"
            );
        }
        assert!(i18n.notes().is_empty(), "{tag}: {:?}", i18n.notes());
    }
}

#[test]
fn an_unknown_condition_has_a_name_of_its_own() {
    let request = LanguageRequest::Tag("zh-CN".to_owned());
    let i18n = I18n::load(&request, |_| None);
    assert_eq!(i18n.condition(Condition::from_u8(4)), "未知");
    assert_eq!(i18n.condition(Condition::from_u8(72)), "未知");
    assert_eq!(
        i18n.condition(Condition::from_u8(61)),
        "小雨",
        "a described code keeps its own name"
    );
}

/// A sandbox run with a terminal that can draw the box-drawing table.
fn terminal(sandbox: &Sandbox) -> Command {
    let mut command = sandbox.cirrocast();
    command.env("TERM", "xterm-256color");
    command
}

#[test]
fn the_flag_selects_chinese_and_the_table_still_lines_up() {
    let sandbox = seeded();
    let chinese = terminal(&sandbox)
        .args(["--lang", "zh-CN", "--offline", LOCATION])
        .assert()
        .success();
    let chinese = String::from_utf8(chinese.get_output().stdout.clone()).expect("stdout is UTF-8");
    assert!(chinese.contains("天气报告："), "{chinese}");
    assert!(chinese.contains("早上"), "{chinese}");
    assert!(chinese.contains("晴"), "{chinese}");

    // Every line of a Chinese table is measured in display columns, not bytes: a border that is
    // one column short would show up as a ragged right edge here.
    let widths: BTreeSet<usize> = chinese
        .lines()
        .filter(|line| line.starts_with(['┌', '│', '├', '└']))
        .map(|line| {
            line.chars()
                .map(|character| unicode_width::UnicodeWidthChar::width(character).unwrap_or(0))
                .sum()
        })
        .collect();
    assert_eq!(
        widths.len(),
        1,
        "the table's rows are one width: {widths:?}"
    );
    assert!(
        !chinese.contains('\u{2068}') && !chinese.contains('\u{2069}'),
        "no bidi isolation marks in terminal output:\n{chinese}"
    );
}

#[test]
fn the_ambient_locale_selects_the_language_and_reports_the_chain() {
    let sandbox = seeded();
    let assert = sandbox
        .cirrocast()
        .env("TERM", "xterm-256color")
        .env("LANG", "zh_CN.UTF-8")
        .args(["-v"])
        .arg("--offline")
        .arg(LOCATION)
        .assert()
        .success();
    let output = String::from_utf8(assert.get_output().stdout.clone()).expect("stdout is UTF-8");
    let notes = String::from_utf8(assert.get_output().stderr.clone()).expect("stderr is UTF-8");
    assert!(output.contains("天气报告："), "{output}");
    // The ambient spelling is echoed, so a user reading `-v` sees which variable decided and how.
    assert!(
        notes.contains("i18n: requested zh_CN.UTF-8 → selected zh-CN"),
        "{notes}"
    );
    assert!(notes.contains("zh-CN → en-US"), "{notes}");
}

#[test]
fn traditional_chinese_is_served_by_the_simplified_catalog_and_says_so_under_v() {
    let sandbox = seeded();
    let assert = run(&sandbox, &["--lang", "zh-TW", "-v"]).assert().success();
    let output = String::from_utf8(assert.get_output().stdout.clone()).expect("stdout is UTF-8");
    let notes = String::from_utf8(assert.get_output().stderr.clone()).expect("stderr is UTF-8");
    assert!(output.contains("天气报告："), "{output}");
    assert!(
        notes.contains("zh-TW") && notes.contains("zh-CN") && notes.contains("en-US"),
        "the chain is printed: {notes}"
    );
    assert!(
        !notes.contains("unsupported language"),
        "a documented fallback is not a warning: {notes}"
    );
}

#[test]
fn an_unsupported_language_warns_and_still_renders_english() {
    let sandbox = seeded();
    for tag in ["bad-TAG", "de-DE", "!!"] {
        let assert = run(&sandbox, &["--lang", tag]).assert().success();
        let output =
            String::from_utf8(assert.get_output().stdout.clone()).expect("stdout is UTF-8");
        let notes = String::from_utf8(assert.get_output().stderr.clone()).expect("stderr is UTF-8");
        assert!(
            notes.contains(&format!("unsupported language \"{tag}\"")),
            "{tag}: {notes}"
        );
        assert!(notes.contains("falling back to en-US"), "{tag}: {notes}");
        assert!(output.contains("Weather report:"), "{tag}: {output}");
    }

    // `-q` silences the warning without changing what is rendered.
    let assert = run(&sandbox, &["--lang", "bad-TAG", "-q"])
        .assert()
        .success();
    let notes = String::from_utf8(assert.get_output().stderr.clone()).expect("stderr is UTF-8");
    assert!(!notes.contains("unsupported language"), "{notes}");
}

#[test]
fn the_language_of_a_document_is_the_language_of_its_values() {
    let sandbox = seeded();

    let json = stdout(&sandbox, &["--lang", "zh-CN", "-f", "json"]);
    let document: serde_json::Value = serde_json::from_str(&json).expect("the document is JSON");
    let text = document
        .pointer("/current/condition/text")
        .and_then(serde_json::Value::as_str)
        .expect("the current condition carries its text");
    assert_eq!(text, "晴", "{json}");
    assert_eq!(
        document
            .pointer("/schema_version")
            .and_then(serde_json::Value::as_u64),
        Some(1),
        "translation never touches the machine-readable keys"
    );

    let plain = stdout(&sandbox, &["--lang", "zh-CN", "-f", "plain"]);
    assert!(
        plain.lines().any(|line| line.starts_with("地点: ")),
        "{plain}"
    );
    assert!(plain.contains("当前: "), "{plain}");
    assert!(plain.contains("逐日 2026-07-15: 早上 "), "{plain}");

    let one_line = stdout(&sandbox, &["--lang", "zh-CN", "-f", "one-line"]);
    assert!(one_line.contains("晴"), "{one_line}");
    assert!(one_line.contains("风"), "{one_line}");
}

#[test]
fn the_configuration_can_pick_a_language_and_a_flag_overrides_it() {
    let sandbox = seeded();
    sandbox.write_config(
        "schema_version = 1\n\
         [defaults]\n\
         language = \"zh-CN\"\n",
    );
    let configured = stdout(&sandbox, &[]);
    assert!(configured.contains("天气报告："), "{configured}");

    let overridden = stdout(&sandbox, &["--lang", "en-US"]);
    assert!(overridden.contains("Weather report:"), "{overridden}");
}

#[test]
fn the_environment_variable_is_honoured_and_named_in_the_decision() {
    let sandbox = seeded();
    let assert = sandbox
        .cirrocast()
        .env("CIRROCAST_LANG", "zh-CN")
        .args(["-v"])
        .arg("--offline")
        .arg(LOCATION)
        .assert()
        .success();
    let output = String::from_utf8(assert.get_output().stdout.clone()).expect("stdout is UTF-8");
    let notes = String::from_utf8(assert.get_output().stderr.clone()).expect("stderr is UTF-8");
    assert!(output.contains("天气报告："), "{output}");
    assert!(
        notes.contains("language: zh-CN (from the environment)"),
        "{notes}"
    );
}

#[test]
fn a_language_request_costs_no_traffic_even_when_the_cache_is_empty() {
    // `--lang` is resolved before the first request, so an unsupported tag cannot make a run fail
    // on a missing key or a network error: it warns and renders, and the offline miss is what
    // stops the run, not the language.
    let sandbox = Sandbox::new();
    let assert = run(&sandbox, &["--lang", "de-DE"]).assert().failure();
    let notes = String::from_utf8(assert.get_output().stderr.clone()).expect("stderr is UTF-8");
    assert!(notes.contains("unsupported language \"de-DE\""), "{notes}");
    assert!(
        !notes.contains("language `de-DE` has no messages"),
        "an unsupported language is never a usage error: {notes}"
    );
}
