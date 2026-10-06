// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Candidate ordering, shared by every geocoding source.
//!
//! The network geocoder returns [`Location`] values, the bundled table returns `City` rows, and
//! both are ordered by the one function here: an exact (folded) name match first, then a prefix
//! match, then larger population, and finally the display name and ascii spelling as a
//! deterministic tiebreak. Because the order depends on the candidates alone, `location search`
//! cannot order the same query differently in offline and network modes — which is what step 18's
//! equivalence test pins — and two runs resolve the same place even if a service returns its hits
//! in another order.

use std::cmp::Ordering;

use crate::geo::fold::fold;
use crate::model::Location;

/// A geocoding candidate's ordering inputs.
///
/// Implementations must be pure: ranking is a total order over these three accessors only, so a
/// re-run of the same query resolves to the same place even when the source's own order changes.
pub trait Candidate {
    /// The display name.
    fn name(&self) -> &str;

    /// A Latin spelling when the source has one (the bundled table stores `GeoNames`' ascii name);
    /// `None` when the display name is all there is, as with the geocoding API's hits.
    fn ascii_name(&self) -> Option<&str> {
        None
    }

    /// Population when the source reports one; larger sorts first.
    fn population(&self) -> Option<u64>;
}

impl Candidate for Location {
    fn name(&self) -> &str {
        &self.name
    }

    fn population(&self) -> Option<u64> {
        self.population
    }
}

/// How well a name matches the query; higher sorts first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Match {
    /// The source offered the candidate for a different reason (`None` query included).
    Other,
    /// The folded name or ascii name starts with the folded query.
    Prefix,
    /// The folded name or ascii name equals the folded query.
    Exact,
}

/// The folded query of one call, built once so a sort does not fold it per comparison.
struct Query {
    folded: String,
}

impl Query {
    fn new(text: &str) -> Self {
        Self { folded: fold(text) }
    }

    /// The match tier of one candidate; an empty folded query matches nothing exactly or by
    /// prefix, so only the population tier is left.
    fn tier<C: Candidate + ?Sized>(&self, candidate: &C) -> Match {
        if self.folded.is_empty() {
            return Match::Other;
        }
        let name = fold(candidate.name());
        if name == self.folded {
            return Match::Exact;
        }
        let prefix = name.starts_with(&self.folded);
        let ascii = candidate.ascii_name().map(fold);
        match ascii {
            Some(ascii) if ascii == self.folded => Match::Exact,
            Some(ascii) if ascii.starts_with(&self.folded) || prefix => Match::Prefix,
            None if prefix => Match::Prefix,
            _ => Match::Other,
        }
    }

    fn order<A: Candidate + ?Sized, B: Candidate + ?Sized>(&self, left: &A, right: &B) -> Ordering {
        self.tier(right)
            .cmp(&self.tier(left))
            .then_with(|| by_population_then_name(left, right))
    }
}

/// Larger population first, then the display name and the ascii spelling.
///
/// The last two make the order total: without them two candidates with equal tier and equal
/// population (including two `None`s collapsed to `0`) would fall back to the order the source
/// happened to return, and a re-run could resolve a different place.
fn by_population_then_name<A: Candidate + ?Sized, B: Candidate + ?Sized>(
    left: &A,
    right: &B,
) -> Ordering {
    right
        .population()
        .unwrap_or(0)
        .cmp(&left.population().unwrap_or(0))
        .then_with(|| left.name().cmp(right.name()))
        .then_with(|| left.ascii_name().cmp(&right.ascii_name()))
}

/// Orders candidates the way [`resolve`](crate::geo::resolve) picks one, keeping at most `limit`.
///
/// The order is a total order over the candidates' own fields, so re-running a query resolves to
/// the same location even when the service returns its hits in a different order.
#[must_use]
pub fn rank<T: Candidate>(mut results: Vec<T>, query: Option<&str>, limit: u8) -> Vec<T> {
    match query {
        Some(query) => {
            let query = Query::new(query);
            results.sort_by(|left, right| query.order(left, right));
        }
        None => results.sort_by(by_population_then_name),
    }
    results.truncate(usize::from(limit));
    results
}

/// Whether `candidate`'s name or ascii name is exactly the query under folding.
///
/// This is the filter behind `:name`/`--exact` and the `Exact` tier of [`rank`], so the two can
/// never disagree about what "the same name" means.
#[must_use]
pub fn same_name<C: Candidate + ?Sized>(candidate: &C, query: &str) -> bool {
    let query = Query::new(query);
    !query.folded.is_empty() && query.tier(candidate) == Match::Exact
}

#[cfg(test)]
mod tests {
    use chrono_tz::Tz;

    use super::{Candidate, rank, same_name};
    use crate::model::{Location, LocationSource};

    /// A candidate with the fields the ranking cares about.
    fn candidate(name: &str, admin1: Option<&str>, population: Option<u64>) -> Location {
        Location {
            name: name.to_owned(),
            admin1: admin1.map(str::to_owned),
            country: "China".to_owned(),
            country_code: Some("CN".to_owned()),
            lat: 39.9075,
            lon: 116.39723,
            tz: Tz::Asia__Shanghai,
            elevation_m: Some(49.0),
            population,
            source: LocationSource::Geocoder,
            station: None,
            named_by: None,
        }
    }

    /// A candidate that carries an ascii spelling the way the bundled table's rows do.
    #[derive(Debug, Clone, PartialEq)]
    struct Spot {
        display: &'static str,
        ascii: &'static str,
        population: Option<u64>,
    }

    impl Candidate for Spot {
        fn name(&self) -> &str {
            self.display
        }

        fn ascii_name(&self) -> Option<&str> {
            Some(self.ascii)
        }

        fn population(&self) -> Option<u64> {
            self.population
        }
    }

    fn spot(display: &'static str, ascii: &'static str, population: Option<u64>) -> Spot {
        Spot {
            display,
            ascii,
            population,
        }
    }

    #[test]
    fn exact_name_beats_population_and_upstream_order() {
        let hits = vec![
            candidate("Beijing City", None, Some(9_000_000)),
            candidate("Beijing", None, Some(10)),
        ];
        let ranked = rank(hits, Some("Beijing"), 10);
        assert_eq!(ranked[0].name, "Beijing");
        assert_eq!(ranked[0].population, Some(10));
    }

    #[test]
    fn population_beats_upstream_order_and_order_breaks_ties() {
        let hits = vec![
            candidate("First", None, Some(100)),
            candidate("Second", None, Some(500)),
            candidate("Third", None, None),
        ];
        let ranked = rank(hits.clone(), None, 10);
        assert_eq!(
            ranked
                .iter()
                .map(|location| location.name.as_str())
                .collect::<Vec<_>>(),
            ["Second", "First", "Third"]
        );
        let ranked = rank(hits, Some("Nope"), 10);
        assert_eq!(ranked[0].name, "Second");
    }

    #[test]
    fn equal_candidates_are_ordered_by_name_not_source_order() {
        // Equal tier and equal population: the name tiebreak must decide, whichever order the
        // source returned the two in.
        let hits = vec![
            candidate("Zeta", None, None),
            candidate("Alpha", None, None),
        ];
        let ranked = rank(hits, None, 10);
        assert_eq!(
            ranked
                .iter()
                .map(|location| location.name.as_str())
                .collect::<Vec<_>>(),
            ["Alpha", "Zeta"]
        );
        let hits = vec![
            candidate("Zeta", None, None),
            candidate("Alpha", None, None),
        ];
        let ranked = rank(hits, Some("Nope"), 10);
        assert_eq!(
            ranked
                .iter()
                .map(|location| location.name.as_str())
                .collect::<Vec<_>>(),
            ["Alpha", "Zeta"]
        );
    }

    #[test]
    fn rank_keeps_the_limit_after_sorting() {
        let hits = vec![
            candidate("Small", None, Some(1)),
            candidate("Big", None, Some(2)),
            candidate("Huge", None, Some(3)),
        ];
        let ranked = rank(hits, None, 2);
        assert_eq!(
            ranked
                .iter()
                .map(|location| location.name.as_str())
                .collect::<Vec<_>>(),
            ["Huge", "Big"]
        );
    }

    #[test]
    fn a_prefix_match_beats_a_larger_place_that_merely_contains_the_query() {
        let hits = vec![
            candidate("Greater Springfield", None, Some(9_000_000)),
            candidate("Springfield", None, Some(10)),
        ];
        let ranked = rank(hits, Some("Springf"), 10);
        assert_eq!(
            ranked
                .iter()
                .map(|location| location.name.as_str())
                .collect::<Vec<_>>(),
            ["Springfield", "Greater Springfield"]
        );
    }

    #[test]
    fn the_ascii_spelling_ranks_like_the_display_name() {
        // `MÜNCHEN` folds onto the ascii spelling, so the row with a different display name is an
        // exact match; a Latin spelling is not.
        let hits = vec![
            spot("Munich", "Munich", Some(1_260_391)),
            spot("Munich Suburb", "Munich Suburb", None),
        ];
        let ranked = rank(hits, Some("MÜNCHEN"), 10);
        assert_eq!(ranked[0].display, "Munich");

        let hits = vec![
            spot("München", "Muenchen", Some(1_000)),
            spot("Muncheberg", "Muncheberg", Some(9_000)),
        ];
        let ranked = rank(hits, Some("munchen"), 10);
        assert_eq!(ranked[0].display, "München");
    }

    #[test]
    fn same_name_uses_the_same_folding_as_the_ranking() {
        assert!(same_name(&spot("München", "Muenchen", None), "MÜNCHEN"));
        assert!(same_name(&candidate("São Paulo", None, None), "sao paulo"));
        assert!(same_name(&candidate("St. Louis", None, None), "St Louis"));
        assert!(!same_name(&candidate("São Paulo", None, None), "Paulo"));
        assert!(!same_name(&candidate("Beijing", None, None), "!!"));
    }
}
