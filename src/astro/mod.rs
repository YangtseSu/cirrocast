// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Moon phase, sun times and phase instants — computed on this machine, with no network at all.
//!
//! The module is pure arithmetic on the location and the clock ([`julian`] holds the time scales,
//! [`moon`] the lunar series and rise/set, [`sun`] the solar ones), which is why it is the one
//! computation in this crate that can be handed to a renderer through the data model without any
//! `Env`: `crate::render` may call [`Astro::compute`] and `crate::model::astro` holds the result,
//! exactly as the air reading is held — except that no request, no cache entry and no key is
//! involved.
//!
//! # What is computed, and for when
//!
//! * the **moon** block is always local: phase, illuminated fraction, age, moonrise/moonset for
//!   the location-local calendar day and the next four phase instants;
//! * the **sun** block prefers the provider's own sunrise/sunset for the day the run is in (the
//!   data the backend was asked for in the first place) and computes them locally when the backend
//!   sends none — `source` records which happened. Polar day and night are reported as such
//!   instead of a clamped `00:00`.
//!
//! All instants are returned at the location's own offset; the `Utc` conversion happens inside
//! [`julian`], where the ΔT offset is applied, so no caller ever mixes TT with UT.
//!
//! The supported range is **1900–2100**: the truncated series of chapters 25/47 degrade slowly
//! outside it and the ΔT fits stop being defined, so a caller comparing years beyond that should
//! expect minutes, not seconds, of error.

pub mod julian;
pub mod moon;
pub mod sun;

use chrono::{DateTime, FixedOffset, NaiveDate, NaiveTime, TimeZone as _, Utc};
use chrono_tz::Tz;

use crate::model::Report;
pub use crate::model::astro::Astro;
use crate::model::astro::{Moon, Sun, SunSource};

pub use julian::{delta_t_seconds, from_julian_day, julian_day};

/// The instant of a Julian Day in a location's zone; `None` only for a JD `chrono` cannot hold.
pub(crate) fn local_instant(tz: Tz, jd_ut: f64) -> Option<DateTime<FixedOffset>> {
    julian::from_julian_day(jd_ut).map(|utc| utc.with_timezone(&tz).fixed_offset())
}

/// Greenwich mean sidereal time, in degrees, Meeus 12.4.
///
/// The mean value: the apparent sidereal time adds Δψ·cos ε ≈ 1″, which is two orders of
/// magnitude below the resolution the rise/set search has for the minute it prints.
pub(crate) fn gmst_deg(jd_ut: f64) -> f64 {
    let t = (jd_ut - 2_451_545.0) / 36_525.0;
    (280.460_618_37 + 360.985_647_366_29 * (jd_ut - 2_451_545.0) + 0.000_387_933 * t * t
        - t * t * t / 38_710_000.0)
        .rem_euclid(360.0)
}

/// The altitude of a body with the given apparent right ascension and declination, in degrees.
pub(crate) fn altitude_from(
    ra_deg: f64,
    dec_deg: f64,
    jd_ut: f64,
    lat_deg: f64,
    lon_deg: f64,
) -> f64 {
    let hour_angle = (gmst_deg(jd_ut) + lon_deg - ra_deg).to_radians();
    let (sin_lat, cos_lat) = lat_deg.to_radians().sin_cos();
    let (sin_dec, cos_dec) = dec_deg.to_radians().sin_cos();
    (sin_lat * sin_dec + cos_lat * cos_dec * hour_angle.cos())
        .asin()
        .to_degrees()
}

/// The location-local calendar day as a window of UT Julian Days.
#[derive(Debug, Clone, Copy)]
pub(crate) struct LocalDay {
    /// The JD of the local midnight that starts the day.
    pub start: f64,
    /// The JD of the local midnight that ends it (23 h, 24 h or 25 h later).
    pub end: f64,
}

impl LocalDay {
    /// The middle of the window: local noon, or as close to it as a transition day allows.
    pub(crate) fn midpoint(self) -> f64 {
        f64::midpoint(self.start, self.end)
    }
}

/// The local day that contains `date`, resolved through the zone's own offsets.
///
/// The bounds go through [`crate::model::resolve_local`], so a fall-back day is 25 hours long and
/// a spring-forward one 23; a wall-clock midnight no offset makes real (a zone that skips it) is
/// read as UTC, which keeps the function total without inventing an hour.
pub(crate) fn local_day(date: NaiveDate, tz: Tz) -> LocalDay {
    let start = midnight(date, tz);
    let next = date.succ_opt().unwrap_or(date);
    let mut end = midnight(next, tz);
    if end <= start {
        end = start + chrono::Duration::days(1);
    }
    LocalDay {
        start: julian::julian_day(start.with_timezone(&Utc)),
        end: julian::julian_day(end.with_timezone(&Utc)),
    }
}

/// Local midnight of `date` in `tz`, total.
fn midnight(date: NaiveDate, tz: Tz) -> DateTime<FixedOffset> {
    let naive = date.and_time(NaiveTime::MIN);
    crate::model::resolve_local(tz, naive).map_or_else(
        |_| tz.from_utc_datetime(&naive).fixed_offset(),
        |local| local.fixed_offset(),
    )
}

/// One crossing of an altitude threshold.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Crossing {
    /// The UT Julian Day of the crossing.
    pub at: f64,
    /// Whether the body was rising (altitude increasing) at the crossing.
    pub ascending: bool,
}

/// Every crossing of `h0_deg` by `altitude` whose *location-local* date is `date`.
///
/// The search runs over the local day widened by three hours on both sides and samples at five
/// minute steps — the Moon's altitude changes by at most ~1.3° in that time, so no event that has
/// a beginning and an end is stepped over — then bisects each sign change to below a second. A
/// candidate is kept only when its instant falls on `date` in `tz`, which is what makes the
/// answer independent of the 23 h and 25 h transition days: the search window is a superset, the
/// calendar day is the filter.
pub(crate) fn crossings_in_local_day(
    date: NaiveDate,
    tz: Tz,
    h0_deg: f64,
    altitude: impl Fn(f64) -> f64,
) -> Vec<Crossing> {
    /// Sampling stride: five minutes.
    const STEP_DAYS: f64 = 5.0 / 1440.0;
    /// How far past the window's edges the search looks, so both transition days are covered.
    const MARGIN_DAYS: f64 = 3.0 / 24.0;

    let day = local_day(date, tz);
    let start = day.start - MARGIN_DAYS;
    let end = day.end + MARGIN_DAYS;

    let mut crossings = Vec::new();
    let mut left = start;
    let mut before = altitude(left) - h0_deg;
    while left < end {
        let right = (left + STEP_DAYS).min(end);
        let after = altitude(right) - h0_deg;
        if before.signum() != after.signum() {
            let (mut low, mut high) = (left, right);
            let mut low_value = before;
            for _ in 0..60 {
                if (high - low) * 86_400.0 < 0.5 {
                    break;
                }
                let middle = f64::midpoint(low, high);
                let value = altitude(middle) - h0_deg;
                if low_value.signum() == value.signum() {
                    low = middle;
                    low_value = value;
                } else {
                    high = middle;
                }
            }
            let at = f64::midpoint(low, high);
            if julian::from_julian_day(at)
                .is_some_and(|instant| instant.with_timezone(&tz).date_naive() == date)
            {
                crossings.push(Crossing {
                    at,
                    ascending: after > before,
                });
            }
        }
        left = right;
        before = after;
    }
    crossings
}

/// The span between two instants in whole seconds, clamped to one day.
///
/// A daylight span cannot exceed 24 hours, so the clamp keeps a provider's clock skew (or a
/// hand-written report) from wrapping the number; the cast then cannot truncate or lose a sign.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub(crate) fn span_secs(from: DateTime<FixedOffset>, to: DateTime<FixedOffset>) -> u32 {
    (to - from).num_seconds().clamp(0, 86_400) as u32
}

/// The moon phase at an instant: the cheap query behind the `%m`/`%M` tokens, which need neither
/// the sun block nor the rise/set search.
#[must_use]
pub fn phase_at(now: DateTime<FixedOffset>) -> crate::model::MoonPhase {
    moon::phase_at(julian::jde(julian::julian_day(now.with_timezone(&Utc))))
}

impl Astro {
    /// Computes the astronomy block for `report` at the instant `now`.
    ///
    /// `now` is the run's clock at the location's own offset — injected, like
    /// [`RenderContext::now`](crate::render::RenderContext), so a test or a snapshot is
    /// deterministic. The provider's sun times for the local day are used when the report carries
    /// them; otherwise the solar values are computed here and `sun.source` says so.
    #[must_use]
    pub fn compute(report: &Report, now: DateTime<FixedOffset>) -> Astro {
        let location = &report.location;
        let date = now.date_naive();
        let jd = julian::julian_day(now.with_timezone(&Utc));
        let jde = julian::jde(jd);

        let (moonrise, moonset) =
            moon::moonrise_moonset(date, location.lat, location.lon, location.tz);
        let next = moon::next_phases(jd, 4)
            .into_iter()
            .map(|(phase, instant)| (phase, instant.with_timezone(&location.tz).fixed_offset()))
            .collect();
        let moon = Moon {
            phase: moon::phase_at(jde),
            illuminated_fraction: moon::illuminated_fraction(jde),
            age_days: moon::age_days(jd),
            moonrise,
            moonset,
            next,
        };

        let today = report
            .days
            .iter()
            .find(|day| day.date == date)
            .or_else(|| report.days.first());
        let provider_sun = today
            .filter(|day| day.sunrise.is_some() || day.sunset.is_some())
            .map(|day| (day.sunrise, day.sunset));
        let sun = match provider_sun {
            Some((sunrise, sunset)) => Sun {
                sunrise,
                sunset,
                daylight_secs: match (sunrise, sunset) {
                    (Some(rise), Some(set)) => Some(span_secs(rise, set)),
                    _ => None,
                },
                polar: None,
                source: SunSource::Provider,
            },
            None => sun::local(date, location.lat, location.lon, location.tz),
        };

        Astro {
            moon,
            sun,
            computed_at: now.with_timezone(&Utc),
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::{NaiveDate, Timelike as _};
    use chrono_tz::Tz;

    use super::{crossings_in_local_day, local_day};

    #[test]
    fn a_spring_forward_local_day_is_twenty_three_hours_and_a_fall_back_one_twenty_five() {
        let zone = Tz::America__New_York;
        let spring = NaiveDate::from_ymd_opt(2026, 3, 8).expect("a date");
        let day = local_day(spring, zone);
        assert!(
            ((day.end - day.start) * 24.0 - 23.0).abs() < 1e-6,
            "{}",
            (day.end - day.start) * 24.0
        );

        let fall = NaiveDate::from_ymd_opt(2026, 11, 1).expect("a date");
        let day = local_day(fall, zone);
        assert!(
            ((day.end - day.start) * 24.0 - 25.0).abs() < 1e-6,
            "{}",
            (day.end - day.start) * 24.0
        );

        let ordinary = NaiveDate::from_ymd_opt(2026, 6, 1).expect("a date");
        let day = local_day(ordinary, zone);
        assert!(((day.end - day.start) * 24.0 - 24.0).abs() < 1e-6);
    }

    #[test]
    fn a_local_midnight_is_the_start_of_the_window() {
        let zone = Tz::Asia__Shanghai;
        let date = NaiveDate::from_ymd_opt(2026, 10, 4).expect("a date");
        let day = local_day(date, zone);
        let local = super::local_instant(zone, day.start).expect("a representable instant");
        assert_eq!(local.date_naive(), date);
        assert_eq!(local.time(), chrono::NaiveTime::MIN);
        let noon = super::local_instant(zone, day.midpoint()).expect("a representable instant");
        assert_eq!(noon.hour(), 12);
    }

    #[test]
    fn crossings_are_classified_and_filtered_by_the_local_date() {
        // A monotonically rising synthetic altitude crosses once, upwards.
        let zone = Tz::Asia__Shanghai;
        let date = NaiveDate::from_ymd_opt(2026, 10, 4).expect("a date");
        let day = local_day(date, zone);
        let crossing = crossings_in_local_day(date, zone, 0.0, |jd| (jd - day.start) * 24.0);
        assert_eq!(crossing.len(), 1);
        assert!(crossing[0].ascending);
        assert!(
            (crossing[0].at - day.start).abs() * 86_400.0 < 1.0,
            "{:?} is not the start of the day",
            crossing[0]
        );

        // A synthetic crossing three hours before local midnight belongs to the previous date
        // and must not be reported for this one.
        let before = day.start - 3.0 / 24.0;
        let crossing = crossings_in_local_day(date, zone, 0.0, move |jd| jd - before);
        assert!(
            crossing.is_empty(),
            "{crossing:?} crossed on the wrong local day"
        );
    }
}
