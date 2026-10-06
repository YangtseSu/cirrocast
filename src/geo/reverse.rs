// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Naming a coordinate: the bundled tables first, Nominatim `/reverse` only when they find nothing
//! (step 25).
//!
//! `@39.9042,116.4074` is a place the user knows and the request key this program uses, but it is
//! not a *name* — a report headed `39.9042, 116.4074` tells nobody anything. This module turns such
//! a coordinate into "Xianghe, China (12 km away)" without changing what is fetched:
//!
//! * the bundled city table (step 18) is scanned for rows within [`RADIUS_KM`], nearest first, and
//!   the country layer (step 25) supplies the country *name* the city table does not carry;
//! * when nothing is that close and the run may open a socket, Nominatim `/reverse` names the
//!   point — the same donated service as `~query`, so the same `User-Agent`, the same one
//!   request/second throttle and the same cached namespace;
//! * `[geo] reverse` (`CIRROCAST_GEO_REVERSE`) picks between `auto`, `offline` (never a socket) and
//!   `off`, and `--offline`/`--offline=geo` forces `offline` for the run.
//!
//! Two rules are worth restating because they are easy to undo by accident:
//!
//! * **a name is a display attribute.** The caller keeps the coordinate's own `lat`/`lon`, its
//!   [`LocationSource::Coordinates`](crate::model::LocationSource::Coordinates) and its
//!   provisional zone; only the display fields are filled in, and
//!   [`Location::named_by`](crate::model::Location::named_by) records who named it so the credit
//!   travels with the name;
//! * **naming never fails the query.** The service that names a point is optional and often
//!   unreachable, so a network or upstream failure of the *online* half becomes "no name" plus a
//!   note a `-v` run prints — the coordinate is still a perfectly good location without it. Only a
//!   configuration mistake (an unknown `[geo] reverse` value) stops the run.
//!
//! The 25 km radius is breezy-weather's `REVERSE_GEOCODING_DISTANCE_LIMIT` (the audit of
//! 2026-10-03), kept here so a later tuning is a deliberate change.

use std::time::Duration;

use crate::cache::Cache;
use crate::error::{Error, Result};
use crate::geo::nominatim::Nominatim;
use crate::http::HttpClient;
use crate::model::Location;
use crate::paths::Paths;

/// How close a bundled city must be to name a coordinate.
pub const RADIUS_KM: f64 = 25.0;

/// The values `[geo] reverse` accepts.
pub const REVERSE_SETTINGS: &[&str] = &["auto", "offline", "off"];

/// How a coordinate may be named.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Policy {
    /// The bundled tables first, Nominatim `/reverse` when they find nothing.
    Auto,
    /// The bundled tables only, never a socket.
    Offline,
    /// Do not name coordinates at all.
    Off,
}

impl Policy {
    /// The policy `setting` selects.
    ///
    /// The value comes from the configuration (or its environment override), so an unknown one is
    /// [`Error::Config`]: a typo must stop the run rather than silently pick a policy.
    pub fn parse(setting: &str) -> Result<Self> {
        match setting.trim() {
            "auto" => Ok(Self::Auto),
            "offline" => Ok(Self::Offline),
            "off" => Ok(Self::Off),
            other => Err(Error::Config(format!(
                "geo.reverse: `{other}` is not one of {}",
                REVERSE_SETTINGS.join(", ")
            ))),
        }
    }
}

/// One candidate name: the place and how far it is from the coordinate.
#[derive(Debug, Clone, PartialEq)]
pub struct Nearby {
    /// The place, with its own name, country and zone.
    pub location: Location,
    /// The distance from the coordinate to the place, in kilometres.
    pub distance_km: f64,
}

/// The inputs one naming run needs.
pub struct Inputs<'a> {
    /// What `[geo] reverse` asks for.
    pub policy: Policy,
    /// The XDG directories, for the bundled or user-installed city table.
    pub paths: &'a Paths,
    /// `[geo] data`: which city table answers.
    pub data: &'a str,
    /// The shared HTTP client.
    pub http: &'a HttpClient,
    /// The cache view of the geo scope.
    pub cache: &'a Cache,
    /// The Nominatim base URL (`network.nominatim_url`, empty = the public service).
    pub nominatim_url: &'a str,
    /// How long an online naming answer is reused (`cache.geocode_ttl_secs`).
    pub ttl: Duration,
    /// How many bundled candidates to keep.
    pub limit: u8,
    /// Whether the geo scope is silenced: the bundled tables only, whatever the policy says.
    pub offline: bool,
    /// Whether the "user table is unusable" warning is silenced (`-q`).
    pub quiet: bool,
}

/// What named a coordinate.
#[derive(Debug, Default)]
pub struct Named {
    /// The candidates, nearest first; empty when nothing named the point.
    pub nearby: Vec<Nearby>,
    /// Why there is no name, for a `-v` run; `None` when a name was found.
    pub note: Option<String>,
}

/// Names `(lat, lon)` according to the policy.
pub fn name(lat: f64, lon: f64, inputs: &Inputs<'_>) -> Result<Named> {
    if inputs.policy == Policy::Off {
        return Ok(Named::default());
    }
    let nearby = offline(lat, lon, inputs)?;
    if !nearby.is_empty() {
        return Ok(Named { nearby, note: None });
    }
    if inputs.policy == Policy::Offline || inputs.offline {
        return Ok(Named {
            nearby,
            note: Some(no_offline_name(lat, lon)),
        });
    }
    match online(lat, lon, inputs) {
        Ok(Some(nearby)) => Ok(Named {
            nearby: vec![nearby],
            note: None,
        }),
        Ok(None) => Ok(Named {
            nearby: Vec::new(),
            note: Some(format!("nominatim knows no place at ({lat}, {lon})")),
        }),
        // A name is a display attribute: the donated service being unreachable must not fail a
        // coordinate query, it must leave the coordinate unnamed with a note.
        Err(error) if is_skippable(&error) => Ok(Named {
            nearby: Vec::new(),
            note: Some(format!("nominatim: {}", error.chain_reason())),
        }),
        Err(error) => Err(error),
    }
}

/// The bundled-table half: the nearest cities within [`RADIUS_KM`], country name included.
fn offline(lat: f64, lon: f64, inputs: &Inputs<'_>) -> Result<Vec<Nearby>> {
    #[cfg(not(feature = "offline-geo"))]
    {
        let _ = (lat, lon, inputs);
        Ok(Vec::new())
    }
    #[cfg(feature = "offline-geo")]
    {
        let table =
            crate::geo::offline::OfflineTable::open(inputs.paths, inputs.data, inputs.quiet)?;
        from_table(&table, lat, lon, inputs.limit)
    }
}

/// The bundled-table half, given an already-open table.
///
/// Split out so a test can pin the naming against the committed table without resolving XDG
/// directories, and so the caller's table choice (`[geo] data`) stays in one place.
#[cfg(feature = "offline-geo")]
pub fn from_table(
    table: &crate::geo::offline::OfflineTable,
    lat: f64,
    lon: f64,
    limit: u8,
) -> Result<Vec<Nearby>> {
    let cities = table.nearby(lat, lon, RADIUS_KM, limit)?;
    if cities.is_empty() {
        return Ok(Vec::new());
    }
    // The city table carries the ISO code only, so the country *name* comes from the layer; a
    // coordinate outside every shape (or in a build without the layer) keeps the code as its
    // country, exactly as a name-resolved offline location does today.
    let country = crate::geo::country::lookup(lat, lon)?;
    Ok(cities
        .into_iter()
        .map(|city| {
            let distance_km = crate::geo::distance_km(lat, lon, city.lat, city.lon);
            let mut location = city.location();
            if let Some(country) = &country {
                if !country.name.is_empty() {
                    location.country.clone_from(&country.name);
                }
                if location.country_code.is_none() && !country.code.is_empty() {
                    location.country_code = Some(country.code.clone());
                }
            }
            Nearby {
                location,
                distance_km,
            }
        })
        .collect())
}

/// The online half: Nominatim `/reverse`, one hit or none.
fn online(lat: f64, lon: f64, inputs: &Inputs<'_>) -> Result<Option<Nearby>> {
    let nominatim = Nominatim::new(inputs.http, inputs.cache, inputs.nominatim_url.to_owned());
    let Some(location) = nominatim.reverse(lat, lon)? else {
        return Ok(None);
    };
    let distance_km = crate::geo::distance_km(lat, lon, location.lat, location.lon);
    Ok(Some(Nearby {
        location,
        distance_km,
    }))
}

/// The note a coordinate with no bundled name gets, naming the reason it had none.
fn no_offline_name(lat: f64, lon: f64) -> String {
    if cfg!(feature = "offline-geo") {
        format!("no city within {RADIUS_KM} km of ({lat}, {lon})")
    } else {
        format!("no bundled city table in this build, so ({lat}, {lon}) cannot be named offline")
    }
}

/// Whether a failure is one that leaves the coordinate unnamed rather than failing the query.
fn is_skippable(error: &Error) -> bool {
    matches!(error, Error::Network(_) | Error::Upstream { .. })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::SystemTime;

    #[cfg(feature = "offline-geo")]
    use super::from_table;
    use super::{Inputs, Policy, RADIUS_KM, REVERSE_SETTINGS, name};
    use crate::cache::{Cache, CacheMode, FakeClock};
    #[cfg(feature = "offline-geo")]
    use crate::geo::offline::OfflineTable;
    use crate::http::{HttpClient, StubTransport};
    #[cfg(feature = "offline-geo")]
    use crate::model::LocationSource;
    use crate::paths::Paths;

    /// A `Paths` pointing at a temporary root, for the code paths that only open the bundled table.
    fn paths(root: &std::path::Path) -> Paths {
        Paths {
            config_dir: root.join("config"),
            config_file: root.join("config/config.toml"),
            keys_file: root.join("config/keys.toml"),
            cache_dir: root.join("cache"),
            data_dir: root.join("data"),
        }
    }

    #[test]
    fn the_policy_parses_its_documented_values() {
        assert_eq!(Policy::parse("auto").expect("auto"), Policy::Auto);
        assert_eq!(
            Policy::parse(" offline ").expect("offline"),
            Policy::Offline
        );
        assert_eq!(Policy::parse("off").expect("off"), Policy::Off);
        for junk in ["", "on", "Offline", "auto,offline"] {
            let error = Policy::parse(junk).expect_err("junk is refused");
            assert_eq!(error.exit_code(), 4, "{junk}");
            assert!(error.to_string().contains("geo.reverse"), "{junk}: {error}");
        }
        assert_eq!(REVERSE_SETTINGS.len(), 3);
    }

    /// `off` never asks anyone: no table, no socket.
    #[test]
    fn the_off_policy_names_nothing() {
        let root = tempfile::tempdir().expect("a temporary root");
        let clock = Arc::new(FakeClock::new(SystemTime::UNIX_EPOCH));
        let transport = Arc::new(StubTransport::new(Vec::new()));
        let cache = Cache::with_root(root.path(), CacheMode::Normal, clock.clone(), 0);
        let client = HttpClient::new(Box::new(Arc::clone(&transport)), 3, clock, 0);
        let paths = paths(root.path());
        let inputs = Inputs {
            policy: Policy::Off,
            paths: &paths,
            data: "bundled",
            http: &client,
            cache: &cache,
            nominatim_url: "",
            ttl: std::time::Duration::from_hours(720),
            limit: 5,
            offline: false,
            quiet: true,
        };

        let named = name(39.9042, 116.4074, &inputs).expect("`off` is not a failure");
        assert_eq!(named.nearby, [] as [super::Nearby; 0]);
        assert!(named.note.is_none(), "{:?}", named.note);
        assert_eq!(transport.calls().len(), 0);
    }

    /// Offline, an unreachable Nominatim is never contacted and the note says why.
    #[test]
    fn offline_never_reaches_the_service() {
        let root = tempfile::tempdir().expect("a temporary root");
        let clock = Arc::new(FakeClock::new(SystemTime::UNIX_EPOCH));
        let transport = Arc::new(StubTransport::new(Vec::new()));
        let cache = Cache::with_root(root.path(), CacheMode::Offline, clock.clone(), 0);
        let client = HttpClient::new(Box::new(Arc::clone(&transport)), 3, clock, 0);
        let paths = paths(root.path());
        let inputs = Inputs {
            policy: Policy::Auto,
            paths: &paths,
            data: "bundled",
            http: &client,
            cache: &cache,
            nominatim_url: "https://nominatim.example.org",
            ttl: std::time::Duration::from_hours(720),
            limit: 5,
            offline: true,
            quiet: true,
        };

        // The mid-Pacific: nothing in the bundled table, and no request may go out.
        let named = name(0.0, -140.0, &inputs).expect("a nameless coordinate is not a failure");
        assert_eq!(named.nearby, [] as [super::Nearby; 0]);
        assert_eq!(transport.calls().len(), 0);
        assert!(
            named
                .note
                .as_deref()
                .is_some_and(|note| note.contains("no city within")),
            "{:?}",
            named.note
        );
    }

    /// The bundled table names Beijing's coordinates with a place inside the radius.
    #[cfg(feature = "offline-geo")]
    #[test]
    fn the_bundled_table_names_a_coordinate() {
        let nearby = from_table(&OfflineTable::bundled(), 39.9042, 116.4074, 5)
            .expect("the committed table decodes");
        assert!(!nearby.is_empty(), "Beijing has neighbours within 25 km");
        let first = &nearby[0];
        assert!(first.distance_km <= RADIUS_KM, "{first:?}");
        assert_eq!(first.location.source, LocationSource::Offline);
        assert_ne!(first.location.name, "");
        // The country layer supplied the name the city table cannot carry.
        assert_eq!(first.location.country, "China");
        assert_eq!(first.location.country_code.as_deref(), Some("CN"));
        // Nearest first.
        assert!(
            nearby
                .windows(2)
                .all(|pair| pair[0].distance_km <= pair[1].distance_km),
            "{nearby:?}"
        );
    }

    /// A point far from every city has no bundled name at all.
    #[cfg(feature = "offline-geo")]
    #[test]
    fn the_open_ocean_has_no_bundled_name() {
        let nearby = from_table(&OfflineTable::bundled(), 0.0, -140.0, 5)
            .expect("the committed table decodes");
        assert!(nearby.is_empty(), "{nearby:?}");
    }
}
