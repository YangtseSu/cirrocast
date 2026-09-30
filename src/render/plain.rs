// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The `plain` format: box-free, colour-free lines that survive a pipe.
//!
//! One line per thing and no wrapping: the location header, the current conditions, then a date
//! line followed by the four day-part lines per day, then the credits the data licences require.
//! Every value goes through [`crate::model::units`], so this renderer is the only place the
//! canonical metric values become degrees, miles or inches.
//!
//! Lines are **clipped** to the context width, never wrapped: a renderer that reflows a table
//! would break the one-line-per-part invariant a script relies on, and a clipped tail is visible
//! where a wrapped line is merely confusing.

use std::fmt::Write as _;

use chrono::{DateTime, FixedOffset};

use super::{RenderContext, Renderer};
use crate::error::Result;
use crate::geo::{attribution_line, location_line};
use crate::model::units::{
    ResolvedUnits, compass_16, format_precip, format_pressure, format_temp, format_visibility,
    format_wind,
};
use crate::model::{Current, DayForecast, DayPart, Report};
use crate::provider::licence_line;

/// The plain renderer.
#[derive(Debug, Clone, Copy, Default)]
pub struct Plain;

impl Renderer for Plain {
    fn render(&self, report: &Report, ctx: &RenderContext) -> Result<String> {
        let mut lines = vec![location_line(&report.location)];

        if let Some(current) = &report.current {
            lines.push(now_line(current, ctx.units));
        }
        for day in &report.days {
            lines.push(day_line(day, ctx.units));
            for part in &day.parts {
                lines.push(part_line(part, ctx.units));
            }
        }

        if let Some(credit) = attribution_line(&report.location) {
            lines.push(credit.to_owned());
        }
        if let Some(licence) = licence_line(&report.attribution.provider) {
            lines.push(format!("Data: {licence}"));
        }

        Ok(lines
            .iter()
            .map(|line| clip(line, ctx.width))
            .collect::<Vec<_>>()
            .join("\n"))
    }
}

/// The current conditions: one line, fields omitted when the provider has no value for them.
fn now_line(current: &Current, units: ResolvedUnits) -> String {
    let mut text = format!(
        "Now: {} (feels {}), {}",
        format_temp(current.temp_c, units.temp),
        format_temp(current.feels_like_c, units.temp),
        current.weather.description_en()
    );
    let _ = write!(
        text,
        ", wind {} {}",
        format_wind(current.wind_kmh, units.wind),
        compass_16(current.wind_dir_deg)
    );
    let _ = write!(text, ", humidity {}%", current.humidity_pct);
    let _ = write!(
        text,
        ", pressure {}",
        format_pressure(current.pressure_hpa, units.pressure)
    );
    if let Some(visibility) = current.visibility_km {
        let _ = write!(
            text,
            ", visibility {}",
            format_visibility(visibility, units.distance)
        );
    }
    let _ = write!(text, ", {}", format_precip(current.precip_mm, units.precip));
    text
}

/// One forecast day: date, extremes and — when upstream reports them — the sun times.
fn day_line(day: &DayForecast, units: ResolvedUnits) -> String {
    let mut text = format!(
        "{}  min {}  max {}",
        day.date,
        format_temp(day.temp_min_c, units.temp),
        format_temp(day.temp_max_c, units.temp)
    );
    if let Some(sunrise) = day.sunrise {
        let _ = write!(text, "  sunrise {}", clock_time(sunrise));
    }
    if let Some(sunset) = day.sunset {
        let _ = write!(text, "  sunset {}", clock_time(sunset));
    }
    text
}

/// One day part: label, representative temperature, condition, precipitation and wind.
fn part_line(part: &DayPart, units: ResolvedUnits) -> String {
    let mut text = format!(
        "  {:<8}{:>5}  {:<16}precip {}",
        part.kind.label(),
        format_temp(part.temp_c, units.temp),
        part.weather.description_en(),
        format_precip(part.precip_mm, units.precip)
    );
    if let Some(probability) = part.precip_prob_pct {
        let _ = write!(text, " ({probability}%)");
    }
    let _ = write!(text, "   wind {}", format_wind(part.wind_kmh, units.wind));
    if let Some(direction) = part.wind_dir_deg {
        let _ = write!(text, " {}", compass_16(direction));
    }
    text
}

/// `HH:MM` at the instant's own offset.
fn clock_time(at: DateTime<FixedOffset>) -> String {
    at.format("%H:%M").to_string()
}

/// Clips a line to `width` characters (not bytes: a wide line with CJK text must not be cut in the
/// middle of a character).
fn clip(line: &str, width: usize) -> String {
    if line.chars().count() <= width {
        return line.to_owned();
    }
    line.chars().take(width).collect()
}

#[cfg(test)]
mod tests {
    // The expected lines are built from literals, so exact float comparison is the assertion.
    #![allow(clippy::float_cmp)]

    use chrono::{FixedOffset, TimeZone as _, Utc};
    use chrono_tz::Tz;

    use super::{Plain, clip};
    use crate::model::units::UnitSystem;
    use crate::model::{
        Attribution, Condition, Current, DayForecast, DayPart, DayPartKind, Location,
        LocationSource, Report,
    };
    use crate::render::{ColorMode, RenderContext, Renderer, TermCaps};

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
        }
    }

    fn current() -> Current {
        Current {
            observed_at: moment(12, 15),
            temp_c: 31.0,
            feels_like_c: 36.0,
            humidity_pct: 66,
            precip_mm: 0.0,
            weather: Condition::from_u8(2),
            cloud_cover_pct: 40,
            pressure_hpa: 1004.0,
            wind_kmh: 12.0,
            wind_dir_deg: 135,
            wind_gust_kmh: None,
            visibility_km: Some(10.0),
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

    fn report() -> Report {
        Report {
            location: location(),
            current: Some(current()),
            days: vec![day()],
            attribution: Attribution {
                provider: "open-meteo".to_owned(),
                url: "https://api.open-meteo.com/v1/forecast".to_owned(),
                fetched_at: Utc
                    .with_ymd_and_hms(2026, 9, 30, 4, 15, 0)
                    .single()
                    .expect("an instant"),
                raw: None,
            },
        }
    }

    fn context(units: UnitSystem, width: usize) -> RenderContext {
        RenderContext {
            units: units
                .resolve(&crate::config::UnitOverrides::default())
                .expect("the default overrides resolve"),
            color: ColorMode::Never,
            width,
            term: TermCaps {
                is_tty: false,
                color: false,
                dumb: false,
            },
            now: moment(12, 30),
            tz: Tz::Asia__Shanghai,
        }
    }

    fn render(report: &Report, units: UnitSystem, width: usize) -> String {
        Plain
            .render(report, &context(units, width))
            .expect("plain always renders")
    }

    #[test]
    fn the_metric_lines_are_exactly_these() {
        let text = render(&report(), UnitSystem::Metric, 200);
        assert_eq!(
            text,
            "\
Beijing, Beijing, China (39.90, 116.41) Asia/Shanghai
Now: 31°C (feels 36°C), Partly cloudy, wind 12 km/h SE, humidity 66%, pressure 1004 hPa, visibility 10 km, 0.0 mm
2026-09-30  min 26°C  max 34°C  sunrise 06:08  sunset 17:58
  Morning  29°C  Partly cloudy   precip 0.0 mm (10%)   wind 8.0 km/h S
  Noon     33°C  Overcast        precip 0.2 mm (25%)   wind 15 km/h SE
  Evening  30°C  Slight rain     precip 1.1 mm (60%)   wind 12 km/h E
  Night    27°C  Clear sky       precip 0.0 mm (5%)   wind 6.0 km/h N
Location data based on GeoNames (CC-BY-4.0) via Open-Meteo — https://open-meteo.com/
Data: Open-Meteo.com (CC BY 4.0)"
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
        assert!(us.contains("Now: 88°F (feels 97°F)"), "{us}");
        assert!(us.contains("wind 7.5 mph SE"), "{us}");
        assert!(us.contains("pressure 29.65 inHg"), "{us}");
        assert!(us.contains("visibility 6.2 mi"), "{us}");
        assert!(us.contains("min 79°F  max 93°F"), "{us}");

        let uk = render(&report, UnitSystem::Uk, 200);
        assert!(uk.contains("Now: 31°C (feels 36°C)"), "{uk}");
        assert!(uk.contains("wind 7.5 mph SE"), "{uk}");
        assert!(uk.contains("pressure 1004 hPa"), "{uk}");
        assert!(uk.contains("visibility 6.2 mi"), "{uk}");
    }

    #[test]
    fn a_report_without_days_prints_the_header_and_the_now_line() {
        let mut report = report();
        report.days.clear();
        let text = render(&report, UnitSystem::Metric, 200);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 4, "{text}");
        assert!(lines[1].starts_with("Now: "));
        assert!(
            !text.contains("Morning"),
            "a current-only report has no day parts: {text}"
        );
    }

    #[test]
    fn a_current_only_report_has_no_now_line() {
        let mut report = report();
        report.current = None;
        report.days.clear();
        let text = render(&report, UnitSystem::Metric, 200);
        assert_eq!(text.lines().count(), 3, "{text}");
        assert!(!text.contains("Now: "));
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
    }

    #[test]
    fn every_line_fits_the_context_width() {
        let text = render(&report(), UnitSystem::Metric, 24);
        for line in text.lines() {
            assert!(line.chars().count() <= 24, "too wide: {line:?}");
        }
        assert_eq!(
            text.lines().count(),
            9,
            "clipping never drops or adds a line"
        );
    }

    #[test]
    fn missing_optional_values_are_omitted_not_faked() {
        let mut report = report();
        let current = report.current.as_mut().expect("a current block");
        current.visibility_km = None;
        let day = report.days.first_mut().expect("a day");
        day.sunrise = None;
        day.sunset = None;
        day.parts[0].precip_prob_pct = None;
        day.parts[0].wind_dir_deg = None;

        let text = render(&report, UnitSystem::Metric, 200);
        assert!(!text.contains("visibility"), "{text}");
        assert!(!text.contains("sunrise"), "{text}");
        assert!(
            text.contains("  Morning  29°C  Partly cloudy   precip 0.0 mm   wind 8.0 km/h\n"),
            "{text}"
        );
    }

    #[test]
    fn clipping_counts_characters_not_bytes() {
        assert_eq!(clip("温度 高", 4), "温度 高");
        assert_eq!(clip("温度 高", 3), "温度 ");
    }
}
