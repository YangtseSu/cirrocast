// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Julian dates and ΔT: the two time-scale conversions of the astronomy module.
//!
//! Every position series in [`super::moon`] and [`super::sun`] wants a **Julian Ephemeris Day**
//! (the TT-based Julian Date), while every public instant in this crate is UTC. The seam between
//! the two scales lives here:
//!
//! * [`julian_day`] and [`from_julian_day`] convert through the Unix epoch, which makes the
//!   arithmetic exact to the microsecond for every date this program can print — Meeus chapter 7's
//!   calendar formulas would only re-derive what `chrono` already counts;
//! * [`delta_t_seconds`] is TT − UT1 from the Espenak–Meeus polynomial fits, the single place
//!   where the offset is applied. `jde()` is the one-liner every caller uses.
//!
//! The supported range is 1900–2100 (the module documentation states what happens beyond it);
//! outside the fits' 1900–2150 window the value is held at the boundary so the function stays
//! total. Nothing here reads the clock or the network.

use chrono::{DateTime, Utc};

/// The Julian Date of the Unix epoch, 1970-01-01T00:00:00Z.
pub const UNIX_EPOCH_JD: f64 = 2_440_587.5;

/// Seconds in a day.
const SECS_PER_DAY: f64 = 86_400.0;

/// The Julian Day of a UTC instant.
///
/// The cast is exact for every instant `chrono` can represent: the Unix timestamp is below 2⁵³,
/// the precision an `f64` holds integers to.
#[allow(clippy::cast_precision_loss)]
#[must_use]
pub fn julian_day(at: DateTime<Utc>) -> f64 {
    let seconds = at.timestamp() as f64 + f64::from(at.timestamp_subsec_nanos()) / 1e9;
    UNIX_EPOCH_JD + seconds / SECS_PER_DAY
}

/// The UTC instant at a Julian Day, rounded to the nearest millisecond.
///
/// A millisecond is the resolution an `f64` Julian Day actually has around the years this program
/// prints: at JD ≈ 2.46×10⁶ the spacing of representable values is about 40 µs, so a claim of
/// microseconds would be false, and without *any* rounding a midnight would round-trip to
/// `23:59:59.99998…` and name the wrong calendar day. `None` for a value `chrono` cannot
/// represent.
///
/// The casts are safe by construction: `seconds` is a finite, rounded millisecond count, the
/// fraction is clamped into `0..=999_999_999`, and an out-of-range second count saturates and is
/// rejected by [`DateTime::from_timestamp`] rather than wrapping.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
#[must_use]
pub fn from_julian_day(jd: f64) -> Option<DateTime<Utc>> {
    if !jd.is_finite() {
        return None;
    }
    let seconds = ((jd - UNIX_EPOCH_JD) * SECS_PER_DAY * 1000.0).round() / 1000.0;
    let whole = seconds.floor();
    let nanos = ((seconds - whole) * 1e9).round();
    DateTime::from_timestamp(whole as i64, nanos.clamp(0.0, 999_999_999.0) as u32)
}

/// The Julian Ephemeris Day of a UT Julian Day: the argument the position series want.
#[must_use]
pub fn jde(jd_ut: f64) -> f64 {
    jd_ut + delta_t_seconds(jd_ut) / SECS_PER_DAY
}

/// TT − UT1 in seconds: the Espenak–Meeus polynomial fits for 1900–2150.
///
/// The fits are the published ones (NASA's eclipse pages carry the same set); their accuracy is
/// a few tenths of a second inside the range, which is far below what the truncated lunar series
/// of this module can resolve. Outside 1900–2150 the year is clamped, so a wildly out-of-range
/// instant degrades to the boundary value instead of a polynomial blow-up.
#[must_use]
pub fn delta_t_seconds(jd_ut: f64) -> f64 {
    let year = 2000.0 + (jd_ut - 2_451_545.0) / 365.25;
    let y = year.clamp(1900.0, 2150.0);

    if y < 1920.0 {
        let t = y - 1900.0;
        -2.79 + t * (1.494_119 + t * (-0.059_893_9 + t * (0.006_196_6 - 0.000_197 * t)))
    } else if y < 1941.0 {
        let t = y - 1920.0;
        21.20 + t * (0.84493 + t * (-0.076_100 + 0.002_093_6 * t))
    } else if y < 1961.0 {
        let t = y - 1950.0;
        29.07 + 0.407 * t - t * t / 233.0 + t * t * t / 2547.0
    } else if y < 1986.0 {
        let t = y - 1975.0;
        45.45 + 1.067 * t - t * t / 260.0 - t * t * t / 718.0
    } else if y < 2005.0 {
        let t = y - 2000.0;
        63.86
            + t * (0.3345
                + t * (-0.060_374 + t * (0.001_727_5 + t * (0.000_651_814 + 0.000_023_735_99 * t))))
    } else if y < 2050.0 {
        let t = y - 2000.0;
        62.92 + t * (0.32217 + 0.005_589 * t)
    } else {
        -20.0 + 32.0 * ((y - 1820.0) / 100.0).powi(2) - 0.5628 * (2150.0 - y)
    }
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, TimeZone as _, Utc};

    use super::{UNIX_EPOCH_JD, delta_t_seconds, from_julian_day, jde, julian_day};

    #[test]
    fn the_epoch_and_the_j2000_instant_are_the_textbook_values() {
        let epoch = Utc
            .with_ymd_and_hms(1970, 1, 1, 0, 0, 0)
            .single()
            .expect("a valid instant");
        assert!((julian_day(epoch) - UNIX_EPOCH_JD).abs() < 1e-9);

        let j2000 = Utc
            .with_ymd_and_hms(2000, 1, 1, 12, 0, 0)
            .single()
            .expect("a valid instant");
        assert!((julian_day(j2000) - 2_451_545.0).abs() < 1e-9);
    }

    #[test]
    fn a_julian_day_round_trips_to_the_millisecond() {
        for iso in [
            "1977-02-18T03:37:42Z",
            "2026-10-04T12:15:00Z",
            "2026-10-03T16:00:00Z",
            "2100-12-31T23:59:59Z",
        ] {
            let at = DateTime::parse_from_rfc3339(iso)
                .expect("a valid instant")
                .with_timezone(&Utc);
            let round_tripped = from_julian_day(julian_day(at)).expect("a representable instant");
            let delta = (round_tripped - at).num_milliseconds().abs();
            assert!(delta <= 1, "{iso}: round trip off by {delta} ms");
        }
        assert!(from_julian_day(f64::NAN).is_none());
        assert!(from_julian_day(f64::INFINITY).is_none());
    }

    #[test]
    fn delta_t_matches_the_known_values_of_its_range() {
        // 1977: the Meeus era tables give 47.7 s (the 1961–1986 fit is the one that answers).
        assert!(
            (47.0..=48.5).contains(&delta_t_seconds(julian_day(
                Utc.with_ymd_and_hms(1977, 2, 18, 3, 0, 0)
                    .single()
                    .expect("a valid instant"),
            )))
        );
        // 2000.0: the fit's own anchor, 63.8 s.
        assert!((63.0..=64.5).contains(&delta_t_seconds(2_451_545.0)));
        // 2044 sits in the 2005–2050 parabola, a little below 90 s.
        assert!((85.0..=95.0).contains(&delta_t_seconds(2_467_635.5)));
        // Outside the fits the value is held at the boundary rather than extrapolated: two
        // instants beyond 2150 answer exactly the same value.
        let year = |y: i32| {
            julian_day(
                Utc.with_ymd_and_hms(y, 1, 1, 0, 0, 0)
                    .single()
                    .expect("a valid instant"),
            )
        };
        assert_eq!(delta_t_seconds(year(2200)), delta_t_seconds(year(2500)));
        assert_eq!(delta_t_seconds(year(1800)), delta_t_seconds(year(1850)));
        assert!(delta_t_seconds(year(2150)).is_finite());
    }

    #[test]
    fn jde_is_the_ut_day_plus_the_offset() {
        let jd = 2_451_545.0;
        let offset = delta_t_seconds(jd) / 86_400.0;
        // The sum is a float near 2.45e6, so it holds ~40 µs: compare at that scale.
        assert!((jde(jd) - jd - offset).abs() < 1e-9);
    }
}
