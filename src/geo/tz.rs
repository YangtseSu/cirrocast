// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Coordinate → IANA time zone, for the backends whose payload carries no zone.
//!
//! Most providers answer with a zone of their own, so a location resolved from raw `@lat,lon`
//! coordinates is corrected from the response (step 06). Aviation weather does not: a METAR
//! carries the observation time in UTC and the station metadata has no zone field, yet the header
//! and the observation line have to show the station's local time. [`lookup`] is that one missing
//! piece, and it is deliberately the only coordinate → zone path in the crate.
//!
//! The lookup is offline: the `tzf-rs` crate embeds the time zone polygons in the binary (with the
//! `bundled` feature), so nothing is read from disk at runtime and no request is made. The finder
//! index is built once per process and reused.
//!
//! Inputs are validated before the lookup: `tzf-rs`'s fuzzy stage rejects a non-finite or
//! out-of-range coordinate by panicking, which is not an option here (the crate forbids a panic on
//! user-triggered input), so such a pair simply has no zone.

use std::sync::LazyLock;

use chrono_tz::Tz;
use tzf_rs::DefaultFinder;

/// The process-wide finder: the embedded index is parsed once, then only queried.
static FINDER: LazyLock<DefaultFinder> = LazyLock::new(DefaultFinder::new);

/// The IANA zone containing `lat`/`lon`, when it is one this build knows and the pair is valid.
///
/// `None` covers three cases the caller treats the same way: a coordinate outside the ranges,
/// an ocean point the dataset maps to a nautical zone this build cannot parse, and a dataset gap.
/// The caller keeps its own fallback (UTC, announced under `-v`) rather than inventing a zone.
#[must_use]
pub fn lookup(lat: f64, lon: f64) -> Option<Tz> {
    if !lat.is_finite() || !lon.is_finite() || !(-90.0..=90.0).contains(&lat) {
        return None;
    }
    if !(-180.0..=180.0).contains(&lon) {
        return None;
    }

    let name = FINDER.get_tz_name(lon, lat);
    if name.is_empty() {
        return None;
    }
    name.parse::<Tz>().ok()
}

#[cfg(test)]
mod tests {
    use super::lookup;

    #[test]
    fn known_places_resolve_to_their_zone() {
        let cases = [
            (39.9042, 116.4074, "Asia/Shanghai"),
            (40.6392, -73.7639, "America/New_York"),
            (49.0150, 2.5340, "Europe/Paris"),
            (40.0820, 116.6030, "Asia/Shanghai"),
            (70.1910, -148.4800, "America/Anchorage"),
            (-4.3870, 15.4480, "Africa/Kinshasa"),
            (-12.4240, 130.8930, "Australia/Darwin"),
        ];
        for (lat, lon, expected) in cases {
            let zone = lookup(lat, lon).unwrap_or_else(|| panic!("{lat},{lon} has a zone"));
            assert_eq!(zone.name(), expected, "{lat},{lon}");
        }
    }

    #[test]
    fn a_zone_survives_the_winter_offset() {
        use chrono::{Offset as _, TimeZone as _};
        let zone = lookup(40.6392, -73.7639).expect("New York has a zone");
        let winter = zone.with_ymd_and_hms(2026, 1, 15, 12, 0, 0).unwrap();
        assert_eq!(winter.offset().fix().local_minus_utc(), -5 * 3600);
        let summer = zone.with_ymd_and_hms(2026, 7, 15, 12, 0, 0).unwrap();
        assert_eq!(summer.offset().fix().local_minus_utc(), -4 * 3600);
    }

    #[test]
    fn invalid_coordinates_have_no_zone() {
        assert!(lookup(f64::NAN, 0.0).is_none());
        assert!(lookup(0.0, f64::INFINITY).is_none());
        assert!(lookup(91.0, 0.0).is_none());
        assert!(lookup(0.0, 181.0).is_none());
    }
}
