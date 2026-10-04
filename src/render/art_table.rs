// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The `art-table` renderer: the wttr.in-style coloured table.
//!
//! # Layout
//!
//! ```text
//! Weather report: Beijing, Beijing, China (39.90, 116.41)
//!
//!    \│/    Partly cloudy
//!   ─(●)─   +22°C (+23°C)
//!    /│\    ↗ 12km/h NE
//!           56% 1013hPa 10km 0.0mm
//!
//! ┌───────────────────┬───────────────────┐
//! │ Today, Sep 30     │ Wed 01 Oct        │
//! ├───────────────────┼───────────────────┤
//! │   \│/    Morning  │   \│/    Morning  │
//! │  ─(●)─   +22°C (+23°C)                │
//! │   /│\    ↗ 12km/h NE                  │
//! │          0.0mm 56%                    │
//! ├───────────────────┼───────────────────┤
//! │  … Noon, Evening and Night follow …   │
//! └───────────────────┴───────────────────┘
//! ```
//!
//! # The day-part cell contract
//!
//! Every day part is exactly [`ART_LINES`] lines. Line `k` carries the art block's line `k` in the
//! first [`ART_W`] columns, then [`GAP`] spaces, then one metric padded to [`METRICS_W`] columns
//! ([`METRICS_W_NARROW`] when three cells have to share a narrow terminal):
//!
//! | line | metric |
//! |---|---|
//! | 0 | the localized part label (`Morning`) |
//! | 1 | `+22°C (+23°C)` — temperature and apparent temperature |
//! | 2 | `↗ 12km/h NE` — wind arrow, speed and cardinal direction |
//! | 3 | `0.0mm 56%` — precipitation and its probability |
//!
//! A day cell is those four blocks stacked, headed by the localized date (`Today, Sep 30`). Cells
//! are joined horizontally with `│`; the header row, the part blocks and the bottom of the box are
//! separated by `├───┼───┤` runs. Below `STACKED_BELOW` columns the horizontal joining is
//! dropped: one section per day, one line per part, with the 3-column glyph from
//! [`art::one_line_art`] where a block would be.
//!
//! # The width invariant
//!
//! Every line leaves through `fit`, which measures display columns (escape sequences take none)
//! and truncates with an ellipsis. Metrics are sized to their column *before* the row is built, so
//! a truncation can only ever drop a table border at widths below the layout's own minimum — never
//! half of an art block.

use std::borrow::Cow;
use std::fmt::Write as _;

use unicode_width::UnicodeWidthChar as _;

use super::art::{self, ART_LINES, ART_W};
use super::color::{self, FG_DEFAULT, paint};
use super::{Charset, ColorDepth, RenderContext, Renderer, Slot, TermKind};
use crate::error::Result;
use crate::geo::{attribution_line, place};
use crate::i18n::keys;
use crate::model::units::{
    ResolvedUnits, UnitStyle, format_precip, format_pressure, format_temp_signed,
    format_visibility, format_wind,
};
use crate::model::{Current, DayForecast, DayPart, DayPartKind, LocationSource, Report};

/// Columns between an art block and the metrics of the same line.
pub const GAP: usize = 1;

/// Width of a day cell's metrics column.
pub const METRICS_W: usize = 13;

/// Width of a day cell's metrics column when the table has to be narrow.
pub const METRICS_W_NARROW: usize = 10;

/// Width of one day cell: art, gap, metrics.
pub const CELL_W: usize = ART_W + GAP + METRICS_W;

/// Width of one day cell below `WIDE_FROM`.
pub const CELL_W_NARROW: usize = ART_W + GAP + METRICS_W_NARROW;

/// Spaces between a cell's content and its border.
pub const PAD: usize = 1;

/// The width of a day part's label, taken from the longest of the four.
const LABEL_W: usize = 7;

/// The width of the one-line glyph column in the stacked layout.
const GLYPH_W: usize = 3;

/// From this width on a cell uses [`CELL_W`] instead of [`CELL_W_NARROW`].
const WIDE_FROM: usize = 74;

/// The most day cells a screen row ever holds.
const CELLS_PER_ROW: usize = 3;

/// Below this width the table drops the horizontal joining and stacks one day per section.
const STACKED_BELOW: usize = 60;

/// The art table, in the character set it was asked to draw in.
#[derive(Debug, Clone, Copy, Default)]
pub struct ArtTable {
    charset: Charset,
    /// Paint nothing, whatever `ctx.color` asks: `--format dumb` is documented as colourless, and
    /// a public `renderer_for` must not be able to break that promise.
    mono: bool,
}

impl ArtTable {
    /// The table for `charset`; a terminal without UTF-8 asks for [`Charset::Ascii`].
    #[must_use]
    pub const fn new(charset: Charset) -> Self {
        Self {
            charset,
            mono: false,
        }
    }

    /// The `--format dumb` table: [`Charset::Ascii`] with the palette forced off, so the format's
    /// "no colour" is a property of the renderer rather than of the CLI that built the context.
    #[must_use]
    pub const fn dumb() -> Self {
        Self {
            charset: Charset::Ascii,
            mono: true,
        }
    }
}

impl Renderer for ArtTable {
    fn render(&self, report: &Report, ctx: &RenderContext<'_>) -> Result<String> {
        let charset = self.charset;
        let depth = if self.mono {
            ColorDepth::Mono
        } else {
            ctx.depth()
        };

        let mut lines: Vec<String> = Vec::new();
        for line in super::alerts::banner(&report.alerts, charset, ctx) {
            lines.push(match line.severity {
                Some(severity) => color::paint_severity(&line.text, severity, depth).into_owned(),
                None => line.text,
            });
        }
        lines.push(header(report, ctx));
        if let Some(current) = &report.current {
            lines.push(String::new());
            lines.extend(current_block(current, ctx, charset, depth));
        }
        // An observation-only backend has no day table to show: the current block states when it
        // was taken, and a footer says why nothing follows. Both are capability-driven — the
        // renderer never looks at the provider id.
        let observation_only = report
            .attribution
            .capabilities
            .as_ref()
            .is_some_and(|capabilities| capabilities.current && !capabilities.daily);
        if observation_only {
            if let Some(current) = &report.current {
                lines.push(observed_line(current, ctx, charset));
            }
            lines.push(no_forecast_footer(report, ctx));
        }
        if !report.days.is_empty() {
            if ctx.width < STACKED_BELOW {
                lines.extend(stacked(&report.days, ctx, charset, depth));
            } else {
                lines.extend(columns(&report.days, ctx, charset, depth));
            }
        }
        // The moon block sits between the forecast and the air panel: it is sky data like the
        // table's own, while the air panel is a separate reading with its own credit.
        let panels = panel_context(ctx, charset);
        let moon = super::moon::panel(report, &panels);
        if !moon.is_empty() {
            lines.push(String::new());
            lines.extend(moon);
        }
        // The air panel sits between the forecast and the credits: the air credit is part of the
        // panel (it belongs to those numbers), the place and forecast credits stay last.
        let panel = super::air::panel(report, &panels, depth);
        if !panel.is_empty() {
            lines.push(String::new());
            lines.extend(panel);
        }
        let credits = credits(report, ctx);
        if !credits.is_empty() {
            lines.push(String::new());
            lines.extend(credits);
        }

        Ok(lines
            .into_iter()
            .map(|line| match charset {
                Charset::Ascii => fit(&fold_ascii(&line), ctx.width, charset).into_owned(),
                Charset::Unicode => fit(&line, ctx.width, charset).into_owned(),
            })
            .collect::<Vec<_>>()
            .join("\n"))
    }

    /// 2–4 locations: one header line and one aligned grid row each, blank line between blocks.
    ///
    /// The grid is only drawn when every odds-and-ends line fits the resolved width and every
    /// successful location has a current condition (or a today part) with a temperature; otherwise
    /// the run falls back to the full per-location tables, which already handle narrow terminals
    /// and data-poor reports. Five locations or more always take the fallback, and
    /// [`slot_note`](Renderer::slot_note) says why once.
    fn render_slots(&self, slots: &[Slot<'_>]) -> Result<String> {
        if let Some(summary) = self.summary(slots) {
            return Ok(summary);
        }
        self.full_tables(slots)
    }

    fn slot_note(&self, slots: usize) -> Option<&'static str> {
        (slots > SUMMARY_LIMIT)
            .then_some("note: art-table summary layout is limited to 4 locations")
    }
}

/// The most locations the combined summary lays out; a longer run uses the full tables.
pub const SUMMARY_LIMIT: usize = 4;

/// One location's cells in the summary grid, before the columns are aligned and painted.
struct SummaryCells {
    /// The compact condition art.
    art: String,
    /// Palette entry of the art (and of the condition text, as in the full table's current block).
    art_fg: u8,
    /// The localized condition text.
    condition: String,
    /// The current (or today's) temperature, signed and folded.
    temp: String,
    /// Palette entry of the temperature.
    temp_fg: u8,
    /// `+24°C/+14°C`, or `n/a` when the report has no day.
    high_low: String,
    /// Palette entry of the high/low pair.
    high_low_fg: u8,
}

impl ArtTable {
    /// The fallback composition: each slot's full table (or its placeholder), blank line between.
    fn full_tables(self, slots: &[Slot<'_>]) -> Result<String> {
        let mut blocks: Vec<String> = Vec::with_capacity(slots.len());
        for slot in slots {
            match (slot.report, slot.ctx.as_ref()) {
                (Some(report), Some(ctx)) => blocks.push(self.render(report, ctx)?),
                _ => blocks.push(slot.placeholder()),
            }
        }
        Ok(blocks.join("\n\n"))
    }

    /// The combined 2–4 location summary, or `None` when it cannot honour the width or the data.
    fn summary(self, slots: &[Slot<'_>]) -> Option<String> {
        if !(2..=SUMMARY_LIMIT).contains(&slots.len()) {
            return None;
        }
        let charset = self.charset;
        let width = slots
            .iter()
            .find_map(|slot| slot.ctx.as_ref())
            .map_or(80, |ctx| ctx.width);
        let depth = if self.mono {
            ColorDepth::Mono
        } else {
            slots
                .iter()
                .find_map(|slot| slot.ctx.as_ref())
                .map_or(ColorDepth::Mono, RenderContext::depth)
        };

        let mut rows: Vec<Option<SummaryCells>> = Vec::with_capacity(slots.len());
        for slot in slots {
            match (slot.report, slot.ctx.as_ref()) {
                (Some(report), Some(ctx)) => {
                    rows.push(Some(summary_cells(report, ctx, charset)?));
                }
                _ => rows.push(None),
            }
        }

        let column = |value: fn(&SummaryCells) -> &str| {
            rows.iter()
                .flatten()
                .map(|cells| display_width(value(cells)))
                .max()
                .unwrap_or(0)
        };
        let art_w = column(|cells| &cells.art);
        let condition_w = column(|cells| &cells.condition);
        let temp_w = column(|cells| &cells.temp);
        let high_w = column(|cells| &cells.high_low);

        let mut blocks: Vec<String> = Vec::with_capacity(slots.len());
        for (slot, row) in slots.iter().zip(&rows) {
            let Some(cells) = row else {
                let line = slot.placeholder();
                if display_width(&line) > width {
                    return None;
                }
                blocks.push(line);
                continue;
            };
            let (Some(report), Some(ctx)) = (slot.report, slot.ctx.as_ref()) else {
                return None;
            };
            let header = folded(&header(report, ctx), charset);
            let art = pad_columns(&cells.art, art_w);
            let condition = pad_columns(&cells.condition, condition_w);
            let temp = pad_left(&cells.temp, temp_w);
            let high_low = pad_left(&cells.high_low, high_w);
            let plain = format!("{art}  {condition}  {temp}  {high_low}");
            if display_width(&header) > width || display_width(&plain) > width {
                return None;
            }
            let row_line = format!(
                "{}  {}  {}  {}",
                paint(&art, cells.art_fg, depth),
                paint(&condition, cells.art_fg, depth),
                paint(&temp, cells.temp_fg, depth),
                paint(&high_low, cells.high_low_fg, depth)
            );
            blocks.push(format!("{header}\n{row_line}"));
        }
        Some(blocks.join("\n\n"))
    }
}

/// One location's summary cells: the current (or today's) condition, temperature and the day's
/// high/low. `None` when there is nothing to show, which falls the whole run back to full tables.
fn summary_cells(
    report: &Report,
    ctx: &RenderContext<'_>,
    charset: Charset,
) -> Option<SummaryCells> {
    let day = crate::template::today(report, ctx.now.date_naive());
    let (condition, is_day, temp_c) = if let Some(current) = &report.current {
        (current.weather, current.is_day, current.temp_c)
    } else {
        let day = day?;
        let kind = crate::template::hour_part(ctx.now);
        let part = day.part(kind);
        (part.weather, kind != DayPartKind::Night, part.temp_c)
    };
    let key = weather_key(condition.art_key(), is_day);
    let art_fg = art::art(key).map_or(FG_DEFAULT, |block| color::art_fg(block.style));
    let temp_unit = ctx.units.temp;
    let (high_low, high_low_fg) = match day {
        Some(day) => (
            format!(
                "{}/{}",
                folded(&format_temp_signed(day.temp_max_c, temp_unit), charset),
                folded(&format_temp_signed(day.temp_min_c, temp_unit), charset)
            ),
            color::temp_fg(day.temp_max_c),
        ),
        None => (ctx.i18n.text(&keys::NA).into_owned(), FG_DEFAULT),
    };
    Some(SummaryCells {
        art: folded(art::one_line_art(key), charset),
        art_fg,
        condition: folded(&ctx.i18n.condition(condition), charset),
        temp: folded(&format_temp_signed(temp_c, temp_unit), charset),
        temp_fg: color::temp_fg(temp_c),
        high_low,
        high_low_fg,
    })
}

/// Right-aligns `text` in `width` display columns.
fn pad_left(text: &str, width: usize) -> String {
    let mut padded = " ".repeat(width.saturating_sub(display_width(text)));
    padded.push_str(text);
    padded
}

/// The context the panels see: an ASCII table (`--format dumb`, or a terminal that cannot draw
/// UTF-8) forces the 7-bit charset on the air and moon blocks too, so a format that promises
/// ASCII cannot end up drawing the unicode moon disc.
fn panel_context<'a>(ctx: &RenderContext<'a>, charset: Charset) -> RenderContext<'a> {
    let mut panels = *ctx;
    if charset == Charset::Ascii {
        panels.term.term = TermKind::Dumb;
        panels.term.utf8 = false;
    }
    panels
}

/// `observed 23:51Z · 12 min ago`: when the observation was taken and how old it is.
///
/// The clock is UTC — that is how the aviation world reads a METAR — while the age comes from the
/// injected `ctx.now`, so the line is deterministic in tests. A report dated in the future (a clock
/// skew, or a station's own clock) clamps to "0 min ago" rather than printing a negative age.
fn observed_line(current: &Current, ctx: &RenderContext<'_>, charset: Charset) -> String {
    let minutes = (ctx.now - current.observed_at).num_minutes().max(0);
    let age = if minutes < 60 {
        ctx.i18n.format(
            &keys::FORMAT_AGE_MINUTES,
            &[(
                "minutes",
                fluent_bundle::FluentValue::from(minutes.to_string()),
            )],
        )
    } else {
        ctx.i18n.format(
            &keys::FORMAT_AGE_HOURS,
            &[(
                "hours",
                fluent_bundle::FluentValue::from((minutes / 60).to_string()),
            )],
        )
    };
    // The separator is part of the charset: `·` has no ASCII form the fold could pick that reads
    // as a separator (it folds to `.`), so a dumb terminal gets a `|` instead.
    let separator = match charset {
        Charset::Unicode => '·',
        Charset::Ascii => '|',
    };
    format!(
        "{} {}Z {separator} {age}",
        ctx.i18n.text(&keys::LABEL_OBSERVED),
        current
            .observed_at
            .with_timezone(&chrono::Utc)
            .format("%H:%M")
    )
}

/// `no forecast: METAR is an observation`: why an observation-only report has no day table.
fn no_forecast_footer(report: &Report, ctx: &RenderContext<'_>) -> String {
    let provider = if report.attribution.display_name.is_empty() {
        report.attribution.provider.clone()
    } else {
        report.attribution.display_name.clone()
    };
    ctx.i18n
        .format(
            &keys::NOTE_NO_FORECAST,
            &[("provider", fluent_bundle::FluentValue::from(provider))],
        )
        .into_owned()
}

// ---------------------------------------------------------------------------------------------
// The header and the current conditions
// ---------------------------------------------------------------------------------------------

/// `Weather report: <place> (<lat>, <lon>)`, without coordinates for a location that *is* a
/// coordinate pair — repeating `39.90, 116.41 (39.90, 116.41)` would say nothing.
fn header(report: &Report, ctx: &RenderContext<'_>) -> String {
    let location = &report.location;
    let mut text = format!("{} {}", ctx.i18n.text(&keys::LABEL_REPORT), place(location));
    if location.source != LocationSource::Coordinates {
        let _ = write!(text, " ({:.2}, {:.2})", location.lat, location.lon);
    }
    text
}

/// The current conditions: four art lines, and to their right the condition, the temperatures, the
/// wind and the measurements.
///
/// The art is chosen from the report's own `is_day`, never from the wall clock; a provider that
/// cannot tell day from night gets the day block.
fn current_block(
    current: &Current,
    ctx: &RenderContext<'_>,
    charset: Charset,
    depth: ColorDepth,
) -> Vec<String> {
    let key = weather_key(current.weather.art_key(), current.is_day);
    let block = art::art(key);
    let [art0, art1, art2, art3] = block.map_or(art::NO_BLOCK, |block| block.lines(charset));
    let fg = block.map_or(FG_DEFAULT, |block| color::art_fg(block.style));

    let condition = ctx.i18n.condition(current.weather);
    let temp = temp_metric(
        current.temp_c,
        current.feels_like_c,
        ctx.units,
        METRICS_W,
        charset,
    );
    let wind = wind_metric(
        current.wind_kmh,
        Some(current.wind_dir_deg),
        ctx,
        charset,
        METRICS_W,
    );

    vec![
        open_line(art0, &paint(&condition, fg, depth), fg, depth),
        open_line(
            art1,
            &paint(&temp, color::temp_fg(current.temp_c), depth),
            fg,
            depth,
        ),
        open_line(
            art2,
            &paint(&wind, color::wind_fg(current.wind_kmh), depth),
            fg,
            depth,
        ),
        open_line(art3, &measurements(current, ctx, depth), fg, depth),
    ]
}

/// One line of the current block: the art line in `fg`, the gap, then `text` — already painted,
/// because the measurements have a palette entry per value.
fn open_line(art_line: &str, text: &str, fg: u8, depth: ColorDepth) -> String {
    let mut line = paint(art_line, fg, depth).into_owned();
    line.push_str(&gap_after(art_line));
    line.push_str(text);
    line
}

/// `56% 1013hPa 10km 0.0mm`: the measurements under the current conditions, each in its own
/// palette entry. Visibility is dropped when the provider has no value for it.
fn measurements(current: &Current, ctx: &RenderContext<'_>, depth: ColorDepth) -> String {
    let units = ctx.units;
    let mut pieces = vec![
        paint(
            &format!("{}%", current.humidity_pct),
            color::humidity_fg(current.humidity_pct),
            depth,
        )
        .into_owned(),
        paint(
            &format_pressure(current.pressure_hpa, units.pressure, UnitStyle::Compact),
            FG_DEFAULT,
            depth,
        )
        .into_owned(),
    ];
    if let Some(visibility) = current.visibility_km {
        pieces.push(
            paint(
                &format_visibility(visibility, units.distance, UnitStyle::Compact),
                FG_DEFAULT,
                depth,
            )
            .into_owned(),
        );
    }
    pieces.push(
        paint(
            &format_precip(current.precip_mm, units.precip, UnitStyle::Compact),
            color::precip_fg(current.precip_mm),
            depth,
        )
        .into_owned(),
    );
    pieces.join(" ")
}

// ---------------------------------------------------------------------------------------------
// The day columns
// ---------------------------------------------------------------------------------------------

/// The boxed day columns: bands of at most [`CELLS_PER_ROW`] days, each band a box of one header
/// row and four day-part blocks.
fn columns(
    days: &[DayForecast],
    ctx: &RenderContext<'_>,
    charset: Charset,
    depth: ColorDepth,
) -> Vec<String> {
    let (cell_w, metrics_w) = if ctx.width >= WIDE_FROM {
        (CELL_W, METRICS_W)
    } else {
        (CELL_W_NARROW, METRICS_W_NARROW)
    };
    let per_row = ((ctx.width - 1) / (cell_w + 3)).clamp(1, CELLS_PER_ROW);

    let mut lines = Vec::new();
    for band in days.chunks(per_row) {
        lines.push(String::new());
        let per_day: Vec<Vec<String>> = band
            .iter()
            .map(|day| day_lines(day, ctx, charset, metrics_w, depth))
            .collect();
        let height = per_day.first().map_or(0, Vec::len);
        lines.push(border(
            Border::Top,
            per_row.min(band.len()),
            cell_w,
            charset,
        ));
        // One buffer for the row's borrowed cells, reused for every row of the band: the cells are
        // the day lines themselves, so nothing is copied between building and drawing a row.
        let mut cells: Vec<&str> = Vec::with_capacity(per_day.len());
        for row in 0..height {
            cells.clear();
            cells.extend(
                per_day
                    .iter()
                    .map(|day| day.get(row).map_or("", String::as_str)),
            );
            lines.push(cell_row(&cells, cell_w, charset));
            if row % ART_LINES == 0 {
                let kind = if row + 1 == height {
                    Border::Bottom
                } else {
                    Border::Middle
                };
                lines.push(border(kind, cells.len(), cell_w, charset));
            }
        }
    }
    lines
}

/// One day's cell content: the date heading, then the four part blocks.
fn day_lines(
    day: &DayForecast,
    ctx: &RenderContext<'_>,
    charset: Charset,
    metrics_w: usize,
    depth: ColorDepth,
) -> Vec<String> {
    let mut lines = vec![ctx.i18n.format_day_heading(day.date, ctx.now.date_naive())];
    for part in &day.parts {
        lines.extend(part_block(part, ctx, charset, metrics_w, depth));
    }
    lines
}

/// One day part as [`ART_LINES`] lines — the contract documented at the top of the module.
fn part_block(
    part: &DayPart,
    ctx: &RenderContext<'_>,
    charset: Charset,
    metrics_w: usize,
    depth: ColorDepth,
) -> [String; ART_LINES] {
    let daytime = part.kind != DayPartKind::Night;
    let key = weather_key(part.weather.art_key(), daytime);
    let block = art::art(key);
    let [art0, art1, art2, art3] = block.map_or(art::NO_BLOCK, |block| block.lines(charset));
    let fg = block.map_or(FG_DEFAULT, |block| color::art_fg(block.style));

    let label = ctx.i18n.day_part(part.kind);
    let temp = temp_metric(
        part.temp_c,
        part.feels_like_c,
        ctx.units,
        metrics_w,
        charset,
    );
    let wind = wind_metric(part.wind_kmh, part.wind_dir_deg, ctx, charset, metrics_w);
    let tail = part_tail(part, ctx.units, true);

    [
        cell_line(art0, &label, FG_DEFAULT, fg, metrics_w, charset, depth),
        cell_line(
            art1,
            &temp,
            color::temp_fg(part.temp_c),
            fg,
            metrics_w,
            charset,
            depth,
        ),
        cell_line(
            art2,
            &wind,
            color::wind_fg(part.wind_kmh),
            fg,
            metrics_w,
            charset,
            depth,
        ),
        cell_line(
            art3,
            &tail,
            color::precip_fg(part.precip_mm),
            fg,
            metrics_w,
            charset,
            depth,
        ),
    ]
}

/// One line of a day cell: the art line, the gap, then the metric padded to the column.
#[allow(clippy::too_many_arguments)]
fn cell_line(
    art_line: &str,
    metric: &str,
    metric_fg: u8,
    art_fg: u8,
    metrics_w: usize,
    charset: Charset,
    depth: ColorDepth,
) -> String {
    let mut line = paint(art_line, art_fg, depth).into_owned();
    line.push_str(&gap_after(art_line));
    let metric = fit(metric, metrics_w, charset);
    line.push_str(&paint(metric.as_ref(), metric_fg, depth));
    line.push_str(&" ".repeat(metrics_w.saturating_sub(display_width(&metric))));
    line
}

/// `0.0mm 56%` — a part's precipitation, and its precipitation probability when the provider
/// reports one and the caller has room for it.
///
/// The probability, not the humidity: this slot pairs with the precipitation amount, exactly as
/// the `plain` document's `0.0mm (0%)` and wttr.in's `0.0 mm | 0%` do. Humidity is a current
/// reading and stays in the conditions block above the table.
fn part_tail(part: &DayPart, units: ResolvedUnits, with_probability: bool) -> String {
    let mut text = format_precip(part.precip_mm, units.precip, UnitStyle::Compact);
    if let Some(probability) = part.precip_prob_pct.filter(|_| with_probability) {
        let _ = write!(text, " {probability}%");
    }
    text
}

// ---------------------------------------------------------------------------------------------
// The stacked layout
// ---------------------------------------------------------------------------------------------

/// One section per day for terminals too narrow for columns: a blank line, the day heading, then
/// one line per part. Same content, no horizontal joining.
fn stacked(
    days: &[DayForecast],
    ctx: &RenderContext<'_>,
    charset: Charset,
    depth: ColorDepth,
) -> Vec<String> {
    // The characters of the stacked layout come from the renderer's charset; `ctx.term` only
    // describes the terminal, and `--format dumb` asks for ASCII even on a unicode one.
    let mut lines = Vec::new();
    for day in days {
        lines.push(String::new());
        lines.push(ctx.i18n.format_day_heading(day.date, ctx.now.date_naive()));
        for part in &day.parts {
            lines.push(stacked_part(part, ctx, charset, depth));
        }
    }
    lines
}

/// `  Morning │ \o/ │ +22°C (+23°C) │ ↗ 12km/h NE │ 0.0mm 56%`
fn stacked_part(
    part: &DayPart,
    ctx: &RenderContext<'_>,
    charset: Charset,
    depth: ColorDepth,
) -> String {
    let daytime = part.kind != DayPartKind::Night;
    let key = weather_key(part.weather.art_key(), daytime);
    let fg = art::art(key).map_or(FG_DEFAULT, |block| color::art_fg(block.style));
    let glyph = pad_columns(art::one_line_art(key), GLYPH_W);
    let label = pad_columns(&ctx.i18n.day_part(part.kind), LABEL_W);
    let separator = format!(" {} ", vertical(charset));

    // The degradation ladder of a narrow terminal. Apparent temperature, cardinal direction (the
    // arrow already names the sector), precipitation probability, the precipitation tail and then
    // the wind are dropped in that order — every one of them a second reading of something already
    // on the line — before the line is clipped at all. The two rungs that drop the tail and the
    // wind are what keep a width between `MIN_WIDTH` and a full line from silently truncating a
    // complete part.
    let mut line = String::new();
    for (with_glyph, feels_like, cardinal, probability, with_tail, with_wind) in [
        (true, true, true, true, true, true),
        (true, true, true, false, true, true),
        (true, false, true, false, true, true),
        (true, false, false, false, true, true),
        (false, false, false, false, true, true),
        (false, false, false, false, false, true),
        (false, false, false, false, false, false),
    ] {
        let temp = temp_metric(
            part.temp_c,
            part.feels_like_c.filter(|_| feels_like),
            ctx.units,
            METRICS_W,
            charset,
        );
        let wind =
            with_wind.then(|| wind_text(part.wind_kmh, part.wind_dir_deg, ctx, charset, cardinal));
        let tail = with_tail.then(|| part_tail(part, ctx.units, probability));

        let mut text = format!("  {label}");
        if with_glyph {
            text.push_str(&separator);
            text.push_str(&paint(&glyph, fg, depth));
        }
        for (value, fg) in [
            (Some(&temp), color::temp_fg(part.temp_c)),
            (wind.as_ref(), color::wind_fg(part.wind_kmh)),
            (tail.as_ref(), color::precip_fg(part.precip_mm)),
        ] {
            if let Some(value) = value {
                text.push_str(&separator);
                text.push_str(&paint(value, fg, depth));
            }
        }
        line = text;
        if display_width(&line) <= ctx.width {
            break;
        }
    }
    line
}

// ---------------------------------------------------------------------------------------------
// Rows, borders and credits
// ---------------------------------------------------------------------------------------------

/// Which border line to draw.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Border {
    /// The top of a band.
    Top,
    /// Between two rows of a band.
    Middle,
    /// The bottom of a band.
    Bottom,
}

/// The three glyphs a border is made of.
fn border_glyphs(kind: Border, charset: Charset) -> (char, char, char) {
    match (kind, charset) {
        (Border::Top, Charset::Unicode) => ('\u{250c}', '\u{252c}', '\u{2510}'),
        (Border::Middle, Charset::Unicode) => ('\u{251c}', '\u{253c}', '\u{2524}'),
        (Border::Bottom, Charset::Unicode) => ('\u{2514}', '\u{2534}', '\u{2518}'),
        (_, Charset::Ascii) => ('+', '+', '+'),
    }
}

/// The vertical border between two cells.
const fn vertical(charset: Charset) -> char {
    match charset {
        Charset::Unicode => '\u{2502}',
        Charset::Ascii => '|',
    }
}

/// A border line: `┌───┬───┐`, `├───┼───┤` or `└───┴───┘`, each run `cell_w + 2 * PAD` wide.
fn border(kind: Border, cells: usize, cell_w: usize, charset: Charset) -> String {
    let (left, join, right) = border_glyphs(kind, charset);
    let run = match charset {
        Charset::Unicode => '\u{2500}',
        Charset::Ascii => '-',
    };
    let mut text = String::new();
    text.push(left);
    for index in 0..cells {
        for _ in 0..cell_w + 2 * PAD {
            text.push(run);
        }
        text.push(if index + 1 == cells { right } else { join });
    }
    text
}

/// One screen row: `│`, then every cell padded to `cell_w` and surrounded by [`PAD`] spaces.
///
/// The cells arrive borrowed and the row is written into one [`String`]; `fit` returns the cell
/// borrowed too, so an untruncated cell — every cell of every committed layout — is copied once,
/// into the row, instead of three times on its way there.
fn cell_row(cells: &[&str], cell_w: usize, charset: Charset) -> String {
    let bar = vertical(charset);
    let mut text = String::with_capacity(cells.len() * (cell_w + 2 * PAD + 1) + 1);
    for cell in cells {
        let fitted = fit(cell, cell_w, charset);
        text.push(bar);
        for _ in 0..PAD {
            text.push(' ');
        }
        text.push_str(&fitted);
        for _ in 0..cell_w.saturating_sub(display_width(&fitted)) {
            text.push(' ');
        }
        for _ in 0..PAD {
            text.push(' ');
        }
    }
    text.push(bar);
    text
}

/// The credits the place and data licences require, next to the data they describe.
fn credits(report: &Report, ctx: &RenderContext<'_>) -> Vec<String> {
    let mut lines = Vec::new();
    if let Some(location) = attribution_line(&report.location) {
        lines.push(location.to_owned());
    }
    if let Some(licence) = report.attribution.licence.as_deref() {
        lines.push(format!("{} {licence}", ctx.i18n.text(&keys::LABEL_DATA)));
    }
    lines.extend(ctx.alert_credits.iter().cloned());
    lines
}

// ---------------------------------------------------------------------------------------------
// Measuring, padding and clipping
// ---------------------------------------------------------------------------------------------

/// The art key to draw, taking the night sibling when the sky is dark.
///
/// [`art::night_variant`] rewrites only the keys that have a night block, so this is identity for
/// everything else.
fn weather_key(key: &str, daytime: bool) -> &str {
    if daytime {
        key
    } else {
        art::night_variant(key)
    }
}

/// The spaces that make `text` occupy `width` display columns, never negative.
fn pad_columns(text: &str, width: usize) -> String {
    let mut padded = text.to_owned();
    padded.push_str(&" ".repeat(width.saturating_sub(display_width(text))));
    padded
}

/// The spaces that separate an art line from the metrics of the same line.
fn gap_after(art_line: &str) -> String {
    " ".repeat(ART_W.saturating_sub(display_width(art_line)) + GAP)
}

/// `text` in the charset's spelling.
fn folded(text: &str, charset: Charset) -> String {
    match charset {
        Charset::Ascii => fold_ascii(text),
        Charset::Unicode => text.to_owned(),
    }
}

/// The 7-bit spelling of the few glyphs the table composes itself and that have no ASCII form:
/// the degree sign of a temperature, the em dash of a credit line, the middle dot of a moonlit
/// block and the micro/superscript of the air panel's units.
///
/// Only the renderer's own chrome is folded. A place name is the user's data: replacing a city
/// with question marks would be worse than the byte a dumb terminal cannot draw.
pub(crate) fn fold_ascii(text: &str) -> String {
    if text.is_ascii() {
        return text.to_owned();
    }
    let mut folded = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '\u{b0}' => {}
            '\u{2014}' => folded.push_str("--"),
            '\u{b7}' => folded.push('.'),
            // `μ` (micro sign) and `μ` (Greek mu) are both in use for μg/m³, and `³` has no ASCII
            // form either; the unit reads `ug/m3` on a dumb terminal.
            '\u{b5}' | '\u{3bc}' => folded.push('u'),
            '\u{b3}' => folded.push('3'),
            other => folded.push(other),
        }
    }
    folded
}

/// The display width of a line: escape sequences take no columns.
pub(crate) fn display_width(line: &str) -> usize {
    let mut width = 0;
    let mut chars = line.chars();
    while let Some(character) = chars.next() {
        if character == '\u{1b}' {
            for escape in chars.by_ref() {
                if escape == 'm' {
                    break;
                }
            }
            continue;
        }
        width += character.width().unwrap_or(0);
    }
    width
}

/// The ellipsis a character set uses for a truncated line.
const fn ellipsis(charset: Charset) -> &'static str {
    match charset {
        Charset::Unicode => "\u{2026}",
        Charset::Ascii => "...",
    }
}

/// Clips a line to `width` display columns, marking the cut with an ellipsis.
///
/// Escape sequences are copied verbatim (they take no columns) and an unterminated colour is
/// closed before the ellipsis, so a truncated line cannot paint the rest of the terminal. This is
/// the last line of defence of the width invariant: the cell builders size their metrics first.
///
/// A line that already fits is returned **borrowed**, so the common case allocates nothing.
pub(crate) fn fit(line: &str, width: usize, charset: Charset) -> Cow<'_, str> {
    if display_width(line) <= width {
        return Cow::Borrowed(line);
    }
    let marker = ellipsis(charset);
    let (marker, budget) = if display_width(marker) <= width {
        (marker, width - display_width(marker))
    } else {
        ("", width)
    };
    let mut clipped = String::new();
    let mut used = 0;
    let mut escapes = 0_u32;
    let mut chars = line.chars();
    while let Some(character) = chars.next() {
        if character == '\u{1b}' {
            clipped.push(character);
            for escape in chars.by_ref() {
                clipped.push(escape);
                if escape == 'm' {
                    break;
                }
            }
            escapes += 1;
            continue;
        }
        let cell = character.width().unwrap_or(0);
        if used + cell > budget {
            break;
        }
        clipped.push(character);
        used += cell;
    }
    if escapes % 2 == 1 {
        clipped.push_str("\u{1b}[0m");
    }
    clipped.push_str(marker);
    debug_assert!(
        display_width(&clipped) <= width,
        "{clipped:?} is wider than {width} columns"
    );
    Cow::Owned(clipped)
}

/// `+22°C (+23°C)`, or just the temperature when the pair does not fit the column.
///
/// Three-digit Fahrenheit readings are the reason the rule exists; dropping the apparent
/// temperature keeps the number a reader came for.
fn temp_metric(
    temp_c: f32,
    feels_like_c: Option<f32>,
    units: ResolvedUnits,
    metrics_w: usize,
    charset: Charset,
) -> String {
    // The ASCII table has no degree sign, and the column width has to be measured in the
    // characters that will really be printed — otherwise every ASCII cell is two columns short.
    let temp = folded(&format_temp_signed(temp_c, units.temp), charset);
    if let Some(feels_like) = feels_like_c {
        let pair = format!(
            "{temp} ({})",
            folded(&format_temp_signed(feels_like, units.temp), charset)
        );
        if display_width(&pair) <= metrics_w {
            return pair;
        }
    }
    fit(&temp, metrics_w, charset).into_owned()
}

/// `↗ 12km/h NE`, or less of it when the column is narrow.
///
/// The cardinal direction goes first — the arrow already names the sector — and only then is the
/// text clipped, so a ten column cell still reads `↗ 12km/h` instead of `↗ 12km…`.
fn wind_metric(
    kmh: f32,
    dir_deg: Option<u16>,
    ctx: &RenderContext<'_>,
    charset: Charset,
    metrics_w: usize,
) -> String {
    let speed = format_wind(kmh, ctx.units.wind, UnitStyle::Compact);
    let full = wind_text(kmh, dir_deg, ctx, charset, true);
    if display_width(&full) <= metrics_w {
        return full;
    }
    let short = wind_text(kmh, dir_deg, ctx, charset, false);
    if display_width(&short) <= metrics_w {
        return short;
    }
    fit(&speed, metrics_w, charset).into_owned()
}

/// `↗ 12km/h NE`, or `↗ 12km/h` when the caller has no room for the cardinal direction.
///
/// The arrow comes from [`art::wind_arrow`] and follows the character set, while the point's name
/// comes from the catalog: a dumb terminal draws an ASCII arrow where a UTF-8 one draws a glyph, and
/// a Chinese run reads `东北风` where an English one reads `NE`.
fn wind_text(
    kmh: f32,
    dir_deg: Option<u16>,
    ctx: &RenderContext<'_>,
    charset: Charset,
    with_cardinal: bool,
) -> String {
    let speed = format_wind(kmh, ctx.units.wind, UnitStyle::Compact);
    match dir_deg {
        Some(dir) => {
            let arrow = art::wind_arrow(dir, charset);
            if with_cardinal {
                format!("{arrow} {speed} {}", ctx.i18n.direction(dir))
            } else {
                format!("{arrow} {speed}")
            }
        }
        None => speed,
    }
}

#[cfg(test)]
mod tests {
    // The expected strings are literals with exact rounding, so float comparison is the assertion.
    #![allow(clippy::float_cmp)]

    use chrono::{FixedOffset, TimeZone as _, Utc};
    use chrono_tz::Tz;

    use super::{ArtTable, CELL_W, METRICS_W, display_width, fit};
    use crate::config::UnitOverrides;
    use crate::i18n::{I18n, LanguageId, LanguageRequest};
    use crate::model::units::UnitSystem;
    use crate::model::{
        Attribution, Condition, Current, DayForecast, DayPart, DayPartKind, Location,
        LocationSource, Report,
    };
    use crate::render::{
        Charset, ColorDepth, ColorMode, Format, RenderContext, Renderer, TermCaps, renderer_for,
    };

    /// The English catalog, loaded the way the CLI loads an unconfigured run.
    fn english() -> I18n {
        I18n::load(&LanguageRequest::Auto, |_| None)
    }
    fn moment(hour: u32, minute: u32) -> chrono::DateTime<FixedOffset> {
        FixedOffset::east_opt(8 * 3600)
            .expect("a valid offset")
            .with_ymd_and_hms(2026, 9, 30, hour, minute, 0)
            .single()
            .expect("a valid local time")
    }

    fn location() -> Location {
        Location {
            name: "Beijing".to_owned(),
            admin1: Some("Beijing".to_owned()),
            country: "China".to_owned(),
            country_code: Some("CN".to_owned()),
            lat: 39.9042,
            lon: 116.4074,
            tz: Tz::Asia__Shanghai,
            elevation_m: None,
            population: None,
            source: LocationSource::Geocoder,
            station: None,
        }
    }

    fn current(is_day: bool, code: u8) -> Current {
        Current {
            observed_at: moment(12, 15),
            temp_c: 22.4,
            feels_like_c: Some(23.6),
            humidity_pct: 56,
            precip_mm: 0.0,
            weather: Condition::from_u8(code),
            cloud_cover_pct: 40,
            pressure_hpa: 1013.0,
            wind_kmh: 12.0,
            wind_dir_deg: 45,
            wind_gust_kmh: None,
            visibility_km: Some(10.0),
            uv_index: Some(5.0),
            is_day,
        }
    }

    fn part(kind: DayPartKind, temp_c: f32, code: u8) -> DayPart {
        DayPart {
            kind,
            temp_c,
            feels_like_c: Some(temp_c + 1.0),
            precip_mm: 0.4,
            precip_prob_pct: Some(20),
            weather: Condition::from_u8(code),
            wind_kmh: 9.0,
            wind_dir_deg: Some(90),
            humidity_pct: Some(61),
            visibility_km: Some(10.0),
        }
    }

    fn day(day_of_month: u32, code: u8) -> DayForecast {
        DayForecast {
            date: chrono::NaiveDate::from_ymd_opt(2026, 9, day_of_month).expect("a date"),
            parts: [
                part(DayPartKind::Morning, 18.0, code),
                part(DayPartKind::Noon, 24.0, code),
                part(DayPartKind::Evening, 21.0, code),
                part(DayPartKind::Night, 16.0, code),
            ],
            temp_min_c: 15.0,
            temp_max_c: 25.0,
            sunrise: None,
            sunset: None,
        }
    }

    /// The provenance a provider-built report carries: the `open-meteo` row, spelled out because
    /// the renderers may not import the provider registry (step 12's layering gate).
    fn attribution() -> Attribution {
        Attribution {
            provider: "open-meteo".to_owned(),
            display_name: "Open-Meteo".to_owned(),
            licence: Some("Open-Meteo.com (CC BY 4.0)".to_owned()),
            capabilities: Some(crate::model::ReportCapabilities::open_meteo_test()),
            url: "https://api.open-meteo.com/v1/forecast".to_owned(),
            fetched_at: Utc
                .with_ymd_and_hms(2026, 9, 30, 4, 15, 0)
                .single()
                .expect("an instant"),
            raw: None,
        }
    }

    fn report(current: Option<Current>, days: Vec<DayForecast>) -> Report {
        Report {
            location: location(),
            current,
            days,
            alerts: Vec::new(),
            air: None,
            astro: None,
            attribution: attribution(),
        }
    }

    fn context(i18n: &I18n, width: usize) -> RenderContext<'_> {
        RenderContext {
            units: UnitSystem::Metric
                .resolve(&UnitOverrides::default())
                .expect("the default overrides resolve"),
            color: ColorMode::Never,
            width,
            term: TermCaps::default(),
            now: moment(12, 30),
            tz: Tz::Asia__Shanghai,
            lang: LanguageId::EN_US,
            i18n,
            alert_credits: &[],
            aqi_index: crate::air::aqi::AqiIndex::Us,
        }
    }

    fn render(report: &Report, width: usize, charset: Charset) -> String {
        let i18n = english();
        let ctx = context(&i18n, width);
        ArtTable::new(charset)
            .render(report, &ctx)
            .expect("the art table always renders")
    }

    #[test]
    fn the_current_block_puts_the_measurements_next_to_the_art() {
        let text = render(
            &report(Some(current(true, 2)), Vec::new()),
            80,
            Charset::Unicode,
        );
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(
            lines.first().copied(),
            Some("Weather report: Beijing, Beijing, China (39.90, 116.41)")
        );
        assert!(lines.contains(&"   \\│/  Partly cloudy"), "{text}");
        assert!(lines.contains(&" ╭───╮  +22°C (+24°C)"), "{text}");
        assert!(lines.contains(&"(     ) ↗ 12km/h NE"), "{text}");
        assert!(lines.contains(&" ╰───╯  56% 1013hPa 10km 0.0mm"), "{text}");
        assert!(
            text.contains("Location data based on GeoNames"),
            "the place credit travels with the data: {text}"
        );
        assert!(text.contains("Data: Open-Meteo.com"), "{text}");
    }

    #[test]
    fn a_night_observation_draws_the_night_block() {
        let text = render(
            &report(Some(current(false, 0)), Vec::new()),
            80,
            Charset::Unicode,
        );
        assert!(text.contains(" · * · "), "{text}");
        assert!(!text.contains('\\'), "no daytime sun at night: {text}");
    }

    #[test]
    fn every_part_is_four_contract_lines() {
        let text = render(
            &report(Some(current(true, 2)), vec![day(30, 3)]),
            80,
            Charset::Unicode,
        );
        let band: Vec<&str> = text
            .lines()
            .skip_while(|line| !line.starts_with('\u{250c}'))
            .take_while(|line| !line.is_empty())
            .collect();
        assert_eq!(band.len(), 1 + 1 + 1 + 4 * 4 + 3 + 1, "band:\n{text}");
        assert!(band[1].starts_with("\u{2502} Today, Sep 30"), "{text}");
        assert_eq!(display_width(band[0]), display_width(band[1]));
        assert_eq!(
            display_width(band[0]),
            display_width(band.last().copied().unwrap_or(""))
        );

        // The four rows of the morning block, in contract order.
        let morning = &band[3..7];
        assert!(morning[0].contains("Morning"), "{text}");
        assert!(morning[1].contains("+18°C (+19°C)"), "{text}");
        assert!(morning[2].contains("→ 9.0km/h E"), "{text}");
        assert!(morning[3].contains("0.4mm 20%"), "{text}");
    }

    #[test]
    fn the_cell_border_positions_line_up() {
        let text = render(
            &report(
                Some(current(true, 2)),
                vec![day(30, 3), day(1, 61), day(2, 95)],
            ),
            80,
            Charset::Unicode,
        );
        let rows: Vec<&str> = text
            .lines()
            .filter(|line| line.starts_with('\u{2502}'))
            .collect();
        assert!(rows.len() >= 17, "{text}");
        let borders: Vec<Vec<usize>> = rows
            .iter()
            .map(|row| {
                row.chars()
                    .enumerate()
                    .filter(|(_, character)| *character == '\u{2502}')
                    .map(|(index, _)| index)
                    .collect()
            })
            .collect();
        for positions in &borders {
            assert_eq!(
                positions.len(),
                4,
                "three cells have four borders: {rows:?}"
            );
        }
        assert!(borders.windows(2).all(|pair| pair[0] == pair[1]), "{text}");
        assert_eq!(borders[0].len(), 4);
        assert_eq!(display_width(rows[0]), 3 * (CELL_W + 3) + 1);
    }

    #[test]
    fn days_wrap_three_three_one() {
        let days: Vec<DayForecast> = (0..7).map(|index| day(30 - index, 3)).collect();
        let text = render(&report(None, days), 80, Charset::Unicode);
        let tops = text
            .lines()
            .filter(|line| line.starts_with('\u{250c}'))
            .count();
        assert_eq!(tops, 3, "seven days is three bands:\n{text}");
        let rows = text
            .lines()
            .filter(|line| line.starts_with('\u{2502}'))
            .count();
        assert_eq!(
            rows,
            3 * 17,
            "one header row and sixteen part lines per band"
        );
        assert_eq!(
            text.matches("\u{2502} Today, Sep 30").count(),
            1,
            "the first day is headed by today, the rest by their dates"
        );
        assert_eq!(
            text.matches("╭───╮").count(),
            28,
            "every part of every day shows its art"
        );
    }

    #[test]
    fn a_narrow_width_stacks_the_days() {
        let report = report(Some(current(true, 2)), vec![day(30, 3), day(1, 61)]);
        let text = render(&report, 58, Charset::Unicode);
        assert!(
            !text.contains('\u{250c}'),
            "no columns below the stacked threshold:\n{text}"
        );
        assert!(
            text.contains("  Morning │ ~~~ │ +18°C (+19°C) │ → 9.0km/h E │ 0.4mm 20%"),
            "{text}"
        );
        assert!(text.contains("Tue 01 Sep"), "{text}");

        let narrow = render(&report, 40, Charset::Unicode);
        assert!(
            narrow.lines().all(|line| display_width(line) <= 40),
            "{narrow}"
        );
    }

    #[test]
    fn the_ascii_charset_draws_the_same_table_in_seven_bit() {
        for width in [80, 40] {
            let text = render(
                &report(Some(current(true, 2)), vec![day(30, 3)]),
                width,
                Charset::Ascii,
            );
            assert!(
                text.is_ascii(),
                "every byte of a dumb table is ASCII at width {width}:\n{text}"
            );
        }
    }

    #[test]
    fn lines_never_exceed_the_requested_width() {
        let report = report(
            Some(current(true, 2)),
            (0..7).map(|index| day(30 - index, 95)).collect(),
        );
        for width in [20, 21, 32, 40, 59, 60, 73, 74, 80, 100, 200] {
            let text = render(&report, width, Charset::Unicode);
            for line in text.lines() {
                assert!(
                    display_width(line) <= width,
                    "width {width}: {line:?} is {} columns",
                    display_width(line)
                );
            }
        }
    }

    #[test]
    fn every_stacked_rung_fits_every_width_it_can() {
        let report = report(Some(current(true, 2)), vec![day(30, 3)]);
        for width in [20, 24, 29, 36] {
            let text = render(&report, width, Charset::Unicode);
            let morning = text
                .lines()
                .find(|line| line.contains("Morning"))
                .unwrap_or_else(|| panic!("no morning line at {width} columns:\n{text}"));
            assert!(
                display_width(morning) <= width,
                "width {width}: {morning:?}"
            );
            assert!(
                !morning.contains('\u{2026}'),
                "width {width} dropped the part's tail instead of using a shorter rung: {morning:?}"
            );
            // The temperature survives every rung: it is the one reading a part cannot lose.
            assert!(morning.contains("+18°C"), "width {width}: {morning:?}");
        }
    }

    #[test]
    fn fitting_measures_display_columns_and_keeps_escapes_balanced() {
        assert_eq!(fit("abc", 5, Charset::Unicode), "abc");
        assert_eq!(fit("abcdef", 4, Charset::Unicode), "abc\u{2026}");
        assert_eq!(fit("abcdef", 4, Charset::Ascii), "a...");
        assert_eq!(fit("北京市", 3, Charset::Unicode), "北\u{2026}");
        assert_eq!(
            fit("北京市", 2, Charset::Unicode),
            "\u{2026}",
            "两 columns cannot hold 北"
        );
        assert_eq!(fit("abcdef", 0, Charset::Unicode), "");
        assert_eq!(fit("abcdef", 1, Charset::Unicode), "\u{2026}");
        assert_eq!(fit("abcdef", 2, Charset::Unicode), "a\u{2026}");

        let painted = "\u{1b}[38;5;196mabcdef\u{1b}[0m";
        assert_eq!(display_width(painted), 6);
        assert_eq!(fit(painted, 80, Charset::Unicode), painted);
        let clipped = fit(painted, 3, Charset::Unicode);
        assert_eq!(clipped, "\u{1b}[38;5;196mab\u{1b}[0m\u{2026}");
        assert_eq!(display_width(&clipped), 3);
    }

    #[test]
    fn colour_reaches_the_line_and_mono_does_not() {
        let report = report(Some(current(true, 2)), Vec::new());
        let i18n = english();
        let mut ctx = context(&i18n, 80);
        ctx.color = ColorMode::Always;
        ctx.term = TermCaps::read(
            |name| (name == "TERM").then(|| "xterm-256color".to_owned()),
            true,
        );
        assert_eq!(ctx.depth(), ColorDepth::Ansi256);

        let painted = ArtTable::new(Charset::Unicode)
            .render(&report, &ctx)
            .expect("renders");
        assert!(painted.contains("38;5;"), "{painted}");

        let text = render(&report, 80, Charset::Unicode);
        assert!(!text.contains('\u{1b}'), "mono emits no escape");
        assert_eq!(METRICS_W, 13);
    }

    /// `--format dumb` promises "the ASCII table, with no colour" whatever the context says: the
    /// suppression has to live in the renderer, because `renderer_for` is public.
    #[test]
    fn the_dumb_format_paints_nothing_even_when_colour_is_always() {
        let report = report(Some(current(true, 2)), Vec::new());
        let i18n = english();
        let caps = TermCaps::read(
            |name| (name == "TERM").then(|| "xterm-256color".to_owned()),
            true,
        );
        let mut ctx = context(&i18n, 80);
        ctx.color = ColorMode::Always;
        ctx.term = caps;
        assert_eq!(ctx.depth(), ColorDepth::Ansi256);

        let text = renderer_for(Format::Dumb, &caps, None)
            .expect("dumb has a renderer")
            .render(&report, &ctx)
            .expect("renders");
        assert!(text.is_ascii(), "{text}");
        assert!(
            !text.contains('\u{1b}'),
            "dumb is documented colourless, but painted:\n{text}"
        );
    }
}
