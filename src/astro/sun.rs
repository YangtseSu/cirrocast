// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The Sun: apparent position and the local day's sunrise, sunset and daylight.
//!
//! The position is Meeus chapter 25 (mean longitude and anomaly, the equation of the centre, then
//! the apparent longitude with the leading nutation and aberration terms), evaluated in TT. It is
//! the same series the Moon's elongation is measured against, so the two halves of the module
//! cannot drift apart.
//!
//! Sunrise and sunset are the instants where the Sun's *centre* reaches the standard altitude
//! h₀ = −0.8333° — refraction and the solar semi-diameter in one constant, as Meeus chapter 15
//! prescribes — found by the shared local-day crossing search over the apparent altitude. Inside
//! the polar circles a day may have no crossing at all: the values are then `None` and
//! [`Polar::Day`]/[`Polar::Night`] says which side of the horizon the Sun stayed on, never a
//! clamped `00:00`. Daylight length is the difference of the two instants, and the sun block's
//! `source` distinguishes this local computation from a provider's own values.

use chrono::{DateTime, FixedOffset, NaiveDate};
use chrono_tz::Tz;

use crate::model::astro::{Polar, Sun, SunSource};

/// The standard altitude of a sunrise or sunset: refraction minus the solar semi-diameter.
const SUN_H0_DEG: f64 = -0.8333;

/// Seconds in a day, for the polar daylight values.
const DAY_SECS: u32 = 86_400;

/// The Sun's apparent geocentric ecliptic longitude, in degrees, Meeus 25.2–25.10.
#[must_use]
pub fn apparent_longitude(jd_tt: f64) -> f64 {
    let t = (jd_tt - 2_451_545.0) / 36_525.0;
    let mean_longitude = 280.466_46 + 36_000.769_83 * t + 0.000_303_2 * t * t;
    let mean_anomaly = 357.529_11 + 35_999.050_29 * t - 0.000_153_7 * t * t;
    let m = mean_anomaly.to_radians();
    let centre = (1.914_602 - 0.004_817 * t - 0.000_014 * t * t) * m.sin()
        + (0.019_993 - 0.000_101 * t) * (2.0 * m).sin()
        + 0.000_289 * (3.0 * m).sin();
    let omega = (125.04 - 1934.136 * t).to_radians();
    (mean_longitude + centre - 0.005_69 - 0.004_78 * omega.sin()).rem_euclid(360.0)
}

/// The true obliquity of the ecliptic, in degrees, Meeus 22.2.
///
/// The mean value only: the nutation increment (≤ 9.2″) is applied by the Moon's own equatorial
/// conversion, where it matters more, and is below the rounding of everything the Sun's arithmetic
/// feeds here.
pub(crate) fn obliquity_deg(jd_tt: f64) -> f64 {
    let t = (jd_tt - 2_451_545.0) / 36_525.0;
    23.0 + 26.0 / 60.0 + 21.448 / 3600.0
        - (46.815_0 * t + 0.000_59 * t * t - 0.001_813 * t * t * t) / 3600.0
}

/// The Sun's apparent right ascension and declination, in degrees, from the apparent longitude.
#[must_use]
pub(crate) fn equatorial(jd_tt: f64) -> (f64, f64) {
    let longitude = apparent_longitude(jd_tt).to_radians();
    let obliquity = obliquity_deg(jd_tt).to_radians();
    let (sin_lon, cos_lon) = longitude.sin_cos();
    let (sin_eps, cos_eps) = obliquity.sin_cos();
    let ra = (cos_eps * sin_lon).atan2(cos_lon).to_degrees();
    let dec = (sin_eps * sin_lon).asin().to_degrees();
    (ra.rem_euclid(360.0), dec)
}

/// The Sun's altitude above the horizon at a UT Julian Day, in degrees.
pub(crate) fn altitude(jd_ut: f64, lat_deg: f64, lon_deg: f64) -> f64 {
    let (ra, dec) = equatorial(super::julian::jde(jd_ut));
    super::altitude_from(ra, dec, jd_ut, lat_deg, lon_deg)
}

/// The local day's sunrise, sunset and polar state.
///
/// Both instants are the events whose *location-local* calendar date is `date`; a day inside the
/// polar circles has no crossing, and the third value then says whether the Sun was above
/// ([`Polar::Day`]) or below ([`Polar::Night`]) the horizon for the whole day.
#[must_use]
pub fn sunrise_sunset(
    date: NaiveDate,
    lat: f64,
    lon: f64,
    tz: Tz,
) -> (
    Option<DateTime<FixedOffset>>,
    Option<DateTime<FixedOffset>>,
    Option<Polar>,
) {
    let crossings =
        super::crossings_in_local_day(date, tz, SUN_H0_DEG, |jd_ut| altitude(jd_ut, lat, lon));
    let event = |ascending: bool| {
        crossings
            .iter()
            .find(|crossing| crossing.ascending == ascending)
            .and_then(|crossing| super::local_instant(tz, crossing.at))
    };
    let polar = crossings.is_empty().then(|| {
        let noon = super::local_day(date, tz).midpoint();
        if altitude(noon, lat, lon) > SUN_H0_DEG {
            Polar::Day
        } else {
            Polar::Night
        }
    });
    (event(true), event(false), polar)
}

/// The sun block computed on this machine, as the fallback for a provider that sends no sun times.
#[must_use]
pub(crate) fn local(date: NaiveDate, lat: f64, lon: f64, tz: Tz) -> Sun {
    let (sunrise, sunset, polar) = sunrise_sunset(date, lat, lon, tz);
    let daylight_secs = match (sunrise, sunset, polar) {
        (Some(rise), Some(set), _) => Some(super::span_secs(rise, set)),
        (_, _, Some(Polar::Day)) => Some(DAY_SECS),
        (_, _, Some(Polar::Night)) => Some(0),
        _ => None,
    };
    Sun {
        sunrise,
        sunset,
        daylight_secs,
        polar,
        source: SunSource::Local,
    }
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;
    use chrono_tz::Tz;

    use super::{apparent_longitude, sunrise_sunset};

    #[test]
    fn meeus_example_25_a_is_reproduced() {
        // 1992 October 13.0 TD: the apparent longitude is 199.90895°.
        let longitude = apparent_longitude(2_448_908.5);
        assert!((longitude - 199.908_95).abs() < 0.000_02, "λ = {longitude}");
    }

    #[test]
    fn a_polar_day_and_a_polar_night_have_no_events_and_a_flag() {
        let longyearbyen = (78.2232, 15.6469, Tz::Arctic__Longyearbyen);
        let summer = NaiveDate::from_ymd_opt(2026, 6, 21).expect("a date");
        let (rise, set, polar) =
            sunrise_sunset(summer, longyearbyen.0, longyearbyen.1, longyearbyen.2);
        assert_eq!((rise, set), (None, None));
        assert_eq!(polar, Some(crate::model::astro::Polar::Day));

        let winter = NaiveDate::from_ymd_opt(2025, 12, 21).expect("a date");
        let (rise, set, polar) =
            sunrise_sunset(winter, longyearbyen.0, longyearbyen.1, longyearbyen.2);
        assert_eq!((rise, set), (None, None));
        assert_eq!(polar, Some(crate::model::astro::Polar::Night));
    }
}
