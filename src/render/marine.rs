// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The marine panel: the wave block the table formats append and the `plain` records carry.
//!
//! The panel is built from the model alone (`Report::marine`), so a renderer never knows whether
//! the reading came from the network, the cache or a test fixture. Its shape:
//!
//! ```text
//! Marine: waves 1.4 m · period 6.8 s · from 280° · swell 0.9 m · sea 14.2 °C
//! sampled at 54.542, 10.208 (1.7 km from the location)
//! Marine data: Open-Meteo.com (CC BY 4.0) — … (Copernicus Marine Service, DWD ICON Wave)
//! ```
//!
//! * The reading is **optional and best-effort**: a run that never asked, or whose fetch degraded,
//!   carries `None` and the panel is empty, exactly like the air panel.
//! * The **sampled cell** line appears only when the answer came from a sea cell further than
//!   [`FAR_CELL_KM`](crate::model::FAR_CELL_KM) from the requested point, so a coastal city's
//!   panel does not repeat its own coordinates and an inland point is never presented as a
//!   shoreline.
//! * `--format dumb`/ASCII terminals get the same panel through the shared folding, so `·` becomes
//!   `.` and `°` reads `deg`.
//! * The credit sits inside the panel, which is what puts it in `art-table`'s body and in the
//!   `plain` document; the block belongs to those numbers.

use crate::i18n::keys;
use crate::model::{Marine, Report};

use super::RenderContext;

/// The panel block, wrapped to `ctx.width`; empty when the report carries no reading.
///
/// Wrapped and folded exactly like the air panel ([`super::air::panel`]): a caller appends the
/// lines to its own output, so the width invariant is the panel's own business.
#[must_use]
pub fn panel(report: &Report, ctx: &RenderContext<'_>) -> Vec<String> {
    let Some(marine) = report.marine.as_ref() else {
        return Vec::new();
    };
    let charset = ctx.term.charset();
    let lines = lines(marine, ctx)
        .into_iter()
        .map(|line| super::air::fold_line(&line, charset))
        .collect();
    super::air::wrap_all(lines, ctx.width, charset)
}

/// The `plain` format's records: one greppable line per value group, the credit last.
///
/// `plain` ignores the width by contract, so the records are not wrapped.
#[must_use]
pub fn records(report: &Report, ctx: &RenderContext<'_>) -> Vec<String> {
    let Some(marine) = report.marine.as_ref() else {
        return Vec::new();
    };
    let mut lines = vec![format!(
        "{} {}",
        super::plain::record_key(&ctx.i18n.text(&keys::MARINE_PANEL_TITLE)),
        current_body(marine, ctx)
    )];
    for day in &marine.days {
        lines.push(format!(
            "{} {}",
            super::plain::record_key(&ctx.i18n.text(&keys::MARINE_PANEL_TITLE)),
            ctx.i18n.format(
                &keys::MARINE_DAILY_MAX,
                &[
                    (
                        "date",
                        fluent_bundle::FluentValue::from(day.date.to_string())
                    ),
                    (
                        "height",
                        fluent_bundle::FluentValue::from(
                            day.wave_height_max_m.map_or_else(|| "—".to_owned(), number),
                        ),
                    ),
                    (
                        "degrees",
                        fluent_bundle::FluentValue::from(
                            day.wave_direction_dominant_deg
                                .map_or_else(|| "—".to_owned(), |degrees| degrees.to_string()),
                        ),
                    ),
                ],
            )
        ));
    }
    if let Some(sampled) = sampled_line(marine, ctx) {
        lines.push(sampled);
    }
    lines.push(ctx.i18n.text(&keys::MARINE_CREDIT_OPEN_METEO).into_owned());
    lines
}

/// The panel before folding and wrapping: the sea state, the far cell and the credit.
fn lines(marine: &Marine, ctx: &RenderContext<'_>) -> Vec<String> {
    let mut lines = vec![format!(
        "{}: {}",
        ctx.i18n.text(&keys::MARINE_PANEL_TITLE),
        current_body(marine, ctx)
    )];
    lines.extend(sampled_line(marine, ctx));
    lines.push(ctx.i18n.text(&keys::MARINE_CREDIT_OPEN_METEO).into_owned());
    lines
}

/// `waves 1.4 m · period 6.8 s · from 280° · swell 0.9 m · sea 14.2 °C`, omitting what the source
/// did not report.
fn current_body(marine: &Marine, ctx: &RenderContext<'_>) -> String {
    let mut parts = Vec::new();
    if let Some(height) = marine.wave_height_m {
        parts.push(entry(ctx, &keys::MARINE_WAVES, "height", number(height)));
    }
    if let Some(period) = marine.wave_period_s {
        parts.push(entry(ctx, &keys::MARINE_PERIOD, "seconds", number(period)));
    }
    if let Some(degrees) = marine.wave_direction_deg {
        parts.push(entry(
            ctx,
            &keys::MARINE_FROM,
            "degrees",
            degrees.to_string(),
        ));
    }
    if let Some(height) = marine.swell_wave_height_m {
        parts.push(entry(ctx, &keys::MARINE_SWELL, "height", number(height)));
    }
    if let Some(temp) = marine.sea_surface_temp_c {
        parts.push(entry(ctx, &keys::MARINE_SEA, "temp", number(temp)));
    }
    parts.join(" · ")
}

/// The sampled-cell line, present only when the cell is far enough that a reader must know.
fn sampled_line(marine: &Marine, ctx: &RenderContext<'_>) -> Option<String> {
    if !marine.sampled_cell_is_far() {
        return None;
    }
    Some(
        ctx.i18n
            .format(
                &keys::MARINE_SAMPLED,
                &[
                    (
                        "lat",
                        fluent_bundle::FluentValue::from(format!("{:.3}", marine.sampled_lat)),
                    ),
                    (
                        "lon",
                        fluent_bundle::FluentValue::from(format!("{:.3}", marine.sampled_lon)),
                    ),
                    (
                        "distance",
                        fluent_bundle::FluentValue::from(number(marine.distance_km)),
                    ),
                ],
            )
            .into_owned(),
    )
}

/// One `label value unit` message with its single argument.
fn entry(
    ctx: &RenderContext<'_>,
    key: &crate::i18n::MessageKey,
    argument: &'static str,
    value: String,
) -> String {
    ctx.i18n
        .format(key, &[(argument, fluent_bundle::FluentValue::from(value))])
        .into_owned()
}

/// One decimal at most, without a trailing `.0` and without a negative zero; the same spelling the
/// air panel uses, so two panels of numbers read alike.
fn number(value: f64) -> String {
    super::air::number(value)
}
