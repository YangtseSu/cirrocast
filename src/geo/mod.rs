// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Location arguments, the geocoder contract and deterministic candidate ranking.
//!
//! A location reaches `cirrocast` in exactly one of the forms enumerated by [`LocationSpec`], so
//! every accepted spelling has one code path and nothing is guessed twice: the one ambiguous form
//! (a fuzzy name) is resolved by the total order implemented in [`rank`], and the winner is echoed
//! by the caller. Coordinates and OpenStreetMap queries never touch the Open-Meteo geocoder.
//!
//! The module is synchronous and side-effect free except for [`Geocoder`] implementations, whose
//! constructors receive the shared HTTP client and cache of steps 05/06. It performs no I/O of its
//! own, which is what makes the parse table and the ranking rules testable without a network.

pub mod ip;
pub mod nominatim;
pub mod open_meteo;
pub mod tz;

use std::fmt;
use std::str::FromStr;

use chrono_tz::Tz;

use crate::error::{Error, Result};
use crate::model::{Location, LocationSource};

/// The four accepted spellings, shared by the usage errors and the CLI help so that the two can
/// never disagree.
pub const USAGE_FORMS: &str = "accepted forms: Beijing | :Beijing | ~Tsinghua | @39.9042,116.4074";

/// The shortest name the geocoding API accepts; shorter queries are rejected before a request.
const MIN_QUERY_CHARS: usize = 2;

/// What the user's location argument means.
///
/// `Default` is the empty argument: the caller substitutes `location.default` from the
/// configuration and parses *that* text, so the default is either another spec or the IP lookup.
#[derive(Debug, Clone, PartialEq)]
pub enum LocationSpec {
    /// No argument: use the configured default location, else the public IP.
    Default,
    /// `Beijing`: fuzzy search, the result is ranked (exact name, population, upstream order).
    Fuzzy(String),
    /// `:Beijing`: only a candidate whose name matches the query case-insensitively.
    Exact(String),
    /// `~Tsinghua`: OpenStreetMap/Nominatim search.
    Osm(String),
    /// `@39.9042,116.4074`: explicit coordinates, no geocoding request at all.
    LatLon(f64, f64),
}

impl LocationSpec {
    /// Parses a location argument; `None`, an empty string and whitespace all mean [`Self::Default`].
    ///
    /// Every rejection is an [`Error::Usage`] (exit code 2) whose message ends with
    /// [`USAGE_FORMS`], so the shell shows what the program would have accepted.
    pub fn parse_arg(arg: Option<&str>) -> Result<Self> {
        let Some(text) = arg.map(str::trim).filter(|text| !text.is_empty()) else {
            return Ok(Self::Default);
        };

        if let Some(rest) = text.strip_prefix('@') {
            return coordinates(rest);
        }
        if let Some(rest) = text.strip_prefix(':') {
            let query = rest.trim();
            if query.is_empty() {
                return Err(usage("`:`, `~` and `@` need a value"));
            }
            check_query_length(query)?;
            return Ok(Self::Exact(query.to_owned()));
        }
        if let Some(rest) = text.strip_prefix('~') {
            let query = rest.trim();
            if query.is_empty() {
                return Err(usage("`:`, `~` and `@` need a value"));
            }
            return Ok(Self::Osm(query.to_owned()));
        }

        check_query_length(text)?;
        Ok(Self::Fuzzy(text.to_owned()))
    }

    /// The text a geocoder gets for this spec, when it needs one.
    #[must_use]
    pub fn query(&self) -> Option<&str> {
        match self {
            Self::Fuzzy(query) | Self::Exact(query) | Self::Osm(query) => Some(query),
            Self::Default | Self::LatLon(..) => None,
        }
    }
}

impl FromStr for LocationSpec {
    type Err = Error;

    fn from_str(text: &str) -> Result<Self> {
        Self::parse_arg(Some(text))
    }
}

impl fmt::Display for LocationSpec {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Default => formatter.write_str("the configured location"),
            Self::Fuzzy(query) => write!(formatter, "`{query}`"),
            Self::Exact(query) => write!(formatter, "`:{query}`"),
            Self::Osm(query) => write!(formatter, "`~{query}`"),
            Self::LatLon(lat, lon) => write!(formatter, "`@{lat},{lon}`"),
        }
    }
}

/// How a resolved location was chosen; reported by the caller and used for the ambiguity note.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolution {
    /// The geocoder returned exactly one candidate.
    Only,
    /// `candidates` hits were ranked; the note mentions the count.
    Fuzzy {
        /// How many candidates the ranking saw.
        candidates: usize,
    },
    /// An exact-name query; only matching candidates were considered.
    Exact,
    /// The user gave coordinates, so no geocoder was asked.
    Coordinates,
}

/// A source of place names: one implementation per geocoding service.
///
/// Synchronous and `&self`, like the provider contract: implementations hold the shared HTTP client
/// and cache handed to their constructor and never open sockets or files themselves.
pub trait Geocoder {
    /// Returns up to `limit` candidates in the service's own order; an empty vector is a legitimate
    /// "nothing matched", while hard failures are [`Error::Network`]/[`Error::Upstream`].
    fn search(&self, query: &str, limit: u8) -> Result<Vec<Location>>;
}

/// Orders candidates the way [`resolve`] picks one: exact name match, then larger population, then
/// the upstream order (the sort is stable), keeping at most `limit` entries.
///
/// The order is total and depends on nothing but the input, so re-running a query resolves to the
/// same location even when the service returns its hits in a different order — the upstream order
/// is only the last tiebreaker.
#[must_use]
pub fn rank(mut results: Vec<Location>, query: Option<&str>, limit: u8) -> Vec<Location> {
    results.sort_by(|a, b| {
        let a_exact = query.is_some_and(|query| name_matches(&a.name, query));
        let b_exact = query.is_some_and(|query| name_matches(&b.name, query));
        b_exact
            .cmp(&a_exact)
            .then_with(|| b.population.unwrap_or(0).cmp(&a.population.unwrap_or(0)))
    });
    results.truncate(usize::from(limit));
    results
}

/// Picks one location out of the geocoder hits for `spec`.
///
/// `LatLon` bypasses ranking and builds the coordinate location itself, because the geocoding
/// endpoint has no reverse lookup; its zone stays UTC and is provisional until a provider reports
/// the real one. Every other spec ranks `results` and fails with [`Error::LocationNotFound`] (exit
/// code 5) when nothing is left — for `Exact` that includes "hits came back, but none of them is
/// named exactly like the query".
pub fn resolve(
    results: Vec<Location>,
    spec: &LocationSpec,
    limit: u8,
) -> Result<(Location, Resolution)> {
    if let LocationSpec::LatLon(lat, lon) = spec {
        return Ok((from_coordinates(*lat, *lon), Resolution::Coordinates));
    }

    let query = spec.query();
    let candidates: Vec<Location> = match spec {
        LocationSpec::Exact(_) => results
            .into_iter()
            .filter(|location| query.is_some_and(|query| name_matches(&location.name, query)))
            .collect(),
        _ => results,
    };

    let mut ranked = rank(candidates, query, limit);
    if ranked.is_empty() {
        return Err(Error::LocationNotFound(format!(
            "no location found for {spec}"
        )));
    }
    let chosen = ranked.remove(0);
    let resolution = match spec {
        LocationSpec::Exact(_) => Resolution::Exact,
        _ if ranked.is_empty() => Resolution::Only,
        _ => Resolution::Fuzzy {
            candidates: ranked.len() + 1,
        },
    };
    Ok((chosen, resolution))
}

/// The one-line note a fuzzy match prints on stderr, or `None` when there was nothing ambiguous.
///
/// Reporting happens once, at the point where the location is chosen: the user sees which of the
/// candidates won and how to require an exact name instead.
#[must_use]
pub fn ambiguity_note(query: &str, chosen: &Location, resolution: Resolution) -> Option<String> {
    note(query, chosen, resolution, true)
}

/// The same note for a fuzzy `~` search, without the advice to prepend `:`.
///
/// `:query` asks the *other* geocoder for a case-insensitive name match, so following the advice
/// inside an OpenStreetMap search would quietly switch the data source instead of narrowing this
/// one; the count and the winning place are still worth reporting.
#[must_use]
pub fn osm_ambiguity_note(
    query: &str,
    chosen: &Location,
    resolution: Resolution,
) -> Option<String> {
    note(query, chosen, resolution, false)
}

/// The shared shape of both notes; `hint` appends the `:query` advice.
fn note(query: &str, chosen: &Location, resolution: Resolution, hint: bool) -> Option<String> {
    let Resolution::Fuzzy { candidates } = resolution else {
        return None;
    };
    if candidates < 2 {
        return None;
    }
    let population = chosen
        .population
        .map(|population| format!(" (population {population})"))
        .unwrap_or_default();
    let hint = if hint {
        format!(" — pass `:{query}` to require an exact name match")
    } else {
        String::new()
    };
    Some(format!(
        "note: {candidates} candidates for `{query}`; using {}{population}{hint}",
        place(chosen)
    ))
}

/// The shared location header: `Name, admin1, country (lat, lon) Asia/Shanghai`.
///
/// Empty parts are skipped, both coordinates use two decimals, and a coordinate or OpenStreetMap
/// location with a provisional UTC zone says so instead of printing `UTC` — the provider that
/// fetches the weather replaces that zone from its own response. For `@lat,lon` the parenthesised
/// coordinates are omitted because the name *is* the pair. Step 06 reuses this line verbatim for
/// the plain renderer's `[LOCATION]` header.
#[must_use]
pub fn location_line(location: &Location) -> String {
    let mut parts = vec![place(location)];
    if location.source != LocationSource::Coordinates {
        parts.push(format!("({:.2}, {:.2})", location.lat, location.lon));
    }
    if provisional_zone(location) {
        parts.push("<timezone resolved at fetch time>".to_owned());
    } else {
        parts.push(location.tz.name().to_owned());
    }
    parts.join(" ")
}

/// The attribution a location's data source requires, or `None` when the source asks for none.
///
/// Displaying a place is displaying someone's data: `GeoNames` publishes the geocoding data that
/// Open-Meteo serves under CC-BY-4.0 (credit plus a link to the service, which is what the licence
/// page asks for next to displayed data), and OpenStreetMap requires the `ODbL` credit. Coordinates
/// and IP answers are the user's own input or the locating service's own answer, and neither
/// `ipwho.is` nor `ipapi.co` asks for a credit line — the privacy disclosure already names whichever
/// one answered.
///
/// The caller prints this next to the location (stderr in the CLI), and step 06's renderers use the
/// same function so a weather report carries its sources too.
#[must_use]
pub fn attribution_line(location: &Location) -> Option<&'static str> {
    match location.source {
        LocationSource::Geocoder => Some(
            "Location data based on GeoNames (CC-BY-4.0) via Open-Meteo — https://open-meteo.com/",
        ),
        LocationSource::Osm => Some("Location data © OpenStreetMap contributors (ODbL)"),
        // A station's coordinates are US-government public-domain metadata, which asks for no
        // credit line; the *weather* credit (`aviationweather.gov`) travels in the report's
        // attribution instead.
        LocationSource::Coordinates
        | LocationSource::Ip
        | LocationSource::Config
        | LocationSource::Station => None,
    }
}

/// `Name, admin1, country`, skipping the parts a geocoder did not report.
#[must_use]
pub fn place(location: &Location) -> String {
    let mut parts = vec![location.name.clone()];
    if let Some(admin1) = location
        .admin1
        .as_deref()
        .map(str::trim)
        .filter(|admin1| !admin1.is_empty())
    {
        parts.push(admin1.to_owned());
    }
    if !location.country.trim().is_empty() {
        parts.push(location.country.clone());
    }
    parts.join(", ")
}

/// The location behind `@lat,lon`: named after the pair, in UTC until a provider says otherwise.
#[must_use]
pub fn from_coordinates(lat: f64, lon: f64) -> Location {
    Location {
        name: format!("{lat}, {lon}"),
        admin1: None,
        country: String::new(),
        country_code: None,
        lat,
        lon,
        tz: Tz::UTC,
        elevation_m: None,
        population: None,
        source: LocationSource::Coordinates,
        station: None,
    }
}

/// Whether the zone is a placeholder rather than something upstream reported.
///
/// Coordinates have no lookup at all, and a Nominatim result only carries a zone when the OSM
/// object is tagged with one, so both fall back to UTC until the forecast response supplies a real
/// zone (step 06). A backend whose response carries no zone at all (`smhi`) must refuse such a
/// location instead of aggregating in UTC.
#[must_use]
pub fn provisional_zone(location: &Location) -> bool {
    matches!(
        location.source,
        LocationSource::Coordinates | LocationSource::Osm
    ) && location.tz == Tz::UTC
}

/// Case-insensitive name comparison for the exact-match ranking key and `:query` filtering.
fn name_matches(candidate: &str, query: &str) -> bool {
    candidate.trim().eq_ignore_ascii_case(query.trim())
}

/// Parses the text after `@`; every failure names the input and the accepted forms.
fn coordinates(text: &str) -> Result<LocationSpec> {
    let mut parts = text.split(',').map(str::trim);
    let (Some(lat), Some(lon), None) = (parts.next(), parts.next(), parts.next()) else {
        return Err(invalid_coordinates(text));
    };
    if lat.is_empty() || lon.is_empty() {
        return Err(invalid_coordinates(text));
    }
    let (Ok(lat), Ok(lon)) = (lat.parse::<f64>(), lon.parse::<f64>()) else {
        return Err(invalid_coordinates(text));
    };
    if !lat.is_finite() || !lon.is_finite() {
        return Err(invalid_coordinates(text));
    }
    if !(-90.0..=90.0).contains(&lat) {
        return Err(usage(format!("latitude {lat} is out of range -90..=90")));
    }
    if !(-180.0..=180.0).contains(&lon) {
        return Err(usage(format!("longitude {lon} is out of range -180..=180")));
    }
    Ok(LocationSpec::LatLon(lat, lon))
}

/// The rejection for `@` arguments that are not two numbers in range.
fn invalid_coordinates(text: &str) -> Error {
    usage(format!(
        "invalid coordinates `@{text}`: expected @<lat>,<lon>, e.g. @39.9042,116.4074"
    ))
}

/// Rejects names the geocoding API cannot match on.
fn check_query_length(query: &str) -> Result<()> {
    if query.chars().count() < MIN_QUERY_CHARS {
        return Err(usage(format!(
            "search term `{query}` is too short; the geocoding API needs at least {MIN_QUERY_CHARS} characters"
        )));
    }
    Ok(())
}

/// Appends the accepted forms to a usage message, so every rejection is self-explanatory.
fn usage(message: impl fmt::Display) -> Error {
    Error::Usage(format!("{message} ({USAGE_FORMS})"))
}

#[cfg(test)]
mod tests {
    use chrono_tz::Tz;

    use super::{LocationSpec, Resolution, ambiguity_note, location_line, rank, resolve};
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
    fn resolve_reports_the_resolution_kind() {
        let only = vec![candidate("Beijing", None, None)];
        let (chosen, resolution) = resolve(only, &LocationSpec::Fuzzy("Beijing".into()), 10)
            .expect("one candidate resolves");
        assert_eq!(chosen.name, "Beijing");
        assert_eq!(resolution, Resolution::Only);

        let many = vec![
            candidate("Beijing", None, Some(1)),
            candidate("Beijing", Some("Shanxi"), Some(2)),
        ];
        let (_, resolution) = resolve(many, &LocationSpec::Fuzzy("Beijing".into()), 10)
            .expect("two candidates resolve");
        assert_eq!(resolution, Resolution::Fuzzy { candidates: 2 });
    }

    #[test]
    fn exact_resolution_filters_and_fails_with_the_query_in_the_message() {
        let hits = vec![candidate("Beijing", None, Some(1))];
        let (chosen, resolution) =
            resolve(hits.clone(), &LocationSpec::Exact("beijing".into()), 10)
                .expect("the case-insensitive match wins");
        assert_eq!(chosen.name, "Beijing");
        assert_eq!(resolution, Resolution::Exact);

        let error = resolve(
            hits,
            &LocationSpec::Exact("Beijing Municipality".into()),
            10,
        )
        .expect_err("no exact match");
        assert!(
            error
                .to_string()
                .contains("no location found for `:Beijing Municipality`"),
            "{error}"
        );
        assert_eq!(error.exit_code(), 5);
    }

    #[test]
    fn coordinates_bypass_the_geocoder() {
        let (chosen, resolution) = resolve(
            vec![candidate("Nowhere", None, None)],
            &LocationSpec::LatLon(39.9042, 116.4074),
            10,
        )
        .expect("coordinates always resolve");
        assert_eq!(resolution, Resolution::Coordinates);
        assert_eq!(chosen.source, LocationSource::Coordinates);
        assert_eq!(chosen.tz, Tz::UTC);
        assert_eq!(chosen.name, "39.9042, 116.4074");
    }

    #[test]
    fn no_hits_names_the_query() {
        let error = resolve(vec![], &LocationSpec::Fuzzy("Atlantis".into()), 10)
            .expect_err("nothing to resolve");
        assert!(
            error
                .to_string()
                .contains("no location found for `Atlantis`")
        );
        assert_eq!(error.exit_code(), 5);
    }

    #[test]
    fn ambiguity_note_is_only_for_fuzzy_matches() {
        let chosen = candidate("Beijing", Some("Beijing Municipality"), Some(18_960_744));
        let note = ambiguity_note("Beijing", &chosen, Resolution::Fuzzy { candidates: 3 })
            .expect("a fuzzy match is noted");
        assert_eq!(
            note,
            "note: 3 candidates for `Beijing`; using Beijing, Beijing Municipality, China (population 18960744) — pass `:Beijing` to require an exact name match"
        );
        assert!(ambiguity_note("Beijing", &chosen, Resolution::Only).is_none());
        assert!(ambiguity_note("Beijing", &chosen, Resolution::Exact).is_none());
        assert!(ambiguity_note("Beijing", &chosen, Resolution::Coordinates).is_none());
        assert!(ambiguity_note("Beijing", &chosen, Resolution::Fuzzy { candidates: 1 }).is_none());

        let mut anonymous = chosen;
        anonymous.population = None;
        let note = ambiguity_note("Beijing", &anonymous, Resolution::Fuzzy { candidates: 2 })
            .expect("a fuzzy match without population is still noted");
        assert!(note.contains("using Beijing, Beijing Municipality, China — pass"));

        let osm =
            super::osm_ambiguity_note("Beijing", &anonymous, Resolution::Fuzzy { candidates: 2 })
                .expect("an OpenStreetMap match is noted too");
        assert_eq!(
            osm,
            "note: 2 candidates for `Beijing`; using Beijing, Beijing Municipality, China"
        );
        assert!(super::osm_ambiguity_note("Beijing", &anonymous, Resolution::Only).is_none());
    }

    #[test]
    fn location_lines_cover_every_source() {
        let geocoded = candidate("Beijing", Some("Beijing Municipality"), None);
        assert_eq!(
            location_line(&geocoded),
            "Beijing, Beijing Municipality, China (39.91, 116.40) Asia/Shanghai"
        );

        let mut coordinates = super::from_coordinates(39.9042, 116.4074);
        assert_eq!(
            location_line(&coordinates),
            "39.9042, 116.4074 <timezone resolved at fetch time>"
        );
        coordinates.tz = Tz::Asia__Shanghai;
        assert_eq!(
            location_line(&coordinates),
            "39.9042, 116.4074 Asia/Shanghai"
        );

        let mut osm = geocoded.clone();
        osm.source = LocationSource::Osm;
        osm.tz = Tz::UTC;
        assert_eq!(
            location_line(&osm),
            "Beijing, Beijing Municipality, China (39.91, 116.40) <timezone resolved at fetch time>"
        );
        osm.tz = Tz::Asia__Shanghai;
        assert_eq!(
            location_line(&osm),
            "Beijing, Beijing Municipality, China (39.91, 116.40) Asia/Shanghai"
        );

        let mut config = geocoded;
        config.source = LocationSource::Config;
        config.admin1 = None;
        config.country = String::new();
        assert_eq!(
            location_line(&config),
            "Beijing (39.91, 116.40) Asia/Shanghai"
        );
    }

    #[test]
    fn attribution_follows_the_data_source() {
        let mut location = candidate("Beijing", None, None);
        assert!(
            super::attribution_line(&location)
                .expect("a geocoded place credits GeoNames and Open-Meteo")
                .contains("GeoNames")
        );
        location.source = LocationSource::Osm;
        assert_eq!(
            super::attribution_line(&location),
            Some("Location data © OpenStreetMap contributors (ODbL)")
        );
        for source in [
            LocationSource::Coordinates,
            LocationSource::Ip,
            LocationSource::Config,
        ] {
            location.source = source;
            assert!(
                super::attribution_line(&location).is_none(),
                "{source:?} needs no attribution"
            );
        }
    }
}
