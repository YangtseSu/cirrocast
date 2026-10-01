// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The OpenStreetMap Nominatim geocoder behind `~query`.
//!
//! Nominatim is a donated service with a usage policy, so this client does three things the other
//! geocoders here do not have to: it identifies itself with [`UA`] on every request, it never
//! sends more than one request per second (shared across processes through a timestamp under the
//! cache root), and it caches every response — a cache hit neither throttles nor sleeps.
//!
//! The throttle reads and writes `ratelimit/nominatim.json` through [`Cache::read_state`] and
//! [`Cache::write_state`], so the state travels with the cache instead of a second private
//! directory, and the send time is recorded *before* the request goes out: a run that is
//! cancelled or crashes still counts against the limit, which is the safe direction for a service
//! nobody pays for. A missing or unparsable state file simply means "no request recorded yet".
//!
//! Responses are raw jsonv2, mapped one hit to one [`Location`] with [`LocationSource::Osm`].
//! OSM objects carry a `timezone` tag only sometimes, so a hit without a usable one keeps the
//! provisional UTC zone that the `Osm` source marks as "replace this from the forecast
//! response"; the caller prints the `ODbL` attribution such data requires. Ranking never happens
//! here: hits leave in the service's own order and [`super::resolve`] orders them.

use std::str::FromStr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono_tz::Tz;
use serde::{Deserialize, Deserializer};
use serde_json::Value;

use super::Geocoder;
use crate::cache::{Cache, CacheKey};
use crate::error::{Error, Result};
use crate::http::{HttpClient, HttpRequest, UA};
use crate::model::{Location, LocationSource};

/// The public OpenStreetMap endpoint, the base URL when nothing else is configured.
pub const DEFAULT_URL: &str = "https://nominatim.openstreetmap.org";

/// How long a recorded response is reused: the usage policy requires caching, OSM objects change
/// slowly, and this is the retention the pipeline documents.
pub const TTL: Duration = Duration::from_hours(720);

/// Where the last request's timestamp lives, relative to the cache root.
const THROTTLE_STATE: &str = "ratelimit/nominatim.json";

/// The minimum spacing the policy allows between two requests of one application.
const MIN_INTERVAL: Duration = Duration::from_secs(1);

/// The Nominatim client: one `GET /search` per query, cached and self-throttled.
///
/// Each search goes through [`Cache::read_or_fetch_json`], so an answer already on disk is served
/// without a request — and therefore without a wait. Every request carries [`UA`], without which
/// the service may block this application, and `accept-language: en` so the OSM names come back
/// in one language whatever the machine's locale is (the sibling Open-Meteo geocoder pins
/// `language=en` for the same reason).
pub struct Nominatim<'a> {
    http: &'a HttpClient,
    cache: &'a Cache,
    base_url: String,
    ttl: Duration,
}

impl<'a> Nominatim<'a> {
    /// Builds the geocoder for the service at `base_url`.
    ///
    /// A trailing `/` is ignored, so `https://example.org` and `https://example.org/` are one
    /// service; an empty URL means [`DEFAULT_URL`], matching the `network.nominatim_url` row,
    /// which documents empty as "the public service".
    #[must_use]
    pub fn new(http: &'a HttpClient, cache: &'a Cache, base_url: impl Into<String>) -> Self {
        Self {
            http,
            cache,
            base_url: normalize_base(&base_url.into()),
            ttl: TTL,
        }
    }

    /// The `/search` request: the parameters the policy and the cache key assume, in wire order.
    fn request(&self, query: &str, limit: u8) -> HttpRequest {
        let base_url = self.base_url.as_str();
        HttpRequest::get(format!("{base_url}/search"))
            .query("format", "jsonv2")
            .query("q", query)
            .query("limit", limit.to_string())
            .query("addressdetails", "1")
            .query("extratags", "1")
            .header("user-agent", UA)
            .header("accept-language", "en")
    }

    /// The cache key of one search: one entry per service, query text (case-insensitively
    /// identical queries share one entry) and limit.
    fn cache_key(&self, query: &str, limit: u8) -> CacheKey {
        let base_url = self.base_url.as_str();
        let query = query.trim().to_lowercase();
        CacheKey::hash("geocode", &format!("nominatim|{base_url}|{query}|{limit}"))
    }

    /// Fetches the raw body: wait out the rate limit first, then send. The throttle is reached
    /// only from here, which is what keeps the cache path silent.
    fn fetch(&self, query: &str, limit: u8) -> Result<(u16, String)> {
        self.throttle()?;
        let response = self.http.send(&self.request(query, limit))?;
        Ok((response.status(), response.body().to_owned()))
    }

    /// Waits out the remainder of the 1 request/second window and records the send time.
    ///
    /// The stamp is written before the request goes out, so a run that crashes or is cancelled
    /// still counts against the limit. Nothing is recorded for a cache hit, because this is never
    /// called for one.
    fn throttle(&self) -> Result<()> {
        let clock = self.cache.clock();
        let now = clock.now();
        if let Some(previous) = self.last_request()?.and_then(instant) {
            // A clock that moved backwards (or a damaged stamp) yields zero elapsed time, which
            // waits the full second: over-throttling is the safe failure for a donated service.
            let elapsed = now.duration_since(previous).unwrap_or_default();
            let remaining = MIN_INTERVAL.saturating_sub(elapsed);
            if !remaining.is_zero() {
                clock.sleep(remaining);
            }
        }
        let stamp = unix_millis(clock.now());
        self.cache
            .write_state(THROTTLE_STATE, &throttle_state(stamp))
    }

    /// The millisecond stamp of the last request, or `None` when there is none to honour.
    ///
    /// A state file this build cannot parse means "no recorded request" rather than an error: a
    /// damaged or hand-edited file must not make the CLI unusable, and the worst case is one
    /// request sent too early.
    fn last_request(&self) -> Result<Option<u128>> {
        Ok(self
            .cache
            .read_state(THROTTLE_STATE)?
            .as_deref()
            .and_then(recorded_stamp))
    }
}

impl Geocoder for Nominatim<'_> {
    fn search(&self, query: &str, limit: u8) -> Result<Vec<Location>> {
        let key = self.cache_key(query, limit);
        let hits: Vec<Hit> = self
            .cache
            .read_or_fetch_json(&key, self.ttl, "nominatim", || self.fetch(query, limit))?;
        hits.into_iter().map(location_from_hit).collect()
    }
}

/// The service root without a trailing `/`, or [`DEFAULT_URL`] when nothing was given.
fn normalize_base(base_url: &str) -> String {
    let trimmed = base_url.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        DEFAULT_URL.to_owned()
    } else {
        trimmed.to_owned()
    }
}

/// One jsonv2 hit: every field this client reads, all optional because Nominatim omits what the
/// OSM object does not have.
#[derive(Debug, Clone, Deserialize)]
struct Hit {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    lat: Option<String>,
    #[serde(default)]
    lon: Option<String>,
    #[serde(default)]
    address: Option<Address>,
    #[serde(default)]
    extratags: Option<Extratags>,
}

/// The subset of `address` this client reads; `admin1` is `state` and `country` is the name.
#[derive(Debug, Clone, Default, Deserialize)]
struct Address {
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    country: Option<String>,
    #[serde(default)]
    country_code: Option<String>,
}

/// The subset of `extratags` this client reads.
#[derive(Debug, Clone, Default, Deserialize)]
struct Extratags {
    #[serde(default)]
    timezone: Option<String>,
    /// OSM stores a population as a string (`"21540000"`), but some extracts emit a number, so
    /// both spellings are accepted.
    #[serde(default, deserialize_with = "population")]
    population: Option<u64>,
}

/// The `{"last_request_unix_ms": …}` payload of the throttle state file.
#[derive(Debug, Deserialize)]
struct ThrottleState {
    last_request_unix_ms: u128,
}

/// Maps one hit; the caller keeps the upstream order.
///
/// A hit whose coordinates are missing or unusable is an error naming that hit, never a silently
/// dropped candidate: a forecast for the wrong place is worse than a failure.
fn location_from_hit(hit: Hit) -> Result<Location> {
    let Hit {
        name,
        display_name,
        lat,
        lon,
        address,
        extratags,
    } = hit;

    // `name` is the OSM object's own name; `display_name` is its full address string, whose first
    // comma-separated segment is the best stand-in when the object has no name.
    let name = name
        .filter(|name| !name.trim().is_empty())
        .or_else(|| first_segment(display_name.as_deref()))
        .unwrap_or_else(|| "unnamed result".to_owned());
    let lat = coordinate(lat.as_deref(), "latitude", &name)?;
    let lon = coordinate(lon.as_deref(), "longitude", &name)?;

    let address = address.unwrap_or_default();
    let (tz, population) = match extratags {
        Some(tags) => (timezone(tags.timezone.as_deref()), tags.population),
        None => (Tz::UTC, None),
    };

    Ok(Location {
        name,
        admin1: address.state,
        country: address.country.unwrap_or_default(),
        country_code: address.country_code,
        lat,
        lon,
        tz,
        elevation_m: None,
        population,
        source: LocationSource::Osm,
        station: None,
    })
}

/// The first comma-separated segment of a `display_name`, trimmed.
fn first_segment(display_name: Option<&str>) -> Option<String> {
    display_name
        .and_then(|text| text.split(',').next())
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .map(str::to_owned)
}

/// Parses one of Nominatim's string coordinates, or reports the hit that has none.
///
/// A value that parses but is not finite is rejected as well: `NaN` or an infinity would travel
/// into a forecast request as a nonsensical coordinate.
fn coordinate(raw: Option<&str>, axis: &str, name: &str) -> Result<f64> {
    let parsed = raw
        .and_then(|text| text.trim().parse::<f64>().ok())
        .filter(|value| value.is_finite());
    parsed.ok_or_else(|| {
        let got = match raw {
            Some(text) => format!("\"{text}\""),
            None => "nothing".to_owned(),
        };
        Error::Upstream {
            provider: "nominatim".to_owned(),
            status: None,
            message: format!("geocoding result `{name}` has no usable {axis} (got {got})"),
        }
    })
}

/// The hit's IANA zone when it carries a usable one, UTC otherwise.
///
/// UTC here is provisional rather than wrong: [`LocationSource::Osm`] tells the renderer that the
/// provider fetching the forecast replaces the zone from its own response. An unparsable tag is
/// treated exactly like a missing one instead of failing the whole search.
fn timezone(tag: Option<&str>) -> Tz {
    tag.map(str::trim)
        .and_then(|name| Tz::from_str(name).ok())
        .unwrap_or(Tz::UTC)
}

/// The population of an `extratags` entry, from either a string or a number.
///
/// Anything else (a null, a boolean, an unparsable string) is `None`: a population is only a
/// ranking tiebreak, so one odd tag must not fail a search.
fn population<'de, D>(deserializer: D) -> Result<Option<u64>, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(match Value::deserialize(deserializer)? {
        Value::String(text) => text.trim().parse().ok(),
        Value::Number(number) => number.as_u64(),
        _ => None,
    })
}

/// The throttle state file's payload for a send time.
fn throttle_state(millis: u128) -> String {
    serde_json::json!({ "last_request_unix_ms": millis }).to_string()
}

/// The send time recorded in a state file, when that file is one.
fn recorded_stamp(text: &str) -> Option<u128> {
    serde_json::from_str::<ThrottleState>(text)
        .ok()
        .map(|state| state.last_request_unix_ms)
}

/// The instant a recorded stamp refers to, when a clock could have produced it.
///
/// An unrepresentable stamp (a hand-edited file, a saturated value) is `None`, which the caller
/// treats as "no recorded request".
fn instant(millis: u128) -> Option<SystemTime> {
    UNIX_EPOCH.checked_add(Duration::from_millis(u64::try_from(millis).ok()?))
}

/// `time` as milliseconds since the Unix epoch; a time before the epoch clamps to zero.
fn unix_millis(time: SystemTime) -> u128 {
    match time.duration_since(UNIX_EPOCH) {
        Ok(elapsed) => elapsed.as_millis(),
        Err(_) => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Parses an `extratags` object the way one hit is parsed.
    fn extratags(json: &str) -> Extratags {
        serde_json::from_str(json).expect("the extratags fixture parses")
    }

    /// Parses one hit the way the response array is parsed.
    fn hit(json: &str) -> Hit {
        serde_json::from_str(json).expect("the hit fixture parses")
    }

    #[test]
    fn population_is_read_as_a_string_or_as_a_number() {
        assert_eq!(
            extratags(r#"{"population": "21540000"}"#).population,
            Some(21_540_000)
        );
        assert_eq!(
            extratags(r#"{"population": 21540000}"#).population,
            Some(21_540_000)
        );
        assert_eq!(extratags(r#"{"population": "unknown"}"#).population, None);
        assert_eq!(
            extratags(r#"{"timezone": "Asia/Shanghai"}"#).population,
            None
        );
    }

    #[test]
    fn a_trailing_slash_is_ignored_and_an_empty_url_is_the_public_service() {
        assert_eq!(normalize_base(DEFAULT_URL), DEFAULT_URL);
        assert_eq!(
            normalize_base("https://example.org/"),
            "https://example.org"
        );
        assert_eq!(
            normalize_base("  https://example.org///  "),
            "https://example.org"
        );
        assert_eq!(normalize_base(""), DEFAULT_URL);
    }

    #[test]
    fn a_usable_timezone_tag_beats_the_provisional_zone() {
        let location = location_from_hit(hit(
            r#"{"name": "Beijing", "lat": "39.9042", "lon": "116.4074",
                "address": {"state": "Beijing", "country": "China", "country_code": "cn"},
                "extratags": {"timezone": "Asia/Shanghai", "population": "21540000"}}"#,
        ))
        .expect("the hit maps");

        assert_eq!(location.tz, Tz::Asia__Shanghai);
        assert_eq!(location.source, LocationSource::Osm);
        assert_eq!(location.admin1.as_deref(), Some("Beijing"));
        assert_eq!(location.country, "China");
        assert_eq!(location.population, Some(21_540_000));
    }

    #[test]
    fn an_unusable_timezone_tag_is_not_a_failure() {
        for tag in [r#""Mars/Olympus""#, r#""""#] {
            let location = location_from_hit(hit(&format!(
                r#"{{"name": "Beijing", "lat": "39.9", "lon": "116.4",
                     "extratags": {{"timezone": {tag}}}}}"#
            )))
            .expect("the hit maps");

            assert_eq!(location.tz, Tz::UTC, "tag {tag}");
            assert_eq!(location.source, LocationSource::Osm, "tag {tag}");
        }
    }

    #[test]
    fn a_hit_without_a_name_falls_back_to_its_display_name() {
        let location = location_from_hit(hit(
            r#"{"display_name": "Tsinghua University, Shuangqing Road, Beijing, China",
                "lat": "40.0022905", "lon": "116.3209630"}"#,
        ))
        .expect("the hit maps");

        assert_eq!(location.name, "Tsinghua University");
        assert_eq!(location.country, "");
        assert_eq!(location.admin1, None);
    }

    #[test]
    fn a_hit_without_usable_coordinates_names_itself() {
        for json in [
            r#"{"name": "Beijing", "display_name": "Beijing, China", "lon": "116.4"}"#,
            r#"{"name": "Beijing", "lat": "north", "lon": "116.4"}"#,
            r#"{"name": "Beijing", "lat": "NaN", "lon": "116.4"}"#,
        ] {
            match location_from_hit(hit(json)) {
                Err(Error::Upstream {
                    provider,
                    status,
                    message,
                }) => {
                    assert_eq!(provider, "nominatim");
                    assert_eq!(status, None);
                    assert!(message.contains("Beijing"), "{message}");
                    assert!(message.contains("latitude"), "{message}");
                }
                other => panic!("expected an upstream error, got {other:?}"),
            }
        }
    }
}
