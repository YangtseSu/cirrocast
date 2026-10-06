// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Location arguments, the geocoder contract and deterministic candidate ranking.
//!
//! A location reaches `cirrocast` in exactly one of the forms enumerated by [`LocationSpec`], so
//! every accepted spelling has one code path and nothing is guessed twice: the one ambiguous form
//! (a fuzzy name) is resolved by the total order implemented in [`rank`](crate::geo::rank), and
//! the winner is echoed by the caller. Coordinates and OpenStreetMap queries never touch the
//! Open-Meteo geocoder, and a name may be answered by the bundled `GeoNames` table instead
//! ([`offline`](crate::geo::offline), step 18) — the ordering rule is shared, so the two sources
//! cannot rank the same query differently.
//!
//! The module is synchronous and side-effect free except for [`Geocoder`] implementations, whose
//! constructors receive the shared HTTP client and cache of steps 05/06. It performs no I/O of its
//! own, which is what makes the parse table and the ranking rules testable without a network.

pub mod chain;
pub mod fold;
pub mod geonames;
pub mod ip;
pub mod merge;
pub mod nominatim;
#[cfg(feature = "offline-geo")]
pub mod offline;
pub mod open_meteo;
pub mod pick;
pub mod rank;
pub mod table;
pub mod tz;
pub mod update;

use std::fmt;
use std::str::FromStr;

use chrono_tz::Tz;

use crate::error::{Error, Result};
use crate::geo::rank::{rank, same_name};
use crate::model::{Location, LocationSource};

/// The accepted spellings, shared by the usage errors and the CLI help so that the two can never
/// disagree.
pub const USAGE_FORMS: &str = "accepted forms: Beijing | :Beijing | ~Tsinghua | @39.9042,116.4074 | @name (an alias from [locations])";

/// The shortest name the geocoding API accepts; shorter queries are rejected before a request.
const MIN_QUERY_CHARS: usize = 2;

/// How many aliases one `@name` may expand through before the chain is refused as runaway.
const ALIAS_DEPTH_CAP: usize = 8;

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
    /// `@home`: a name from the `[locations]` alias table, expanded before resolution.
    Alias(String),
}

impl LocationSpec {
    /// Parses a location argument; `None`, an empty string and whitespace all mean [`Self::Default`].
    ///
    /// Every rejection is an [`Error::Usage`] (exit code 2) whose message ends with
    /// [`USAGE_FORMS`], so the shell shows what the program would have accepted.
    ///
    /// `@` text is coordinates when it is exactly `lat,lon` with both sides finite and in range;
    /// anything else is an alias name (`@home`), which the caller expands against `[locations]`.
    /// The order matters: `@39.9,116.4` must never be looked up as an alias called
    /// `39.9,116.4`.
    pub fn parse_arg(arg: Option<&str>) -> Result<Self> {
        let Some(text) = arg.map(str::trim).filter(|text| !text.is_empty()) else {
            return Ok(Self::Default);
        };

        if let Some(rest) = text.strip_prefix('@') {
            let rest = rest.trim();
            if rest.is_empty() {
                return Err(usage("`@` needs coordinates (`@lat,lon`) or an alias name"));
            }
            return Ok(match coordinates(rest) {
                Some((lat, lon)) => Self::LatLon(lat, lon),
                None => Self::Alias(rest.to_owned()),
            });
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
            Self::Default | Self::LatLon(..) | Self::Alias(_) => None,
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
            Self::Alias(name) => write!(formatter, "`@{name}`"),
        }
    }
}

/// Expands every `@name` in `spec` through the `[locations]` table until a non-alias spec is left.
///
/// Alias values are location arguments themselves, so `home = "work"` and `work = "@39.9,116.4"`
/// chain; the visited set catches a cycle (`a → b → a`) and reports the chain, and
/// [`ALIAS_DEPTH_CAP`] catches a chain long enough to look like a mistake. A cycle or exhaustion
/// is [`Error::Config`] because it is a property of the document, while an unknown name is
/// [`Error::Usage`] — the user typed it — with the closest configured names suggested.
pub fn expand_aliases(
    spec: LocationSpec,
    aliases: &std::collections::BTreeMap<String, String>,
) -> Result<LocationSpec> {
    let mut current = spec;
    let mut visited: Vec<String> = Vec::new();
    loop {
        let LocationSpec::Alias(name) = current else {
            return Ok(current);
        };
        if let Some(position) = visited.iter().position(|seen| *seen == name) {
            let chain: Vec<String> = visited[position..]
                .iter()
                .chain(std::iter::once(&name))
                .map(|link| format!("@{link}"))
                .collect();
            return Err(Error::Config(format!(
                "location alias cycle: {}",
                chain.join(" -> ")
            )));
        }
        if visited.len() >= ALIAS_DEPTH_CAP {
            let chain: Vec<String> = visited
                .iter()
                .chain(std::iter::once(&name))
                .map(|link| format!("@{link}"))
                .collect();
            return Err(Error::Config(format!(
                "location alias chain is deeper than {ALIAS_DEPTH_CAP}: {}",
                chain.join(" -> ")
            )));
        }
        let Some(value) = aliases.get(&name) else {
            return Err(unknown_alias(&name, aliases));
        };
        visited.push(name);
        current = LocationSpec::parse_arg(Some(value))?;
    }
}

/// The rejection for an alias the `[locations]` table does not define.
///
/// Up to three configured names within edit distance 2 of the typed one are suggested (never
/// selected: the user still has to type the corrected name), sorted by distance and name so the
/// message is deterministic. Without a candidate, the known names are listed, bounded so a huge
/// table cannot flood the terminal.
fn unknown_alias(name: &str, aliases: &std::collections::BTreeMap<String, String>) -> Error {
    // A `@` name that parses as a coordinate pair but lies outside the world is almost always a
    // typo for `@lat,lon` (the resolver only accepts a pair when both sides are in range), so the
    // message names the rejected pair instead of leaving the user to guess.
    let hint = match out_of_range_coordinates(name) {
        Some((lat, lon)) => format!(
            " (`{lat},{lon}` is not inside the world: latitude -90..=90, longitude -180..=180)"
        ),
        None => String::new(),
    };
    let mut candidates: Vec<(usize, &str)> = aliases
        .keys()
        .map(|key| {
            (
                edit_distance(&name.to_lowercase(), &key.to_lowercase()),
                key.as_str(),
            )
        })
        .filter(|(distance, _)| *distance <= 2)
        .collect();
    candidates.sort_unstable();
    candidates.truncate(3);
    if !candidates.is_empty() {
        let suggestions = candidates
            .iter()
            .map(|(_, key)| format!("@{key}"))
            .collect::<Vec<_>>()
            .join(", ");
        return usage(format!(
            "unknown location alias `@{name}`{hint}; did you mean {suggestions}?"
        ));
    }
    if aliases.is_empty() {
        return usage(format!(
            "unknown location alias `@{name}`{hint}; no `[locations]` aliases are configured"
        ));
    }
    let known: Vec<&str> = aliases.keys().take(8).map(String::as_str).collect();
    let more = if aliases.len() > known.len() {
        ", …"
    } else {
        ""
    };
    usage(format!(
        "unknown location alias `@{name}`{hint}; known aliases: {}{more}",
        known.join(", ")
    ))
}

/// The pair in a `@` name that parses as coordinates but lies outside the world, when it is one.
///
/// [`coordinates`] accepts a pair only inside the ranges, so `@91,0` falls through to the alias
/// lookup; this recognises it again, so the rejection can say which check it failed.
fn out_of_range_coordinates(name: &str) -> Option<(f64, f64)> {
    let mut parts = name.split(',').map(str::trim);
    let (Some(lat), Some(lon), None) = (parts.next(), parts.next(), parts.next()) else {
        return None;
    };
    let (Ok(lat), Ok(lon)) = (lat.parse::<f64>(), lon.parse::<f64>()) else {
        return None;
    };
    let inside = (-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon);
    (lat.is_finite() && lon.is_finite() && !inside).then_some((lat, lon))
}

/// The Levenshtein edit distance between two strings, for the alias suggestion list.
///
/// Two rolling rows, not a full matrix: only the distance is needed, and the strings are alias
/// names.
fn edit_distance(left: &str, right: &str) -> usize {
    let right: Vec<char> = right.chars().collect();
    let mut previous: Vec<usize> = (0..=right.len()).collect();
    let mut current: Vec<usize> = vec![0; right.len() + 1];
    for (row, left_char) in left.chars().enumerate() {
        current[0] = row + 1;
        for (column, right_char) in right.iter().enumerate() {
            let substitution = previous[column] + usize::from(*right_char != left_char);
            current[column + 1] = substitution
                .min(previous[column + 1] + 1)
                .min(current[column] + 1);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[right.len()]
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

/// A resolved location together with the ranked candidates it was chosen from.
///
/// `location` is `candidates[0]` whenever the list is non-empty, so the place a non-interactive
/// run takes and the `[1]` an interactive one offers can never disagree; the list is what the
/// candidate picker (step 20) prints and what `location search --all` tabulates.
#[derive(Debug, Clone, PartialEq)]
pub struct Resolved {
    /// The ranked winner.
    pub location: Location,
    /// Every ranked candidate, winner first; empty when no ranking happened (`@lat,lon`).
    pub candidates: Vec<Location>,
    /// How the winner was chosen.
    pub resolution: Resolution,
}

/// Picks one location out of the geocoder hits for `spec`, keeping the whole ranked list.
///
/// `LatLon` bypasses ranking and builds the coordinate location itself, because the geocoding
/// endpoint has no reverse lookup; its zone stays UTC and is provisional until a provider reports
/// the real one, and its candidate list is empty. Every other spec ranks `results` with the shared
/// [`rank`](crate::geo::rank::rank) order and fails with [`Error::LocationNotFound`] (exit code 5)
/// when nothing is left — for `Exact` that includes "hits came back, but none of them is named
/// exactly like the query".
pub fn resolve_candidates(
    results: Vec<Location>,
    spec: &LocationSpec,
    limit: u8,
) -> Result<Resolved> {
    if let LocationSpec::LatLon(lat, lon) = spec {
        return Ok(Resolved {
            location: from_coordinates(*lat, *lon),
            candidates: Vec::new(),
            resolution: Resolution::Coordinates,
        });
    }
    if matches!(spec, LocationSpec::Default | LocationSpec::Alias(_)) {
        return Err(Error::Config(format!(
            "location spec {spec} must be resolved through the configured/IP path before ranking"
        )));
    }

    let query = spec.query();
    let candidates: Vec<Location> = match spec {
        LocationSpec::Exact(_) => results
            .into_iter()
            .filter(|location| query.is_some_and(|query| same_name(location, query)))
            .collect(),
        _ => results,
    };

    let candidates = rank(candidates, query, limit);
    let Some(location) = candidates.first().cloned() else {
        return Err(Error::LocationNotFound(format!(
            "no location found for {spec}; check the spelling, or pass coordinates (`@lat,lon`) \
             or an OpenStreetMap search (`~name`) instead"
        )));
    };
    let resolution = match spec {
        LocationSpec::Exact(_) => Resolution::Exact,
        _ if candidates.len() == 1 => Resolution::Only,
        _ => Resolution::Fuzzy {
            candidates: candidates.len(),
        },
    };
    Ok(Resolved {
        location,
        candidates,
        resolution,
    })
}

/// [`resolve_candidates`]'s decision without the list, for callers that only need the winner.
pub fn resolve(
    results: Vec<Location>,
    spec: &LocationSpec,
    limit: u8,
) -> Result<(Location, Resolution)> {
    resolve_candidates(results, spec, limit)
        .map(|resolved| (resolved.location, resolved.resolution))
}

/// The one-line note a fuzzy match prints on stderr, or `None` when there was nothing ambiguous.
///
/// Reporting happens once, at the point where the location is chosen: the user sees which of the
/// candidates won, how to require an exact name instead, and how to choose interactively
/// (`--pick`) or accept the winner (`--yes`, step 20).
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
///
/// Both notes end with the picker advice, because both describe a choice the picker can take over
/// (step 20): `--pick` asks, `--yes` keeps the winner.
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
    let pick = "`--pick` to choose one, or `--yes` to keep the winner";
    let hint = if hint {
        format!(" — pass `:{query}` to require an exact name match, {pick}")
    } else {
        format!(" — {pick}")
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
        LocationSource::Offline | LocationSource::Geonames => {
            Some("Location data by GeoNames (CC BY 4.0) — https://www.geonames.org/")
        }
        LocationSource::Osm => Some("Location data © OpenStreetMap contributors (ODbL)"),
        // A station's coordinates are US-government public-domain metadata, which asks for no
        // credit line; the *weather* credit (`aviationweather.gov`) travels in the report's
        // attribution instead.
        LocationSource::Coordinates | LocationSource::Ip | LocationSource::Station => None,
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

/// Whether a country code is the two ASCII letters every consumer assumes.
///
/// `auto` provider selection and alert coverage read `country_code`, so a value that is not an ISO
/// 3166-1 alpha-2 code (`GeoNames` reports `-99` for the shapes without one, and a three-letter
/// code is an IOC or `GeoNames`-internal spelling) could only mislead. Shared by the `GeoNames`
/// decoder and the candidate merge (step 25).
pub(crate) fn is_country_code(code: &str) -> bool {
    code.len() == 2 && code.bytes().all(|byte| byte.is_ascii_alphabetic())
}

/// The great-circle distance between two coordinates, in kilometres (haversine).
///
/// The location layer keeps its own copy instead of reaching into the provider side
/// (`metar::station_table::distance_km`, `open_meteo_marine`'s private twin): `geo` never depends
/// on `provider`. Used by the candidate merge's "the same place within 5 km" rule (step 25).
pub(crate) fn distance_km(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    const EARTH_RADIUS_KM: f64 = 6_371.008_8;
    let (lat1, lat2) = (lat1.to_radians(), lat2.to_radians());
    let delta_lat = lat2 - lat1;
    let delta_lon = (lon2 - lon1).to_radians();
    let a =
        (delta_lat / 2.0).sin().powi(2) + lat1.cos() * lat2.cos() * (delta_lon / 2.0).sin().powi(2);
    2.0 * EARTH_RADIUS_KM * a.sqrt().asin()
}

/// The error a name lookup the bundled table could not answer reports (step 18).
///
/// Shared with [`offline`](crate::geo::offline) and the CLI so the `(no offline match)` marker is
/// written once and scripts can rely on it.
pub(crate) fn offline_not_found(query: &str) -> Error {
    Error::LocationNotFound(format!(
        "no location found for `{query}` (no offline match)"
    ))
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

/// Recognises the text after `@` as a coordinate pair, when it is one.
///
/// The recogniser is deliberately narrow: exactly one comma, both sides parse as finite `f64`,
/// latitude in `[-90, 90]` and longitude in `[-180, 180]`. Anything else — `@39.9`, `@91,0`,
/// `@a,116` — is not a coordinate pair, and [`LocationSpec::parse_arg`] treats it as an alias name
/// instead. That order is what keeps `@39.9,116.4` from ever being looked up in `[locations]`.
fn coordinates(text: &str) -> Option<(f64, f64)> {
    let mut parts = text.split(',').map(str::trim);
    let (Some(lat), Some(lon), None) = (parts.next(), parts.next(), parts.next()) else {
        return None;
    };
    let (Ok(lat), Ok(lon)) = (lat.parse::<f64>(), lon.parse::<f64>()) else {
        return None;
    };
    if !lat.is_finite() || !lon.is_finite() {
        return None;
    }
    if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) {
        return None;
    }
    Some((lat, lon))
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

    use super::{LocationSpec, Resolution, ambiguity_note, location_line, resolve};
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
            "note: 3 candidates for `Beijing`; using Beijing, Beijing Municipality, China (population 18960744) — pass `:Beijing` to require an exact name match, `--pick` to choose one, or `--yes` to keep the winner"
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
            "note: 2 candidates for `Beijing`; using Beijing, Beijing Municipality, China — `--pick` to choose one, or `--yes` to keep the winner"
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
    }

    #[test]
    fn attribution_follows_the_data_source() {
        let mut location = candidate("Beijing", None, None);
        assert!(
            super::attribution_line(&location)
                .expect("a geocoded place credits GeoNames and Open-Meteo")
                .contains("GeoNames")
        );
        location.source = LocationSource::Offline;
        assert_eq!(
            super::attribution_line(&location),
            Some("Location data by GeoNames (CC BY 4.0) — https://www.geonames.org/")
        );
        location.source = LocationSource::Osm;
        assert_eq!(
            super::attribution_line(&location),
            Some("Location data © OpenStreetMap contributors (ODbL)")
        );
        for source in [LocationSource::Coordinates, LocationSource::Ip] {
            location.source = source;
            assert!(
                super::attribution_line(&location).is_none(),
                "{source:?} needs no attribution"
            );
        }
    }

    /// The `[locations]` table a CLI run would have parsed.
    fn aliases(pairs: &[(&str, &str)]) -> std::collections::BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect()
    }

    #[test]
    fn aliases_expand_through_chains() {
        let table = aliases(&[
            ("home", "@39.9,116.4"),
            ("work", "@home"),
            ("city", "Beijing"),
            ("exact", ":Beijing"),
            ("cosy", "~Tsinghua"),
        ]);
        let expand = |name: &str| {
            super::expand_aliases(LocationSpec::Alias(name.to_owned()), &table)
                .unwrap_or_else(|error| panic!("@{name}: {error}"))
        };
        assert_eq!(expand("home"), LocationSpec::LatLon(39.9, 116.4));
        assert_eq!(expand("work"), LocationSpec::LatLon(39.9, 116.4));
        assert_eq!(expand("city"), LocationSpec::Fuzzy("Beijing".to_owned()));
        assert_eq!(expand("exact"), LocationSpec::Exact("Beijing".to_owned()));
        assert_eq!(expand("cosy"), LocationSpec::Osm("Tsinghua".to_owned()));
        // A spec that is not an alias is returned unchanged.
        assert_eq!(
            super::expand_aliases(LocationSpec::Fuzzy("Beijing".to_owned()), &table)
                .expect("not an alias"),
            LocationSpec::Fuzzy("Beijing".to_owned())
        );
    }

    #[test]
    fn an_alias_cycle_names_the_chain() {
        let table = aliases(&[("home", "@work"), ("work", "@home")]);
        let error = super::expand_aliases(LocationSpec::Alias("home".to_owned()), &table)
            .expect_err("a cycle never resolves");
        assert_eq!(error.exit_code(), 4);
        assert!(
            error
                .to_string()
                .contains("location alias cycle: @home -> @work -> @home"),
            "{error}"
        );
    }

    #[test]
    fn an_overlong_alias_chain_is_refused() {
        let mut table = std::collections::BTreeMap::new();
        for index in 0..12 {
            table.insert(format!("a{index}"), format!("@a{}", index + 1));
        }
        table.insert("a12".to_owned(), "Beijing".to_owned());
        let error = super::expand_aliases(LocationSpec::Alias("a0".to_owned()), &table)
            .expect_err("the chain is deeper than the cap");
        assert_eq!(error.exit_code(), 4);
        assert!(error.to_string().contains("deeper than 8"), "{error}");
        // A legal chain just under the cap still works.
        let mut legal = std::collections::BTreeMap::new();
        for index in 0..7 {
            legal.insert(format!("b{index}"), format!("@b{}", index + 1));
        }
        legal.insert("b7".to_owned(), "Beijing".to_owned());
        assert_eq!(
            super::expand_aliases(LocationSpec::Alias("b0".to_owned()), &legal)
                .expect("eight links resolve"),
            LocationSpec::Fuzzy("Beijing".to_owned())
        );
    }

    #[test]
    fn an_unknown_alias_suggests_close_names() {
        let table = aliases(&[("home", "@39.9,116.4"), ("work", ":Shanghai")]);
        let error = super::expand_aliases(LocationSpec::Alias("hom".to_owned()), &table)
            .expect_err("never an alias");
        assert_eq!(error.exit_code(), 2);
        let message = error.to_string();
        assert!(message.contains("did you mean @home"), "{message}");
        assert!(message.contains(super::USAGE_FORMS), "{message}");

        // A name nothing is close to lists the configured names instead.
        let error = super::expand_aliases(LocationSpec::Alias("zzz".to_owned()), &table)
            .expect_err("never an alias");
        assert!(
            error.to_string().contains("known aliases: home, work"),
            "{error}"
        );

        // No table at all is its own message.
        let error = super::expand_aliases(
            LocationSpec::Alias("home".to_owned()),
            &std::collections::BTreeMap::new(),
        )
        .expect_err("nothing configured");
        assert!(
            error.to_string().contains("no `[locations]` aliases"),
            "{error}"
        );
    }

    #[test]
    fn an_out_of_range_coordinate_spelling_is_explained() {
        // `@91,0` is not a coordinate pair, so it falls through to the alias lookup; the
        // rejection says which range check the pair failed.
        let spec = LocationSpec::parse_arg(Some("@91.0,0")).expect("an alias name");
        let error = super::expand_aliases(spec, &std::collections::BTreeMap::new())
            .expect_err("nothing configured");
        let message = error.to_string();
        assert!(message.contains("not inside the world"), "{message}");

        // A name that is not a pair at all keeps the plain message.
        let error = super::expand_aliases(
            LocationSpec::Alias("home".to_owned()),
            &std::collections::BTreeMap::new(),
        )
        .expect_err("nothing configured");
        assert!(!error.to_string().contains("inside the world"), "{error}");
    }

    #[test]
    fn edit_distance_is_symmetric_and_bounded() {
        assert_eq!(super::edit_distance("home", "home"), 0);
        assert_eq!(super::edit_distance("home", "hom"), 1);
        assert_eq!(super::edit_distance("hom", "home"), 1);
        assert_eq!(super::edit_distance("home", "work"), 3);
        assert_eq!(super::edit_distance("", "abc"), 3);
    }

    /// The one country-code rule the `GeoNames` decoder and the merge share.
    #[test]
    fn a_country_code_is_two_ascii_letters() {
        for good in ["CN", "us", "XK"] {
            assert!(super::is_country_code(good), "{good}");
        }
        for bad in ["", "-99", "C", "CHN", "中国", "C1"] {
            assert!(!super::is_country_code(bad), "{bad}");
        }
    }

    /// The haversine the merge's 5 km rule uses: one degree of latitude is ~111 km.
    #[test]
    fn distance_km_matches_the_degree_scale() {
        let one_degree = super::distance_km(0.0, 0.0, 1.0, 0.0);
        assert!((one_degree - 111.2).abs() < 0.5, "{one_degree}");
        assert_eq!(super::distance_km(39.9, 116.4, 39.9, 116.4), 0.0);
        // Symmetric, and the same at the poles as at the equator for a pure longitude step.
        assert!(
            (super::distance_km(1.0, 2.0, 3.0, 4.0) - super::distance_km(3.0, 4.0, 1.0, 2.0)).abs()
                < 1e-9
        );
    }
}
