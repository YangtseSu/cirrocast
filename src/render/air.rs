// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The air-quality panel: one block shared by the table formats, the `plain` records and the
//! standalone `--format aqi` view.
//!
//! The panel is built from the model alone (`Report::air`), so a renderer never knows whether the
//! reading came from the network, the cache or a test fixture. Its shape:
//!
//! ```text
//! Air quality: US AQI 43 (Good) · European AQI 42 (Good)
//! PM2.5 8.2 · PM10 13.3 · O3 38 · NO2 27.9 · SO2 3 · CO 251 μg/m³
//! Pollen: alder 0 · birch 0 · grass 0 · mugwort 0 · olive 0 · ragweed 0 grains/m³
//! UV 5 (moderate) · weather data
//! Air quality data by Open-Meteo.com (CAMS ENSEMBLE)
//! ```
//!
//! * The **selected scale** (`--aqi-index` / `[air] index`) is painted with the category ramp and
//!   drives the one-line `%q` token; the other scale is shown as a plain number, because two
//!   coloured scales in one line would read as one index.
//! * Every line is wrapped to the resolved width, so the four-to-five line form appears from
//!   roughly 80 columns on. Below [`STACKED_BELOW`] columns the panel switches to one key per
//!   line instead of wrapping a dense one.
//! * `--format dumb`/ASCII terminals get the same panel through [`super::art_table::fold_ascii`]:
//!   the middle dot becomes `.` and `μg/m³` reads `ug/m3`.
//! * The credits travel with the data: the Open-Meteo line sits in the panel (and therefore in
//!   `art-table`'s footer area and in the standalone view), and `plain` keeps it as a document
//!   line like the other licences.

use std::borrow::Cow;

use chrono::SecondsFormat;

use super::art_table::{display_width, fit, fold_ascii};
use super::color;
use super::{Charset, ColorDepth, RenderContext, Renderer};
use crate::air::aqi::{AqiCategory, AqiIndex};
use crate::error::Result;
use crate::geo::location_line;
use crate::i18n::{MessageKey, keys};
use crate::model::units::fmt_int;
use crate::model::{AirQuality, AirSource, Report};

/// Below this width the panel is one key per line instead of a wrapped compact form.
const STACKED_BELOW: usize = 60;

/// The `--format aqi` renderer: the standalone air-quality view.
///
/// Without a reading — the run asked, the fetch degraded — it prints the `air quality unavailable`
/// line rather than an empty document, so a script reading stdout still learns what happened.
#[derive(Debug, Clone, Copy, Default)]
pub struct Air;

impl Renderer for Air {
    fn render(&self, report: &Report, ctx: &RenderContext<'_>) -> Result<String> {
        let charset = ctx.term.charset();
        let Some(air) = report.air.as_ref() else {
            return Ok(fold_line(&ctx.i18n.text(&keys::AQI_UNAVAILABLE), charset));
        };
        let mut lines = vec![location_line(&report.location), observed_line(air, ctx)];
        lines.extend(panel_lines(report, ctx, ctx.depth()));
        // Fold to ASCII first: folding can widen a line (`—` becomes `--`), so wrapping afterwards
        // is what keeps the width invariant for the header and the body alike.
        let folded = lines
            .into_iter()
            .map(|line| fold_line(&line, charset))
            .collect();
        Ok(wrap_all(folded, ctx.width, charset).join("\n"))
    }
}

/// The panel block, wrapped to `ctx.width`; empty when the report carries no reading.
///
/// `depth` is the palette the calling renderer may emit with (`dumb` forces mono even when
/// `--color always` is set).
#[must_use]
pub fn panel(report: &Report, ctx: &RenderContext<'_>, depth: ColorDepth) -> Vec<String> {
    let charset = ctx.term.charset();
    let lines = panel_lines(report, ctx, depth)
        .into_iter()
        .map(|line| fold_line(&line, charset))
        .collect();
    wrap_all(lines, ctx.width, charset)
}

/// The panel block before folding and wrapping; empty when the report carries no reading.
fn panel_lines(report: &Report, ctx: &RenderContext<'_>, depth: ColorDepth) -> Vec<String> {
    let Some(air) = report.air.as_ref() else {
        return Vec::new();
    };
    if ctx.width < STACKED_BELOW {
        stacked(report, air, ctx, depth)
    } else {
        compact(report, air, ctx, depth)
    }
}

/// The `plain` format's records: one greppable line per value group, the source credit last.
///
/// `plain` ignores the width by contract, so the records are not wrapped.
#[must_use]
pub fn records(report: &Report, ctx: &RenderContext<'_>) -> Vec<String> {
    let Some(air) = report.air.as_ref() else {
        return Vec::new();
    };
    let mut lines = Vec::new();
    let indices: Vec<String> = AqiIndex::ALL
        .into_iter()
        .filter_map(|index| index_entry(air, index, ctx, ColorDepth::Mono))
        .collect();
    if !indices.is_empty() {
        lines.push(format!(
            "{} {}",
            super::plain::record_key(&ctx.i18n.text(&keys::AQI_PANEL_TITLE)),
            indices.join(" · ")
        ));
    }
    for (key, value) in pollutants(air) {
        if let Some(value) = value {
            lines.push(format!(
                "{} {} {}",
                super::plain::record_key(&ctx.i18n.text(&key)),
                number(value),
                ctx.i18n.text(&keys::UNIT_UG_M3)
            ));
        }
    }
    lines.push(format!(
        "{} {}",
        super::plain::record_key(&ctx.i18n.text(&keys::AQI_POLLEN_TITLE)),
        pollen_body(air, ctx)
    ));
    if let Some(line) = uv_line(report, ctx) {
        lines.push(format!(
            "{} {line}",
            super::plain::record_key(&ctx.i18n.text(&keys::AQI_UV_LABEL))
        ));
    }
    lines.push(ctx.i18n.text(&credit_key(air.source)).into_owned());
    lines
}

// ---------------------------------------------------------------------------------------------
// The two layouts
// ---------------------------------------------------------------------------------------------

/// The compact form: title + indices, pollutants, pollen, UV, credit.
fn compact(
    report: &Report,
    air: &AirQuality,
    ctx: &RenderContext<'_>,
    depth: ColorDepth,
) -> Vec<String> {
    let mut lines = vec![index_line(air, ctx, depth)];
    if let Some(line) = pollutant_line(air, ctx) {
        lines.push(line);
    }
    lines.push(format!(
        "{}: {}",
        ctx.i18n.text(&keys::AQI_POLLEN_TITLE),
        pollen_body(air, ctx)
    ));
    if let Some(line) = uv_line(report, ctx) {
        lines.push(line);
    }
    lines.push(ctx.i18n.text(&credit_key(air.source)).into_owned());
    lines
}

/// The stacked form for narrow terminals: the title, then one key per line.
fn stacked(
    report: &Report,
    air: &AirQuality,
    ctx: &RenderContext<'_>,
    depth: ColorDepth,
) -> Vec<String> {
    let mut lines = vec![ctx.i18n.text(&keys::AQI_PANEL_TITLE).into_owned()];
    for index in AqiIndex::ALL {
        if let Some(entry) = index_entry(air, index, ctx, depth) {
            lines.push(entry);
        }
    }
    for (key, value) in pollutants(air) {
        if let Some(value) = value {
            lines.push(format!(
                "{}: {} {}",
                ctx.i18n.text(&key),
                number(value),
                ctx.i18n.text(&keys::UNIT_UG_M3)
            ));
        }
    }
    lines.push(ctx.i18n.text(&keys::AQI_POLLEN_TITLE).into_owned());
    match &air.pollen {
        Some(pollen) => {
            let mut any = false;
            for (key, value) in pollen_species(pollen) {
                if let Some(value) = value {
                    any = true;
                    lines.push(format!(
                        "{}: {} {}",
                        ctx.i18n.text(&key),
                        number(value),
                        ctx.i18n.text(&keys::UNIT_GRAINS_M3)
                    ));
                }
            }
            if !any {
                lines.push(ctx.i18n.text(&keys::AQI_NO_COVERAGE).into_owned());
            }
        }
        None => lines.push(ctx.i18n.text(&keys::AQI_NO_COVERAGE).into_owned()),
    }
    if let Some(line) = uv_line(report, ctx) {
        lines.push(line);
    }
    lines.push(ctx.i18n.text(&credit_key(air.source)).into_owned());
    lines
}

// ---------------------------------------------------------------------------------------------
// Line pieces
// ---------------------------------------------------------------------------------------------

/// `Air quality: US AQI 43 (Good) · European AQI 42 (Good)`.
fn index_line(air: &AirQuality, ctx: &RenderContext<'_>, depth: ColorDepth) -> String {
    let entries: Vec<String> = AqiIndex::ALL
        .into_iter()
        .filter_map(|index| index_entry(air, index, ctx, depth))
        .collect();
    format!(
        "{}: {}",
        ctx.i18n.text(&keys::AQI_PANEL_TITLE),
        entries.join(" · ")
    )
}

/// One scale's `US AQI 43 (Good)`, painted with the category ramp when it is the selected scale.
fn index_entry(
    air: &AirQuality,
    index: AqiIndex,
    ctx: &RenderContext<'_>,
    depth: ColorDepth,
) -> Option<String> {
    let (value, category) = match index {
        AqiIndex::Us => (air.aqi_us, air.aqi_us.map(AqiCategory::from_us)),
        AqiIndex::European => (
            air.aqi_european,
            air.aqi_european.map(AqiCategory::from_european),
        ),
    };
    let value = value?;
    let label = ctx.i18n.text(&index.label_key());
    let Some(category) = category else {
        return Some(format!("{label} {value}"));
    };
    let word = ctx.i18n.text(&category.i18n_key());
    let entry = format!("{value} ({word})");
    // Only the selected scale is painted: the other index is context, not the answer.
    let painted: Cow<'_, str> = if index == ctx.aqi_index {
        color::paint(&entry, color::aqi_fg(category), depth)
    } else {
        Cow::Borrowed(&entry)
    };
    Some(format!("{label} {painted}"))
}

/// `PM2.5 8.2 · PM10 13.3 · O3 38 · NO2 27.9 · SO2 3 · CO 251 μg/m³`, or `None` when the source
/// reported no pollutant at all.
fn pollutant_line(air: &AirQuality, ctx: &RenderContext<'_>) -> Option<String> {
    let parts: Vec<String> = pollutants(air)
        .into_iter()
        .filter_map(|(key, value)| {
            value.map(|value| format!("{} {}", ctx.i18n.text(&key), number(value)))
        })
        .collect();
    if parts.is_empty() {
        return None;
    }
    Some(format!(
        "{} {}",
        parts.join(" · "),
        ctx.i18n.text(&keys::UNIT_UG_M3)
    ))
}

/// The pollen half of a `Pollen`-carrying reading: `alder 0 · … · ragweed 0 grains/m³`; the
/// no-coverage answer otherwise. A species the source did not report is omitted, never shown as
/// `0`.
fn pollen_body(air: &AirQuality, ctx: &RenderContext<'_>) -> String {
    let Some(pollen) = &air.pollen else {
        return ctx.i18n.text(&keys::AQI_NO_COVERAGE).into_owned();
    };
    let entries: Vec<String> = pollen_species(pollen)
        .into_iter()
        .filter_map(|(key, value)| {
            value.map(|value| format!("{} {}", ctx.i18n.text(&key), number(value)))
        })
        .collect();
    if entries.is_empty() {
        return ctx.i18n.text(&keys::AQI_NO_COVERAGE).into_owned();
    }
    format!(
        "{} {}",
        entries.join(" · "),
        ctx.i18n.text(&keys::UNIT_GRAINS_M3)
    )
}

/// `UV 5 (moderate) · weather data`, or `None` when the weather report carries no UV reading.
fn uv_line(report: &Report, ctx: &RenderContext<'_>) -> Option<String> {
    let uv = report.current.as_ref()?.uv_index?;
    let band = ctx.i18n.uv_band(uv).into_owned();
    let value = ctx
        .i18n
        .format(
            &keys::FORMAT_UV,
            &[
                ("value", fluent_bundle::FluentValue::from(fmt_int(uv))),
                ("band", fluent_bundle::FluentValue::from(band)),
            ],
        )
        .into_owned();
    Some(format!(
        "{} {value} · {}",
        ctx.i18n.text(&keys::AQI_UV_LABEL),
        ctx.i18n.text(&keys::AQI_UV_SOURCE)
    ))
}

/// `updated 2026-10-03T20:00:00+02:00 · Open-Meteo`, the standalone view's provenance line.
fn observed_line(air: &AirQuality, ctx: &RenderContext<'_>) -> String {
    format!(
        "{} {} · {}",
        ctx.i18n.text(&keys::LABEL_UPDATED),
        air.time.to_rfc3339_opts(SecondsFormat::Secs, false),
        air.source.display_name()
    )
}

/// The six pollutants with their catalog labels, in panel order.
fn pollutants(air: &AirQuality) -> [(MessageKey, Option<f64>); 6] {
    [
        (keys::POLLUTANTS[0], air.pm2_5),
        (keys::POLLUTANTS[1], air.pm10),
        (keys::POLLUTANTS[2], air.o3),
        (keys::POLLUTANTS[3], air.no2),
        (keys::POLLUTANTS[4], air.so2),
        (keys::POLLUTANTS[5], air.co),
    ]
}

/// The six pollen species with their catalog labels, in `Pollen::SPECIES` order; a `None` member
/// was not measured and the callers omit it.
fn pollen_species(pollen: &crate::model::Pollen) -> [(MessageKey, Option<f64>); 6] {
    [
        (keys::POLLEN_SPECIES[0], pollen.alder),
        (keys::POLLEN_SPECIES[1], pollen.birch),
        (keys::POLLEN_SPECIES[2], pollen.grass),
        (keys::POLLEN_SPECIES[3], pollen.mugwort),
        (keys::POLLEN_SPECIES[4], pollen.olive),
        (keys::POLLEN_SPECIES[5], pollen.ragweed),
    ]
}

// ---------------------------------------------------------------------------------------------
// Formatting helpers
// ---------------------------------------------------------------------------------------------

/// One decimal at most, without a trailing `.0` and without a negative zero.
///
/// The panel carries six values per line; `8.2`, `13.3`, `38`, `27.9`, `3`, `251` is what keeps the
/// line a line. Anything smaller than 0.05 rounds to `0`.
fn number(value: f64) -> String {
    let rounded = crate::model::units::normalise_zero_f64((value * 10.0).round() / 10.0);
    let integral = rounded.round();
    if (rounded - integral).abs() < 0.05 {
        format!("{integral:.0}")
    } else {
        format!("{rounded:.1}")
    }
}

/// The catalog key for one air source's credit line.
fn credit_key(source: AirSource) -> MessageKey {
    match source {
        AirSource::OpenMeteo => keys::AIR_CREDIT_OPEN_METEO,
    }
}

/// Wraps each line to `width` display columns at spaces, clipping a single word that cannot fit.
///
/// Escape sequences take no display columns (the shared [`display_width`]). A break inside a
/// painted span closes the span at the end of the line and re-opens it at the start of the next,
/// so no line is left with an unterminated SGR — which would paint the rest of the terminal — and
/// a value split across two lines stays painted on both.
fn wrap_all(lines: Vec<String>, width: usize, charset: Charset) -> Vec<String> {
    lines
        .into_iter()
        .flat_map(|line| wrap(&line, width, charset))
        .collect()
}

/// [`wrap_all`] for one line.
fn wrap(line: &str, width: usize, charset: Charset) -> Vec<String> {
    if display_width(line) <= width {
        return vec![line.to_owned()];
    }
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in line.split(' ') {
        let word_width = display_width(word);
        let separator = usize::from(!current.is_empty());
        if word_width <= width && display_width(&current) + separator + word_width <= width {
            if separator == 1 {
                current.push(' ');
            }
            current.push_str(word);
            continue;
        }
        // The word does not fit: end the line, closing any paint it left open and re-opening it on
        // the next line.
        if !current.is_empty() {
            let reopen = open_escape(&current).map(str::to_owned);
            if reopen.is_some() {
                current.push_str("\u{1b}[0m");
            }
            lines.push(std::mem::take(&mut current));
            if let Some(sequence) = &reopen {
                current.push_str(sequence);
            }
        }
        if word_width > width {
            // A single word wider than the whole width is clipped on a line of its own; `fit`
            // balances that word's own escapes.
            current.clear();
            lines.push(fit(word, width, charset).into_owned());
        } else {
            current.push_str(word);
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

/// The SGR sequence left open at the end of `text`, if any: the last `\u{1b}[…m` sequence that is
/// not a reset.
fn open_escape(text: &str) -> Option<&str> {
    let mut open = None;
    let mut rest = text;
    while let Some(start) = rest.find('\u{1b}') {
        let tail = &rest[start..];
        let Some(end) = tail.find('m') else {
            break;
        };
        let sequence = &tail[..=end];
        open = if sequence == "\u{1b}[0m" || sequence == "\u{1b}[m" {
            None
        } else {
            Some(sequence)
        };
        rest = &tail[end + 1..];
    }
    open
}

/// The ASCII spelling of one line when the charset asks for it.
fn fold_line(line: &str, charset: Charset) -> String {
    match charset {
        Charset::Ascii => fold_ascii(line),
        Charset::Unicode => line.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        display_width, fold_ascii, fold_line, number, open_escape, pollen_body, wrap, wrap_all,
    };
    use crate::render::Charset;

    #[test]
    fn folding_before_wrapping_keeps_a_line_within_the_width() {
        // `fold_ascii` widens `—` to `--`, so wrapping first and folding afterwards can push a
        // line past the width; the standalone view and the panel fold first.
        let line = "Location data based on GeoNames — https://www.geonames.org/".to_owned();
        let width = 33;
        let folded = fold_line(&line, Charset::Ascii);
        for part in wrap_all(vec![folded], width, Charset::Ascii) {
            assert!(display_width(&part) <= width, "{part:?} exceeds {width}");
        }
        // The inverse order (which the view used before the fix) widens a clipped line.
        let overflow = wrap_all(vec![line], width, Charset::Unicode)
            .into_iter()
            .any(|part| display_width(&fold_ascii(&part)) > width);
        assert!(
            overflow,
            "the fixture exposes the old order's overflow at {width}"
        );
    }

    /// Drops every SGR sequence, so the visible text of two spellings can be compared.
    fn strip_escapes(text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        let mut rest = text;
        while let Some(start) = rest.find('\u{1b}') {
            out.push_str(&rest[..start]);
            let tail = &rest[start..];
            if let Some(end) = tail.find('m') {
                rest = &tail[end + 1..];
            } else {
                rest = "";
                break;
            }
        }
        out.push_str(rest);
        out
    }

    #[test]
    fn numbers_keep_one_decimal_at_most_and_never_a_negative_zero() {
        assert_eq!(number(0.0), "0");
        assert_eq!(number(-0.0), "0");
        assert_eq!(number(0.04), "0");
        assert_eq!(number(8.2), "8.2");
        assert_eq!(number(13.34), "13.3");
        assert_eq!(number(38.0), "38");
        assert_eq!(number(251.0), "251");
    }

    #[test]
    fn a_line_wraps_at_spaces_and_never_exceeds_the_width() {
        for width in [20_usize, 30, 40, 60] {
            for line in [
                "Air quality: US AQI 43 (Good) · European AQI 42 (Good)",
                "PM2.5 8.2 · PM10 13.3 · O3 38 · NO2 27.9 · SO2 3 · CO 251 μg/m³",
                "Air quality data by Open-Meteo.com (CAMS ENSEMBLE)",
            ] {
                let parts = wrap(line, width, Charset::Unicode);
                for part in &parts {
                    assert!(display_width(part) <= width, "{part:?} exceeds {width}");
                }
                assert_eq!(
                    parts.join(" "),
                    line,
                    "wrapping loses or reorders nothing at {width}"
                );
            }
            // A single word wider than the width is clipped by `fit`, so it cannot be rejoined.
            let long = "supercalifragilisticexpialidocious";
            for part in wrap(long, width, Charset::Unicode) {
                assert!(display_width(&part) <= width, "{part:?} exceeds {width}");
            }
        }
    }

    #[test]
    fn a_wrap_inside_a_painted_value_leaves_every_line_balanced() {
        // `index_entry` paints `43 (Good)`, a value that contains the space the wrap splits on, so
        // a naive wrap can push the closing `\x1b[0m` onto the next line and leave the SGR open.
        let line =
            "Air quality: US AQI \u{1b}[38;5;226m43 (Good)\u{1b}[0m · European AQI 42 (Good)";
        assert!(
            display_width(line) > 28,
            "the fixture must wrap at these widths"
        );
        for width in 22..=28 {
            let parts = wrap(line, width, Charset::Unicode);
            for part in &parts {
                assert!(display_width(part) <= width, "{part:?} exceeds {width}");
                assert_eq!(
                    open_escape(part),
                    None,
                    "{part:?} leaves an SGR open at {width}"
                );
            }
            let visible: Vec<String> = parts.iter().map(|part| strip_escapes(part)).collect();
            assert_eq!(
                visible.join(" "),
                strip_escapes(line),
                "the visible text survives the wrap at {width}"
            );
        }
    }

    #[test]
    fn an_unmeasured_pollen_species_is_omitted_not_zeroed() {
        use crate::i18n::{I18n, LanguageRequest};
        use crate::model::units::UnitSystem;
        use crate::model::{AirQuality, AirSource, LocalTimes, Pollen};
        use crate::render::{ColorMode, RenderContext, TermCaps};

        let i18n = I18n::load(&LanguageRequest::Tag("en-US".to_owned()), |_| None);
        let times = LocalTimes::new(
            chrono::DateTime::parse_from_rfc3339("2026-10-03T18:00:00Z").expect("an instant"),
            chrono_tz::Tz::Europe__Berlin,
        );
        let ctx = RenderContext {
            units: UnitSystem::Metric
                .resolve(&crate::config::UnitOverrides::default())
                .expect("the default overrides resolve"),
            color: ColorMode::Never,
            width: 80,
            term: TermCaps::default(),
            times,
            lang: i18n.lang(),
            i18n: &i18n,
            alert_credits: &[],
            aqi_index: crate::air::aqi::AqiIndex::Us,
        };
        let air = AirQuality {
            time: chrono::DateTime::parse_from_rfc3339("2026-10-03T20:00:00+02:00")
                .expect("a valid instant"),
            aqi_us: Some(43),
            aqi_european: Some(42),
            pm2_5: None,
            pm10: None,
            o3: None,
            no2: None,
            so2: None,
            co: None,
            pollen: Some(Pollen {
                alder: Some(1.2),
                birch: None,
                grass: Some(4.5),
                mugwort: None,
                olive: None,
                ragweed: Some(0.0),
            }),
            source: AirSource::OpenMeteo,
        };
        let body = pollen_body(&air, &ctx);
        assert!(body.contains("alder 1.2"), "{body}");
        assert!(body.contains("grass 4.5"), "{body}");
        assert!(body.contains("ragweed 0"), "a measured zero prints: {body}");
        for unmeasured in ["birch", "mugwort", "olive"] {
            assert!(
                !body.contains(unmeasured),
                "`{unmeasured}` was not measured and must be omitted: {body}"
            );
        }
    }
}
