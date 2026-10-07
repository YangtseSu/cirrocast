// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The alert banner every format shares, and the `--format alerts` listing.
//!
//! The banner is one line per alert, strongest first (the report's alert list is already ordered by
//! the fetch layer), capped at [`BANNER_MAX`] lines plus a `… and N more` tail so a bad day cannot
//! push the forecast off the screen:
//!
//! ```text
//! ⚠ Tornado Warning — Extreme · until 13:15 CDT · Take shelter now
//! ```
//!
//! The `⚠` is the Unicode charset; an ASCII terminal gets `!` (the glyph is folded by the renderer,
//! but a folded `⚠` has no useful ASCII form). Severity is both a colour and a word, so a
//! monochrome run loses nothing.
//!
//! `--format alerts` prints the same list without the cap and with the fields a reader needs to
//! judge one warning: source, areas, the since/until window, headline, description and the
//! instruction, followed by the per-source credits.
//!
//! This format ignores the resolved width **by contract**: the CAP headline, description and
//! instruction are the issuing agency's own wording, so they are printed verbatim (trimmed of
//! surrounding whitespace, never re-wrapped), because folding a warning's prose to the terminal
//! would edit what the issuer said. Every other format wraps the text it lays out.

use std::fmt::Write as _;

use fluent_bundle::FluentValue;

use super::color::paint_severity;
use super::{Charset, RenderContext, Renderer};
use crate::error::Result;
use crate::i18n::DateStyle;
use crate::i18n::keys;
use crate::model::{Alert, Report, Severity};

/// How many alerts the banner shows before collapsing into the more-count tail.
pub const BANNER_MAX: usize = 3;

/// One banner line: the severity to paint it with, or `None` for the more-count tail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BannerLine {
    /// The severity of the alert the line belongs to; `None` for the tail.
    pub severity: Option<Severity>,
    /// The glyph, the text and the details, ready to print.
    pub text: String,
}

/// The banner lines for `alerts`, at most [`BANNER_MAX`] alerts plus the more-count tail.
///
/// `charset` is the one the calling renderer draws in — the ASCII table and a dumb terminal take
/// the same banner with `!` where a UTF-8 terminal shows `⚠`.
#[must_use]
pub fn banner(alerts: &[Alert], charset: Charset, ctx: &RenderContext<'_>) -> Vec<BannerLine> {
    let glyph = match charset {
        Charset::Unicode => "⚠",
        Charset::Ascii => "!",
    };
    let mut lines: Vec<BannerLine> = alerts
        .iter()
        .take(BANNER_MAX)
        .map(|alert| BannerLine {
            severity: Some(alert.severity),
            text: format!("{glyph} {}", banner_text(alert, ctx)),
        })
        .collect();
    if let Some(more) = alerts
        .len()
        .checked_sub(BANNER_MAX)
        .filter(|more| *more > 0)
    {
        lines.push(BannerLine {
            severity: None,
            text: ctx
                .i18n
                .format(
                    &keys::ALERT_MORE_COUNT,
                    &[("count", FluentValue::from(more.to_string()))],
                )
                .into_owned(),
        });
    }
    lines
}

/// `Tornado Warning — Extreme · until 13:15 CDT · Take shelter now`.
///
/// The event and the severity come from [`keys::ALERT_BANNER_LINE`], so a translation controls the
/// punctuation; the two details are appended with the same separators the English catalog uses,
/// and a detail the alert does not carry is left out entirely.
#[must_use]
pub fn banner_text(alert: &Alert, ctx: &RenderContext<'_>) -> String {
    let mut text = headline_text(alert, ctx);
    let mut detail = Vec::new();
    if let Some(end) = alert.effective_end() {
        detail.push(
            ctx.i18n
                .format(&keys::ALERT_UNTIL, &[("time", time_value(end, ctx))])
                .into_owned(),
        );
    }
    if let Some(instruction) = alert.instruction.as_deref().map(str::trim)
        && !instruction.is_empty()
    {
        detail.push(short_instruction(instruction));
    }
    if !detail.is_empty() {
        text.push_str(" · ");
        text.push_str(&detail.join(" · "));
    }
    text
}

/// The first non-empty line of an instruction, capped so one verbose agency paragraph cannot fill
/// the banner; the full text is still in `--format alerts` and the JSON document.
fn short_instruction(instruction: &str) -> String {
    let first = instruction
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default();
    let mut short: String = first.chars().take(72).collect();
    if first.chars().count() > 72 {
        short.push('…');
    }
    short
}

/// `Tornado Warning — Extreme`, without the details.
#[must_use]
fn headline_text(alert: &Alert, ctx: &RenderContext<'_>) -> String {
    ctx.i18n
        .format(
            &keys::ALERT_BANNER_LINE,
            &[
                ("event", FluentValue::from(alert.event.clone())),
                (
                    "severity",
                    FluentValue::from(ctx.i18n.alert_severity(alert.severity).into_owned()),
                ),
            ],
        )
        .into_owned()
}

/// The since/until window of one alert, e.g. `since 12:37 CDT · until 13:15 CDT`.
fn window_text(alert: &Alert, ctx: &RenderContext<'_>) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(onset) = alert.onset {
        parts.push(
            ctx.i18n
                .format(&keys::ALERT_SINCE, &[("time", time_value(onset, ctx))])
                .into_owned(),
        );
    }
    if let Some(end) = alert.effective_end() {
        parts.push(
            ctx.i18n
                .format(&keys::ALERT_UNTIL, &[("time", time_value(end, ctx))])
                .into_owned(),
        );
    }
    (!parts.is_empty()).then(|| parts.join(" · "))
}

/// An instant as the location's wall clock and zone (`13:15 CDT`).
fn time_value(
    at: chrono::DateTime<chrono::FixedOffset>,
    ctx: &RenderContext<'_>,
) -> FluentValue<'static> {
    let local = at.with_timezone(&ctx.times.tz);
    let clock = local.format("%H:%M %Z").to_string();
    if local.date_naive() == ctx.times.date {
        FluentValue::from(clock)
    } else {
        FluentValue::from(format!(
            "{} {clock}",
            ctx.i18n.format_date(local.date_naive(), DateStyle::Short)
        ))
    }
}

/// The `--format alerts` renderer: the full list, one block per alert.
#[derive(Debug, Clone, Copy, Default)]
pub struct Alerts;

impl Renderer for Alerts {
    fn render(&self, report: &Report, ctx: &RenderContext<'_>) -> Result<String> {
        if report.alerts.is_empty() {
            return Ok(ctx.i18n.text(&keys::ALERT_NONE).into_owned());
        }
        let depth = ctx.depth();
        let mut out = String::new();
        for (index, alert) in report.alerts.iter().enumerate() {
            if index > 0 {
                out.push('\n');
            }
            let headline = headline_text(alert, ctx);
            let _ = writeln!(out, "{}", paint_severity(&headline, alert.severity, depth));

            let mut meta = ctx.i18n.alert_source(alert.source).into_owned();
            if !alert.areas.is_empty() {
                let _ = write!(meta, " · {}", alert.areas.join(", "));
            }
            let _ = writeln!(out, "{meta}");

            if let Some(window) = window_text(alert, ctx) {
                let _ = writeln!(out, "{window}");
            }
            if alert.headline != alert.event && !alert.headline.trim().is_empty() {
                let _ = writeln!(out, "{}", alert.headline.trim());
            }
            if let Some(description) = alert.description.as_deref().map(str::trim)
                && !description.is_empty()
            {
                let _ = writeln!(out, "{description}");
            }
            if let Some(instruction) = alert.instruction.as_deref().map(str::trim)
                && !instruction.is_empty()
            {
                let _ = writeln!(out, "{instruction}");
            }
        }
        for credit in ctx.alert_credits {
            let _ = writeln!(out, "{credit}");
        }
        Ok(out.trim_end().to_owned())
    }
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, FixedOffset};
    use chrono_tz::Tz;

    use super::{BANNER_MAX, banner, banner_text};
    use crate::i18n::{I18n, LanguageRequest};
    use crate::model::{Alert, AlertSource, Certainty, LocalTimes, Severity, Urgency};
    use crate::render::alerts::Alerts;
    use crate::render::{Charset, ColorMode, RenderContext, Renderer, TermCaps};

    /// The fixture clock, derived once: the context borrows it, and every test wants this instant.
    static TIMES: std::sync::LazyLock<LocalTimes> = std::sync::LazyLock::new(|| {
        LocalTimes::new(at("2026-10-03T12:40:00-05:00"), Tz::America__Chicago)
    });

    fn at(text: &str) -> DateTime<FixedOffset> {
        DateTime::parse_from_rfc3339(text).expect("a valid instant")
    }

    fn alert(severity: Severity, event: &str) -> Alert {
        Alert {
            id: format!("id-{event}"),
            source: AlertSource::Nws,
            event: event.to_owned(),
            severity,
            urgency: Urgency::Immediate,
            certainty: Certainty::Observed,
            onset: Some(at("2026-10-03T12:37:00-05:00")),
            expires: Some(at("2026-10-03T13:15:00-05:00")),
            ends: None,
            areas: vec!["Cleveland, OK".to_owned()],
            headline: format!("{event} issued by NWS"),
            description: Some("A confirmed tornado.".to_owned()),
            instruction: Some("Take shelter now.".to_owned()),
            sender: Some("NWS Norman OK".to_owned()),
            credit: Vec::new(),
        }
    }

    fn english() -> I18n {
        I18n::load(&LanguageRequest::Tag("en-US".to_owned()), |_| None)
    }

    fn context<'a>(i18n: &'a I18n, credits: &'a [String]) -> RenderContext<'a> {
        RenderContext {
            units: crate::model::units::UnitSystem::Metric
                .resolve(&crate::config::UnitOverrides::default())
                .expect("the default overrides resolve"),
            color: ColorMode::Never,
            width: 80,
            term: TermCaps::default(),
            times: TIMES.clone(),
            lang: i18n.lang(),
            i18n,
            alert_credits: credits,
            aqi_index: crate::air::aqi::AqiIndex::Us,
        }
    }

    #[test]
    fn the_banner_carries_the_glyph_the_details_and_the_more_count() {
        let i18n = english();
        let ctx = context(&i18n, &[]);
        let alerts: Vec<Alert> = ["A", "B", "C", "D", "E"]
            .iter()
            .map(|name| alert(Severity::Severe, name))
            .collect();
        let lines = banner(&alerts, Charset::Unicode, &ctx);
        assert_eq!(lines.len(), BANNER_MAX + 1);
        assert_eq!(
            lines[0].text,
            "⚠ A — Severe · until 13:15 CDT · Take shelter now."
        );
        assert_eq!(lines[0].severity, Some(Severity::Severe));
        assert_eq!(lines[3].text, "… and 2 more");
        assert_eq!(lines[3].severity, None);

        let ascii = banner(&alerts, Charset::Ascii, &ctx);
        assert!(ascii[0].text.starts_with("! "), "{}", ascii[0].text);
    }

    #[test]
    fn a_detail_the_alert_lacks_is_left_out() {
        let i18n = english();
        let ctx = context(&i18n, &[]);
        let mut bare = alert(Severity::Minor, "Gale");
        bare.instruction = None;
        assert_eq!(banner_text(&bare, &ctx), "Gale — Minor · until 13:15 CDT");
        bare.expires = None;
        bare.ends = None;
        assert_eq!(banner_text(&bare, &ctx), "Gale — Minor");
    }

    #[test]
    fn the_listing_shows_every_alert_and_the_credits() {
        let i18n = english();
        let credits = vec!["Warnings by the WMO SWIC".to_owned()];
        let ctx = context(&i18n, &credits);
        let alerts = vec![alert(Severity::Extreme, "Tornado Warning")];
        let mut report = crate::model::Report {
            location: crate::model::Location {
                name: "Norman".to_owned(),
                admin1: None,
                country: "United States".to_owned(),
                country_code: Some("US".to_owned()),
                lat: 35.22,
                lon: -97.44,
                tz: Tz::America__Chicago,
                elevation_m: None,
                population: None,
                source: crate::model::LocationSource::Geocoder,
                station: None,
                named_by: None,
            },
            current: None,
            days: Vec::new(),
            alerts,
            air: None,
            astro: None,
            marine: None,
            normals: None,
            mode: crate::model::ReportMode::Forecast,
            attribution: crate::model::Attribution::unregistered(
                "nws",
                "https://api.weather.gov/alerts/active",
                chrono::Utc::now(),
                None,
            ),
        };
        let text = Alerts.render(&report, &ctx).expect("the listing renders");
        assert!(text.starts_with("Tornado Warning — Extreme\n"), "{text}");
        assert!(
            text.contains("US National Weather Service · Cleveland, OK"),
            "{text}"
        );
        assert!(text.contains("since 12:37 CDT · until 13:15 CDT"), "{text}");
        assert!(text.contains("Take shelter now."), "{text}");
        assert!(text.contains("Warnings by the WMO SWIC"), "{text}");

        report.alerts.clear();
        assert_eq!(
            Alerts
                .render(&report, &ctx)
                .expect("the empty listing renders"),
            "no active weather alerts"
        );
    }
}
