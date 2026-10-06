// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The climate-normals surface: the comparison line `art-table` closes with, the `plain` record
//! and the standalone `--format normals` view.
//!
//! The block is built from the model alone (`Report::normals` plus the forecast days it compares
//! against), so a renderer never knows whether the normal came from the network, the cache or a
//! test fixture. Its shape:
//!
//! ```text
//! vs normal 1991–2020: high 26.4 °C (-2.4) · low 16.1 °C (-2.1) · precip 48.9 mm/mo (-100%)
//! Climate normals 1991–2020 · BEIJING, CH (CHM00054511) 12 km · high 26.4 °C (-2.4) · low 16.1 °C (-2.1) · precip 48.9 mm/mo (-100%) · 22 years
//! Climate normals computed from NOAA NCEI Global Summary of the Month (public domain)
//! ```
//!
//! * The comparison is **today's** high and low (`days[0]`) against the normal's mean high and
//!   low, and the **requested span's** precipitation against the normal's share of the same number
//!   of days. A monthly total compared with a three-day forecast would read as a permanent
//!   drought, so the share is what makes the two numbers the same kind of quantity.
//! * Every difference carries its **sign as text** (`+1.4` / `-0.6`), so `--color never` and
//!   `--format dumb` keep the meaning; the temperature differences additionally take the existing
//!   temperature ramp, the precipitation total the precipitation one.
//! * The reading is **optional and best-effort**: a run that never asked, or whose fetch degraded,
//!   carries `None` and every surface is empty — except the standalone view, which says
//!   `climate normals unavailable` like the air view does.
//! * The **credit** sits inside the block (it belongs to those numbers), which is what puts it in
//!   `art-table`'s body and in the `plain` document.

use std::fmt::Write as _;

use chrono::Datelike as _;
use fluent_bundle::FluentValue;

use crate::error::Result;
use crate::geo::location_line;
use crate::i18n::{MessageKey, keys};
use crate::model::Report;
use crate::model::normals::Normals;
use crate::model::units::{
    UnitStyle, format_distance, format_precip, format_temp_delta, format_temp_prec,
};

use super::{RenderContext, Renderer};

/// The `--format normals` renderer: the standalone climate-normals view.
///
/// Without a normal — the run asked, the fetch degraded, or a gate refused because no station was
/// in range or the record was too thin — it prints the `climate normals unavailable` line rather
/// than an empty document, so a script reading stdout still learns what happened.
#[derive(Debug, Clone, Copy, Default)]
pub struct NormalsView;

impl Renderer for NormalsView {
    fn render(&self, report: &Report, ctx: &RenderContext<'_>) -> Result<String> {
        let charset = ctx.term.charset();
        let Some(normals) = report.normals.as_ref() else {
            return Ok(super::air::fold_line(
                &ctx.i18n.text(&keys::NORMALS_UNAVAILABLE),
                charset,
            ));
        };
        let lines = vec![
            location_line(&report.location),
            format!(
                "{}: {}",
                ctx.i18n.text(&keys::NORMALS_PANEL_TITLE),
                body(normals, report, ctx)
            ),
            ctx.i18n.text(&keys::NORMALS_CREDIT_NCEI).into_owned(),
        ];
        // Fold to ASCII first: folding can widen a line (`–` becomes `-`, `—` becomes `--`), so
        // wrapping afterwards is what keeps the width invariant.
        let folded = lines
            .into_iter()
            .map(|line| super::air::fold_line(&line, charset))
            .collect();
        Ok(super::air::wrap_all(folded, ctx.width, charset).join("\n"))
    }
}

/// The `art-table` line: the comparison, wrapped to `ctx.width`; empty when the report carries no
/// normal.
///
/// Wrapped and folded exactly like the air and marine panels: a caller appends the lines to its own
/// output, so the width invariant is the panel's own business.
#[must_use]
pub fn panel(report: &Report, ctx: &RenderContext<'_>) -> Vec<String> {
    let Some(normals) = report.normals.as_ref() else {
        return Vec::new();
    };
    let charset = ctx.term.charset();
    let line = format!(
        "{}: {}",
        ctx.i18n.format(
            &keys::NORMALS_VS,
            &[("period", FluentValue::from(period_label(normals)))]
        ),
        comparison(normals, report, ctx)
    );
    let folded = super::air::fold_line(&line, charset);
    super::air::wrap_all(vec![folded], ctx.width, charset)
}

/// The `plain` format's records: the greppable line and the credit, in that order.
///
/// `plain` ignores the width by contract, so the record is not wrapped.
#[must_use]
pub fn records(report: &Report, ctx: &RenderContext<'_>) -> Vec<String> {
    let Some(normals) = report.normals.as_ref() else {
        return Vec::new();
    };
    vec![
        format!(
            "{} {}",
            super::plain::record_key(&ctx.i18n.text(&keys::NORMALS_PANEL_TITLE)),
            body(normals, report, ctx)
        ),
        ctx.i18n.text(&keys::NORMALS_CREDIT_NCEI).into_owned(),
    ]
}

/// The values line the `plain` record and the standalone view share:
/// `<period> · <station> · <comparison> · <years>`.
fn body(normals: &Normals, report: &Report, ctx: &RenderContext<'_>) -> String {
    format!(
        "{} · {} · {} · {}",
        period_label(normals),
        station_label(normals, ctx),
        comparison(normals, report, ctx),
        ctx.i18n.format(
            &keys::NORMALS_YEARS,
            &[("count", FluentValue::from(normals.years.to_string()))]
        )
    )
}

/// The configured period in its display spelling, `1991-2020` → `1991–2020`.
fn period_label(normals: &Normals) -> String {
    normals.period.replace('-', "\u{2013}")
}

/// `BEIJING, CH (CHM00054511) 12 km`: the station the normal was computed from and how far it sits
/// from the requested point, in the display units.
fn station_label(normals: &Normals, ctx: &RenderContext<'_>) -> String {
    let distance = format_distance(
        station_distance_km(normals),
        ctx.units.distance,
        UnitStyle::Spaced,
    );
    ctx.i18n
        .format(
            &keys::NORMALS_STATION,
            &[
                ("name", FluentValue::from(normals.station_name.clone())),
                ("id", FluentValue::from(normals.station.clone())),
                ("distance", FluentValue::from(distance)),
            ],
        )
        .into_owned()
}

/// A station's distance in the unit formatter's precision (`f32`).
///
/// The model keeps the distance as `f64` because the decode computes it in double precision; by the
/// time it is printed it is a whole-kilometre-scale figure, so the narrowing is the display's.
#[allow(clippy::cast_possible_truncation)]
fn station_distance_km(normals: &Normals) -> f32 {
    normals.distance_km as f32
}

/// `high 26.4 °C (-2.4) · low 16.1 °C (-2.1) · precip 48.9 mm/mo (-100%)`.
fn comparison(normals: &Normals, report: &Report, ctx: &RenderContext<'_>) -> String {
    let day = report.days.first();
    [
        temperature_part(
            normals.temp_max_c,
            day.map(|day| day.temp_max_c),
            &keys::NORMALS_HIGH,
            ctx,
        ),
        temperature_part(
            normals.temp_min_c,
            day.map(|day| day.temp_min_c),
            &keys::NORMALS_LOW,
            ctx,
        ),
        precipitation_part(normals, report, ctx),
    ]
    .join(" · ")
}

/// The normal's mean value with, when the report has a day to compare, the signed difference.
///
/// The difference is painted with the temperature ramp of the *forecast* value, so a warm day's
/// anomaly reads warm; the sign is text and survives `--color never`.
fn temperature_part(
    normal_c: f32,
    forecast_c: Option<f32>,
    label: &MessageKey,
    ctx: &RenderContext<'_>,
) -> String {
    let value = format_temp_prec(normal_c, ctx.units.temp, 1);
    let mut text = ctx
        .i18n
        .format(label, &[("value", FluentValue::from(value))])
        .into_owned();
    if let Some(forecast_c) = forecast_c {
        let delta = format_temp_delta(forecast_c - normal_c, ctx.units.temp, 1);
        let painted = super::color::paint(&delta, super::color::temp_fg(forecast_c), ctx.depth())
            .into_owned();
        let _ = write!(text, " ({painted})");
    }
    text
}

/// The month's normal precipitation total with, when the report's span can be compared, the
/// forecast span's total as a signed percentage of the normal's share.
fn precipitation_part(normals: &Normals, report: &Report, ctx: &RenderContext<'_>) -> String {
    let value = format_precip(normals.precip_mm, ctx.units.precip, UnitStyle::Spaced);
    let painted = super::color::paint(
        &value,
        super::color::precip_fg(normals.precip_mm),
        ctx.depth(),
    )
    .into_owned();
    let mut text = ctx
        .i18n
        .format(
            &keys::NORMALS_PRECIP,
            &[("value", FluentValue::from(painted))],
        )
        .into_owned();
    if let Some(percentage) = precipitation_percentage(normals, report, ctx) {
        let _ = write!(text, " ({percentage:+}%)");
    }
    text
}

/// The forecast span's precipitation total against the normal's share of the same number of days,
/// rounded to whole percent; `None` when there is nothing to compare.
///
/// The normal is a *monthly* total: comparing it with a three-day forecast directly would read as a
/// near-total drought every day, so the share the same number of days would normally hold is the
/// baseline. A station whose normal is zero has no share to measure against, and a report without
/// days has no forecast span.
fn precipitation_percentage(
    normals: &Normals,
    report: &Report,
    ctx: &RenderContext<'_>,
) -> Option<i64> {
    let span_days = f64::from(u32::try_from(report.days.len()).ok()?);
    let month_days = f64::from(days_in_month(ctx.times.date, normals.month));
    let expected = f64::from(normals.precip_mm) * span_days / month_days;
    if expected <= 0.0 {
        return None;
    }
    let total: f64 = report
        .days
        .iter()
        .flat_map(|day| day.parts.iter())
        .map(|part| f64::from(part.precip_mm))
        .sum();
    // The percentage is clamped to a displayable range: a normal of 0.1 mm against a 20 mm
    // forecast span is a real +19 900%, which the line has no room to spell. The clamp also bounds
    // the cast, so the narrowing cannot lose anything but the fraction the `round` removed.
    #[allow(clippy::cast_possible_truncation)]
    let percentage = ((total - expected) / expected * 100.0)
        .round()
        .clamp(-999.0, 999.0) as i64;
    Some(percentage)
}

/// How many days the normal's calendar month has in the run's own year, so a February normal is
/// measured against a leap February when the run happens in one.
fn days_in_month(run_date: chrono::NaiveDate, month: u8) -> u32 {
    let first = chrono::NaiveDate::from_ymd_opt(run_date.year(), u32::from(month), 1);
    let next = first.and_then(|first| first.checked_add_months(chrono::Months::new(1)));
    match (first, next) {
        (Some(first), Some(next)) => {
            u32::try_from(next.signed_duration_since(first).num_days()).unwrap_or(30)
        }
        // Unreachable from a validated configuration (the month is `1..=12`); a 30-day month keeps
        // the comparison meaningful rather than dropping it.
        _ => 30,
    }
}

#[cfg(test)]
mod tests {
    use super::{days_in_month, period_label};

    use crate::model::normals::Normals;

    /// A well-formed run has a month length for every calendar month; the fallback only exists for
    /// an impossible month, which the configuration validator and the decoder both refuse.
    #[test]
    fn a_leap_february_is_twenty_nine_days() {
        let run_date = chrono::NaiveDate::from_ymd_opt(2024, 2, 29).expect("a valid date");
        assert_eq!(days_in_month(run_date, 2), 29);
        assert_eq!(days_in_month(run_date, 1), 31);
        assert_eq!(days_in_month(run_date, 4), 30);
        assert_eq!(
            days_in_month(run_date, 13),
            30,
            "an impossible month falls back"
        );
    }

    /// The display period is an en dash; the configuration value and the `json` object keep the
    /// ASCII spelling they were given.
    #[test]
    fn the_period_reads_as_an_en_dash() {
        let normals = Normals {
            station: "CHM00054511".to_owned(),
            station_name: "BEIJING, CH".to_owned(),
            distance_km: 12.447,
            period: "1991-2020".to_owned(),
            month: 9,
            temp_mean_c: 21.2318,
            temp_max_c: 26.35,
            temp_min_c: 16.0818,
            precip_mm: 48.8818,
            years: 22,
        };
        assert_eq!(period_label(&normals), "1991\u{2013}2020");
    }
}
