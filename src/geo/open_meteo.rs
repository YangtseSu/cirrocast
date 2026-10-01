// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The Open-Meteo geocoding client: the [`Geocoder`] behind a fuzzy location name.
//!
//! One lookup, one request, one cache entry. The endpoint answers `GET …/v1/search` with a
//! `results` array of `GeoNames` places — Open-Meteo redistributes `GeoNames` data under
//! CC-BY-4.0, so every caller that renders a hit owes that attribution — and this module does
//! nothing to the array beyond translating it: no filtering, no ranking, no truncation. The
//! `:name` exact filter and the population order live in
//! [`crate::geo::resolve`]/[`crate::geo::rank`], which is what keeps "the service returned them in
//! this order" and "we picked this one" separate decisions.
//!
//! Two rules are worth restating because they are easy to undo by accident:
//!
//! * a missing or empty `results` key is *no hits*, not an error — a two-letter query for a place
//!   that does not exist is a legitimate answer;
//! * a result without a usable IANA zone is an error, never a silent `UTC`. Day-part aggregation
//!   (step 06) buckets forecast hours by the location's local clock, so guessing the zone here
//!   would move every bucket.

use std::str::FromStr;
use std::time::Duration;

use chrono_tz::Tz;
use serde::Deserialize;

use crate::cache::{Cache, CacheKey};
use crate::error::{Error, Result};
use crate::geo::Geocoder;
use crate::http::{HttpClient, HttpRequest};
use crate::model::{Location, LocationSource};

/// The geocoding endpoint every name lookup goes through.
pub const GEOCODE_URL: &str = "https://geocoding-api.open-meteo.com/v1/search";

/// The `provider` field of the errors this module reports.
const PROVIDER: &str = "open-meteo-geocoding";

/// A geocoder over the shared HTTP client and cache.
///
/// The client owns the retry policy and the User-Agent, the cache owns the TTL and the
/// `--offline`/`--refresh` behaviour, and both are borrowed so a run keeps a single connection
/// pool and a single cache root.
pub struct OpenMeteoGeocoder<'a> {
    http: &'a HttpClient,
    cache: &'a Cache,
    ttl: Duration,
}

impl<'a> OpenMeteoGeocoder<'a> {
    /// Binds the geocoder to the client and cache it should use, caching responses for `ttl`.
    #[must_use]
    pub const fn new(http: &'a HttpClient, cache: &'a Cache, ttl: Duration) -> Self {
        Self { http, cache, ttl }
    }
}

impl Geocoder for OpenMeteoGeocoder<'_> {
    fn search(&self, query: &str, limit: u8) -> Result<Vec<Location>> {
        let language = query_language(query);
        let key = cache_key(query, limit, language);
        let response: Response = self.cache.read_or_fetch_json(
            &key,
            self.ttl,
            "open-meteo",
            &format!("the query `{query}`"),
            || {
                let request = HttpRequest::get(GEOCODE_URL)
                    .query("name", query)
                    .query("count", limit.to_string())
                    .query("language", language)
                    .query("format", "json");
                let response = self.http.send(&request)?;
                Ok((response.status(), response.body().to_owned()))
            },
        )?;
        response.into_locations()
    }
}

/// The `language` parameter that can match `query` at all.
///
/// The endpoint searches `GeoNames`' alternate names *per language*, so the script of the query
/// decides which index is worth asking: `name=新乡&language=en` is a guaranteed no-hit while
/// `language=zh` answers it, and the same holds for Cyrillic (`ru`), Greek (`el`), Arabic (`ar`),
/// Hebrew (`he`) and Thai (`th`) — all probed against the live service.
///
/// Latin queries keep `en`, and not only because that is the documented request shape: the other
/// indexes answer *different* places for the same Latin text (`Xinxiang` with `language=zh` returns
/// a village called 南干道), so switching them would silently move a user's query.
///
/// Kana, Hangul and Devanagari have no index that matches upstream (also probed), so those queries
/// keep `en` with everything else and fail as "no location found"; step 09 owns output-language
/// negotiation, this function is only about being able to find a name in the first place.
fn query_language(query: &str) -> &'static str {
    for character in query.chars() {
        match character as u32 {
            0x0370..=0x03ff => return "el",
            0x0400..=0x04ff => return "ru",
            0x0590..=0x05ff => return "he",
            0x0600..=0x06ff | 0x0750..=0x077f => return "ar",
            0x0e00..=0x0e7f => return "th",
            0x3400..=0x4dbf | 0x4e00..=0x9fff | 0xf900..=0xfaff => return "zh",
            _ => {}
        }
    }
    "en"
}

/// The cache key of one lookup: the trimmed, lower-cased name plus the parameters that change the
/// answer.
///
/// The text is *not* the request URL — the URL is derived from the parameters, and two spellings
/// (`Beijing`, `beijing`) mean the same lookup, so keying on the parameters keeps one entry per
/// lookup instead of one per capitalisation. The language is part of the key because it changes
/// which index is searched.
fn cache_key(query: &str, limit: u8, language: &str) -> CacheKey {
    CacheKey::hash(
        "geocode",
        &format!(
            "open-meteo|{}|{limit}|{language}",
            query.trim().to_lowercase()
        ),
    )
}

/// The response envelope. `results` is absent when nothing matched, which is not an error.
#[derive(Debug, Deserialize)]
struct Response {
    /// The matching places, in the order the service ranked them.
    results: Option<Vec<Hit>>,
}

impl Response {
    /// Translates every hit; the first unusable one fails the whole lookup.
    fn into_locations(self) -> Result<Vec<Location>> {
        self.results
            .unwrap_or_default()
            .into_iter()
            .map(Hit::into_location)
            .collect()
    }
}

/// One place, as the endpoint reports it. Unknown fields are ignored on purpose: the payload
/// carries `GeoNames` bookkeeping (`id`, `feature_code`, `admin2_id`, …) that no caller wants.
#[derive(Debug, Deserialize)]
struct Hit {
    /// Display name, e.g. `Beijing`.
    name: String,
    /// Latitude in degrees, WGS 84.
    latitude: f64,
    /// Longitude in degrees, WGS 84.
    longitude: f64,
    /// Elevation above sea level in metres.
    elevation: Option<f64>,
    /// IANA time zone name, e.g. `Asia/Shanghai`.
    timezone: Option<String>,
    /// Country name, e.g. `China`.
    country: Option<String>,
    /// ISO 3166-1 alpha 2 country code, e.g. `CN`.
    country_code: Option<String>,
    /// First level administrative division, e.g. `Beijing Municipality`.
    admin1: Option<String>,
    /// Population, used only to rank ambiguous matches; never rendered.
    population: Option<u64>,
}

impl Hit {
    /// The canonical [`Location`] for this hit.
    fn into_location(self) -> Result<Location> {
        let tz = self.timezone()?;
        Ok(Location {
            name: self.name,
            admin1: self.admin1,
            country: self.country.unwrap_or_default(),
            country_code: self.country_code,
            lat: self.latitude,
            lon: self.longitude,
            tz,
            elevation_m: self.elevation,
            population: self.population,
            source: LocationSource::Geocoder,
            station: None,
        })
    }

    /// The hit's IANA zone, or the error that says why there is none.
    ///
    /// Deliberately no `UTC` fallback: a coordinate location is *provisionally* UTC until a
    /// provider reports the real zone, but a geocoded name was resolved precisely so that its
    /// local time is known, so a missing or unknown zone has to fail loudly.
    fn timezone(&self) -> Result<Tz> {
        let Some(reported) = self.timezone.as_deref() else {
            return Err(self.unknown_timezone("no timezone reported"));
        };
        Tz::from_str(reported).map_err(|_| self.unknown_timezone(&format!("got \"{reported}\"")))
    }

    /// The one error shape both timezone failures share.
    fn unknown_timezone(&self, detail: &str) -> Error {
        let name = &self.name;
        Error::Upstream {
            provider: PROVIDER.to_owned(),
            status: None,
            message: format!("geocoding result `{name}` has no usable IANA timezone ({detail})"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Response, query_language};

    /// The script of the query picks the index that can actually match it.
    #[test]
    fn the_query_script_selects_the_language_index() {
        assert_eq!(query_language("Beijing"), "en");
        assert_eq!(query_language("Beijing, CN"), "en");
        assert_eq!(query_language("Zürich"), "en");
        assert_eq!(query_language("新乡"), "zh");
        assert_eq!(query_language("東京"), "zh");
        assert_eq!(query_language("Москва"), "ru");
        assert_eq!(query_language("Αθήνα"), "el");
        assert_eq!(query_language("القاهرة"), "ar");
        assert_eq!(query_language("תל אביב"), "he");
        assert_eq!(query_language("กรุงเทพ"), "th");
        // No upstream index matches these scripts, so they keep the documented default.
        assert_eq!(query_language("とうきょう"), "en");
        assert_eq!(query_language("서울"), "en");
    }

    /// Both no-hit shapes are legitimate empty results, not errors.
    #[test]
    fn a_missing_or_empty_results_array_is_no_hits() {
        for body in [r#"{"generationtime_ms":0.4}"#, r#"{"results":[]}"#] {
            let response: Response = serde_json::from_str(body).expect("the envelope parses");
            let locations = response.into_locations().expect("no hits is not a failure");
            assert_eq!(
                locations,
                Vec::<super::Location>::new(),
                "`{body}` should hold no locations"
            );
        }
    }
}
