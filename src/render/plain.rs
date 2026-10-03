// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The `plain` format: box-free, ANSI-free lines that survive a pipe.
//!
//! One line per record, each starting with its label, so `grep '^day'` and `awk -F': '` work
//! without parsing prose:
//!
//! ```text
//! location: Beijing, Beijing, China (39.90, 116.41) Asia/Shanghai
//! updated: 2026-09-30T12:15:00+08:00
//! current: Partly cloudy 31°C (feels 36°C) wind 12km/h SE humidity 66% precip 0.0mm pressure 1004hPa visibility 10km
//! day 2026-09-30: Morning Partly cloudy 29°C 0.0mm (10%) wind 8km/h S | …
//! Location data based on GeoNames (CC-BY-4.0) via Open-Meteo — https://open-meteo.com/
//! Data: Open-Meteo.com (CC BY 4.0)
//! attribution: open-meteo https://api.open-meteo.com/v1/forecast
//! ```
//!
//! The record keys — `location`, `updated`, `current`, `day`, `attribution` — are catalog labels
//! rather than literals, so a translated run reads `地点: 北京…`; the line shape, including the
//! colon, is the format's contract.
//!
//! Every value goes through [`crate::model::units`], and the formatters are the ones the
//! `one-line` tokens use ([`super::one_line`]), so the two formats cannot drift into two
//! conversion paths.
//!
//! # Width
//!
//! Unlike the table formats, `plain` **ignores the resolved width**: a line here is a record, and
//! truncating a record to fit a terminal silently deletes the values the format exists to carry
//! (the day summary is wider than 80 columns on purpose). A pipe wraps or not at its leisure, and
//! the renderer's job is to keep every value it was given. `--width` shapes `art-table`/`dumb`.

use std::fmt::Write as _;

use super::{RenderContext, Renderer};
use crate::error::Result;
use crate::geo::{attribution_line, location_line};
use crate::i18n::keys;
use crate::model::units::{
    UnitStyle, format_precip, format_pressure, format_temp, format_visibility, format_wind,
};
use crate::model::{Current, DayForecast, DayPart, Report};

/// The plain renderer.
#[derive(Debug, Clone, Copy, Default)]
pub struct Plain;

impl Renderer for Plain {
    fn render(&self, report: &Report, ctx: &RenderContext<'_>) -> Result<String> {
        let mut lines = vec![format!(
            "{} {}",
            record_key(&ctx.i18n.text(&keys::LABEL_LOCATION)),
            location_line(&report.location)
        )];

        // The banner, degraded to the record shape: the warning sign and the colour are dropped
        // (a pipe gets no escapes), and the record key keeps the line greppable.
        for alert in &report.alerts {
            lines.push(format!(
                "{} {}",
                record_key(&ctx.i18n.text(&keys::LABEL_ALERT)),
                super::alerts::banner_text(alert, ctx)
            ));
        }

        if let Some(current) = &report.current {
            lines.push(updated_line(current, ctx));
            lines.push(current_line(current, ctx));
        }
        for day in &report.days {
            lines.push(day_line(day, ctx));
        }
        lines.extend(super::air::records(report, ctx));

        if let Some(credit) = attribution_line(&report.location) {
            lines.push(credit.to_owned());
        }
        if let Some(licence) = report.attribution.licence.as_deref() {
            lines.push(format!("{} {licence}", ctx.i18n.text(&keys::LABEL_DATA)));
        }
        lines.extend(ctx.alert_credits.iter().cloned());
        lines.push(format!(
            "{} {} {}",
            record_key(&ctx.i18n.text(&keys::LABEL_ATTRIBUTION)),
            report.attribution.provider,
            endpoint(&report.attribution.url)
        ));

        Ok(lines.join("\n"))
    }
}

/// When the current conditions were observed, at the location's own offset.
fn updated_line(current: &Current, ctx: &RenderContext<'_>) -> String {
    format!(
        "{} {}",
        record_key(&ctx.i18n.text(&keys::LABEL_UPDATED)),
        current
            .observed_at
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, false)
    )
}

/// A record key: the catalog's label in the shape the format documents — lower case, no spaces.
///
/// English labels are already spelled that way (`location`), so the key a script greps for does not
/// change; a language whose label contains spaces or capitals gets the same stable shape instead of
/// a second format.
pub(crate) fn record_key(label: &str) -> String {
    let mut key: String = label
        .chars()
        .map(|character| {
            if character.is_whitespace() {
                '_'
            } else {
                character
            }
        })
        .collect();
    key.make_ascii_lowercase();
    key.push(':');
    key
}

/// The current conditions: one line, the parts the provider has no value for omitted.
fn current_line(current: &Current, ctx: &RenderContext<'_>) -> String {
    let units = ctx.units;
    let mut text = format!(
        "{} {} {}",
        record_key(&ctx.i18n.text(&keys::LABEL_CURRENT)),
        ctx.i18n.condition(current.weather),
        format_temp(current.temp_c, units.temp),
    );
    if let Some(feels_like) = current.feels_like_c {
        let _ = write!(
            text,
            " ({} {})",
            ctx.i18n.text(&keys::LABEL_FEELS),
            format_temp(feels_like, units.temp),
        );
    }
    let _ = write!(
        text,
        " {} {} {} {} {} {} {} {} {}",
        ctx.i18n.text(&keys::LABEL_WIND),
        format_wind(current.wind_kmh, units.wind, UnitStyle::Compact),
        ctx.i18n.direction(current.wind_dir_deg),
        ctx.i18n.text(&keys::LABEL_HUMIDITY),
        ctx.i18n.format(
            &keys::FORMAT_HUMIDITY,
            &[(
                "value",
                fluent_bundle::FluentValue::from(current.humidity_pct.to_string())
            )]
        ),
        ctx.i18n.text(&keys::LABEL_PRECIP),
        format_precip(current.precip_mm, units.precip, UnitStyle::Compact),
        ctx.i18n.text(&keys::LABEL_PRESSURE),
        format_pressure(current.pressure_hpa, units.pressure, UnitStyle::Compact),
    );
    if let Some(visibility) = current.visibility_km {
        let _ = write!(
            text,
            " {} {}",
            ctx.i18n.text(&keys::LABEL_VISIBILITY),
            format_visibility(visibility, units.distance, UnitStyle::Compact)
        );
    }
    text
}

/// One forecast day: the date, then the four parts in display order.
fn day_line(day: &DayForecast, ctx: &RenderContext<'_>) -> String {
    let parts: Vec<String> = day
        .parts
        .iter()
        .map(|part| part_summary(part, ctx))
        .collect();
    format!(
        "{} {}: {}",
        ctx.i18n.text(&keys::LABEL_DAY),
        day.date,
        parts.join(" | ")
    )
}

/// One part of a day: label, condition, temperature, precipitation and wind.
fn part_summary(part: &DayPart, ctx: &RenderContext<'_>) -> String {
    let units = ctx.units;
    let mut text = format!(
        "{} {} {} {}",
        ctx.i18n.day_part(part.kind),
        ctx.i18n.condition(part.weather),
        format_temp(part.temp_c, units.temp),
        format_precip(part.precip_mm, units.precip, UnitStyle::Compact),
    );
    if let Some(probability) = part.precip_prob_pct {
        let _ = write!(text, " ({probability}%)");
    }
    let _ = write!(
        text,
        " {} {}",
        ctx.i18n.text(&keys::LABEL_WIND),
        format_wind(part.wind_kmh, units.wind, UnitStyle::Compact)
    );
    if let Some(direction) = part.wind_dir_deg {
        let _ = write!(text, " {}", ctx.i18n.direction(direction));
    }
    text
}

/// The endpoint of a request URL: everything before the query string.
fn endpoint(url: &str) -> &str {
    url.split('?').next().unwrap_or(url)
}

#[cfg(test)]
mod tests {
    // The expected lines are built from literals, so exact float comparison is the assertion.
    #![allow(clippy::float_cmp)]

    use chrono::{FixedOffset, TimeZone as _, Utc};
    use chrono_tz::Tz;

    use super::Plain;
    use crate::i18n::{I18n, LanguageId, LanguageRequest};
    use crate::model::units::UnitSystem;
    use crate::model::{
        Attribution, Condition, Current, DayForecast, DayPart, DayPartKind, Location,
        LocationSource, Report,
    };
    use crate::render::{ColorMode, RenderContext, Renderer, TermCaps};

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

    fn current() -> Current {
        Current {
            observed_at: moment(12, 15),
            temp_c: 31.0,
            feels_like_c: Some(36.0),
            humidity_pct: 66,
            precip_mm: 0.0,
            weather: Condition::from_u8(2),
            cloud_cover_pct: 40,
            pressure_hpa: 1004.0,
            wind_kmh: 12.0,
            wind_dir_deg: 135,
            wind_gust_kmh: None,
            visibility_km: Some(10.0),
            uv_index: Some(5.0),
            is_day: true,
        }
    }

    fn part(
        kind: DayPartKind,
        temp_c: f32,
        code: u8,
        precip_mm: f32,
        pct: u8,
        wind: f32,
        dir: u16,
    ) -> DayPart {
        DayPart {
            kind,
            temp_c,
            feels_like_c: Some(temp_c),
            precip_mm,
            precip_prob_pct: Some(pct),
            weather: Condition::from_u8(code),
            wind_kmh: wind,
            wind_dir_deg: Some(dir),
            humidity_pct: Some(60),
            visibility_km: Some(10.0),
        }
    }

    fn day() -> DayForecast {
        DayForecast {
            date: chrono::NaiveDate::from_ymd_opt(2026, 9, 30).expect("a date"),
            parts: [
                part(DayPartKind::Morning, 29.0, 2, 0.0, 10, 8.0, 180),
                part(DayPartKind::Noon, 33.0, 3, 0.2, 25, 15.0, 135),
                part(DayPartKind::Evening, 30.0, 61, 1.1, 60, 12.0, 90),
                part(DayPartKind::Night, 27.0, 0, 0.0, 5, 6.0, 0),
            ],
            temp_min_c: 26.0,
            temp_max_c: 34.0,
            sunrise: Some(moment(6, 8)),
            sunset: Some(moment(17, 58)),
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
            url: "https://api.open-meteo.com/v1/forecast?latitude=39.9042&longitude=116.4074"
                .to_owned(),
            fetched_at: Utc
                .with_ymd_and_hms(2026, 9, 30, 4, 15, 0)
                .single()
                .expect("an instant"),
            raw: None,
        }
    }

    fn report() -> Report {
        Report {
            location: location(),
            current: Some(current()),
            days: vec![day()],
            alerts: Vec::new(),
            air: None,
            astro: None,
            attribution: attribution(),
        }
    }

    fn context(i18n: &I18n, units: UnitSystem, width: usize) -> RenderContext<'_> {
        RenderContext {
            units: units
                .resolve(&crate::config::UnitOverrides::default())
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

    fn render(report: &Report, units: UnitSystem, width: usize) -> String {
        let i18n = english();
        Plain
            .render(report, &context(&i18n, units, width))
            .expect("plain always renders")
    }

    #[test]
    fn the_metric_lines_are_exactly_these() {
        let text = render(&report(), UnitSystem::Metric, 200);
        assert_eq!(
            text,
            "\
location: Beijing, Beijing, China (39.90, 116.41) Asia/Shanghai
updated: 2026-09-30T12:15:00+08:00
current: Partly cloudy 31°C (feels 36°C) wind 12km/h SE humidity 66% precip 0.0mm pressure 1004hPa visibility 10km
day 2026-09-30: Morning Partly cloudy 29°C 0.0mm (10%) wind 8.0km/h S | Noon Overcast 33°C 0.2mm (25%) wind 15km/h SE | Evening Slight rain 30°C 1.1mm (60%) wind 12km/h E | Night Clear sky 27°C 0.0mm (5%) wind 6.0km/h N
Location data based on GeoNames (CC-BY-4.0) via Open-Meteo — https://open-meteo.com/
Data: Open-Meteo.com (CC BY 4.0)
attribution: open-meteo https://api.open-meteo.com/v1/forecast"
        );
        assert!(
            !text.contains('\u{1b}'),
            "plain never emits an escape sequence"
        );
    }

    #[test]
    fn the_other_two_unit_systems_convert_at_display_time_only() {
        let report = report();
        assert_eq!(
            report.current.as_ref().expect("a current block").temp_c,
            31.0
        );

        let us = render(&report, UnitSystem::Us, 200);
        assert!(
            us.contains("current: Partly cloudy 88°F (feels 97°F)"),
            "{us}"
        );
        assert!(us.contains("wind 7.5mph SE"), "{us}");
        assert!(us.contains("pressure 29.65inHg"), "{us}");
        assert!(us.contains("visibility 6.2mi"), "{us}");
        assert!(us.contains("Morning Partly cloudy 84°F"), "{us}");

        let uk = render(&report, UnitSystem::Uk, 200);
        assert!(
            uk.contains("current: Partly cloudy 31°C (feels 36°C)"),
            "{uk}"
        );
        assert!(uk.contains("wind 7.5mph SE"), "{uk}");
        assert!(uk.contains("pressure 1004hPa"), "{uk}");
        assert!(uk.contains("visibility 6.2mi"), "{uk}");
    }

    #[test]
    fn a_report_without_days_keeps_the_header_the_update_and_the_current_line() {
        let mut report = report();
        report.days.clear();
        let text = render(&report, UnitSystem::Metric, 200);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 6, "{text}");
        assert!(lines[1].starts_with("updated: "));
        assert!(lines[2].starts_with("current: "));
        assert!(
            !text.contains("\nday "),
            "a current-only report has no day lines: {text}"
        );
    }

    #[test]
    fn a_report_without_current_conditions_omits_the_updated_and_current_lines() {
        let mut report = report();
        report.current = None;
        let text = render(&report, UnitSystem::Metric, 200);
        assert!(!text.contains("updated:"), "{text}");
        assert!(!text.contains("current:"), "{text}");
        assert!(text.starts_with("location: "), "{text}");
        assert!(text.contains("\nday 2026-09-30: "), "{text}");
    }

    #[test]
    fn an_osm_location_carries_the_odbld_credit() {
        let mut report = report();
        report.location.source = LocationSource::Osm;
        let text = render(&report, UnitSystem::Metric, 200);
        assert!(text.contains("Location data © OpenStreetMap contributors (ODbL)"));
        assert!(!text.contains("GeoNames"));
    }

    #[test]
    fn a_coordinate_location_carries_no_location_credit() {
        let mut report = report();
        report.location.source = LocationSource::Coordinates;
        let text = render(&report, UnitSystem::Metric, 200);
        assert!(!text.contains("GeoNames"));
        assert!(!text.contains("OpenStreetMap"));
        assert!(text.contains("Data: Open-Meteo.com (CC BY 4.0)"));
        assert!(
            text.ends_with("attribution: open-meteo https://api.open-meteo.com/v1/forecast"),
            "the provenance line closes the document: {text}"
        );
    }

    #[test]
    fn the_width_never_truncates_a_record() {
        let wide = render(&report(), UnitSystem::Metric, 80);
        let narrow = render(&report(), UnitSystem::Metric, 20);
        assert_eq!(
            wide, narrow,
            "plain is a record format: the width must not change its content"
        );
        assert!(
            narrow.lines().any(|line| line.chars().count() > 20),
            "a day summary is longer than 20 columns"
        );
    }

    #[test]
    fn missing_optional_values_are_omitted_not_faked() {
        let mut report = report();
        let current = report.current.as_mut().expect("a current block");
        current.visibility_km = None;
        let day = report.days.first_mut().expect("a day");
        day.parts[0].precip_prob_pct = None;
        day.parts[0].wind_dir_deg = None;

        let text = render(&report, UnitSystem::Metric, 200);
        assert!(!text.contains("visibility"), "{text}");
        assert!(
            text.contains("Morning Partly cloudy 29°C 0.0mm wind 8.0km/h |"),
            "{text}"
        );
    }
}
