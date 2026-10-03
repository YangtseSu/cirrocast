// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The width invariant of the `art-table` renderer.
//!
//! The table promises that no line it emits is wider than the width it was given, at every width
//! the CLI can resolve (`--width`, then `COLUMNS`, then the terminal, then 80 columns, never below
//! the 20 column minimum) and for every day count the tool offers. This is a property test rather
//! than a snapshot because the failure mode is arithmetic: one padding rule that forgets a wide
//! glyph, one border run that does not match its row.
//!
//! Lines are measured in display columns with the same rule the renderer uses, and the layout's
//! own boundaries — the stacked threshold, the narrow-to-wide cell switch, the two- and
//! three-cell rows — are asserted as well, so a change to a threshold cannot silently move a
//! `--width 59` from one layout into another.

mod common;

use unicode_width::UnicodeWidthStr;

use cirrocast::config::UnitOverrides;
use cirrocast::i18n::{I18n, LanguageRequest};
use cirrocast::model::Report;
use cirrocast::model::units::UnitSystem;
use cirrocast::render::{ColorMode, Format, RenderContext, TermCaps, renderer_for};

/// The English catalog, loaded the way the CLI loads an unconfigured run.
fn english() -> I18n {
    I18n::load(&LanguageRequest::Auto, |_| None)
}

/// The narrowest width the CLI resolves; everything below is raised to it.
const MIN_WIDTH: usize = 20;

/// The width below which the renderer stacks the days instead of joining columns.
const STACKED_BELOW: usize = 60;

/// The width from which a cell is laid out in its wide form.
const WIDE_FROM: usize = 74;

/// Renders `report` at `width` with `format`, in the charset a UTF-8 terminal gets.
fn render_in(report: &Report, width: usize, format: Format) -> String {
    render_with(report, width, format, ColorMode::Never)
}

/// The same, with the colour mode injected.
fn render_with(report: &Report, width: usize, format: Format, color: ColorMode) -> String {
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
        color,
        width,
        term: caps,
        now: common::fixture_now(report),
        tz: report.location.tz,
        lang: i18n.lang(),
        i18n: &i18n,
        alert_credits: &[],
    };
    renderer_for(format, &caps, None)
        .expect("the format has a renderer")
        .render(report, &ctx)
        .expect("the fixture renders")
}

/// Renders `report` at `width` as the default format.
fn render(report: &Report, width: usize) -> String {
    render_in(report, width, Format::ArtTable)
}

/// The report with exactly `days` forecast days.
fn report_with(days: usize) -> Report {
    let mut report = common::fixture_report("beijing-7d.json");
    report.days.truncate(days);
    report
}

#[test]
fn no_line_is_wider_than_the_requested_width() {
    for days in [1_usize, 3, 7] {
        let report = report_with(days);
        for width in MIN_WIDTH..=200 {
            let text = render(&report, width);
            for line in text.lines() {
                assert!(
                    line.width() <= width,
                    "{days} days at {width} columns: {line:?} is {} columns",
                    line.width()
                );
            }
            assert!(
                text.lines().count() > 4,
                "{days} days at {width} columns rendered almost nothing:\n{text}"
            );
        }
    }
}

#[test]
fn the_layout_switches_where_the_constants_say_it_does() {
    let report = report_with(3);

    let stacked = render(&report, STACKED_BELOW - 1);
    assert!(
        !stacked.contains('\u{250c}') && stacked.contains("Morning \u{2502}"),
        "below {STACKED_BELOW} columns the days stack:\n{stacked}"
    );

    let columns = render(&report, STACKED_BELOW);
    assert!(
        columns.contains('\u{250c}'),
        "at {STACKED_BELOW} columns the days are boxed:\n{columns}"
    );

    // Three cells need `3 * (cell + 3) + 1` columns; the wide cell form needs `3 * 24 + 1`.
    let narrow = render(&report, WIDE_FROM - 1);
    let wide = render(&report, WIDE_FROM);
    let border = |text: &str| {
        text.lines()
            .find(|line| line.starts_with('\u{250c}'))
            .map_or(0, UnicodeWidthStr::width)
    };
    assert_eq!(
        border(&narrow),
        3 * (18 + 3) + 1,
        "just below the switch the cells are narrow"
    );
    assert_eq!(
        border(&wide),
        3 * (21 + 3) + 1,
        "from {WIDE_FROM} on they are wide"
    );
}

#[test]
fn every_row_of_a_band_is_as_wide_as_its_border() {
    let report = report_with(7);
    for width in [74, 80, 120, 200] {
        assert_bands_are_square(&render(&report, width), '\u{250c}', '\u{2502}', width, 3);
    }
}

#[test]
fn every_row_of_a_dumb_band_is_as_wide_as_its_border() {
    let report = report_with(7);
    for width in [74, 80, 120, 200] {
        assert_bands_are_square(&render_in(&report, width, Format::Dumb), '+', '|', width, 3);
    }
}

/// Every section of `text` that looks like a band must have one width for all of its rows — the
/// failure this catches is a metric that was measured before the character set folded it.
fn assert_bands_are_square(text: &str, border: char, row: char, width: usize, expected: usize) {
    let mut bands = 0;
    for section in text.split("\n\n") {
        let rows: Vec<usize> = section
            .lines()
            .filter(|line| line.starts_with(border) || line.starts_with(row))
            .map(UnicodeWidthStr::width)
            .collect();
        if rows.len() < 3 {
            continue;
        }
        bands += 1;
        assert!(
            rows.iter().all(|line| *line == rows[0]),
            "a band is ragged at {width} columns: {rows:?}\n{section}"
        );
    }
    assert_eq!(bands, expected, "at {width} columns:\n{text}");
}

#[test]
fn a_report_without_days_is_only_a_header_and_current_conditions() {
    let report = common::fixture_report("current-only.json");
    let text = render(&report, 80);
    assert!(
        text.starts_with("Weather report: "),
        "an empty render must not pass this test:\n{text}"
    );
    let condition = report
        .current
        .as_ref()
        .expect("the fixture has current conditions")
        .weather
        .description_en();
    assert!(
        text.contains(condition),
        "the current conditions are missing:\n{text}"
    );
    assert!(!text.contains('\u{250c}'), "{text}");
    assert!(text.lines().count() >= 3, "{text}");
    assert!(text.lines().count() <= 10, "{text}");
    for line in text.lines() {
        assert!(line.width() <= 80, "{line:?}");
    }
}

#[test]
fn the_dumb_table_is_ascii_at_every_width() {
    let report = report_with(3);
    for width in [MIN_WIDTH, 33, 59, 60, 80, 160] {
        let text = render_in(&report, width, Format::Dumb);
        assert!(text.is_ascii(), "width {width}:\n{text}");
        for line in text.lines() {
            assert!(line.width() <= width, "width {width}: {line:?}");
        }
    }
}

/// Colour is presentation, never layout, at every width.
///
/// The escape bookkeeping is strict — every SGR open is matched by an SGR reset — because a
/// lenient strip cannot see a missing reset, and the width invariant would hide it.
#[test]
fn colour_never_changes_the_width_or_leaves_a_colour_open() {
    let report = report_with(7);
    for width in [
        MIN_WIDTH,
        21,
        36,
        40,
        STACKED_BELOW - 1,
        STACKED_BELOW,
        WIDE_FROM,
        80,
        200,
    ] {
        let coloured = render_with(&report, width, Format::ArtTable, ColorMode::Always);
        assert!(
            coloured.contains("38;5;"),
            "colour was requested at {width} columns"
        );
        for line in coloured.lines() {
            let plain = strip_sgr(line);
            assert!(
                plain.width() <= width,
                "width {width}: {line:?} is {} columns",
                plain.width()
            );
            assert_eq!(
                line.matches("\u{1b}[38;5;").count(),
                line.matches("\u{1b}[0m").count(),
                "a colour is left open at {width} columns: {line:?}"
            );
        }
        assert_eq!(
            strip_sgr(&coloured),
            render(&report, width),
            "colour changed the layout at {width} columns"
        );
    }
}

/// The text without its `38;5;<n>` colour sequences and their resets — the escapes `paint` emits.
fn strip_sgr(text: &str) -> String {
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
