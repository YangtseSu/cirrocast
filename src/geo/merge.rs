// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Merging the candidates several geocoding sources answered with (step 25).
//!
//! A name query under `[geo] search = "auto"` asks more than one source, and the same place comes
//! back from each of them with slightly different coordinates, names and populations (Open-Meteo
//! and `GeoNames` both redistribute the same `GeoNames` rows; Nominatim names the same town from an
//! OSM object). Showing the user the same place three times would make the picker useless, so the
//! per-source lists are collapsed here before the shared ranking decides a winner:
//!
//! * two candidates are the same place when their folded names and their country codes are equal
//!   and they lie within [`SAME_PLACE_KM`] of each other — the earlier source's record wins whole,
//!   population and zone included, so a merge never invents a field;
//! * a candidate whose country code is present but not two ASCII letters is dropped: `GeoNames`
//!   reports `-99` for the shapes that have no ISO code, and `auto` provider selection reads the
//!   code, so such a candidate could only mislead;
//! * the merged list is ordered by the shared step-04 keys (population, then name) — the query-aware
//!   tier of the ranking runs later, in [`resolve_candidates`](crate::geo::resolve_candidates),
//!   which is why a merge cannot change how a query resolves, only which candidates are on the
//!   list.
//!
//! The 5 km constant is breezy-weather's `CLOSE_DISTANCE` (the audit of 2026-10-03), kept here so
//! a later tuning is a deliberate change.

use crate::geo::chain::GeoSource;
use crate::geo::fold::fold;
use crate::geo::rank::rank;
use crate::model::Location;

/// How close two candidates of the same name and country must be to count as one place.
const SAME_PLACE_KM: f64 = 5.0;

/// De-duplicates the candidates `sources` contributed, keeping source order for the survivors.
///
/// The earlier source's record wins, so a merge never mixes fields from two sources; the caller
/// ranks the result for the query it asked (the limit is the caller's too).
#[must_use]
pub fn merge(sources: &[(GeoSource, Vec<Location>)]) -> Vec<Location> {
    // The folded name travels beside the record so the comparison folds each candidate once.
    let mut merged: Vec<(String, Location)> = Vec::new();
    for (_, hits) in sources {
        for location in hits {
            if invalid_country_code(location) {
                continue;
            }
            let name = fold(&location.name);
            let duplicate = merged
                .iter()
                .any(|(kept_name, kept)| *kept_name == name && same_place(kept, location));
            if !duplicate {
                merged.push((name, location.clone()));
            }
        }
    }
    let mut locations: Vec<Location> = merged.into_iter().map(|(_, location)| location).collect();
    locations = rank(locations, None, u8::MAX);
    locations
}

/// Whether the candidate carries a country code nothing downstream can use.
///
/// A *missing* code is not an invalid one: a Nominatim hit whose OSM object has no country tag is
/// still a real place, it simply never merges with anyone.
fn invalid_country_code(location: &Location) -> bool {
    location
        .country_code
        .as_deref()
        .is_some_and(|code| !crate::geo::is_country_code(code))
}

/// Whether two candidates describe the same place: same folded name, same country, within
/// [`SAME_PLACE_KM`].
///
/// A missing code on either side never matches: without a country there is nothing that says the
/// two `Springfield`s are the same one, and keeping them apart is the safe direction.
fn same_place(left: &Location, right: &Location) -> bool {
    same_country(left.country_code.as_deref(), right.country_code.as_deref())
        && crate::geo::distance_km(left.lat, left.lon, right.lat, right.lon) <= SAME_PLACE_KM
}

/// Whether two country codes are the same country, ignoring case.
///
/// Nominatim reports its codes lower-cased (`cn`) where `GeoNames` and Open-Meteo use the ISO
/// spelling (`CN`), so an exact comparison would keep the same place twice.
fn same_country(left: Option<&str>, right: Option<&str>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => left.eq_ignore_ascii_case(right),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use chrono_tz::Tz;

    use super::merge;
    use crate::geo::chain::GeoSource;
    use crate::model::{Location, LocationSource};

    /// A candidate with the fields a merge reads.
    fn candidate(
        name: &str,
        lat: f64,
        lon: f64,
        country: Option<&str>,
        population: Option<u64>,
    ) -> Location {
        Location {
            name: name.to_owned(),
            admin1: None,
            country: "China".to_owned(),
            country_code: country.map(str::to_owned),
            lat,
            lon,
            tz: Tz::Asia__Shanghai,
            elevation_m: None,
            population,
            source: LocationSource::Geocoder,
            station: None,
        }
    }

    /// The Beijing overlap: Open-Meteo's and `GeoNames`' rows for the same place collapse to one,
    /// and the earlier source's record (its population) is the one kept.
    #[test]
    fn the_same_place_from_two_sources_collapses_to_the_earlier_record() {
        let open_meteo = vec![candidate(
            "Beijing",
            39.9075,
            116.39723,
            Some("CN"),
            Some(18_960_744),
        )];
        let geonames = vec![
            candidate("Beijing", 39.9080, 116.3980, Some("CN"), Some(18_960_000)),
            // The Shanxi village called Beijing, 700 km away, is a different place.
            candidate("Beijing", 35.20917, 110.73278, Some("CN"), None),
        ];
        let merged = merge(&[
            (GeoSource::OpenMeteo, open_meteo),
            (GeoSource::GeoNames, geonames),
        ]);
        assert_eq!(merged.len(), 2, "{merged:?}");
        assert_eq!(
            merged[0].population,
            Some(18_960_744),
            "the earlier record wins"
        );
        assert_eq!(merged[0].lat, 39.9075);
        assert_eq!(merged[1].lat, 35.20917);
    }

    /// The 5 km boundary: 4.9 km apart is one place, 5.1 km is two.
    #[test]
    fn the_dedup_radius_is_five_kilometres() {
        // 0.01° of latitude is ~1.11 km; 0.044° is ~4.89 km and 0.046° ~5.11 km.
        let inside = merge(&[
            (
                GeoSource::OpenMeteo,
                vec![candidate("Springfield", 39.0, 116.0, Some("CN"), Some(1))],
            ),
            (
                GeoSource::GeoNames,
                vec![candidate("Springfield", 39.044, 116.0, Some("CN"), Some(2))],
            ),
        ]);
        assert_eq!(inside.len(), 1, "{inside:?}");

        let outside = merge(&[
            (
                GeoSource::OpenMeteo,
                vec![candidate("Springfield", 39.0, 116.0, Some("CN"), Some(1))],
            ),
            (
                GeoSource::GeoNames,
                vec![candidate("Springfield", 39.046, 116.0, Some("CN"), Some(2))],
            ),
        ]);
        assert_eq!(outside.len(), 2, "{outside:?}");
    }

    /// A `-99` code is dropped; a lowercase ISO spelling still merges with the uppercase one.
    #[test]
    fn invalid_codes_are_dropped_and_case_is_ignored() {
        let merged = merge(&[
            (
                GeoSource::OpenMeteo,
                vec![candidate("Kosovo Polje", 42.66, 21.20, Some("XK"), Some(1))],
            ),
            (
                GeoSource::GeoNames,
                vec![
                    candidate("Disputed", 42.0, 21.0, Some("-99"), Some(2)),
                    candidate("Kosovo Polje", 42.6601, 21.2001, Some("xk"), Some(3)),
                ],
            ),
        ]);
        assert_eq!(merged.len(), 1, "{merged:?}");
        assert_eq!(merged[0].country_code.as_deref(), Some("XK"));
        assert_eq!(merged[0].source, LocationSource::Geocoder);

        // A candidate without any code survives: it simply never merges.
        let merged = merge(&[(
            GeoSource::Nominatim,
            vec![candidate("Tsinghua", 40.0, 116.3, None, Some(4))],
        )]);
        assert_eq!(merged.len(), 1, "{merged:?}");
    }

    /// Two places with the same name and no country to tell them apart stay separate.
    #[test]
    fn two_springfields_stay_separate() {
        let merged = merge(&[
            (
                GeoSource::OpenMeteo,
                vec![
                    candidate("Springfield", 39.78, 89.65, Some("US"), Some(170_188)),
                    candidate("Springfield", 37.22, 93.29, Some("US"), Some(154_341)),
                ],
            ),
            (GeoSource::GeoNames, Vec::new()),
        ]);
        assert_eq!(merged.len(), 2, "{merged:?}");
        // The ranking is the shared population order.
        assert_eq!(merged[0].population, Some(170_188));
    }

    /// The merged list is ordered by the shared keys, whatever order the sources returned.
    #[test]
    fn the_result_is_ranked_by_population_then_name() {
        let merged = merge(&[
            (
                GeoSource::OpenMeteo,
                vec![
                    candidate("Zeta", 10.0, 10.0, Some("CN"), None),
                    candidate("Alpha", 11.0, 11.0, Some("CN"), None),
                ],
            ),
            (
                GeoSource::GeoNames,
                vec![candidate("Beta", 12.0, 12.0, Some("CN"), Some(500))],
            ),
        ]);
        let names: Vec<&str> = merged
            .iter()
            .map(|location| location.name.as_str())
            .collect();
        assert_eq!(names, ["Beta", "Alpha", "Zeta"]);
    }
}
