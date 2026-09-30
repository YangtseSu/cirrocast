// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Snapshot coverage for the `art-table` renderer, plus the two invariants a snapshot cannot
//! express: that colour never changes the layout, and that the ambient locale never changes the
//! language of the output.
//!
//! Every snapshot is rendered from a hand-written `tests/fixtures/report/*.json` document with a
//! fixed instant taken from the fixture itself, so nothing here depends on the network or on the
//! clock. Snapshots are committed and reviewed by eye; CI runs with `INSTA_UPDATE=no`, so a
//! layout change shows up as a failing test instead of a silently rewritten file.

mod common;

use std::sync::Arc;
use std::time::Duration;

use cirrocast::cache::{Cache, CacheKey, CacheMode, SystemClock};
use cirrocast::config::UnitOverrides;
use cirrocast::i18n::{I18n, LanguageId, LanguageRequest};
use cirrocast::model::Report;
use cirrocast::model::condition::Condition;
use cirrocast::model::units::UnitSystem;
use cirrocast::paths::Paths;
use cirrocast::render::art::{art, night_variant, one_line_art};
use cirrocast::render::{ColorMode, Format, RenderContext, TermCaps, renderer_for};

/// The English catalog, loaded the way the CLI loads an unconfigured run.
fn english() -> I18n {
    I18n::load(&LanguageRequest::Auto, |_| None)
}

/// One rendered configuration.
#[derive(Clone, Copy)]
struct Case {
    file: &'static str,
    units: UnitSystem,
    width: usize,
    format: Format,
    color: ColorMode,
}

/// The `art-table` case of a fixture: metric or us, one width, no colour.
const fn case(file: &'static str, units: UnitSystem, width: usize) -> Case {
    Case {
        file,
        units,
        width,
        format: Format::ArtTable,
        color: ColorMode::Never,
    }
}

/// A terminal that can do everything the renderer offers: UTF-8, a tty, 256 colours.
fn capable_terminal() -> TermCaps {
    TermCaps::read(
        |name| match name {
            "TERM" => Some("xterm-256color".to_owned()),
            "LANG" => Some("en_US.UTF-8".to_owned()),
            _ => None,
        },
        true,
    )
}

/// Renders a case through the same entry point the CLI uses.
fn render(case: Case) -> String {
    let report = common::fixture_report(case.file);
    let i18n = english();
    let caps = capable_terminal();
    let ctx = RenderContext {
        units: case
            .units
            .resolve(&UnitOverrides::default())
            .expect("the default overrides resolve"),
        color: case.color,
        width: case.width,
        term: caps,
        now: common::fixture_now(&report),
        tz: report.location.tz,
        lang: LanguageId::EN_US,
        i18n: &i18n,
    };
    renderer_for(case.format, &caps, None)
        .expect("the format has a renderer")
        .render(&report, &ctx)
        .expect("the fixture renders")
}

/// Snapshots one case under `name` in the shared `tests/snapshots` directory.
///
/// `prepend_module_to_snapshot` is off so that step 08's `one_line`/`json` snapshots can live in
/// the same directory under names that describe the output rather than the test binary.
macro_rules! snapshot {
    ($name:literal, $case:expr) => {
        snapshot_text!($name, render($case))
    };
}

/// The same for text the test built itself (the art gallery).
macro_rules! snapshot_text {
    ($name:literal, $value:expr) => {
        insta::with_settings!({ prepend_module_to_snapshot => false }, {
            insta::assert_snapshot!($name, $value);
        })
    };
}

#[test]
fn metric_units_at_the_documented_widths() {
    snapshot!(
        "art_table_metric_d1_w80",
        case("beijing-1d.json", UnitSystem::Metric, 80)
    );
    snapshot!(
        "art_table_metric_d3_w80",
        case("beijing-3d-day.json", UnitSystem::Metric, 80)
    );
    snapshot!(
        "art_table_metric_d7_w80",
        case("beijing-7d.json", UnitSystem::Metric, 80)
    );
    snapshot!(
        "art_table_metric_d3_w60",
        case("beijing-3d-day.json", UnitSystem::Metric, 60)
    );
    snapshot!(
        "art_table_metric_d3_w40",
        case("beijing-3d-day.json", UnitSystem::Metric, 40)
    );
}

#[test]
fn us_units_at_the_documented_widths() {
    snapshot!(
        "art_table_us_d1_w80",
        case("beijing-1d.json", UnitSystem::Us, 80)
    );
    snapshot!(
        "art_table_us_d3_w80",
        case("beijing-3d-day.json", UnitSystem::Us, 80)
    );
    snapshot!(
        "art_table_us_d7_w80",
        case("beijing-7d.json", UnitSystem::Us, 80)
    );
    snapshot!(
        "art_table_us_d3_w60",
        case("beijing-3d-day.json", UnitSystem::Us, 60)
    );
    snapshot!(
        "art_table_us_d3_w40",
        case("beijing-3d-day.json", UnitSystem::Us, 40)
    );
}

#[test]
fn colour_is_part_of_the_output() {
    snapshot!(
        "art_table_colour_metric_d3_w80",
        Case {
            color: ColorMode::Always,
            ..case("beijing-3d-day.json", UnitSystem::Metric, 80)
        }
    );
    snapshot!(
        "art_table_colour_us_d1_w80",
        Case {
            color: ColorMode::Always,
            ..case("beijing-1d.json", UnitSystem::Us, 80)
        }
    );
}

#[test]
fn a_night_observation_draws_the_night_blocks() {
    snapshot!(
        "art_table_night_d2_w80",
        case("beijing-night.json", UnitSystem::Metric, 80)
    );
}

#[test]
fn the_dumb_format_is_ascii_without_colour() {
    snapshot!(
        "art_table_dumb_d3_w80",
        Case {
            format: Format::Dumb,
            ..case("beijing-3d-day.json", UnitSystem::Metric, 80)
        }
    );
}

#[test]
fn a_report_without_days_shows_the_current_block_only() {
    snapshot!(
        "art_table_current_only_metric",
        case("current-only.json", UnitSystem::Metric, 80)
    );
    snapshot!(
        "art_table_current_only_us",
        case("current-only.json", UnitSystem::Us, 80)
    );
}

/// Every condition in the corpus, through the real cell path: the eight day fixture carries all
/// twenty-nine described codes plus one undescribed one, so this table is the block corpus seen
/// the way a user sees it — and the only snapshot that exercises the `unknown` block in a cell.
#[test]
fn a_week_of_every_condition() {
    snapshot!(
        "art_table_all_conditions_d8_w80",
        case("all-conditions.json", UnitSystem::Metric, 80)
    );
}

/// Colour is presentation, never layout: stripping the escapes from a coloured render must
/// reproduce the monochrome one byte for byte, over the whole combination matrix.
///
/// This is why the snapshot set carries two coloured cases instead of eighteen: the remaining
/// sixteen combinations would only differ by escape sequences.
#[test]
fn colour_changes_no_layout() {
    let files = ["beijing-1d.json", "beijing-3d-day.json", "beijing-7d.json"];
    for units in [UnitSystem::Metric, UnitSystem::Us] {
        for file in files {
            for width in [40, 60, 80] {
                let mono = render(case(file, units, width));
                let coloured = render(Case {
                    color: ColorMode::Always,
                    ..case(file, units, width)
                });
                assert!(
                    coloured.contains("38;5;"),
                    "colour was requested for {file} at {width} columns"
                );
                assert_eq!(strip_ansi(&coloured), mono, "{file} {units} {width}");
            }
        }
    }
}

/// The text without the `38;5;<n>` colour sequences — the only escapes the renderer emits.
fn strip_ansi(text: &str) -> String {
    let mut plain = String::with_capacity(text.len());
    let mut characters = text.chars();
    while let Some(character) = characters.next() {
        if character == '\u{1b}' {
            for escape in characters.by_ref() {
                if escape == 'm' {
                    break;
                }
            }
            continue;
        }
        plain.push(character);
    }
    plain
}

/// Every block in the corpus, beside its ASCII transcription and its one-line glyph.
///
/// The gallery is the one place the artwork is reviewed as artwork: a snapshot of a single
/// forecast only shows the two or three blocks that forecast happens to use.
#[test]
fn art_gallery() {
    let mut lines: Vec<String> = Vec::new();
    let mut seen: Vec<&str> = Vec::new();
    for code in 0..=99_u8 {
        let key = Condition::from_u8(code).art_key();
        if seen.contains(&key) {
            continue;
        }
        seen.push(key);
        lines.push(key.to_owned());
        gallery_block(&mut lines, key);
        let night = night_variant(key);
        if night != key {
            lines.push(String::new());
            lines.push(night.to_owned());
            gallery_block(&mut lines, night);
        }
        lines.push(String::new());
    }
    snapshot_text!("art_gallery", lines.join("\n"));
}

/// Four unicode lines, the four ASCII lines beside them, and the one-line glyph in between.
fn gallery_block(lines: &mut Vec<String>, key: &str) {
    let block = art(key).expect("every gallery key has a block");
    for (index, (unicode, ascii)) in block.unicode.iter().zip(block.ascii).enumerate() {
        let glyph = if index == 1 { one_line_art(key) } else { "" };
        lines.push(format!("  {unicode:<7} │ {ascii:<7} │ {glyph}"));
    }
}

/// The configuration the locale runs use: a coordinate location (no geocoding), one day, the
/// built-in language spelled out.
const LOCALE_CONFIG: &str = "\
schema_version = 1
[defaults]
provider = \"open-meteo\"
format = \"art-table\"
units = \"metric\"
days = 1
language = \"en-US\"
[location]
default = \"@39.9042,116.4074\"
[cache]
enabled = true
";

/// The output language comes from the configuration, never from `LANG`/`LC_ALL`.
///
/// Three ambient UTF-8 locales — including a Turkish one, whose case rules would break a naive
/// `to_lowercase` — must produce byte-identical output. The fourth run pins the one thing the
/// locale *may* decide: a non-UTF-8 locale falls back to the ASCII table, with the same layout.
#[test]
fn the_output_does_not_depend_on_the_ambient_locale() {
    let sandbox = common::Sandbox::new();
    sandbox.write_config(LOCALE_CONFIG);
    seed_weather_cache(&sandbox);

    let baseline = cli_output(&sandbox, &[("LANG", "C.UTF-8"), ("LC_ALL", "")]);
    assert!(
        baseline.contains("Weather report:"),
        "the offline fixture is served from the cache:\n{baseline}"
    );
    for locale in [
        [("LANG", "zh_CN.UTF-8"), ("LC_ALL", "zh_CN.UTF-8")],
        [("LANG", "en_US.UTF-8"), ("LC_ALL", "tr_TR.UTF-8")],
    ] {
        assert_eq!(
            cli_output(&sandbox, &locale),
            baseline,
            "the ambient locale leaked into the output"
        );
    }

    let ascii = cli_output(&sandbox, &[("LANG", "C"), ("LC_ALL", "C")]);
    assert!(
        ascii.is_ascii(),
        "a non-UTF-8 locale gets the ASCII table:\n{ascii}"
    );
    assert_eq!(
        ascii.lines().count(),
        baseline.lines().count(),
        "the character set must not change the layout"
    );
}

/// Runs the real binary offline over the sandbox cache and returns its stdout.
fn cli_output(sandbox: &common::Sandbox, env: &[(&str, &str)]) -> String {
    let mut command = sandbox.cirrocast();
    for (name, value) in env {
        command.env(name, value);
    }
    let assert = command
        .args(["--offline", "@39.9042,116.4074"])
        .assert()
        .success();
    String::from_utf8(assert.get_output().stdout.clone()).expect("stdout is UTF-8")
}

/// Writes the recorded Open-Meteo response under the key the provider computes for the sandbox
/// coordinate, so `--offline` never has to reach the network.
fn seed_weather_cache(sandbox: &common::Sandbox) {
    let paths = Paths {
        config_dir: sandbox.config_dir(),
        config_file: sandbox.config_file(),
        keys_file: sandbox.keys_file(),
        cache_dir: sandbox.cache_dir(),
        data_dir: sandbox.home().join("data"),
    };
    let cache = Cache::with_root(paths.cache_dir, CacheMode::Normal, Arc::new(SystemClock), 0);
    let body = common::fixture("open_meteo/forecast_beijing_2026-07-15.json");
    let today = chrono::Utc::now().date_naive();
    for date in [today, today - chrono::Duration::days(1)] {
        let key = CacheKey::weather("open-meteo", 39.9042, 116.4074, 1, date);
        cache
            .write(&key, 200, &body, Duration::from_secs(600))
            .expect("the recorded response is written to the sandbox cache");
    }
}

/// The fixture a snapshot renders must be a `Report`; this pins that the loader and the renderer
/// agree on the type as much as on the bytes.
#[test]
fn the_fixtures_load_as_reports() {
    let report: Report = common::fixture_report("beijing-3d-day.json");
    assert_eq!(report.days.len(), 3);
    assert!(report.current.is_some());
    assert_eq!(report.location.tz, chrono_tz::Tz::Asia__Shanghai);
}
