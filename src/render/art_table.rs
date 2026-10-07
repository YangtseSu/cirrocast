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

/// The document under construction: one output buffer, plus the scratch the current line is
/// composed in.
///
/// Every line is written into [`Table::line`] and handed to [`Table::flush`], which folds it for
/// a 7-bit terminal and clips it to the resolved width exactly once, on the way out. That is the
/// same two steps the old `Vec<String>` + `join` composition ran per line — without the vector,
/// without a `String` per line, and without the second copy of the whole document `join` made.
/// The two buffers are allocated once and reused for every line of the table.
struct Table {
    /// The document, newline-separated, no trailing newline.
    out: String,
    /// The line being composed; cleared by every `flush`.
    line: String,
    /// The 7-bit spelling of `line`, reused by the ASCII fold.
    folded: String,
    /// The charset the document is drawn in.
    charset: Charset,
    /// The width every line is clipped to.
    width: usize,
    /// Whether a line has been flushed; the newline separator is written between lines, exactly
    /// like `join("\n")`, including around empty lines.
    started: bool,
}

impl Table {
    fn new(charset: Charset, width: usize) -> Self {
        Self {
            out: String::with_capacity(4096),
            line: String::with_capacity(256),
            folded: String::with_capacity(256),
            charset,
            width,
            started: false,
        }
    }

    /// Ends the current line: folds and clips it, appends it to the document, and clears the
    /// scratch for the next line.
    fn flush(&mut self) {
        if self.started {
            self.out.push('\n');
        }
        self.started = true;
        match self.charset {
            Charset::Unicode => self
                .out
                .push_str(&fit(&self.line, self.width, self.charset)),
            Charset::Ascii => {
                self.folded.clear();
                write_fold_ascii(&mut self.folded, &self.line);
                self.out
                    .push_str(&fit(&self.folded, self.width, self.charset));
            }
        }
        self.line.clear();
    }

    /// A blank line: the block separator between the table's sections.
    fn blank(&mut self) {
        self.line.clear();
        self.flush();
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
        let mut table = Table::new(charset, ctx.width);

        for line in super::alerts::banner(&report.alerts, charset, ctx) {
            match line.severity {
                Some(severity) => {
                    color::write_paint_severity(&mut table.line, &line.text, severity, depth);
                }
                None => table.line.push_str(&line.text),
            }
            table.flush();
        }
        write_header(&mut table.line, report, ctx);
        table.flush();
        if let Some(current) = &report.current {
            table.blank();
            current_block(&mut table, current, ctx, charset, depth);
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
                write_observed_line(&mut table.line, current, ctx, charset);
                table.flush();
            }
            write_no_forecast_footer(&mut table.line, report, ctx);
            table.flush();
        }
        if !report.days.is_empty() {
            if ctx.width < STACKED_BELOW {
                stacked(&mut table, &report.days, ctx, charset, depth);
            } else {
                columns(&mut table, &report.days, ctx, charset, depth);
            }
        }
        // The moon block sits between the forecast and the air panel: it is sky data like the
        // table's own, while the air panel is a separate reading with its own credit.
        let panels = panel_context(ctx, charset);
        let moon = super::moon::panel(report, &panels);
        if !moon.is_empty() {
            table.blank();
            for line in moon {
                table.line.push_str(&line);
                table.flush();
            }
        }
        // The air panel sits between the forecast and the credits: the air credit is part of the
        // panel (it belongs to those numbers), the place and forecast credits stay last.
        let panel = super::air::panel(report, &panels, depth);
        if !panel.is_empty() {
            table.blank();
            for line in panel {
                table.line.push_str(&line);
                table.flush();
            }
        }
        // The marine panel closes the readings: it is the other best-effort supplementary fetch,
        // and like the air block its credit travels inside the panel.
        let marine = super::marine::panel(report, &panels);
        if !marine.is_empty() {
            table.blank();
            for line in marine {
                table.line.push_str(&line);
                table.flush();
            }
        }
        // The normals line is not a reading: it measures the forecast against the climate, so it
        // closes the block after the readings and before the credits.
        let normals = super::normals::panel(report, &panels);
        if !normals.is_empty() {
            table.blank();
            for line in normals {
                table.line.push_str(&line);
                table.flush();
            }
        }
        write_credits(&mut table, report, ctx);

        Ok(table.out)
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
        let mut out = String::with_capacity(4096);
        let mut first = true;
        for slot in slots {
            if !first {
                out.push_str("\n\n");
            }
            first = false;
            match (slot.report, slot.ctx.as_ref()) {
                (Some(report), Some(ctx)) => out.push_str(&self.render(report, ctx)?),
                _ => out.push_str(&slot.placeholder()),
            }
        }
        Ok(out)
    }

    /// The combined 2–4 location summary, or `None` when it cannot honour the width or the data.
    fn summary(self, slots: &[Slot<'_>]) -> Option<String> {
        if !(2..=SUMMARY_LIMIT).contains(&slots.len()) {
            return None;
        }
        let charset = self.charset;
        // Every slot's context is built from the one `RenderSetup` the run resolved, so the width
        // and the palette are identical across the slots and the first available context speaks
        // for the whole run. A slot that failed carries no context, which is why the search is
        // `find_map` rather than an index.
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

        let mut out = String::with_capacity(4096);
        let mut first = true;
        for (slot, row) in slots.iter().zip(&rows) {
            if !first {
                out.push_str("\n\n");
            }
            first = false;
            let Some(cells) = row else {
                let line = slot.placeholder();
                if display_width(&line) > width {
                    return None;
                }
                out.push_str(&line);
                continue;
            };
            let (Some(report), Some(ctx)) = (slot.report, slot.ctx.as_ref()) else {
                return None;
            };
            let mut header_text = String::with_capacity(64);
            write_header(&mut header_text, report, ctx);
            let header = folded(&header_text, charset);
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
            out.push_str(&header);
            out.push('\n');
            out.push_str(&row_line);
        }
        Some(out)
    }
}

/// One location's summary cells: the current (or today's) condition, temperature and the day's
/// high/low. `None` when there is nothing to show, which falls the whole run back to full tables.
fn summary_cells(
    report: &Report,
    ctx: &RenderContext<'_>,
    charset: Charset,
) -> Option<SummaryCells> {
    let day = crate::template::today(report, ctx.times.date);
    let (condition, is_day, temp_c) = if let Some(current) = &report.current {
        (current.weather, current.is_day, current.temp_c)
    } else {
        let day = day?;
        let kind = ctx.times.part;
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
    let mut padded = String::with_capacity(text.len() + width);
    write_spaces(&mut padded, width.saturating_sub(display_width(text)));
    padded.push_str(text);
    padded
}

/// The context the panels see: an ASCII table (`--format dumb`, or a terminal that cannot draw
/// UTF-8) forces the 7-bit charset on the air and moon blocks too, so a format that promises
/// ASCII cannot end up drawing the unicode moon disc.
fn panel_context<'a>(ctx: &RenderContext<'a>, charset: Charset) -> RenderContext<'a> {
    let mut panels = ctx.clone();
    if charset == Charset::Ascii {
        panels.term.term = TermKind::Dumb;
        panels.term.utf8 = false;
    }
    panels
}

/// `observed 23:51+0800 · 12 min ago`: when the observation was taken and how old it is.
///
/// The clock and its offset are the observation's own (the location's) offset, formatted once, per
/// the contract's `times` clause — the line never converts to UTC and never prints a bare `Z`. The
/// age comes from the injected `ctx.times.now`, so the line is deterministic in tests. A report
/// dated in the future (a clock skew, or a station's own clock) clamps to "0 min ago" rather than
/// printing a negative age.
fn write_observed_line(
    out: &mut String,
    current: &Current,
    ctx: &RenderContext<'_>,
    charset: Charset,
) {
    let minutes = (ctx.times.now - current.observed_at).num_minutes().max(0);
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
    let _ = write!(
        out,
        "{} {} {separator} {age}",
        ctx.i18n.text(&keys::LABEL_OBSERVED),
        current.observed_at.format("%H:%M%z"),
    );
}

/// `no forecast: METAR is an observation`: why an observation-only report has no day table.
fn write_no_forecast_footer(out: &mut String, report: &Report, ctx: &RenderContext<'_>) {
    let provider = if report.attribution.display_name.is_empty() {
        report.attribution.provider.clone()
    } else {
        report.attribution.display_name.clone()
    };
    let text = ctx.i18n.format(
        &keys::NOTE_NO_FORECAST,
        &[("provider", fluent_bundle::FluentValue::from(provider))],
    );
    out.push_str(&text);
}

// ---------------------------------------------------------------------------------------------
// The header and the current conditions
// ---------------------------------------------------------------------------------------------

/// `Weather report: <place> (<lat>, <lon>)`, without coordinates for a location that *is* a
/// coordinate pair — repeating `39.90, 116.41 (39.90, 116.41)` would say nothing.
fn write_header(out: &mut String, report: &Report, ctx: &RenderContext<'_>) {
    let location = &report.location;
    let _ = write!(
        out,
        "{} {}",
        ctx.i18n.text(&keys::LABEL_REPORT),
        place(location)
    );
    if location.source != LocationSource::Coordinates {
        let _ = write!(out, " ({:.2}, {:.2})", location.lat, location.lon);
    }
    // A historical answer says so in the header, with the dates it covers: `--date` and
    // `--history` produce blocks that look exactly like a forecast otherwise.
    if let Some(span) = super::archive_span(report) {
        let _ = write!(out, " · {span} · {}", ctx.i18n.text(&keys::MODE_ARCHIVE));
    }
}

/// The current conditions: four art lines, and to their right the condition, the temperatures, the
/// wind and the measurements.
///
/// The art is chosen from the report's own `is_day`, never from the wall clock; a provider that
/// cannot tell day from night gets the day block.
fn current_block(
    table: &mut Table,
    current: &Current,
    ctx: &RenderContext<'_>,
    charset: Charset,
    depth: ColorDepth,
) {
    let key = weather_key(current.weather.art_key(), current.is_day);
    let block = art::art(key);
    let [art0, art1, art2, art3] = block.map_or(art::NO_BLOCK, |block| block.lines(charset));
    let fg = block.map_or(FG_DEFAULT, |block| color::art_fg(block.style));

    let condition = ctx.i18n.condition(current.weather);
    let mut metric = String::with_capacity(48);

    start_art_line(table, art0, fg, depth);
    color::write_paint(&mut table.line, &condition, fg, depth);
    table.flush();

    start_art_line(table, art1, fg, depth);
    write_temp_metric(
        &mut metric,
        current.temp_c,
        current.feels_like_c,
        ctx.units,
        METRICS_W,
        charset,
    );
    color::write_paint(
        &mut table.line,
        &metric,
        color::temp_fg(current.temp_c),
        depth,
    );
    table.flush();

    start_art_line(table, art2, fg, depth);
    metric.clear();
    write_wind_metric(
        &mut metric,
        current.wind_kmh,
        current.wind_dir_deg,
        ctx,
        charset,
        METRICS_W,
    );
    color::write_paint(
        &mut table.line,
        &metric,
        color::wind_fg(current.wind_kmh),
        depth,
    );
    table.flush();

    start_art_line(table, art3, fg, depth);
    write_measurements(&mut table.line, current, ctx, depth);
    table.flush();
}

/// Starts a line with the art block in the block's own colour and the gap after it.
fn start_art_line(table: &mut Table, art_line: &str, fg: u8, depth: ColorDepth) {
    table.line.clear();
    color::write_paint(&mut table.line, art_line, fg, depth);
    write_spaces(
        &mut table.line,
        ART_W.saturating_sub(display_width(art_line)) + GAP,
    );
}

/// `56% 1013hPa 10km 0.0mm`: the measurements under the current conditions, each in its own
/// palette entry. Visibility is dropped when the provider has no value for it.
fn write_measurements(
    out: &mut String,
    current: &Current,
    ctx: &RenderContext<'_>,
    depth: ColorDepth,
) {
    let units = ctx.units;
    let mut value = String::with_capacity(24);
    if let Some(humidity) = current.humidity_pct {
        let _ = write!(value, "{humidity}%");
        color::write_paint(out, &value, color::humidity_fg(humidity), depth);
        out.push(' ');
    }
    color::write_paint(
        out,
        &format_pressure(current.pressure_hpa, units.pressure, UnitStyle::Compact),
        FG_DEFAULT,
        depth,
    );
    if let Some(visibility) = current.visibility_km {
        out.push(' ');
        color::write_paint(
            out,
            &format_visibility(visibility, units.distance, UnitStyle::Compact),
            FG_DEFAULT,
            depth,
        );
    }
    out.push(' ');
    color::write_paint(
        out,
        &format_precip(current.precip_mm, units.precip, UnitStyle::Compact),
        color::precip_fg(current.precip_mm),
        depth,
    );
}

// ---------------------------------------------------------------------------------------------
// The day columns
// ---------------------------------------------------------------------------------------------

/// The boxed day columns: bands of at most [`CELLS_PER_ROW`] days, each band a box of one header
/// row and four day-part blocks.
///
/// Each day's cell content is composed once into a single text (heading and parts, newline
/// separated) and the rows read their line back out of it, so a cell is never materialised as its
/// own `String` and the row buffer is filled directly.
fn columns(
    table: &mut Table,
    days: &[DayForecast],
    ctx: &RenderContext<'_>,
    charset: Charset,
    depth: ColorDepth,
) {
    let (cell_w, metrics_w) = if ctx.width >= WIDE_FROM {
        (CELL_W, METRICS_W)
    } else {
        (CELL_W_NARROW, METRICS_W_NARROW)
    };
    let per_row = ((ctx.width - 1) / (cell_w + 3)).clamp(1, CELLS_PER_ROW);

    for band in days.chunks(per_row) {
        table.blank();
        let mut day_texts: Vec<String> = Vec::with_capacity(band.len());
        for day in band {
            let mut text = String::with_capacity(512);
            write_day(&mut text, day, ctx, charset, metrics_w, depth);
            day_texts.push(text);
        }
        let height = day_texts.first().map_or(0, |text| text.split('\n').count());

        table.line.clear();
        write_border(
            &mut table.line,
            Border::Top,
            day_texts.len(),
            cell_w,
            charset,
        );
        table.flush();
        for row in 0..height {
            table.line.clear();
            write_cell_row(&mut table.line, &day_texts, row, cell_w, charset);
            table.flush();
            if row % ART_LINES == 0 {
                let kind = if row + 1 == height {
                    Border::Bottom
                } else {
                    Border::Middle
                };
                table.line.clear();
                write_border(&mut table.line, kind, day_texts.len(), cell_w, charset);
                table.flush();
            }
        }
    }
}

/// The `index`-th line of a day text built by [`write_day`]; `""` past the end.
fn nth_line(text: &str, index: usize) -> &str {
    text.split('\n').nth(index).unwrap_or("")
}

/// One day's cell content: the date heading, then the four part blocks, newline separated.
fn write_day(
    text: &mut String,
    day: &DayForecast,
    ctx: &RenderContext<'_>,
    charset: Charset,
    metrics_w: usize,
    depth: ColorDepth,
) {
    let _ = write!(
        text,
        "{}",
        ctx.i18n.format_day_heading(day.date, ctx.times.date)
    );
    let mut metric = String::with_capacity(64);
    for part in &day.parts {
        write_part(text, part, ctx, charset, metrics_w, depth, &mut metric);
    }
}

/// One day part as [`ART_LINES`] lines — the contract documented at the top of the module.
fn write_part(
    text: &mut String,
    part: &DayPart,
    ctx: &RenderContext<'_>,
    charset: Charset,
    metrics_w: usize,
    depth: ColorDepth,
    metric: &mut String,
) {
    let daytime = part.kind != DayPartKind::Night;
    let key = weather_key(part.weather.art_key(), daytime);
    let block = art::art(key);
    let [art0, art1, art2, art3] = block.map_or(art::NO_BLOCK, |block| block.lines(charset));
    let fg = block.map_or(FG_DEFAULT, |block| color::art_fg(block.style));

    let label = ctx.i18n.day_part(part.kind);
    text.push('\n');
    write_cell(
        text, art0, &label, FG_DEFAULT, fg, metrics_w, charset, depth,
    );

    metric.clear();
    write_temp_metric(
        metric,
        part.temp_c,
        part.feels_like_c,
        ctx.units,
        metrics_w,
        charset,
    );
    text.push('\n');
    write_cell(
        text,
        art1,
        metric,
        color::temp_fg(part.temp_c),
        fg,
        metrics_w,
        charset,
        depth,
    );

    metric.clear();
    write_wind_metric(
        metric,
        part.wind_kmh,
        part.wind_dir_deg,
        ctx,
        charset,
        metrics_w,
    );
    text.push('\n');
    write_cell(
        text,
        art2,
        metric,
        color::wind_fg(part.wind_kmh),
        fg,
        metrics_w,
        charset,
        depth,
    );

    metric.clear();
    write_part_tail(metric, part, ctx.units, true);
    text.push('\n');
    write_cell(
        text,
        art3,
        metric,
        color::precip_fg(part.precip_mm),
        fg,
        metrics_w,
        charset,
        depth,
    );
}

/// One line of a day cell: the art line, the gap, then the metric padded to the column.
#[allow(clippy::too_many_arguments)]
fn write_cell(
    out: &mut String,
    art_line: &str,
    metric: &str,
    metric_fg: u8,
    art_fg: u8,
    metrics_w: usize,
    charset: Charset,
    depth: ColorDepth,
) {
    color::write_paint(out, art_line, art_fg, depth);
    write_spaces(out, ART_W.saturating_sub(display_width(art_line)) + GAP);
    let fitted = fit(metric, metrics_w, charset);
    color::write_paint(out, &fitted, metric_fg, depth);
    write_spaces(out, metrics_w.saturating_sub(display_width(&fitted)));
}

/// `0.0mm 56%` — a part's precipitation, and its precipitation probability when the provider
/// reports one and the caller has room for it.
///
/// The probability, not the humidity: this slot pairs with the precipitation amount, exactly as
/// the `plain` document's `0.0mm (0%)` and wttr.in's `0.0 mm | 0%` do. Humidity is a current
/// reading and stays in the conditions block above the table.
fn write_part_tail(out: &mut String, part: &DayPart, units: ResolvedUnits, with_probability: bool) {
    out.push_str(&format_precip(
        part.precip_mm,
        units.precip,
        UnitStyle::Compact,
    ));
    if let Some(probability) = part.precip_prob_pct.filter(|_| with_probability) {
        let _ = write!(out, " {probability}%");
    }
}

// ---------------------------------------------------------------------------------------------
// The stacked layout
// ---------------------------------------------------------------------------------------------

/// One section per day for terminals too narrow for columns: a blank line, the day heading, then
/// one line per part. Same content, no horizontal joining.
fn stacked(
    table: &mut Table,
    days: &[DayForecast],
    ctx: &RenderContext<'_>,
    charset: Charset,
    depth: ColorDepth,
) {
    // The characters of the stacked layout come from the renderer's charset; `ctx.term` only
    // describes the terminal, and `--format dumb` asks for ASCII even on a unicode one.
    let mut scratch = String::with_capacity(64);
    for day in days {
        table.blank();
        let _ = write!(
            table.line,
            "{}",
            ctx.i18n.format_day_heading(day.date, ctx.times.date)
        );
        table.flush();
        for part in &day.parts {
            table.line.clear();
            write_stacked_part(&mut table.line, part, ctx, charset, depth, &mut scratch);
            table.flush();
        }
    }
}

/// `  Morning │ \o/ │ +22°C (+23°C) │ ↗ 12km/h NE │ 0.0mm 56%`
fn write_stacked_part(
    out: &mut String,
    part: &DayPart,
    ctx: &RenderContext<'_>,
    charset: Charset,
    depth: ColorDepth,
    scratch: &mut String,
) {
    let daytime = part.kind != DayPartKind::Night;
    let key = weather_key(part.weather.art_key(), daytime);
    let fg = art::art(key).map_or(FG_DEFAULT, |block| color::art_fg(block.style));
    let glyph = art::one_line_art(key);
    let label = ctx.i18n.day_part(part.kind);
    let separator = vertical(charset);

    // The degradation ladder of a narrow terminal. Apparent temperature, cardinal direction (the
    // arrow already names the sector), precipitation probability, the precipitation tail and then
    // the wind are dropped in that order — every one of them a second reading of something already
    // on the line — before the line is clipped at all. The two rungs that drop the tail and the
    // wind are what keep a width between `MIN_WIDTH` and a full line from silently truncating a
    // complete part.
    for (with_glyph, feels_like, cardinal, probability, with_tail, with_wind) in [
        (true, true, true, true, true, true),
        (true, true, true, false, true, true),
        (true, false, true, false, true, true),
        (true, false, false, false, true, true),
        (false, false, false, false, true, true),
        (false, false, false, false, false, true),
        (false, false, false, false, false, false),
    ] {
        out.clear();
        out.push_str("  ");
        out.push_str(&label);
        write_spaces(out, LABEL_W.saturating_sub(display_width(&label)));
        if with_glyph {
            let _ = write!(out, " {separator} ");
            scratch.clear();
            scratch.push_str(glyph);
            write_spaces(scratch, GLYPH_W.saturating_sub(display_width(glyph)));
            color::write_paint(out, scratch, fg, depth);
        }

        scratch.clear();
        write_temp_metric(
            scratch,
            part.temp_c,
            part.feels_like_c.filter(|_| feels_like),
            ctx.units,
            METRICS_W,
            charset,
        );
        let _ = write!(out, " {separator} ");
        color::write_paint(out, scratch, color::temp_fg(part.temp_c), depth);

        if with_wind {
            scratch.clear();
            write_wind_text(
                scratch,
                part.wind_kmh,
                part.wind_dir_deg,
                ctx,
                charset,
                cardinal,
            );
            let _ = write!(out, " {separator} ");
            color::write_paint(out, scratch, color::wind_fg(part.wind_kmh), depth);
        }
        if with_tail {
            scratch.clear();
            write_part_tail(scratch, part, ctx.units, probability);
            let _ = write!(out, " {separator} ");
            color::write_paint(out, scratch, color::precip_fg(part.precip_mm), depth);
        }

        if display_width(out) <= ctx.width {
            break;
        }
    }
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
fn write_border(out: &mut String, kind: Border, cells: usize, cell_w: usize, charset: Charset) {
    let (left, join, right) = border_glyphs(kind, charset);
    let run = match charset {
        Charset::Unicode => '\u{2500}',
        Charset::Ascii => '-',
    };
    out.reserve(cells * (cell_w + 2 * PAD + 1) + 1);
    out.push(left);
    for index in 0..cells {
        for _ in 0..cell_w + 2 * PAD {
            out.push(run);
        }
        out.push(if index + 1 == cells { right } else { join });
    }
}

/// One screen row: `│`, then every cell padded to `cell_w` and surrounded by [`PAD`] spaces.
///
/// The cells are the `row`-th line of each day text, read back out of it and fitted borrowed, so
/// an untruncated cell — every cell of every committed layout — is copied once, into the row.
fn write_cell_row(out: &mut String, days: &[String], row: usize, cell_w: usize, charset: Charset) {
    let bar = vertical(charset);
    out.reserve(days.len() * (cell_w + 2 * PAD + 1) + 1);
    for day in days {
        out.push(bar);
        write_spaces(out, PAD);
        let fitted = fit(nth_line(day, row), cell_w, charset);
        out.push_str(&fitted);
        write_spaces(out, cell_w.saturating_sub(display_width(&fitted)));
        write_spaces(out, PAD);
    }
    out.push(bar);
}

/// The credits the place and data licences require, next to the data they describe.
fn write_credits(table: &mut Table, report: &Report, ctx: &RenderContext<'_>) {
    let location_credit = attribution_line(&report.location);
    if location_credit.is_none()
        && report.attribution.licence.is_none()
        && ctx.alert_credits.is_empty()
    {
        return;
    }
    table.blank();
    if let Some(location) = location_credit {
        table.line.push_str(location);
        table.flush();
    }
    if let Some(licence) = report.attribution.licence.as_deref() {
        let _ = write!(table.line, "{} {licence}", ctx.i18n.text(&keys::LABEL_DATA));
        table.flush();
    }
    for credit in ctx.alert_credits {
        table.line.push_str(credit);
        table.flush();
    }
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

/// Writes `count` spaces — the padding every cell and gap of the table is made of, without a
/// temporary `String` per call.
fn write_spaces(out: &mut String, count: usize) {
    for _ in 0..count {
        out.push(' ');
    }
}

/// The spaces that make `text` occupy `width` display columns, never negative.
fn pad_columns(text: &str, width: usize) -> String {
    let mut padded = String::with_capacity(text.len() + width);
    padded.push_str(text);
    write_spaces(&mut padded, width.saturating_sub(display_width(text)));
    padded
}

/// `text` in the charset's spelling.
fn folded(text: &str, charset: Charset) -> String {
    let mut folded = String::with_capacity(text.len());
    write_folded(&mut folded, text, charset);
    folded
}

/// Writes `text` in the charset's spelling into `out`.
fn write_folded(out: &mut String, text: &str, charset: Charset) {
    match charset {
        Charset::Ascii => write_fold_ascii(out, text),
        Charset::Unicode => out.push_str(text),
    }
}

/// The 7-bit spelling of the few glyphs the table composes itself and that have no ASCII form:
/// the degree sign of a temperature, the em dash of a credit line, the en dash of a period, the
/// middle dot of a moonlit block and the micro/superscript of the air panel's units.
///
/// Only the renderer's own chrome is folded. A place name is the user's data: replacing a city
/// with question marks would be worse than the byte a dumb terminal cannot draw.
pub(crate) fn fold_ascii(text: &str) -> String {
    if text.is_ascii() {
        return text.to_owned();
    }
    let mut folded = String::with_capacity(text.len());
    write_fold_ascii(&mut folded, text);
    folded
}

/// Writes the 7-bit spelling of `text` into `out`, with the rules of [`fold_ascii`].
pub(crate) fn write_fold_ascii(out: &mut String, text: &str) {
    for character in text.chars() {
        match character {
            '\u{b0}' => {}
            '\u{2014}' => out.push_str("--"),
            '\u{2013}' => out.push('-'),
            '\u{b7}' => out.push('.'),
            // `μ` (micro sign) and `μ` (Greek mu) are both in use for μg/m³, and `³` has no ASCII
            // form either; the unit reads `ug/m3` on a dumb terminal.
            '\u{b5}' | '\u{3bc}' => out.push('u'),
            '\u{b3}' => out.push('3'),
            other => out.push(other),
        }
    }
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
/// temperature keeps the number a reader came for. The caller's buffer is the scratch: it is
/// cleared, filled with the pair, and shortened back to the single reading when the pair is too
/// wide.
fn write_temp_metric(
    out: &mut String,
    temp_c: f32,
    feels_like_c: Option<f32>,
    units: ResolvedUnits,
    metrics_w: usize,
    charset: Charset,
) {
    // The ASCII table has no degree sign, and the column width has to be measured in the
    // characters that will really be printed — otherwise every ASCII cell is two columns short.
    out.clear();
    write_folded(out, &format_temp_signed(temp_c, units.temp), charset);
    if let Some(feels_like) = feels_like_c {
        let single = out.len();
        out.push_str(" (");
        write_folded(out, &format_temp_signed(feels_like, units.temp), charset);
        out.push(')');
        if display_width(out) <= metrics_w {
            return;
        }
        out.truncate(single);
    }
    if display_width(out) > metrics_w {
        let fitted = fit(out, metrics_w, charset).into_owned();
        out.clear();
        out.push_str(&fitted);
    }
}

/// `↗ 12km/h NE`, or less of it when the column is narrow.
///
/// The cardinal direction goes first — the arrow already names the sector — and only then is the
/// text clipped, so a ten column cell still reads `↗ 12km/h` instead of `↗ 12km…`. The caller's
/// buffer holds whichever candidate survives.
fn write_wind_metric(
    out: &mut String,
    kmh: f32,
    dir_deg: Option<u16>,
    ctx: &RenderContext<'_>,
    charset: Charset,
    metrics_w: usize,
) {
    out.clear();
    write_wind_text(out, kmh, dir_deg, ctx, charset, true);
    if display_width(out) <= metrics_w {
        return;
    }
    out.clear();
    write_wind_text(out, kmh, dir_deg, ctx, charset, false);
    if display_width(out) <= metrics_w {
        return;
    }
    out.clear();
    let speed = format_wind(kmh, ctx.units.wind, UnitStyle::Compact);
    out.push_str(&fit(&speed, metrics_w, charset));
}

/// `↗ 12km/h NE`, or `↗ 12km/h` when the caller has no room for the cardinal direction.
///
/// The arrow comes from [`art::wind_arrow`] and follows the character set, while the point's name
/// comes from the catalog: a dumb terminal draws an ASCII arrow where a UTF-8 one draws a glyph, and
/// a Chinese run reads `东北风` where an English one reads `NE`. `out` must be empty (or already
/// hold a prefix the caller wants).
fn write_wind_text(
    out: &mut String,
    kmh: f32,
    dir_deg: Option<u16>,
    ctx: &RenderContext<'_>,
    charset: Charset,
    with_cardinal: bool,
) {
    let speed = format_wind(kmh, ctx.units.wind, UnitStyle::Compact);
    match dir_deg {
        Some(dir) => {
            let arrow = art::wind_arrow(dir, charset);
            if with_cardinal {
                let _ = write!(out, "{arrow} {speed} {}", ctx.i18n.direction(dir));
            } else {
                let _ = write!(out, "{arrow} {speed}");
            }
        }
        None => out.push_str(&speed),
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
        Attribution, Condition, Current, DayForecast, DayPart, DayPartKind, LocalTimes, Location,
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
            named_by: None,
        }
    }

    fn current(is_day: bool, code: u8) -> Current {
        Current {
            observed_at: moment(12, 15),
            temp_c: 22.4,
            feels_like_c: Some(23.6),
            humidity_pct: Some(56),
            precip_mm: 0.0,
            weather: Condition::from_u8(code),
            cloud_cover_pct: Some(40),
            pressure_hpa: 1013.0,
            wind_kmh: 12.0,
            wind_dir_deg: Some(45),
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
            marine: None,
            normals: None,
            mode: crate::model::ReportMode::Forecast,
            attribution: attribution(),
        }
    }

    fn context(i18n: &I18n, width: usize) -> RenderContext<'_> {
        static TIMES: std::sync::LazyLock<LocalTimes> =
            std::sync::LazyLock::new(|| LocalTimes::new(moment(12, 30), Tz::Asia__Shanghai));
        RenderContext {
            units: UnitSystem::Metric
                .resolve(&UnitOverrides::default())
                .expect("the default overrides resolve"),
            color: ColorMode::Never,
            width,
            term: TermCaps::default(),
            times: TIMES.clone(),
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
    fn a_missing_humidity_is_omitted_from_the_measurements_line() {
        let mut current = current(true, 2);
        current.humidity_pct = None;
        let text = render(&report(Some(current), Vec::new()), 80, Charset::Unicode);
        assert!(
            !text.contains('%'),
            "no humidity percent is invented: {text}"
        );
        assert!(
            text.lines().any(|line| line.contains("1013hPa 10km 0.0mm")),
            "the other measurements stay: {text}"
        );
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
    fn the_observed_line_prints_the_location_offset_not_utc() {
        let mut report = report(Some(current(true, 2)), Vec::new());
        let mut capabilities = crate::model::ReportCapabilities::open_meteo_test();
        capabilities.daily = false;
        report.attribution.capabilities = Some(capabilities);

        let text = render(&report, 80, Charset::Unicode);
        let observed = text
            .lines()
            .find(|line| line.starts_with("observed "))
            .expect("the observation line");
        assert!(observed.contains("12:15+0800"), "{observed}");
        assert!(
            !observed.contains('Z'),
            "the location offset, not a bare `Z`: {observed}"
        );
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
