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
use cirrocast::i18n::{I18n, LanguageRequest};
use cirrocast::model::Report;
use cirrocast::model::condition::Condition;
use cirrocast::model::units::UnitSystem;
use cirrocast::paths::Paths;
use cirrocast::render::art::{art, night_variant, one_line_art};
use cirrocast::render::{ColorMode, Format, RenderContext, TermCaps, renderer_for};
use unicode_width::UnicodeWidthChar as _;

/// The English catalog, loaded the way the CLI loads an unconfigured run.
fn english() -> I18n {
    I18n::load(&LanguageRequest::Auto, |_| None)
}

/// A catalog for one language tag, for the locale runs.
fn catalog(tag: &str) -> I18n {
    I18n::load(&LanguageRequest::Tag(tag.to_owned()), |_| None)
}

/// One rendered configuration.
#[derive(Clone, Copy)]
struct Case {
    file: &'static str,
    units: UnitSystem,
    width: usize,
    format: Format,
    color: ColorMode,
    lang: &'static str,
}

/// The `art-table` case of a fixture: metric or us, one width, no colour, English.
const fn case(file: &'static str, units: UnitSystem, width: usize) -> Case {
    Case {
        file,
        units,
        width,
        format: Format::ArtTable,
        color: ColorMode::Never,
        lang: "en-US",
    }
}

/// The same case in another language.
const fn translated(case: Case, lang: &'static str) -> Case {
    Case { lang, ..case }
}

/// The `plain` case of a fixture.
const fn plain(file: &'static str, units: UnitSystem, width: usize) -> Case {
    Case {
        file,
        units,
        width,
        format: Format::Plain,
        color: ColorMode::Never,
        lang: "en-US",
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
    let i18n = match case.lang {
        "en-US" => english(),
        tag => catalog(tag),
    };
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
        lang: i18n.lang(),
        i18n: &i18n,
        alert_credits: &[],
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

/// The Chinese table: the same layout, every label and condition from the `zh-CN` catalog.
///
/// Chinese is the width test the English snapshots cannot be: its labels are two double-width
/// glyphs where English has five or six single-width ones, so a border that is one column off
/// shows up here and nowhere else. Both layouts are covered — the columns at 80 and the stacked
/// form at 40, where a part is a line rather than a cell.
#[test]
fn the_chinese_table_keeps_its_borders_aligned() {
    let three_days = translated(case("beijing-3d-day.json", UnitSystem::Metric, 80), "zh-CN");
    let stacked = translated(case("beijing-3d-day.json", UnitSystem::Metric, 40), "zh-CN");
    let night = translated(case("beijing-night.json", UnitSystem::Metric, 80), "zh-CN");
    snapshot!("art_table_zh_metric_d3_w80", three_days);
    snapshot!("art_table_zh_metric_d3_w40", stacked);
    snapshot!("art_table_zh_night_d2_w80", night);
}

/// The Chinese table obeys the width invariant as strictly as the English one.
///
/// The rows are built from localized strings of different display widths, so this walks the same
/// widths the English property test does and checks every line of every width.
#[test]
fn the_chinese_table_never_exceeds_its_width() {
    for width in [20, 32, 40, 59, 60, 80, 120] {
        let text = render(translated(
            case("beijing-7d.json", UnitSystem::Metric, width),
            "zh-CN",
        ));
        for line in text.lines() {
            let columns = line
                .chars()
                .map(|character| character.width().unwrap_or(0))
                .sum::<usize>();
            assert!(
                columns <= width,
                "width {width}: {line:?} is {columns} columns"
            );
        }
        // The first line is the localized header in every layout, truncated or not.
        assert!(
            text.lines()
                .next()
                .is_some_and(|line| line.starts_with("天气报告：")),
            "width {width}:\n{text}"
        );
    }
}

/// The `plain` records in Chinese: the values are translated, the shape of a record is not.
#[test]
fn the_chinese_plain_records_keep_their_shape() {
    let text = render(translated(
        plain("beijing-3d-day.json", UnitSystem::Metric, 80),
        "zh-CN",
    ));
    let lines: Vec<&str> = text.lines().collect();
    assert!(lines[0].starts_with("地点: "), "{text}");
    assert!(lines[1].starts_with("更新: 2026-09-30T"), "{text}");
    assert!(lines[2].starts_with("当前: 多云 "), "{text}");
    assert!(lines[3].starts_with("逐日 2026-09-30: 早上 "), "{text}");
    assert!(text.contains("数据： Open-Meteo.com"), "{text}");
    insta::with_settings!({ prepend_module_to_snapshot => false }, {
        insta::assert_snapshot!("plain_zh_metric_d3", text);
    });
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

/// Every file in `tests/fixtures/report/` loads as a [`Report`].
///
/// This is the real invariant behind the fixture set: the loader and the renderer share the type,
/// and a fixture added for a new layout cannot rot half-parsed without failing here.
#[test]
fn every_report_fixture_loads_as_a_report() {
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/report");
    let mut names: Vec<String> = std::fs::read_dir(&directory)
        .unwrap_or_else(|error| panic!("{}: {error}", directory.display()))
        .map(|entry| {
            entry
                .expect("a readable fixture directory entry")
                .file_name()
        })
        .filter_map(|name| name.to_str().map(str::to_owned))
        .filter(|name| {
            std::path::Path::new(name)
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
        })
        .collect();
    names.sort_unstable();
    assert!(
        !names.is_empty(),
        "{} holds no report fixtures",
        directory.display()
    );

    for name in &names {
        let report: Report = common::fixture_report(name);
        assert!(
            !report.location.name.is_empty(),
            "{name} loaded as a report with no place"
        );
    }
}
