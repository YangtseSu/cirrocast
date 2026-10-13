// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The `QWeather` `GeoAPI` `city/lookup` geocoder: the name source that answers with `QWeather`'s own
//! city identity (step 30).
//!
//! `[geo] search = "qweather"` (or `CIRROCAST_GEO_SEARCH=qweather`) is the only way to select it:
//! the source is **explicit-selection only**, never part of `auto`. Two properties make it unfit to
//! be a default. It is credential-bound, where every other source is keyless, and **its rows carry
//! no ISO country code** — the response names a country in words (`China`, `中国`) and nothing else
//! — while [`Location::country_code`] drives `auto` provider selection and the alert registry's
//! coverage rule. A default path that silently lost those would degrade both to bounding boxes.
//!
//! What it adds over the other three is the reason it exists: neither the bundled table (`GeoNames`'
//! `cities15000`, towns above ~15 000 people), Open-Meteo's redistribution of it, `GeoNames`'
//! `searchJSON` nor Nominatim can resolve a Chinese district or township to *the point `QWeather`
//! serves weather for*. `新乡县` answers with the district (`101180313`, 35.19084, 113.80521) rather
//! than with its parent city, and the row carries the administrative chain (`adm1` 河南省) and a
//! 1:1 IANA zone outright.
//!
//! Measured 2026-10-13 on the account host:
//!
//! * the endpoint is `GET {host}/geo/v2/city/lookup` with `location` (required), `number` (1..=20,
//!   default 10) and `lang`; the envelope is `{"code":"200","location":[…],"refer":{…}}`, where
//!   `refer` is the vendor's own `sources`/`license` block;
//! * a row is `{name, id, lat, lon, adm2, adm1, country, tz, utcOffset, isDst, type, rank,
//!   fxLink}` with **`lat`/`lon` as strings** (like `GeoNames`, unlike Open-Meteo); `name` is
//!   the place (`新乡县`, `Beijing`), `adm1` its first-level division (河南省, `Beijing`), `adm2` the
//!   second-level one (新乡) — a level this model has no field for — and `country` a **localized
//!   name** with no ISO code anywhere in the payload;
//! * the rows arrive in the service's own relevance order, and that is the order this geocoder
//!   returns them in — the [`Geocoder`] contract. `rank` is a separate numeric hint (a capital is
//!   10, a town 33+) that is deliberately not decoded: the shared ranking orders the merged
//!   candidates for the query afterwards, and `rank` is not a population;
//! * a query that matches nothing is **not** an HTTP 200 with an empty `location[]`: the service
//!   answers `400` with `{"error":{"status":400,"type":"…#no-such-location",…}}`, which
//!   [`is_no_such_location`] turns back into an empty result — the chain reports a no-hit, not an
//!   upstream failure;
//! * a credential the service will not accept is `401` with `…#unauthorized`, mapped to
//!   [`Error::InvalidCredential`] (exit 6), the shared `QWeather` taxonomy.
//!
//! **The storage rule.** `QWeather`'s cache policy
//! (<https://dev.qweather.com/docs/best-practices/cache/>) is blunt: 你不能缓存、提取、批量存储 `GeoAPI`
//! 中提供的所有数据 — most of the underlying providers permit real-time use and forbid *any* form of
//! storage. This geocoder therefore writes **no cache entry at all** — not even a short-lived one —
//! keeps no index, table or cross-query store, and is the one source that ignores the geo cache: the
//! type holds no [`Cache`](crate::cache::Cache), only the shared clock the credential needs. A
//! repeat query asks again, which is the price of using this source; `-v` says so, and
//! `docs/providers.md` records it beside the terms. "Cache the lookup for a day" and "pre-seed the
//! bundled table from `GeoAPI`" are both out of bounds for the same reason.
//!
//! The credential and host are the `qweather` provider's (`providers.qweather.host` and whichever of
//! the API key / JWT the key store resolves, step 27), through the same [`QWeatherAuth`] helper the
//! forecast backend, the alert source and the air source use, so all four follow the configured
//! authentication mode with no second code path.

use std::str::FromStr;

use chrono_tz::Tz;
use serde::Deserialize;

use crate::auth::QWeatherAuth;
use crate::cache::Clock;
use crate::config::keys::Credential;
use crate::error::{Error, Result};
use crate::geo::Geocoder;
use crate::http::{HttpClient, HttpRequest};
use crate::model::{Location, LocationSource};

/// The endpoint path, after the account host.
pub const PATH: &str = "/geo/v2/city/lookup";

/// The credential name in the key store, and the `provider` of the errors this module reports.
pub const CREDENTIAL: &str = "qweather";

/// The environment variable that can supply the API key instead of `keys.toml`.
pub const ENV_VAR: &str = "CIRROCAST_QWEATHER_KEY";

/// The largest `number` the service accepts; a bigger `--limit` asks for this many.
const MAX_NUMBER: u8 = 20;

/// The vendor's machine-readable marker for "this query matches no place".
///
/// The refusal travels back as an [`Error::Upstream`] whose message is the response body — bounded
/// by `http::error_message`, which quotes the body's first 200 characters, and this fragment sits
/// well inside that window.
const NO_SUCH_LOCATION: &str = "#no-such-location";

/// A geocoder over the shared HTTP client, authenticated with the `qweather` credential.
///
/// Deliberately holds no cache: the service's terms forbid storing `GeoAPI` data in any form, so
/// there is nothing here for a cache to hold (see the module docs). The clock is only what a JWT
/// credential needs to be minted for the request.
pub struct QWeatherGeocoder<'a> {
    http: &'a HttpClient,
    /// The shared run clock, for the JWT form of the credential.
    clock: &'a dyn Clock,
    /// The account host (`providers.qweather.host`), already trimmed.
    host: &'a str,
    /// The API key or JWT credential the key store resolved.
    credential: &'a Credential,
    /// Whether the geo scope is silenced: the source has no cache to answer from, so it is refused
    /// rather than allowed to open a socket.
    offline: bool,
}

impl<'a> QWeatherGeocoder<'a> {
    /// Binds the geocoder to the client, the run clock, the account host and the credential.
    #[must_use]
    pub const fn new(
        http: &'a HttpClient,
        clock: &'a dyn Clock,
        host: &'a str,
        credential: &'a Credential,
        offline: bool,
    ) -> Self {
        Self {
            http,
            clock,
            host,
            credential,
            offline,
        }
    }
}

impl Geocoder for QWeatherGeocoder<'_> {
    fn search(&self, query: &str, limit: u8) -> Result<Vec<Location>> {
        if self.offline {
            // The shared chain turns a `Network` failure under a silenced scope into "no cached
            // answer"; for this source that is literally true, since it never stores one.
            return Err(Error::Network(format!(
                "offline: the `{CREDENTIAL}` geo source answers over the network and its terms \
                 forbid storing its data, so there is no cached answer to serve"
            )));
        }
        if self.host.trim().is_empty() {
            return Err(Error::Config(format!(
                "geo source `{CREDENTIAL}` needs `providers.{CREDENTIAL}.host` (see \
                 https://console.qweather.com/)"
            )));
        }
        let auth = QWeatherAuth::resolve(self.credential, self.clock.now())?;
        let request = auth.apply(
            HttpRequest::get(format!("{}{PATH}", self.host.trim()))
                .query("location", query)
                .query("number", limit.clamp(1, MAX_NUMBER).to_string())
                .query("lang", query_language(query)),
        );
        let response = match self.http.send(&request) {
            Ok(response) => response,
            // "The place you asked for does not exist" is an answer, not a failure: the keyless
            // sources spell it as an empty hit list and the chain must see the same thing here.
            Err(error) if is_no_such_location(&error) => return Ok(Vec::new()),
            Err(error) => return Err(invalid_credential(error)),
        };
        response.json::<Response>().map(Response::into_locations)
    }
}

/// Whether a failure is the service's "no place matches this query" answer.
fn is_no_such_location(error: &Error) -> bool {
    matches!(error, Error::Upstream { message, .. } if message.contains(NO_SUCH_LOCATION))
}

/// The `401` the service answers for a credential it will not accept, translated to the
/// invalid-credential failure; every other failure is passed through untouched.
///
/// The shared helper's `InvalidCredential` (exit 6) is what every other `QWeather` request in this
/// crate reports, because the remedy depends on which form of the credential was stored and the
/// message names both.
fn invalid_credential(error: Error) -> Error {
    if matches!(
        error,
        Error::Upstream {
            status: Some(401),
            ..
        }
    ) {
        return Error::InvalidCredential {
            provider: CREDENTIAL.to_owned(),
            status: 401,
        };
    }
    error
}

/// The `lang` parameter the query's script asks for.
///
/// `zh` for a CJK query and `en` otherwise, the same script rule
/// [`open_meteo`](crate::geo::open_meteo::query_language) applies to the alternate-name indexes:
/// the service *matches* a Chinese query under either setting but *spells* the answer in the
/// requested language (`新乡县` → `Xinxiang County` under `lang=en`), so asking for the query's own
/// script keeps a row foldable against the rows the keyless sources return for the same place —
/// `北京` and `Beijing` are one city, and the merge can only see that when both sides are spelled
/// alike. The opposite direction is the same rule read the other way: a Latin query keeps `en`
/// rather than coming back with a Chinese `adm1` that nothing else can fold.
///
/// Kana, Hangul and every non-CJK script keep `en` with everything else; this function is only about
/// being able to read a name back, and step 09 owns output-language negotiation.
#[must_use]
fn query_language(query: &str) -> &'static str {
    for character in query.chars() {
        // The unified ideographs and their extensions: the ranges Open-Meteo's own script rule
        // treats as the `zh` index.
        if matches!(
            character as u32,
            0x3400..=0x4dbf | 0x4e00..=0x9fff | 0xf900..=0xfaff
        ) {
            return "zh";
        }
    }
    "en"
}

/// The response envelope.
///
/// `location` is absent when nothing matched, which is not an error.
#[derive(Debug, Deserialize)]
struct Response {
    /// The matching places, in the service's own relevance order.
    #[serde(default)]
    location: Option<Vec<Hit>>,
}

impl Response {
    /// Translates every usable hit, in the service's own order.
    ///
    /// A row that cannot become a [`Location`] is dropped rather than failing the query: this source
    /// is one of several a name query merges, and a candidate without a zone would move every day
    /// part of the forecast (step 06 buckets hours by the location's local clock).
    fn into_locations(self) -> Vec<Location> {
        self.location
            .unwrap_or_default()
            .into_iter()
            .filter_map(Hit::into_location)
            .collect()
    }
}

/// One place, as the service reports it. Unknown fields are ignored on purpose: the payload also
/// carries `id` (the `LocationID`, which nothing here consumes — the request is built from
/// coordinates), `utcOffset`/`isDst` (both redundant beside `tz`), `type` (measured `city` even for
/// a district), `fxLink` (upstream presentation) and **`rank`**.
///
/// `rank` is `QWeather`'s own importance ordering (a capital is 10, a town 33+), and it is deliberately
/// not decoded: neither the model field it resembles nor the one it would order belongs to it.
/// A [`Geocoder`] answers "in the service's own order", which is the relevance order the response
/// already comes in, and the crate's shared ranking ([`crate::geo::rank`]) orders the merged
/// candidates for the query afterwards — so `rank` would only re-sort a list that is re-sorted
/// anyway. It is certainly not a population, which is what `Location::population` means and what
/// the ranking reads.
#[derive(Debug, Deserialize)]
struct Hit {
    /// Display name, e.g. `新乡县` or `Beijing`.
    #[serde(default)]
    name: Option<String>,
    /// Latitude in degrees, WGS 84, as text.
    #[serde(default)]
    lat: Option<String>,
    /// Longitude in degrees, WGS 84, as text.
    #[serde(default)]
    lon: Option<String>,
    /// First-level administrative division, e.g. `河南省`.
    #[serde(default)]
    adm1: Option<String>,
    /// Country name (`China`, `中国`); the payload carries no ISO code for it.
    #[serde(default)]
    country: Option<String>,
    /// IANA zone name, e.g. `Asia/Shanghai`.
    #[serde(default)]
    tz: Option<String>,
}

impl Hit {
    /// The canonical [`Location`] for this hit, or `None` when the row cannot be one.
    fn into_location(self) -> Option<Location> {
        let name = text(self.name.as_deref())?;
        let (lat, lon) = (
            coordinate(self.lat.as_deref())?,
            coordinate(self.lon.as_deref())?,
        );
        if !crate::geo::nominatim::usable_coordinates(lat, lon) {
            return None;
        }
        let tz = timezone(self.tz.as_deref())?;
        Some(Location {
            name,
            admin1: text(self.adm1.as_deref()),
            country: text(self.country.as_deref()).unwrap_or_default(),
            // The service publishes no ISO 3166-1 code at all (measured 2026-10-13): `country` is a
            // localized name, `refer` names the vendor and `fxLink` only repeats the `LocationID`.
            // Inventing a code from the name would be a guess the whole provider selection runs on.
            country_code: None,
            lat,
            lon,
            tz,
            // Neither is published: the service reports no elevation, and `rank` is not a
            // population.
            elevation_m: None,
            population: None,
            source: LocationSource::QWeather,
            station: None,
            named_by: None,
        })
    }
}

/// One of the service's text coordinates, parsed; `None` when it is missing or not a number.
fn coordinate(raw: Option<&str>) -> Option<f64> {
    raw.and_then(|text| text.trim().parse::<f64>().ok())
        .filter(|value| value.is_finite())
}

/// The row's IANA zone, when it has a usable one.
///
/// Deliberately no `UTC` fallback, exactly as in the `GeoNames` and Open-Meteo geocoders: a
/// geocoded name was resolved so that its local time is known, and a row whose zone is missing or
/// unknown is dropped rather than resolved to the wrong day parts.
fn timezone(raw: Option<&str>) -> Option<Tz> {
    let name = text(raw)?;
    Tz::from_str(&name).ok()
}

/// A field's text, or `None` when the answer omits it, sends `null` or sends blanks.
fn text(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::{Response, query_language};

    /// The script of the query picks the spelling the service answers in.
    #[test]
    fn the_query_script_selects_the_language() {
        assert_eq!(query_language("Beijing"), "en");
        assert_eq!(query_language("Berlin, DE"), "en");
        assert_eq!(query_language("Zürich"), "en");
        assert_eq!(query_language("新乡县"), "zh");
        assert_eq!(query_language("北京"), "zh");
        assert_eq!(query_language("東京"), "zh");
        // No `zh` index matches these scripts, so they keep the documented default.
        assert_eq!(query_language("とうきょう"), "en");
        assert_eq!(query_language("서울"), "en");
        assert_eq!(query_language("Москва"), "en");
    }

    /// Both no-hit shapes are legitimate empty results, not errors.
    #[test]
    fn a_missing_or_empty_location_array_is_no_hits() {
        for body in [r#"{"code":"200"}"#, r#"{"code":"200","location":[]}"#] {
            let response: Response = serde_json::from_str(body).expect("the envelope parses");
            assert_eq!(
                response.into_locations(),
                Vec::<super::Location>::new(),
                "`{body}` should hold no locations"
            );
        }
    }

    /// The rows that cannot become a location are dropped, and the usable ones keep their order.
    #[test]
    fn unusable_rows_are_dropped_and_the_rest_kept_in_order() {
        let body = r#"{"code":"200","location":[
            {"name":"Town","lat":"35.0","lon":"113.0","adm1":"河南省","country":"中国",
             "tz":"Asia/Shanghai","rank":"33"},
            {"name":"City","lat":"35.3","lon":"113.9","adm1":"河南省","country":"中国",
             "tz":"Asia/Shanghai","rank":"15"},
            {"name":"Bad Zone","lat":"35.2","lon":"113.2","adm1":"河南省","country":"中国",
             "tz":"Mars/Olympus","rank":"1"},
            {"name":"No Zone","lat":"35.2","lon":"113.2","adm1":"河南省","country":"中国","rank":"1"},
            {"name":"Out Of Range","lat":"91.0","lon":"113.2","adm1":"河南省","country":"中国",
             "tz":"Asia/Shanghai","rank":"1"},
            {"name":"Not A Number","lat":"east","lon":"113.2","adm1":"河南省","country":"中国",
             "tz":"Asia/Shanghai","rank":"1"},
            {"lat":"35.2","lon":"113.2","adm1":"河南省","country":"中国","tz":"Asia/Shanghai"}
        ]}"#;
        let response: Response = serde_json::from_str(body).expect("the envelope parses");
        let locations = response.into_locations();
        let names: Vec<&str> = locations
            .iter()
            .map(|location| location.name.as_str())
            .collect();
        assert_eq!(names, ["Town", "City"], "{locations:#?}");
        assert_eq!(locations[0].source, crate::model::LocationSource::QWeather);
        assert_eq!(locations[0].admin1.as_deref(), Some("河南省"));
        assert_eq!(locations[0].country, "中国");
        assert_eq!(
            locations[0].country_code, None,
            "the service publishes no ISO code"
        );
        assert_eq!(locations[0].tz, chrono_tz::Tz::Asia__Shanghai);
        assert_eq!(locations[0].population, None, "`rank` is not a population");
    }
}
