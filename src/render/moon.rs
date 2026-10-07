// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The moon and sun block: the four-line panel, the `plain` records and the standalone view.
//!
//! The panel is built from the model alone (`Report::astro`), so a renderer never knows how the
//! value got there. Its shape is one art block and four metric lines, the same geometry as the day
//! cells:
//!
//! ```text
//!    ░░░▒█    Moon: Waxing Crescent
//!   ░░░░░▒█   23% illuminated (geocentric) · age 6.5 d
//!   ░░░░░▒█   Moonrise 20:31 · Moonset 11:24
//!    ░░░▒█    Sunrise 06:10 · Sunset 17:57 · daylight 11h 47m
//! ```
//!
//! * A rise or set the day does not have prints the catalog's placeholder (`—`), never `00:00`;
//!   a polar day or night replaces the whole sun line with its name, because there is no time to
//!   show. The values are the ones [`crate::astro::Astro::compute`] produced, so the provider's
//!   sun times and the local fallback look identical here — `astro.sun.source` is what tells them
//!   apart, and the `--verbose` note speaks it out loud.
//! * `--format moon` prints the standalone view: the place, the provenance line (the block is
//!   computed on this machine and costs no request), the panel and the next four phase instants.
//!   The view computes the block itself when the report does not carry one, so the format *is* the
//!   request; the panels of the other formats stay gated by `Report::astro`, which only `--moon`
//!   (or `--format moon`) attaches.
//! * `plain` keeps one record per block (`moon:` and `sun:`), the same dense shape as its other
//!   records, and the ASCII charset swaps the block for its 7-bit transcription.

use std::fmt::Write as _;

use fluent_bundle::FluentValue;
use unicode_width::UnicodeWidthStr as _;

use super::art::ART_W;
use super::art_table::GAP;
use super::plain::record_key;
use super::{RenderContext, Renderer};
use crate::error::Result;
use crate::geo::location_line;
use crate::i18n::keys;
use crate::model::Report;
use crate::model::astro::{Astro, Moon, Polar, Sun};

/// The standalone `--format moon` view.
#[derive(Debug, Clone, Copy, Default)]
pub struct MoonView;

impl Renderer for MoonView {
    fn render(&self, report: &Report, ctx: &RenderContext<'_>) -> Result<String> {
        // The format is the request: a report that does not carry the block (a hand-built one, or
        // a caller that used the library directly) still renders instead of printing nothing.
        let computed;
        let astro = if let Some(astro) = report.astro.as_ref() {
            astro
        } else {
            computed = Astro::compute(report, ctx.times.now);
            &computed
        };
        let mut lines = vec![location_line(&report.location), computed_line(astro, ctx)];
        lines.extend(panel_lines(astro, ctx));
        lines.extend(next_lines(astro, ctx));
        let charset = ctx.term.charset();
        // Fold to ASCII before fitting: folding can widen a line (`—` becomes `--`), so clipping
        // first would let the fold push the line past `--width`.
        Ok(lines
            .into_iter()
            .map(|line| {
                let folded = match charset {
                    super::Charset::Ascii => super::art_table::fold_ascii(&line),
                    super::Charset::Unicode => line,
                };
                super::art_table::fit(&folded, ctx.width, charset).into_owned()
            })
            .collect::<Vec<_>>()
            .join("\n"))
    }
}

/// The block of `art-table`: four lines of moon art and their metrics, or empty without the block.
#[must_use]
pub fn panel(report: &Report, ctx: &RenderContext<'_>) -> Vec<String> {
    report
        .astro
        .as_ref()
        .map_or_else(Vec::new, |astro| panel_lines(astro, ctx))
}

/// The `plain` format's records: one greppable line per block, empty without the block.
#[must_use]
pub fn records(report: &Report, ctx: &RenderContext<'_>) -> Vec<String> {
    let Some(astro) = report.astro.as_ref() else {
        return Vec::new();
    };
    vec![
        format!(
            "{} {}",
            record_key(&ctx.i18n.text(&keys::LABEL_MOON)),
            moon_summary(&astro.moon, ctx)
        ),
        format!(
            "{} {}",
            record_key(&ctx.i18n.text(&keys::LABEL_SUN)),
            sun_summary(&astro.sun, ctx)
        ),
    ]
}

// ---------------------------------------------------------------------------------------------
// The lines
// ---------------------------------------------------------------------------------------------

/// The four lines of the panel: moon art on the left, one metric per line.
fn panel_lines(astro: &Astro, ctx: &RenderContext<'_>) -> Vec<String> {
    let metrics = [
        format!(
            "{}: {}",
            ctx.i18n.text(&keys::LABEL_MOON),
            ctx.i18n.moon_phase(astro.moon.phase)
        ),
        illumination(&astro.moon, ctx),
        moon_events(&astro.moon, ctx),
        sun_summary(&astro.sun, ctx),
    ];
    let charset = ctx.term.charset();
    let drawing = super::art::moon_art(astro.moon.phase, ctx.term.icons, charset);
    metrics
        .iter()
        .enumerate()
        .map(|(index, metric)| {
            let art_line = drawing.line(index);
            format!("{art_line}{}{metric}", gap_after(&art_line))
        })
        .collect()
}

/// The provenance line of the standalone view, and the label for `--format moon`.
fn computed_line(astro: &Astro, ctx: &RenderContext<'_>) -> String {
    let at = astro
        .computed_at
        .with_timezone(&ctx.times.tz)
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, false);
    ctx.i18n
        .format(&keys::ASTRO_COMPUTED, &[("time", FluentValue::from(at))])
        .into_owned()
}

/// The `Next phases:` heading and one indented line per instant, so a long list is never
/// truncated to fit the width; empty when no phase is known.
fn next_lines(astro: &Astro, ctx: &RenderContext<'_>) -> Vec<String> {
    if astro.moon.next.is_empty() {
        return Vec::new();
    }
    let mut lines = vec![format!("{}:", ctx.i18n.text(&keys::ASTRO_NEXT))];
    lines.extend(astro.moon.next.iter().map(|(phase, at)| {
        let local = at.with_timezone(&ctx.times.tz);
        format!(
            "  {} {} {}",
            ctx.i18n.moon_phase(*phase),
            ctx.i18n
                .format_date(local.date_naive(), crate::i18n::DateStyle::Short),
            local.format("%H:%M")
        )
    }));
    lines
}

/// `23% illuminated (geocentric) · age 6.5 d`.
fn illumination(moon: &Moon, ctx: &RenderContext<'_>) -> String {
    let percent = format!(
        "{:.0}",
        (moon.illuminated_fraction * 100.0).clamp(0.0, 100.0)
    );
    let illuminated = ctx.i18n.format(
        &keys::ASTRO_ILLUMINATION,
        &[("percent", fluent_bundle::FluentValue::from(percent))],
    );
    let age = ctx.i18n.format(
        &keys::ASTRO_AGE_DAYS,
        &[(
            "days",
            fluent_bundle::FluentValue::from(format!("{:.1}", moon.age_days)),
        )],
    );
    format!("{illuminated} · {age}")
}

/// `Moonrise 20:31 · Moonset 11:24`, with the placeholder for an event the day does not have.
fn moon_events(moon: &Moon, ctx: &RenderContext<'_>) -> String {
    format!(
        "{} {} · {} {}",
        ctx.i18n.text(&keys::ASTRO_MOONRISE),
        moon.moonrise.map_or_else(|| no_event(ctx), clock_time),
        ctx.i18n.text(&keys::ASTRO_MOONSET),
        moon.moonset.map_or_else(|| no_event(ctx), clock_time),
    )
}

/// The whole moon summary of a `plain` record.
fn moon_summary(moon: &Moon, ctx: &RenderContext<'_>) -> String {
    format!(
        "{}, {}, {} {}, {} {}",
        ctx.i18n.moon_phase(moon.phase),
        illumination(moon, ctx),
        ctx.i18n.text(&keys::ASTRO_MOONRISE),
        moon.moonrise.map_or_else(|| no_event(ctx), clock_time),
        ctx.i18n.text(&keys::ASTRO_MOONSET),
        moon.moonset.map_or_else(|| no_event(ctx), clock_time),
    )
}

/// `Sunrise 06:10 · Sunset 17:57 · daylight 11h 47m`, or the polar label that replaces it.
fn sun_summary(sun: &Sun, ctx: &RenderContext<'_>) -> String {
    if let Some(polar) = sun.polar {
        return ctx
            .i18n
            .text(match polar {
                Polar::Day => &keys::ASTRO_POLAR_DAY,
                Polar::Night => &keys::ASTRO_POLAR_NIGHT,
            })
            .into_owned();
    }
    let mut line = format!(
        "{} {} · {} {}",
        ctx.i18n.text(&keys::ASTRO_SUNRISE),
        sun.sunrise.map_or_else(|| no_event(ctx), clock_time),
        ctx.i18n.text(&keys::ASTRO_SUNSET),
        sun.sunset.map_or_else(|| no_event(ctx), clock_time),
    );
    if let Some(seconds) = sun.daylight_secs {
        let _ = write!(
            line,
            " · {} {}",
            ctx.i18n.text(&keys::ASTRO_DAYLIGHT),
            daylight(seconds)
        );
    }
    line
}

/// The placeholder for an event the local day does not have.
fn no_event(ctx: &RenderContext<'_>) -> String {
    ctx.i18n.text(&keys::ASTRO_NO_RISE).into_owned()
}

// ---------------------------------------------------------------------------------------------
// Formatting helpers
// ---------------------------------------------------------------------------------------------

/// `HH:MM` at the instant's own offset.
fn clock_time(at: chrono::DateTime<chrono::FixedOffset>) -> String {
    at.format("%H:%M").to_string()
}

/// `11h 47m`; `24h 00m` cannot appear because a polar day is labelled instead.
fn daylight(seconds: u32) -> String {
    format!("{}h {:02}m", seconds / 3600, (seconds % 3600) / 60)
}

/// The spaces that put the metrics after an art line, the same gap the day cells use.
fn gap_after(art_line: &str) -> String {
    " ".repeat(ART_W.saturating_sub(art_line.width()) + GAP)
}

#[cfg(test)]
mod tests {
    use chrono_tz::Tz;

    use super::{MoonView, daylight, panel, records};
    use crate::config::UnitOverrides;
    use crate::i18n::{I18n, LanguageRequest};
    use crate::model::astro::{Astro, Moon, MoonPhase, Polar, Sun, SunSource};
    use crate::model::units::UnitSystem;
    use crate::model::{Attribution, LocalTimes, Location, LocationSource, Report};
    use crate::render::{Charset, ColorMode, RenderContext, Renderer, TermCaps};

    fn english() -> I18n {
        I18n::load(&LanguageRequest::Tag("en-US".to_owned()), |_| None)
    }

    fn moment(text: &str) -> chrono::DateTime<chrono::FixedOffset> {
        chrono::DateTime::parse_from_rfc3339(text).expect("a valid instant")
    }

    fn report(astro: Option<Astro>) -> Report {
        Report {
            location: Location {
                name: "Beijing".to_owned(),
                admin1: None,
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
            },
            current: None,
            days: Vec::new(),
            alerts: Vec::new(),
            air: None,
            astro,
            marine: None,
            normals: None,
            mode: crate::model::ReportMode::Forecast,
            attribution: Attribution::unregistered(
                "test",
                "https://example.invalid",
                moment("2026-10-04T00:00:00Z").with_timezone(&chrono::Utc),
                None,
            ),
        }
    }

    fn computed(astro: &str) -> Astro {
        Astro::compute(
            &report(None),
            moment(astro).with_timezone(&chrono::FixedOffset::east_opt(8 * 3600).expect("offset")),
        )
    }

    /// A terminal that can draw the unicode block and the middle dot.
    fn capable() -> TermCaps {
        TermCaps::read(
            |name| match name {
                "TERM" => Some("xterm-256color".to_owned()),
                "LANG" => Some("en_US.UTF-8".to_owned()),
                _ => None,
            },
            true,
        )
    }

    fn context(i18n: &I18n) -> RenderContext<'_> {
        static TIMES: std::sync::LazyLock<LocalTimes> = std::sync::LazyLock::new(|| {
            LocalTimes::new(moment("2026-10-04T12:30:00+08:00"), Tz::Asia__Shanghai)
        });
        RenderContext {
            units: UnitSystem::Metric
                .resolve(&UnitOverrides::default())
                .expect("the default overrides resolve"),
            color: ColorMode::Never,
            width: 80,
            term: capable(),
            times: TIMES.clone(),
            lang: i18n.lang(),
            i18n,
            alert_credits: &[],
            aqi_index: crate::air::aqi::AqiIndex::Us,
        }
    }

    fn fixture_astro() -> Astro {
        Astro {
            moon: Moon {
                phase: MoonPhase::WaxingCrescent,
                illuminated_fraction: 0.231,
                age_days: 6.54,
                moonrise: Some(moment("2026-10-04T07:12:00+08:00")),
                moonset: Some(moment("2026-10-04T21:34:00+08:00")),
                next: vec![
                    (MoonPhase::Full, moment("2026-10-10T22:09:00+08:00")),
                    (MoonPhase::LastQuarter, moment("2026-10-18T12:44:00+08:00")),
                ],
            },
            sun: Sun {
                sunrise: Some(moment("2026-10-04T06:05:00+08:00")),
                sunset: Some(moment("2026-10-04T18:55:00+08:00")),
                daylight_secs: Some(43_200),
                polar: None,
                source: SunSource::Provider,
            },
            computed_at: moment("2026-10-04T12:30:00+08:00").with_timezone(&chrono::Utc),
        }
    }

    #[test]
    fn the_panel_is_four_art_lines_with_one_metric_each() {
        let i18n = english();
        let report = report(Some(fixture_astro()));
        let lines = panel(&report, &context(&i18n));
        assert_eq!(lines.len(), 4, "{lines:#?}");
        // The art column is part of the contract: the metrics follow their own art line.
        let art = super::super::art::moon_lines(MoonPhase::WaxingCrescent, Charset::Unicode);
        for (line, art_line) in lines.iter().zip(art) {
            assert!(
                line.starts_with(art_line),
                "{line:?} lost the art line {art_line:?}"
            );
        }
        assert!(lines[0].contains("Moon: Waxing Crescent"), "{lines:#?}");
        assert!(
            lines[1].contains("23% illuminated (geocentric) · age 6.5 d"),
            "{lines:#?}"
        );
        assert!(
            lines[2].contains("Moonrise 07:12 · Moonset 21:34"),
            "{lines:#?}"
        );
        assert!(
            lines[3].contains("Sunrise 06:05 · Sunset 18:55 · daylight 12h 00m"),
            "{lines:#?}"
        );
        for line in &lines {
            assert!(
                super::super::art_table::display_width(line) <= 80,
                "{line:?} is too wide"
            );
        }

        // The ASCII charset swaps the block for the 7-bit transcription; the middle dot and the
        // em dash are folded by the formats that carry the block (art-table and the view itself).
        let mut ctx = context(&i18n);
        ctx.term = TermCaps::read(|_| None, false);
        let ascii = panel(&report, &ctx);
        assert!(
            ascii
                .iter()
                .all(|line| { super::super::art_table::fold_ascii(line).is_ascii() })
        );
    }

    #[test]
    fn a_missing_event_and_a_polar_day_are_named_not_zeroed() {
        let i18n = english();
        let mut astro = fixture_astro();
        astro.moon.moonrise = None;
        astro.sun.sunrise = None;
        astro.sun.sunset = None;
        astro.sun.daylight_secs = Some(0);
        astro.sun.polar = Some(Polar::Night);
        let lines = panel(&report(Some(astro.clone())), &context(&i18n));
        assert!(
            lines[2].contains("Moonrise — · Moonset 21:34"),
            "{lines:#?}"
        );
        assert!(lines[3].contains("polar night"), "{lines:#?}");
        assert!(!lines[3].contains("00:00"), "{lines:#?}");
        assert!(
            !lines[3].contains("daylight"),
            "a polar day has no daylight span"
        );
        assert!(astronomy_is_finite(&astro));
    }

    /// The block never invents a zero: every number is finite and every fraction in range.
    fn astronomy_is_finite(astro: &Astro) -> bool {
        astro.moon.illuminated_fraction.is_finite()
            && (0.0..=1.0).contains(&astro.moon.illuminated_fraction)
            && astro.moon.age_days.is_finite()
            && astro.moon.age_days >= 0.0
    }

    #[test]
    fn the_records_are_one_line_per_block() {
        let i18n = english();
        let lines = records(&report(Some(fixture_astro())), &context(&i18n));
        assert_eq!(lines.len(), 2, "{lines:#?}");
        assert!(lines[0].starts_with("moon: Waxing Crescent,"), "{lines:#?}");
        assert!(lines[0].contains("Moonrise 07:12"), "{lines:#?}");
        assert!(lines[1].starts_with("sun: Sunrise 06:05"), "{lines:#?}");
        assert!(lines[1].contains("daylight 12h 00m"), "{lines:#?}");
        assert_eq!(
            records(&report(None), &context(&i18n)),
            [] as [std::string::String; 0]
        );
    }

    #[test]
    fn the_standalone_view_names_the_place_the_provenance_and_the_next_phases() {
        let i18n = english();
        let text = MoonView
            .render(&report(Some(fixture_astro())), &context(&i18n))
            .expect("the view renders");
        let lines: Vec<&str> = text.lines().collect();
        assert!(
            lines[0].starts_with("Beijing, China (39.90, 116.41)"),
            "{text}"
        );
        assert!(
            lines[1].starts_with("computed locally (no network) at 2026-10-04T12:30:00+08:00"),
            "{text}"
        );
        assert!(lines[2].contains("Moon: Waxing Crescent"), "{text}");
        assert_eq!(lines[6], "Next phases:", "{text}");
        assert!(
            lines[7].starts_with("  Full Moon Sat 10 Oct 22:09"),
            "{text}"
        );
        assert!(
            lines[8].starts_with("  Last Quarter Sun 18 Oct 12:44"),
            "{text}"
        );
    }

    #[test]
    fn the_standalone_view_computes_the_block_when_the_report_has_none() {
        let i18n = english();
        let text = MoonView
            .render(&report(None), &context(&i18n))
            .expect("the view renders");
        assert!(text.contains("Moon: "), "{text}");
        assert!(text.contains("Sunrise "), "{text}");
        assert!(text.contains("Moonrise "), "{text}");
    }

    #[test]
    fn a_computed_block_renders_without_inventing_values() {
        let i18n = english();
        let astro = computed("2026-10-04T12:30:00+08:00");
        let lines = panel(&report(Some(astro)), &context(&i18n));
        assert_eq!(lines.len(), 4);
        assert!(!lines.iter().any(|line| line.contains("NaN")), "{lines:#?}");
        assert!(
            !lines.iter().any(|line| line.contains("00:00")),
            "a computed block must not clamp a missing event to midnight: {lines:#?}"
        );
    }

    #[test]
    fn the_daylight_span_is_hours_and_padded_minutes() {
        assert_eq!(daylight(0), "0h 00m");
        assert_eq!(daylight(59), "0h 00m");
        assert_eq!(daylight(3_600), "1h 00m");
        assert_eq!(daylight(43_259), "12h 00m");
        assert_eq!(daylight(45_000), "12h 30m");
    }

    #[test]
    fn the_charset_survives_the_ascii_fold() {
        let i18n = english();
        let mut ctx = context(&i18n);
        ctx.term = TermCaps::read(|name| (name == "TERM").then(|| "dumb".to_owned()), false);
        assert_eq!(ctx.term.charset(), Charset::Ascii);
        let text = MoonView
            .render(&report(Some(fixture_astro())), &ctx)
            .expect("the view renders");
        for line in text.lines() {
            assert!(
                super::super::art_table::display_width(line) <= ctx.width,
                "{line:?}"
            );
        }
    }

    #[test]
    fn the_ascii_fold_happens_before_the_width_is_applied() {
        let i18n = english();
        let mut astro = fixture_astro();
        // A day without a moonrise prints the `—` placeholder, which the ASCII fold widens to
        // `--`; folding after clipping would emit a line one column over `--width`.
        astro.moon.moonrise = None;
        let mut ctx = context(&i18n);
        ctx.term = TermCaps::read(|name| (name == "TERM").then(|| "dumb".to_owned()), false);
        ctx.width = 30;
        let text = MoonView
            .render(&report(Some(astro)), &ctx)
            .expect("the view renders");
        assert!(
            text.contains("Moonrise --"),
            "the fold ran on the clipped text: {text}"
        );
        assert!(!text.contains('\u{2014}'), "no em dash survives: {text}");
        for line in text.lines() {
            assert!(
                super::super::art_table::display_width(line) <= ctx.width,
                "{line:?} exceeds {}",
                ctx.width
            );
        }
    }
}
