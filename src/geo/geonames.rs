// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The `GeoNames` `searchJSON` geocoder: the BYOK fuzzy search the bundled table cannot do.
//!
//! The bundled city table (step 18) answers exact and prefix matches over folded spellings; it
//! cannot rank a misspelt or alternate-spelling query the way `GeoNames`' own `fuzzy` index does, and
//! it only carries the `cities15000` snapshot. This client is the second source behind a name
//! query: `q`, `fuzzy=0.8`, `maxRows=<limit>`, `style=FULL` and the account name, answered from
//! `secure.geonames.org` and cached like every other geocoding answer.
//!
//! Three rules are worth restating because they are easy to undo by accident:
//!
//! * the account name is a credential, not a query parameter to print: it is sent as `username`
//!   and marked [`HttpRequest::secret`], so no log line, error message or cache envelope can
//!   contain it;
//! * a `401` is the service refusing the *account name* — the measured no-username answer and the
//!   measured "invalid user" answer are the same status and the same `value: 10` — so it becomes
//!   [`Error::MissingKey`] (exit 6) naming `CIRROCAST_GEONAMES_USER` and `cirrocast key set
//!   geonames`, not the upstream's own wording. A `200` whose body carries a `status` object is
//!   the quota refusal (values 18/19/20 are the documented limits) and is [`Error::Upstream`]
//!   quoting the service's message, never a silently empty result;
//! * a row that cannot become a usable [`Location`] — the `0, 0` sentinel, a missing or
//!   non-ISO country code, an unusable IANA zone — is dropped instead of failing the query or
//!   being guessed at. This source is one of several a name query merges, so one odd row must not
//!   take the whole answer down with it, and a candidate without a zone would move every day part
//!   of the forecast (step 06 buckets hours by the location's local clock).
//!
//! The account is free to register and needs no card; without one the chain simply skips this
//! source under `[geo] search = "auto"` (step 25).

use std::str::FromStr;
use std::time::Duration;

use chrono_tz::Tz;
use serde::Deserialize;

use crate::cache::{Cache, CacheKey};
use crate::error::{Error, Result};
use crate::geo::Geocoder;
use crate::http::{HttpClient, HttpRequest};
use crate::model::{Location, LocationSource};

/// The search endpoint.
///
/// `secure.` is the host with a certificate that matches; the older `api.geonames.org` name does
/// not (measured 2026-10-06: `SSL: no alternative certificate subject name matches target
/// hostname`), so it is not used.
pub const SEARCH_URL: &str = "https://secure.geonames.org/searchJSON";

/// The credential name in the key store, and the `provider` of the errors this module reports.
pub const CREDENTIAL: &str = "geonames";

/// The environment variable that can supply the account name instead of `keys.toml`.
pub const ENV_VAR: &str = "CIRROCAST_GEONAMES_USER";

/// The fuzzy threshold the request asks for: `GeoNames`' own scale, where 0 is an exact match and 1
/// matches everything.
const FUZZY: &str = "0.8";

/// A geocoder over the shared HTTP client and cache, authenticated with one account name.
pub struct GeoNamesGeocoder<'a> {
    http: &'a HttpClient,
    cache: &'a Cache,
    ttl: Duration,
    username: String,
}

impl<'a> GeoNamesGeocoder<'a> {
    /// Binds the geocoder to the client, the cache and the account name, caching answers for
    /// `ttl` (the caller passes `cache.geocode_ttl_secs`, as the other geocoders do).
    #[must_use]
    pub fn new(
        http: &'a HttpClient,
        cache: &'a Cache,
        ttl: Duration,
        username: impl Into<String>,
    ) -> Self {
        Self {
            http,
            cache,
            ttl,
            username: username.into(),
        }
    }
}

impl Geocoder for GeoNamesGeocoder<'_> {
    fn search(&self, query: &str, limit: u8) -> Result<Vec<Location>> {
        let key = cache_key(query, limit);
        let response: Response = self.cache.read_or_fetch_json(
            &key,
            self.ttl,
            CREDENTIAL,
            "search answer",
            &format!("the query `{query}`"),
            || {
                let request = HttpRequest::get(SEARCH_URL)
                    .query("q", query)
                    .query("fuzzy", FUZZY)
                    .query("maxRows", limit.to_string())
                    .query("style", "FULL")
                    .query("username", self.username.clone())
                    .secret(self.username.clone());
                match self.http.send(&request) {
                    Ok(response) => Ok((response.status(), response.body().to_owned())),
                    Err(error) => Err(missing_credential(error)),
                }
            },
        )?;
        response.into_locations()
    }
}

/// The `401` the service answers for an account name it will not accept, translated to the
/// missing-key failure; every other failure is passed through untouched.
///
/// The two measured spellings — no `username` at all and an unregistered one — are both `401` with
/// `{"status":{"value":10}}`, and both are fixed by the same two commands, so the user is told
/// those instead of the service's own sentence.
fn missing_credential(error: Error) -> Error {
    if matches!(
        error,
        Error::Upstream {
            status: Some(401),
            ..
        }
    ) {
        return Error::MissingKey {
            provider: CREDENTIAL.to_owned(),
            env: ENV_VAR.to_owned(),
        };
    }
    error
}

/// The cache key of one lookup: the trimmed, lower-cased query, the limit and the source.
///
/// The account name is deliberately *not* part of the key: the cache root belongs to the one user
/// whose account it is, and the answer for a query does not depend on which account asked.
fn cache_key(query: &str, limit: u8) -> CacheKey {
    CacheKey::hash(
        "geocode",
        &format!("geonames|{}|{limit}", query.trim().to_lowercase()),
    )
}

/// The response envelope. `geonames` is absent when nothing matched, which is not an error.
#[derive(Debug, Deserialize)]
struct Response {
    /// The matching places, in the order the service ranked them.
    #[serde(default)]
    geonames: Option<Vec<Hit>>,
    /// The service's refusal object, which a `200` carries for a quota problem.
    #[serde(default)]
    status: Option<Status>,
}

impl Response {
    /// Translates every usable hit; rows that cannot become a location are dropped.
    fn into_locations(self) -> Result<Vec<Location>> {
        if let Some(status) = self.status {
            return Err(status.into_error());
        }
        Ok(self
            .geonames
            .unwrap_or_default()
            .into_iter()
            .filter_map(Hit::into_location)
            .collect())
    }
}

/// The refusal object of a `200` answer: a quota (values 18, 19 and 20 are the documented limits)
/// or an account problem that was not a `401`.
#[derive(Debug, Deserialize)]
struct Status {
    /// The numeric code, e.g. `18`.
    value: Option<u64>,
    /// The service's own sentence, e.g. `the daily limit of 20000 credits for … has been exceeded`.
    message: Option<String>,
}

impl Status {
    /// The upstream failure this refusal is, quoting the service.
    fn into_error(self) -> Error {
        let detail = self.message.as_deref().map_or_else(
            || "no message given".to_owned(),
            |message| {
                let message = message.trim();
                if message.is_empty() {
                    "no message given".to_owned()
                } else {
                    message.to_owned()
                }
            },
        );
        let value = self
            .value
            .map(|value| format!(" (status {value})"))
            .unwrap_or_default();
        Error::Upstream {
            provider: CREDENTIAL.to_owned(),
            status: Some(200),
            message: format!("{detail}{value}"),
        }
    }
}

/// One place, as `style=FULL` reports it. Unknown fields are ignored on purpose: the payload also
/// carries `toponymName`, `fcl`, `bbox`, `alternateNames`, … that no caller wants.
///
/// The coordinates are *strings* in this API (`"39.9075"`), unlike every other geocoder here, which
/// is why they are read as text and parsed.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Hit {
    /// Display name, e.g. `Beijing`.
    #[serde(default)]
    name: Option<String>,
    /// Latitude in degrees, WGS 84, as text.
    #[serde(default)]
    lat: Option<String>,
    /// Longitude in degrees, WGS 84, as text.
    #[serde(default)]
    lng: Option<String>,
    /// ISO 3166-1 alpha 2 country code, e.g. `CN`.
    #[serde(default)]
    country_code: Option<String>,
    /// Country name, e.g. `China`.
    #[serde(default)]
    country_name: Option<String>,
    /// First level administrative division, e.g. `Beijing`.
    #[serde(default)]
    admin_name1: Option<String>,
    /// Population, used only to rank ambiguous matches; never rendered.
    #[serde(default)]
    population: Option<u64>,
    /// The zone object; only its IANA name is used.
    #[serde(default)]
    timezone: Option<Timezone>,
}

impl Hit {
    /// The canonical [`Location`] for this hit, or `None` when the row cannot be one.
    fn into_location(self) -> Option<Location> {
        let name = text(self.name.as_deref())?;
        let (lat, lon) = (
            coordinate(self.lat.as_deref())?,
            coordinate(self.lng.as_deref())?,
        );
        if !crate::geo::nominatim::usable_coordinates(lat, lon) || (lat == 0.0 && lon == 0.0) {
            return None;
        }
        let country_code =
            text(self.country_code.as_deref()).filter(|code| crate::geo::is_country_code(code))?;
        Some(Location {
            name,
            admin1: text(self.admin_name1.as_deref()),
            country: text(self.country_name.as_deref()).unwrap_or_default(),
            country_code: Some(country_code),
            lat,
            lon,
            tz: timezone(self.timezone.as_ref()?)?,
            elevation_m: None,
            population: self.population.filter(|population| *population > 0),
            source: LocationSource::Geonames,
            station: None,
            named_by: None,
        })
    }
}

/// The `timezone` object of a `style=FULL` hit.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Timezone {
    /// IANA zone name, e.g. `Asia/Shanghai`.
    #[serde(default)]
    time_zone_id: Option<String>,
}

/// A field's text, or `None` when the answer omits it, sends `null` or sends blanks.
fn text(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

/// One of the API's text coordinates, parsed; `None` when it is missing or not a number.
fn coordinate(raw: Option<&str>) -> Option<f64> {
    raw.and_then(|text| text.trim().parse::<f64>().ok())
        .filter(|value| value.is_finite())
}

/// The hit's IANA zone, when it has a usable one.
///
/// Deliberately no `UTC` fallback, exactly as in the Open-Meteo geocoder: a geocoded name was
/// resolved so that its local time is known, and a row whose zone is missing or unknown is dropped
/// rather than resolved to the wrong day parts.
fn timezone(timezone: &Timezone) -> Option<Tz> {
    let name = text(timezone.time_zone_id.as_deref())?;
    Tz::from_str(&name).ok()
}

#[cfg(test)]
mod tests {
    use super::{Response, cache_key};

    /// The recorded quota refusal is an error naming the service and quoting its message.
    #[test]
    fn a_quota_body_is_an_upstream_error_not_an_empty_result() {
        let body = r#"{"status":{"message":"the daily limit of 20000 credits for demo has been exceeded.","value":18}}"#;
        let response: Response = serde_json::from_str(body).expect("the envelope parses");
        let error = response
            .into_locations()
            .expect_err("a quota body is a failure");
        let text = error.to_string();
        assert!(text.contains("geonames"), "{text}");
        assert!(text.contains("daily limit"), "{text}");
        assert!(text.contains("status 18"), "{text}");
        assert_eq!(error.exit_code(), 3);
    }

    /// A missing `geonames` key is no hits, not a failure.
    #[test]
    fn a_missing_results_array_is_no_hits() {
        let response: Response =
            serde_json::from_str(r#"{"totalResultsCount":0}"#).expect("the envelope parses");
        assert_eq!(
            response.into_locations().expect("no hits is not a failure"),
            Vec::<super::Location>::new()
        );
    }

    /// The rows that cannot become a location are dropped, and the usable ones survive in order.
    #[test]
    fn unusable_rows_are_dropped_without_failing_the_query() {
        let body = r#"{"geonames":[
            {"name":"Beijing","lat":"39.9075","lng":"116.39723","countryCode":"CN","countryName":"China",
             "adminName1":"Beijing","population":18960744,"geonameId":1816670,
             "timezone":{"timeZoneId":"Asia/Shanghai"}},
            {"name":"Null Island","lat":"0","lng":"0","countryCode":"CN","countryName":"China",
             "timezone":{"timeZoneId":"UTC"}},
            {"name":"Nowhere","lat":"1.0","lng":"2.0","countryName":"Nowhere",
             "timezone":{"timeZoneId":"UTC"}},
            {"name":"Bad Code","lat":"1.0","lng":"2.0","countryCode":"-99","countryName":"Nowhere",
             "timezone":{"timeZoneId":"UTC"}},
            {"name":"No Zone","lat":"1.0","lng":"2.0","countryCode":"CN","countryName":"China"},
            {"name":"Bad Zone","lat":"1.0","lng":"2.0","countryCode":"CN","countryName":"China",
             "timezone":{"timeZoneId":"Mars/Olympus"}},
            {"name":"Not A Number","lat":"east","lng":"2.0","countryCode":"CN","countryName":"China",
             "timezone":{"timeZoneId":"UTC"}}
        ]}"#;
        let response: Response = serde_json::from_str(body).expect("the envelope parses");
        let locations = response.into_locations().expect("the usable row survives");
        assert_eq!(locations.len(), 1, "{locations:?}");
        assert_eq!(locations[0].name, "Beijing");
        assert_eq!(locations[0].admin1.as_deref(), Some("Beijing"));
        assert_eq!(locations[0].country, "China");
        assert_eq!(locations[0].country_code.as_deref(), Some("CN"));
        assert_eq!(locations[0].population, Some(18_960_744));
        assert_eq!(locations[0].source, crate::model::LocationSource::Geonames);
    }

    /// Two spellings of one query are one cache entry; another limit is another entry.
    #[test]
    fn the_cache_key_normalises_the_query() {
        assert_eq!(
            cache_key("Beijing", 10),
            cache_key("  beijing ", 10),
            "case and surrounding blanks are one lookup"
        );
        assert_ne!(cache_key("Beijing", 10), cache_key("Beijing", 5));
    }
}
