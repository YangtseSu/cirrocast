// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The canonical day-part aggregation, shared by every backend that serves hourly samples.
//!
//! A provider's job is to turn its native payload into [`HourSample`]s in the location's zone —
//! that is where the native slot length (1 h, 3 h, 6 h, 12 h) lives. This module owns everything
//! after that, so all backends aggregate the same way and the written rules below have one home:
//!
//! 1. `Morning` is 06:00–11:59, `Noon` 12:00–17:59, `Evening` 18:00–23:59 and `Night` 00:00–05:59
//!    **of the same local date**; the night row is that day's small hours, not the following night.
//! 2. Temperature, apparent temperature, humidity, wind, wind direction and visibility come from
//!    the single sample closest to the part's midpoint (09:00, 15:00, 21:00, 03:00; a tie takes the
//!    earlier hour).
//! 3. Precipitation is the part **sum** (a sample's value covers the interval that ends at its own
//!    instant, so summing accumulates correctly whatever the step length), precipitation
//!    probability the part **maximum**.
//! 4. The part's condition is the code with the highest [`Condition::severity_rank`] present, ties
//!    broken by the higher number of samples and then by the earlier instant; a part in which every
//!    code is undescribed falls back to the representative sample's code.
//! 5. A part with no sample at all is an upstream error (the response cannot be rendered into the
//!    canonical shape); a part with fewer samples than a plain day is aggregated normally, which is
//!    what a daylight-saving transition and a coarse trailing step produce.

use chrono::{DateTime, FixedOffset, NaiveDate, Timelike};
use chrono_tz::Tz;

use crate::error::{Error, Result};
use crate::model::{Condition, DayForecast, DayPart, DayPartKind};

/// One usable hourly sample, already localised and converted to canonical units.
///
/// An hour whose temperature, apparent temperature, precipitation, wind speed or weather code is
/// missing is **dropped** by the provider rather than filled with a zero: a missing reading must
/// not become a plausible-looking 0 °C. The optional fields keep their own nullness.
#[derive(Debug, Clone, PartialEq)]
pub struct HourSample {
    /// The instant, in the location's zone.
    pub at: DateTime<Tz>,
    /// Air temperature in °C.
    pub temp_c: f32,
    /// Apparent temperature in °C, when the backend reports one.
    pub feels_like_c: Option<f32>,
    /// Precipitation in mm.
    pub precip_mm: f32,
    /// Precipitation probability in percent.
    pub precip_prob_pct: Option<u8>,
    /// The condition at this instant.
    pub weather: Condition,
    /// Wind speed in km/h.
    pub wind_kmh: f32,
    /// Direction the wind blows from, in degrees.
    pub wind_dir_deg: Option<u16>,
    /// Relative humidity in percent.
    pub humidity_pct: Option<u8>,
    /// Horizontal visibility in km.
    pub visibility_km: Option<f32>,
}

/// The first `days` local dates whose four parts all have at least one sample.
///
/// A series that starts at the current hour leaves the location-local today incomplete (its earlier
/// parts are in the past); a day no `DayPart` can be built for is skipped rather than filled with
/// invented values, so a backend emits the first fully covered days.
pub fn covered_days(samples: &[HourSample], tz: Tz, days: u8) -> Vec<NaiveDate> {
    let mut dates: Vec<NaiveDate> = samples
        .iter()
        .map(|sample| sample.at.with_timezone(&tz).date_naive())
        .collect();
    dates.sort_unstable();
    dates.dedup();
    dates
        .into_iter()
        .filter(|date| covers_every_part(samples, *date, tz))
        .take(usize::from(days))
        .collect()
}

/// Whether every day part of `date` has a sample.
fn covers_every_part(samples: &[HourSample], date: NaiveDate, tz: Tz) -> bool {
    DayPartKind::ALL.iter().all(|kind| {
        samples.iter().any(|sample| {
            let local = sample.at.with_timezone(&tz);
            let hour = local.hour();
            local.date_naive() == date
                && hour >= u32::from(kind.hours().start)
                && hour < u32::from(kind.hours().end)
        })
    })
}

/// Aggregates one local calendar day of samples into the canonical four parts.
///
/// `provider` only names the backend in error messages. The daily extremes and the sun times are
/// the caller's: a backend that has no daily block derives the extremes from `hours` and passes
/// `None` for sunrise/sunset.
#[allow(clippy::too_many_arguments)] // the shape is fixed by the canonical model
pub fn aggregate_day(
    hours: &[HourSample],
    date: NaiveDate,
    tz: Tz,
    provider: &str,
    temp_min_c: f32,
    temp_max_c: f32,
    sunrise: Option<DateTime<FixedOffset>>,
    sunset: Option<DateTime<FixedOffset>>,
) -> Result<DayForecast> {
    Ok(DayForecast {
        date,
        parts: [
            aggregate_part(DayPartKind::Morning, hours, date, tz, provider)?,
            aggregate_part(DayPartKind::Noon, hours, date, tz, provider)?,
            aggregate_part(DayPartKind::Evening, hours, date, tz, provider)?,
            aggregate_part(DayPartKind::Night, hours, date, tz, provider)?,
        ],
        temp_min_c,
        temp_max_c,
        sunrise,
        sunset,
    })
}

/// The extremes of one local calendar day, from its samples.
///
/// Used by backends without a daily block; a day with no sample is an error, like a missing part.
pub fn extremes(
    hours: &[HourSample],
    date: NaiveDate,
    tz: Tz,
    provider: &str,
) -> Result<(f32, f32)> {
    let temps: Vec<f32> = hours
        .iter()
        .filter(|sample| sample.at.with_timezone(&tz).date_naive() == date)
        .map(|sample| sample.temp_c)
        .collect();
    let min = temps.iter().copied().fold(f32::INFINITY, f32::min);
    let max = temps.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    if temps.is_empty() {
        return Err(Error::Upstream {
            provider: provider.to_owned(),
            status: None,
            message: format!("no hourly data for {date} ({tz})"),
        });
    }
    Ok((min, max))
}

/// Aggregates one part of `date`, per the module's written rules.
fn aggregate_part(
    kind: DayPartKind,
    hours: &[HourSample],
    date: NaiveDate,
    tz: Tz,
    provider: &str,
) -> Result<DayPart> {
    let part: Vec<&HourSample> = hours
        .iter()
        .filter(|sample| {
            let local = sample.at.with_timezone(&tz);
            let hour = local.hour();
            local.date_naive() == date
                && hour >= u32::from(kind.hours().start)
                && hour < u32::from(kind.hours().end)
        })
        .collect();

    let Some(first) = part.first() else {
        return Err(Error::Upstream {
            provider: provider.to_owned(),
            status: None,
            message: format!(
                "no hourly data for {date} {} ({tz})",
                kind.label().to_lowercase()
            ),
        });
    };

    // Closest to the part's midpoint, ties taking the earlier hour; the instant breaks a tie
    // between two samples of the same local hour.
    let representative = part.iter().copied().fold(*first, |best, sample| {
        if proximity(sample, kind) < proximity(best, kind) {
            sample
        } else {
            best
        }
    });

    let precip_mm = part.iter().map(|sample| sample.precip_mm).sum();
    let precip_prob_pct = part
        .iter()
        .filter_map(|sample| sample.precip_prob_pct)
        .max();

    Ok(DayPart {
        kind,
        temp_c: representative.temp_c,
        feels_like_c: representative.feels_like_c,
        precip_mm,
        precip_prob_pct,
        weather: dominant_condition(&part).unwrap_or(representative.weather),
        wind_kmh: representative.wind_kmh,
        wind_dir_deg: representative.wind_dir_deg,
        humidity_pct: representative.humidity_pct,
        visibility_km: representative.visibility_km,
    })
}

/// The sort key that decides which sample represents a part.
fn proximity(sample: &HourSample, kind: DayPartKind) -> (i16, i16, DateTime<Tz>) {
    let hour = i16::try_from(sample.at.hour()).unwrap_or_default();
    let midpoint = i16::from(kind.midpoint_hour());
    ((hour - midpoint).abs(), hour, sample.at)
}

/// The part's condition: highest severity rank, then the higher sample count, then the earlier
/// instant, and `None` when every code in the part is undescribed.
fn dominant_condition(part: &[&HourSample]) -> Option<Condition> {
    let mut groups: Vec<(Condition, usize, DateTime<Tz>)> = Vec::new();
    for sample in part {
        match groups
            .iter_mut()
            .find(|(code, _, _)| *code == sample.weather)
        {
            Some((_, count, _)) => *count += 1,
            None => groups.push((sample.weather, 1, sample.at)),
        }
    }
    groups
        .into_iter()
        .filter(|(code, _, _)| code.is_known())
        .max_by(|a, b| {
            a.0.severity_rank()
                .cmp(&b.0.severity_rank())
                .then_with(|| a.1.cmp(&b.1))
                // The earlier instant wins, so it has to compare as the larger value.
                .then_with(|| b.2.cmp(&a.2))
        })
        .map(|(code, _, _)| code)
}

#[cfg(test)]
mod tests {
    // The aggregation compares values that came from literals, so exact equality is the point;
    // approximate comparison would hide a wrong sample being picked.
    #![allow(clippy::float_cmp)]

    use chrono::TimeZone as _;
    use chrono_tz::Tz;

    use super::{HourSample, aggregate_day, extremes};
    use crate::model::{Condition, DayPartKind};

    fn sample(at: chrono::DateTime<Tz>, temp_c: f32, code: u8) -> HourSample {
        HourSample {
            at,
            temp_c,
            feels_like_c: Some(temp_c - 1.0),
            precip_mm: 0.5,
            precip_prob_pct: Some(10),
            weather: Condition::from_u8(code),
            wind_kmh: 12.0,
            wind_dir_deg: Some(180),
            humidity_pct: Some(60),
            visibility_km: Some(20.0),
        }
    }

    /// A plain local day: one sample per hour, 00:00 through 23:00.
    fn day(date: (i32, u32, u32), tz: Tz) -> Vec<HourSample> {
        let date = chrono::NaiveDate::from_ymd_opt(date.0, date.1, date.2).expect("a valid date");
        (0..24)
            .map(|hour| {
                let at = tz
                    .from_local_datetime(&date.and_hms_opt(hour, 0, 0).expect("a valid time"))
                    .single()
                    .expect("no DST transition at the top of an hour here");
                sample(at, 10.0 + f32::from(u8::try_from(hour).expect("0..24")), 1)
            })
            .collect()
    }

    #[test]
    fn the_four_parts_cover_the_local_day() {
        let tz = Tz::Europe__Stockholm;
        let date = chrono::NaiveDate::from_ymd_opt(2026, 7, 15).expect("a valid date");
        let forecast = aggregate_day(
            &day((2026, 7, 15), tz),
            date,
            tz,
            "test",
            8.0,
            33.0,
            None,
            None,
        )
        .expect("a full day aggregates");
        assert_eq!(forecast.parts[DayPartKind::Morning.index()].temp_c, 19.0);
        assert_eq!(forecast.parts[DayPartKind::Noon.index()].temp_c, 25.0);
        assert_eq!(forecast.parts[DayPartKind::Evening.index()].temp_c, 31.0);
        assert_eq!(forecast.parts[DayPartKind::Night.index()].temp_c, 13.0);
        // Six samples per part, each 0.5 mm: the sum, not the representative's value.
        assert_eq!(forecast.parts[DayPartKind::Morning.index()].precip_mm, 3.0);
    }

    #[test]
    fn extremes_come_from_the_days_own_samples() {
        let tz = Tz::Europe__Stockholm;
        let date = chrono::NaiveDate::from_ymd_opt(2026, 7, 15).expect("a valid date");
        let (min, max) =
            extremes(&day((2026, 7, 15), tz), date, tz, "test").expect("samples exist");
        assert_eq!((min, max), (10.0, 33.0));
    }

    #[test]
    fn a_day_without_samples_is_an_error() {
        let tz = Tz::Europe__Stockholm;
        let date = chrono::NaiveDate::from_ymd_opt(2026, 7, 15).expect("a valid date");
        let error = aggregate_day(&[], date, tz, "test", 0.0, 0.0, None, None)
            .expect_err("an empty day cannot aggregate");
        assert_eq!(error.exit_code(), 3);
        assert!(error.to_string().contains("no hourly data"), "{error}");
    }
}
