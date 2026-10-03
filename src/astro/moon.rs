// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The Moon: geocentric position, phase, illuminated fraction and rise/set — all local arithmetic.
//!
//! The position is the truncated ELP-2000/82 series of Meeus, *Astronomical Algorithms*, 2nd ed.,
//! chapter 47: the 60 longitude/distance terms of table 47.A and the 60 latitude terms of 47.B,
//! evaluated in TT and returned as the geometric ecliptic coordinates of the date. Nutation from
//! the abridged chapter-22 series is added where an apparent coordinate is wanted. Nothing is
//! copied from another weather client; the coefficient tables are the published lunar theory as
//! Meeus tabulates it.
//!
//! Measured against JPL Horizons DE441 (geocentric, quantity 10), the illuminated fraction this
//! module computes deviates by at most 0.14 pp over the 2026 fixtures in `tests/astro.rs`, and the
//! 1977 and 2044 phase instants of Meeus' examples 49.a/49.b are reproduced to 32 s and 23 s —
//! tighter than the ±0.5 pp and ±2 min the tests allow. The series is documented for 1900–2100.
//!
//! The phase definition is Meeus': the instant of New Moon is when the apparent geocentric
//! longitudes of the Moon and the Sun are equal, and [`next_phases`] finds exactly those instants
//! by root-finding on [`synodic_elongation`], so the printed phase name, the illuminated fraction
//! and the phase instants can never disagree with one another.
//!
//! Rise and set use the standard lunar altitude h₀ = +0.125° (parallax, refraction and the
//! semi-diameter folded into one constant, as Meeus chapter 15 prescribes) and the shared
//! local-day crossing search of [`super::crossings_in_local_day`]; the Moon's fast declination
//! makes these the least accurate values in the module (a few minutes), and a day on which an
//! event does not happen simply has no value for it.

use chrono::{DateTime, FixedOffset, NaiveDate, Utc};
use chrono_tz::Tz;

use super::julian::{from_julian_day, jde};
use crate::model::astro::MoonPhase;

/// The mean synodic month in days (Meeus 49.1's leading term).
pub const SYNODIC_MONTH_DAYS: f64 = 29.530_588_853;

/// The mean rate of the synodic elongation, in degrees per day: 360°/29.53 d.
const ELONGATION_RATE_DEG_PER_DAY: f64 = 360.0 / SYNODIC_MONTH_DAYS;

/// The standard lunar altitude of an event: parallax minus refraction and the semi-diameter.
const MOON_H0_DEG: f64 = 0.125;

/// The Moon's geocentric ecliptic coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Position {
    /// Geocentric ecliptic longitude of the date, in degrees, without nutation.
    pub longitude_deg: f64,
    /// Geocentric ecliptic latitude, in degrees.
    pub latitude_deg: f64,
    /// Distance from the centre of the Earth, in kilometres.
    pub distance_km: f64,
}

/// Table 47.A — `D, M, M', F, Σl (1e-6 deg), Σr (1e-3 km)`, the 60 terms in Meeus' order.
const LONGITUDE_TERMS: [[i32; 6]; 60] = [
    [0, 0, 1, 0, 6_288_774, -20_905_355],
    [2, 0, -1, 0, 1_274_027, -3_699_111],
    [2, 0, 0, 0, 658_314, -2_955_968],
    [0, 0, 2, 0, 213_618, -569_925],
    [0, 1, 0, 0, -185_116, 48_888],
    [0, 0, 0, 2, -114_332, -3149],
    [2, 0, -2, 0, 58_793, 246_158],
    [2, -1, -1, 0, 57_066, -152_138],
    [2, 0, 1, 0, 53_322, -170_733],
    [2, -1, 0, 0, 45_758, -204_586],
    [0, 1, -1, 0, -40_923, -129_620],
    [1, 0, 0, 0, -34_720, 108_743],
    [0, 1, 1, 0, -30_383, 104_755],
    [2, 0, 0, -2, 15_327, 10_321],
    [0, 0, 1, 2, -12_528, 0],
    [0, 0, 1, -2, 10_980, 79_661],
    [4, 0, -1, 0, 10_675, -34_782],
    [0, 0, 3, 0, 10_034, -23_210],
    [4, 0, -2, 0, 8548, -21_636],
    [2, 1, -1, 0, -7888, 24_208],
    [2, 1, 0, 0, -6766, 30_824],
    [1, 0, -1, 0, -5163, -8379],
    [1, 1, 0, 0, 4987, -16_675],
    [2, -1, 1, 0, 4036, -12_831],
    [2, 0, 2, 0, 3994, -10_445],
    [4, 0, 0, 0, 3861, -11_650],
    [2, 0, -3, 0, 3665, 14_403],
    [0, 1, -2, 0, -2689, -7003],
    [2, 0, -1, 2, -2602, 0],
    [2, -1, -2, 0, 2390, 10_056],
    [1, 0, 1, 0, -2348, 6322],
    [2, -2, 0, 0, 2236, -9884],
    [0, 1, 2, 0, -2120, 5751],
    [0, 2, 0, 0, -2069, 0],
    [2, -2, -1, 0, 2048, -4950],
    [2, 0, 1, -2, -1773, 4130],
    [2, 0, 0, 2, -1595, 0],
    [4, -1, -1, 0, 1215, -3958],
    [0, 0, 2, 2, -1110, 0],
    [3, 0, -1, 0, -892, 3258],
    [2, 1, 1, 0, -810, 2616],
    [4, -1, -2, 0, 759, -1897],
    [0, 2, -1, 0, -713, -2117],
    [2, 2, -1, 0, -700, 2354],
    [2, 1, -2, 0, 691, 0],
    [2, -1, 0, -2, 596, 0],
    [4, 0, 1, 0, 549, -1423],
    [0, 0, 4, 0, 537, -1117],
    [4, -1, 0, 0, 520, -1571],
    [1, 0, -2, 0, -487, -1739],
    [2, 1, 0, -2, -399, 0],
    [0, 0, 2, -2, -381, -4421],
    [1, 1, 1, 0, 351, 0],
    [3, 0, -2, 0, -340, 0],
    [4, 0, -3, 0, 330, 0],
    [2, -1, 2, 0, 327, 0],
    [0, 2, 1, 0, -323, 1165],
    [1, 1, -1, 0, 299, 0],
    [2, 0, 3, 0, 294, 0],
    [2, 0, -1, -2, 0, 8752],
];

/// Table 47.B — `D, M, M', F, Σb (1e-6 deg)`, the 60 terms in Meeus' order.
const LATITUDE_TERMS: [[i32; 5]; 60] = [
    [0, 0, 0, 1, 5_128_122],
    [0, 0, 1, 1, 280_602],
    [0, 0, 1, -1, 277_693],
    [2, 0, 0, -1, 173_237],
    [2, 0, -1, 1, 55_413],
    [2, 0, -1, -1, 46_271],
    [2, 0, 0, 1, 32_573],
    [0, 0, 2, 1, 17_198],
    [2, 0, 1, -1, 9266],
    [0, 0, 2, -1, 8822],
    [2, -1, 0, -1, 8216],
    [2, 0, -2, -1, 4324],
    [2, 0, 1, 1, 4200],
    [2, 1, 0, -1, -3359],
    [2, -1, -1, 1, 2463],
    [2, -1, 0, 1, 2211],
    [2, -1, -1, -1, 2065],
    [0, 1, -1, -1, -1870],
    [4, 0, -1, -1, 1828],
    [0, 1, 0, 1, -1794],
    [0, 0, 0, 3, -1749],
    [0, 1, -1, 1, -1565],
    [1, 0, 0, 1, -1491],
    [0, 1, 1, 1, -1475],
    [0, 1, 1, -1, -1410],
    [0, 1, 0, -1, -1344],
    [1, 0, 0, -1, -1335],
    [0, 0, 3, 1, 1107],
    [4, 0, 0, -1, 1021],
    [4, 0, -1, 1, 833],
    [0, 0, 1, -3, 777],
    [4, 0, -2, 1, 671],
    [2, 0, 0, -3, 607],
    [2, 0, 2, -1, 596],
    [2, -1, 1, -1, 491],
    [2, 0, -2, 1, -451],
    [0, 0, 3, -1, 439],
    [2, 0, 2, 1, 422],
    [2, 0, -3, -1, 421],
    [2, 1, -1, 1, -366],
    [2, 1, 0, 1, -351],
    [4, 0, 0, 1, 331],
    [2, -1, 1, 1, 315],
    [2, -2, 0, -1, 302],
    [0, 0, 1, 3, -283],
    [2, 1, 1, -1, -229],
    [1, 1, 0, -1, 223],
    [1, 1, 0, 1, 223],
    [0, 1, -2, -1, -220],
    [2, 1, -1, -1, -220],
    [1, 0, 1, 1, -185],
    [2, -1, -2, -1, 181],
    [0, 1, 2, 1, -177],
    [4, 0, -2, -1, 176],
    [4, -1, -1, -1, 166],
    [1, 0, 1, -1, -164],
    [4, 0, 1, -1, 132],
    [1, 0, -1, -1, -119],
    [4, -1, 0, -1, 115],
    [2, -2, 0, 1, 107],
];

/// The Moon's geocentric ecliptic position at a Julian Ephemeris Day (TT), Meeus 47.1–47.6.
#[must_use]
pub fn position(jd_tt: f64) -> Position {
    use std::f64::consts::PI;

    let t = (jd_tt - 2_451_545.0) / 36_525.0;
    let t2 = t * t;
    let t3 = t2 * t;
    let t4 = t3 * t;
    let l_prime = 218.316_447_7 + 481_267.881_234_21 * t - 0.001_578_6 * t2 + t3 / 538_841.0
        - t4 / 65_194_000.0;
    let d = 297.850_192_1 + 445_267.111_403_4 * t - 0.001_881_9 * t2 + t3 / 545_868.0
        - t4 / 113_065_000.0;
    let m = 357.529_109_2 + 35_999.050_290_9 * t - 0.000_153_6 * t2 + t3 / 24_490_000.0;
    let m_prime = 134.963_396_4 + 477_198.867_505_5 * t + 0.008_741_4 * t2 + t3 / 69_699.0
        - t4 / 14_712_000.0;
    let f = 93.272_095 + 483_202.017_523_3 * t - 0.003_653_9 * t2 - t3 / 3_526_000.0
        + t4 / 863_310_000.0;
    let a1 = 119.75 + 131.849 * t;
    let a2 = 53.09 + 479_264.290 * t;
    let a3 = 313.45 + 481_266.484 * t;

    // The eccentricity correction E, applied to the terms that carry M (Meeus 47.6).
    let correction = 1.0 - 0.002_516 * t - 0.000_007_4 * t2;
    let eccentricity = |m_coefficient: i32| match m_coefficient.abs() {
        1 => correction,
        2 => correction * correction,
        _ => 1.0,
    };

    let mut sigma_l = 0.0_f64;
    let mut sigma_r = 0.0_f64;
    for [d_c, m_c, mp_c, f_c, l_c, r_c] in LONGITUDE_TERMS {
        let argument = (f64::from(d_c) * d
            + f64::from(m_c) * m
            + f64::from(mp_c) * m_prime
            + f64::from(f_c) * f)
            .to_radians();
        let factor = eccentricity(m_c);
        sigma_l += f64::from(l_c) * factor * argument.sin();
        sigma_r += f64::from(r_c) * factor * argument.cos();
    }
    let mut sigma_b = 0.0_f64;
    for [d_c, m_c, mp_c, f_c, b_c] in LATITUDE_TERMS {
        let argument = (f64::from(d_c) * d
            + f64::from(m_c) * m
            + f64::from(mp_c) * m_prime
            + f64::from(f_c) * f)
            .to_radians();
        sigma_b += f64::from(b_c) * eccentricity(m_c) * argument.sin();
    }

    // The additive corrections for the actions of Venus (A1), Jupiter (A2) and the flattening of
    // the Earth (the `L' - F` and `L' ± M'` terms), Meeus 47.7/47.8.
    let sin = |deg: f64| (deg * PI / 180.0).sin();
    sigma_l += 3958.0 * sin(a1) + 1962.0 * sin(l_prime - f) + 318.0 * sin(a2);
    sigma_b += -2235.0 * sin(l_prime)
        + 382.0 * sin(a3)
        + 175.0 * sin(a1 - f)
        + 175.0 * sin(a1 + f)
        + 127.0 * sin(l_prime - m_prime)
        - 115.0 * sin(l_prime + m_prime);

    Position {
        longitude_deg: (l_prime + sigma_l / 1e6).rem_euclid(360.0),
        latitude_deg: sigma_b / 1e6,
        distance_km: 385_000.56 + sigma_r / 1000.0,
    }
}

/// Nutation in longitude Δψ and the increment of the obliquity Δε, in degrees.
///
/// The abridged chapter-22 series — the four largest terms, good to about 0.5″, far finer than the
/// truncated lunar series needs.
pub(crate) fn nutation_obliquity(jd_tt: f64) -> (f64, f64) {
    use std::f64::consts::PI;

    let t = (jd_tt - 2_451_545.0) / 36_525.0;
    let radians = |deg: f64| deg * PI / 180.0;
    let omega = radians(125.044_52 - 1_934.136_261 * t);
    let l = radians(280.4665 + 36_000.769_8 * t);
    let l_prime = radians(218.3165 + 481_267.881_3 * t);
    let dpsi = (-17.20 * omega.sin() - 1.32 * (2.0 * l).sin() - 0.23 * (2.0 * l_prime).sin()
        + 0.21 * (2.0 * omega).sin())
        / 3600.0;
    let deps = (9.20 * omega.cos() + 0.57 * (2.0 * l).cos() + 0.10 * (2.0 * l_prime).cos()
        - 0.09 * (2.0 * omega).cos())
        / 3600.0;
    (dpsi, deps)
}

/// The Moon's apparent geocentric right ascension and declination, in degrees, Meeus 13.3/13.4.
pub(crate) fn equatorial(jd_tt: f64) -> (f64, f64) {
    let position = position(jd_tt);
    let (dpsi, deps) = nutation_obliquity(jd_tt);
    let longitude = (position.longitude_deg + dpsi).to_radians();
    let latitude = position.latitude_deg.to_radians();
    let obliquity = (super::sun::obliquity_deg(jd_tt) + deps).to_radians();

    let (sin_lon, cos_lon) = longitude.sin_cos();
    let (sin_lat, cos_lat) = latitude.sin_cos();
    let (sin_eps, cos_eps) = obliquity.sin_cos();
    let ra = (sin_lon * cos_eps - (sin_lat / cos_lat) * sin_eps).atan2(cos_lon);
    let dec = (sin_lat * cos_eps + cos_lat * sin_eps * sin_lon).asin();
    (ra.to_degrees().rem_euclid(360.0), dec.to_degrees())
}

/// The synodic elongation `E ∈ [0, 360)`: `0` new, `90` first quarter, `180` full, `270` last.
///
/// This is the difference of the apparent geocentric longitudes, which is also the definition the
/// phase instants are found on.
#[must_use]
pub fn synodic_elongation(jd_tt: f64) -> f64 {
    let (dpsi, _) = nutation_obliquity(jd_tt);
    let moon = position(jd_tt).longitude_deg + dpsi;
    (moon - super::sun::apparent_longitude(jd_tt)).rem_euclid(360.0)
}

/// The illuminated fraction of the disc, 0 (new) to 1 (full), 0.5 at the quarters.
///
/// The phase angle uses both the elongation and the Moon's ecliptic latitude, which is what makes
/// the value hold up near New Moon: at a 3.6° latitude the latitude term alone is worth 0.1 pp.
#[must_use]
pub fn illuminated_fraction(jd_tt: f64) -> f64 {
    let elongation = synodic_elongation(jd_tt).to_radians();
    let latitude = position(jd_tt).latitude_deg.to_radians();
    (1.0 - elongation.cos() * latitude.cos()) / 2.0
}

/// The phase a synodic elongation names, by the eight 45°-wide windows of the plan.
#[must_use]
pub fn phase_of(elongation_deg: f64) -> MoonPhase {
    match elongation_deg.rem_euclid(360.0) {
        e if e < 22.5 || e >= 337.5 => MoonPhase::New,
        e if e < 67.5 => MoonPhase::WaxingCrescent,
        e if e < 112.5 => MoonPhase::FirstQuarter,
        e if e < 157.5 => MoonPhase::WaxingGibbous,
        e if e < 202.5 => MoonPhase::Full,
        e if e < 247.5 => MoonPhase::WaningGibbous,
        e if e < 292.5 => MoonPhase::LastQuarter,
        _ => MoonPhase::WaningCrescent,
    }
}

/// The four cardinal phases with the elongation they are defined by.
const CARDINAL: [(MoonPhase, f64); 4] = [
    (MoonPhase::New, 0.0),
    (MoonPhase::FirstQuarter, 90.0),
    (MoonPhase::Full, 180.0),
    (MoonPhase::LastQuarter, 270.0),
];

/// The phase at a Julian Ephemeris Day.
#[must_use]
pub fn phase_at(jd_tt: f64) -> MoonPhase {
    phase_of(synodic_elongation(jd_tt))
}

/// Days since the preceding New Moon, 0..≈29.53.
///
/// A new moon is always found: the seed is at most a day away from it (the mean rate differs from
/// the true one by ±20%), which the bracket search covers.
#[must_use]
pub fn age_days(jd_ut: f64) -> f64 {
    let elongation = synodic_elongation(jde(jd_ut));
    let seed = jd_ut - elongation / ELONGATION_RATE_DEG_PER_DAY;
    crossing_near(seed, 0.0).map_or(0.0, |new_moon| (jd_ut - new_moon).max(0.0))
}

/// The next `count` phase instants after `jd_ut` (a UT Julian Day), in chronological order.
///
/// Each instant is the root of `elongation(tt) = target` found by bisection, so the sequence is
/// exactly what [`phase_at`] reports at those instants. `Utc` instants are returned even though
/// the root is solved in TT: the ΔT offset is removed at the boundary.
#[must_use]
pub fn next_phases(jd_ut: f64, count: usize) -> Vec<(MoonPhase, DateTime<Utc>)> {
    let mut phases = Vec::with_capacity(count);
    let mut cursor = jd_ut;
    for _ in 0..count {
        let mut next: Option<(f64, MoonPhase)> = None;
        for (phase, target) in CARDINAL {
            if let Some(at) = next_crossing(cursor, target)
                && next.is_none_or(|(instant, _)| at < instant)
            {
                next = Some((at, phase));
            }
        }
        let Some((at, phase)) = next else { break };
        let Some(instant) = from_julian_day(at) else {
            break;
        };
        phases.push((phase, instant));
        // Every phase is at least seven days from the one before it: half a day is past the root
        // and far from the next one, without stepping over it.
        cursor = at + 0.5;
    }
    phases
}

/// The Moon's altitude above the horizon at a UT Julian Day, in degrees.
pub(crate) fn altitude(jd_ut: f64, lat_deg: f64, lon_deg: f64) -> f64 {
    let (ra, dec) = equatorial(jde(jd_ut));
    let hour_angle = super::gmst_deg(jd_ut) + lon_deg - ra;
    let h = (lat_deg.to_radians().sin() * dec.to_radians().sin()
        + lat_deg.to_radians().cos() * dec.to_radians().cos() * hour_angle.to_radians().cos())
    .asin();
    h.to_degrees()
}

/// The moonrise and moonset of the location-local calendar day, when they happen on that day.
///
/// A lunar day is about 24 h 50 m, so a calendar day can have both events, only one of them, or
/// — inside the polar circles — neither; a missing event is `None`, never a clamped midnight.
#[must_use]
pub fn moonrise_moonset(
    date: NaiveDate,
    lat: f64,
    lon: f64,
    tz: Tz,
) -> (Option<DateTime<FixedOffset>>, Option<DateTime<FixedOffset>>) {
    let crossings =
        super::crossings_in_local_day(date, tz, MOON_H0_DEG, |jd_ut| altitude(jd_ut, lat, lon));
    let event = |ascending: bool| {
        crossings
            .iter()
            .find(|crossing| crossing.ascending == ascending)
            .and_then(|crossing| super::local_instant(tz, crossing.at))
    };
    (event(true), event(false))
}

/// The next instant (a UT Julian Day) at which the elongation reaches `target_deg` at or after
/// `jd_ut`.
///
/// The mean-rate estimate is within a day of the true crossing, and the bracket around it is
/// widened until the wrapped difference changes sign — so a 20 % error in the rate can never
/// silently return another cycle's root.
fn next_crossing(jd_ut: f64, target_deg: f64) -> Option<f64> {
    let elongation = (target_deg - synodic_elongation(jde(jd_ut))).rem_euclid(360.0);
    let seed = jd_ut + elongation / ELONGATION_RATE_DEG_PER_DAY;
    crossing_near(seed, target_deg)
}

/// Bisects the wrapped difference `elongation - target` around `seed`, widening the bracket until
/// the difference changes sign; the root is refined to below one second.
fn crossing_near(seed: f64, target_deg: f64) -> Option<f64> {
    let wrapped = |jd_ut: f64| {
        let difference = synodic_elongation(jde(jd_ut)) - target_deg;
        (difference + 180.0).rem_euclid(360.0) - 180.0
    };

    let mut low = seed - 1.0;
    let mut high = seed + 1.0;
    while wrapped(low) >= 0.0 && seed - low < 5.0 {
        low -= 0.5;
    }
    while wrapped(high) <= 0.0 && high - seed < 5.0 {
        high += 0.5;
    }
    if wrapped(low) >= 0.0 || wrapped(high) <= 0.0 {
        return None;
    }
    for _ in 0..60 {
        if (high - low) * 86_400.0 < 0.5 {
            break;
        }
        let middle = f64::midpoint(low, high);
        if wrapped(middle) < 0.0 {
            low = middle;
        } else {
            high = middle;
        }
    }
    Some(f64::midpoint(low, high))
}

#[cfg(test)]
mod tests {
    use super::{age_days, illuminated_fraction, next_phases, phase_of, position};
    use crate::model::astro::MoonPhase;

    #[test]
    fn meeus_example_47_a_is_reproduced() {
        // 1992 April 12.0 TD: λ = 133.162655°, β = −3.229126°, Δ = 368409.7 km.
        let position = position(2_448_724.5);
        assert!(
            (position.longitude_deg - 133.162_655).abs() < 1e-5,
            "λ = {}",
            position.longitude_deg
        );
        assert!(
            (position.latitude_deg + 3.229_126).abs() < 1e-5,
            "β = {}",
            position.latitude_deg
        );
        assert!(
            (position.distance_km - 368_409.7).abs() < 0.1,
            "Δ = {}",
            position.distance_km
        );
    }

    #[test]
    fn the_eight_phase_windows_break_where_the_plan_says() {
        for (elongation, expected) in [
            (0.0, MoonPhase::New),
            (22.499, MoonPhase::New),
            (337.5, MoonPhase::New),
            (359.999, MoonPhase::New),
            (22.5, MoonPhase::WaxingCrescent),
            (67.499, MoonPhase::WaxingCrescent),
            (67.5, MoonPhase::FirstQuarter),
            (112.499, MoonPhase::FirstQuarter),
            (112.5, MoonPhase::WaxingGibbous),
            (157.499, MoonPhase::WaxingGibbous),
            (157.5, MoonPhase::Full),
            (202.499, MoonPhase::Full),
            (202.5, MoonPhase::WaningGibbous),
            (247.499, MoonPhase::WaningGibbous),
            (247.5, MoonPhase::LastQuarter),
            (292.499, MoonPhase::LastQuarter),
            (292.5, MoonPhase::WaningCrescent),
            (337.499, MoonPhase::WaningCrescent),
            (-10.0, MoonPhase::New),
            (360.0, MoonPhase::New),
        ] {
            assert_eq!(phase_of(elongation), expected, "E = {elongation}");
        }
    }

    #[test]
    fn the_book_new_moon_of_1977_is_within_two_minutes() {
        // Meeus example 49.a: 1977 February 18 at 3h37m42s TD, JDE 2443192.65118. The solver
        // works in TT, so the comparison stays in TT.
        let jd_td = 2_443_192.651_18;
        let root = next_phases(jd_td - 1.0, 1)
            .into_iter()
            .next()
            .map(|(phase, instant)| {
                assert_eq!(phase, MoonPhase::New);
                let jd_ut = crate::astro::julian::julian_day(instant);
                jd_ut + crate::astro::julian::delta_t_seconds(jd_ut) / 86_400.0
            })
            .expect("a new moon follows");
        assert!(
            (root - jd_td).abs() * 1440.0 < 2.0,
            "off by {:.1} min",
            (root - jd_td).abs() * 1440.0
        );
    }

    #[test]
    fn the_illuminated_fraction_is_symmetric_around_full_and_new() {
        // A real new moon from the solver, then its full moon a fortnight later.
        let (_, new_moon) = next_phases(2_461_000.0, 4)
            .into_iter()
            .find(|(phase, _)| *phase == MoonPhase::New)
            .expect("a new moon follows");
        let new = crate::astro::julian::julian_day(new_moon);
        let full = new + crate::astro::moon::SYNODIC_MONTH_DAYS / 2.0;
        assert!(
            illuminated_fraction(crate::astro::julian::jde(new)) < 0.01,
            "{}",
            illuminated_fraction(crate::astro::julian::jde(new))
        );
        assert!(
            illuminated_fraction(crate::astro::julian::jde(full)) > 0.99,
            "{}",
            illuminated_fraction(crate::astro::julian::jde(full))
        );
        assert!(age_days(new) < 0.5 || age_days(new) > 29.0);
        assert!((0.0..=1.0).contains(&illuminated_fraction(crate::astro::julian::jde(new + 7.0))));
    }
}
