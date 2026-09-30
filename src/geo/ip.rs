// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! IP-derived locations: where the forecast is for when the user names no place at all.
//!
//! Two donated services answer the same question, so one is the fallback for the other: `ipwho.is`
//! first, because its answer carries the zone in IANA form and its refusals are explicit, then
//! `ipapi.co`. Only the *kinds* of failure a second service can plausibly answer differently fall
//! through — a network failure and an upstream failure, which includes the `success: false`
//! envelope a service sends for an address it refuses to place (a reserved or blocked range, a
//! rate limit). Anything else stops the run. When every service fails, the last failure is returned
//! unchanged, so the message names the service the run actually ended on.
//!
//! Two rules are worth restating because they are easy to undo by accident:
//!
//! * a missing name, latitude, longitude or zone is an error naming the service and the field,
//!   never a default — a guessed `0.0` would query the weather for the Gulf of Guinea, and a guessed
//!   `UTC` would shift every day part of the forecast (step 06 buckets hours by the location's
//!   local clock); a country the answer does not name simply stays empty, because the renderers
//!   already skip an empty part;
//! * the answers are cached per service ([`CacheKey::ip`]), so a fallback result never masquerades
//!   as the primary's, and the [`Location`] keeps [`LocationSource::Ip`] as its provenance.
//!
//! Nothing here prints: the caller renders the privacy disclosure, because only the caller knows
//! whether the user asked for an IP lookup at all.

use std::str::FromStr;
use std::time::Duration;

use chrono_tz::Tz;
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::Value;

use crate::cache::{Cache, CacheKey};
use crate::error::{Error, Result};
use crate::http::{HttpClient, HttpRequest};
use crate::model::{Location, LocationSource};

/// `ipwho.is`: no key, an IANA zone in the answer, and a `success: false` envelope instead of a
/// bogus location.
const IPWHO_IS_URL: &str = "https://ipwho.is/";

/// `ipapi.co`: the address-of-the-caller endpoint of the fallback service.
const IPAPI_CO_URL: &str = "https://ipapi.co/json/";

/// What [`IpService::chain`] accepts, appended to every rejection so the message is actionable.
const ACCEPTED: &str = "accepted: auto, ipwhois, ipapi";

/// A source of locations derived from the machine's public IP address.
///
/// One method and no arguments: the address is whatever the service sees, so there is nothing to
/// parameterise and nothing to get wrong at the call site. Implementations borrow the shared HTTP
/// client and cache, are synchronous, and print nothing.
pub trait IpLocator {
    /// The location the public address resolves to.
    fn locate(&self) -> Result<Location>;
}

/// The IP location services, best first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IpService {
    /// The primary: `ipwho.is`.
    IpWhoIs,
    /// The fallback: `ipapi.co`.
    IpApiCo,
}

impl IpService {
    /// The service's short id, which names its cache file: `ipwho-is`, `ipapi-co`.
    #[must_use]
    pub const fn slug(self) -> &'static str {
        match self {
            Self::IpWhoIs => "ipwho-is",
            Self::IpApiCo => "ipapi-co",
        }
    }

    /// The service's host name, used as the `provider` of any error it causes.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::IpWhoIs => "ipwho.is",
            Self::IpApiCo => "ipapi.co",
        }
    }

    /// The services `setting` selects, in the order they are tried.
    ///
    /// `auto` — the default — is `ipwho.is` first with `ipapi.co` behind it; the two
    /// single-service spellings exist so a user whose network blocks one of them can pin the other.
    /// Anything else is [`Error::Usage`]: a typo in the configuration must stop the run, not
    /// silently pick a service.
    pub fn chain(setting: &str) -> Result<Vec<Self>> {
        match setting.trim() {
            "auto" => Ok(vec![Self::IpWhoIs, Self::IpApiCo]),
            "ipwhois" => Ok(vec![Self::IpWhoIs]),
            "ipapi" => Ok(vec![Self::IpApiCo]),
            other => Err(Error::Usage(format!(
                "unknown IP location service {other:?} ({ACCEPTED})"
            ))),
        }
    }

    /// The endpoint this service is asked at: one `GET`, no query pairs, no extra headers.
    const fn endpoint(self) -> &'static str {
        match self {
            Self::IpWhoIs => IPWHO_IS_URL,
            Self::IpApiCo => IPAPI_CO_URL,
        }
    }
}

/// The fallback chain: the configured services, tried in order.
///
/// There is no type per service, because the two differ only in the shape of their JSON, and that
/// shape is decoded immediately before it becomes a [`Location`].
pub struct IpLocatorChain<'a> {
    http: &'a HttpClient,
    cache: &'a Cache,
    ttl: Duration,
    services: Vec<IpService>,
}

impl<'a> IpLocatorChain<'a> {
    /// Binds the chain to the shared client and cache, caching every answer for `ttl`.
    #[must_use]
    pub fn new(
        http: &'a HttpClient,
        cache: &'a Cache,
        services: Vec<IpService>,
        ttl: Duration,
    ) -> Self {
        Self {
            http,
            cache,
            ttl,
            services,
        }
    }

    /// Looks the public address up, reporting the service whose answer won.
    ///
    /// The caller needs the service for the privacy disclosure line, which is why it comes back
    /// here rather than being pushed into a log. Services are tried in order and the next one is
    /// only reached on [`Error::Network`] or [`Error::Upstream`]; with nothing left, the last error
    /// is returned as it was, so exit codes and wording stay the last service's.
    pub fn locate_with_service(&self) -> Result<(Location, IpService)> {
        let mut last: Option<Error> = None;
        for &service in &self.services {
            match self.locate_via(service) {
                Ok(location) => return Ok((location, service)),
                Err(error) => {
                    // Only the two failures a second service can plausibly answer differently move
                    // the chain on; anything else is the caller's problem, not the next service's.
                    if !matches!(error, Error::Network(_) | Error::Upstream { .. }) {
                        return Err(error);
                    }
                    last = Some(error);
                }
            }
        }
        Err(last.unwrap_or_else(|| {
            Error::Usage(format!("no IP location service is configured ({ACCEPTED})"))
        }))
    }

    /// One service's answer: from the cache while it is fresh, from the network otherwise.
    ///
    /// The key is the service's own, so an answer `ipapi.co` produced is never later served as if
    /// `ipwho.is` had produced it.
    fn locate_via(&self, service: IpService) -> Result<Location> {
        let key = CacheKey::ip(service.slug());
        let body: Value = self
            .cache
            .read_or_fetch_json(&key, self.ttl, service.slug(), || {
                let request = HttpRequest::get(service.endpoint());
                let response = self.http.send(&request)?;
                Ok((response.status(), response.body().to_owned()))
            })?;
        match service {
            IpService::IpWhoIs => from_ipwho_is(service, &body),
            IpService::IpApiCo => from_ipapi_co(service, &body),
        }
    }
}

impl IpLocator for IpLocatorChain<'_> {
    /// The location, without the detail the disclosure line needs.
    fn locate(&self) -> Result<Location> {
        self.locate_with_service().map(|(location, _)| location)
    }
}

/// Translates an `ipwho.is` answer.
///
/// Its `success: false` envelope is a refusal with a reason, not a location: it becomes
/// [`Error::Upstream`] so the chain moves on to the fallback instead of inventing a place.
fn from_ipwho_is(service: IpService, body: &Value) -> Result<Location> {
    let answer: IpWhoIsAnswer = decode(service, body)?;
    if !answer.success {
        return Err(envelope_error(service, None, answer.message.as_deref()));
    }
    Ok(Location {
        name: required(service, "city", text(answer.city))?,
        admin1: text(answer.region),
        country: text(answer.country).unwrap_or_default(),
        country_code: text(answer.country_code),
        lat: required(service, "latitude", answer.latitude)?,
        lon: required(service, "longitude", answer.longitude)?,
        tz: zone(service, answer.timezone.and_then(|timezone| timezone.id))?,
        elevation_m: None,
        population: None,
        source: LocationSource::Ip,
    })
}

/// Translates an `ipapi.co` answer.
///
/// The trap here is the field naming: `country` holds the two-letter *code* and `country_name` the
/// display name, the opposite of what the two names suggest at a glance. The `{ip}/json/` spelling
/// of the endpoint sends only `country_code`, so that spelling is accepted for the code as well.
fn from_ipapi_co(service: IpService, body: &Value) -> Result<Location> {
    let answer: IpApiCoAnswer = decode(service, body)?;
    if answer.error == Some(true) {
        let reason = answer.reason.as_deref();
        let message = answer.message.as_deref();
        return Err(envelope_error(service, reason, message));
    }
    Ok(Location {
        name: required(service, "city", text(answer.city))?,
        admin1: text(answer.region),
        country: text(answer.country_name).unwrap_or_default(),
        country_code: text(answer.country).or_else(|| text(answer.country_code)),
        lat: required(service, "latitude", answer.latitude)?,
        lon: required(service, "longitude", answer.longitude)?,
        tz: zone(service, answer.timezone)?,
        elevation_m: None,
        population: None,
        source: LocationSource::Ip,
    })
}

/// The `ipwho.is` answer, reduced to the fields that become a [`Location`].
///
/// Every other key the service sends — `continent`, `flag`, `connection`, `utc_offset`, … — is
/// ignored on purpose: nothing downstream reads it.
#[derive(Debug, Deserialize)]
struct IpWhoIsAnswer {
    /// `false` for an address the service will not place, with `message` saying why.
    success: bool,
    /// The refusal's reason; only read when `success` is `false`.
    message: Option<String>,
    /// City name, e.g. `Beijing`.
    city: Option<String>,
    /// Region or province name, e.g. `Beijing`.
    region: Option<String>,
    /// Country name, e.g. `China`.
    country: Option<String>,
    /// ISO 3166-1 alpha 2 country code, e.g. `CN`.
    country_code: Option<String>,
    /// Latitude in degrees, WGS 84.
    latitude: Option<f64>,
    /// Longitude in degrees, WGS 84.
    longitude: Option<f64>,
    /// The zone object; only its IANA name is used.
    timezone: Option<IpWhoIsTimezone>,
}

/// The `timezone` object of an `ipwho.is` answer.
#[derive(Debug, Deserialize)]
struct IpWhoIsTimezone {
    /// IANA zone name, e.g. `Asia/Shanghai`.
    id: Option<String>,
}

/// The `ipapi.co` answer, reduced to the fields that become a [`Location`].
#[derive(Debug, Deserialize)]
struct IpApiCoAnswer {
    /// City name, e.g. `Beijing`.
    city: Option<String>,
    /// Region or province name, e.g. `Beijing`.
    region: Option<String>,
    /// ISO 3166-1 alpha 2 country *code* — the display name is `country_name`.
    country: Option<String>,
    /// The same code under the name the `{ip}/json/` spelling of the endpoint uses.
    country_code: Option<String>,
    /// Country display name, e.g. `China`.
    country_name: Option<String>,
    /// Latitude in degrees, WGS 84.
    latitude: Option<f64>,
    /// Longitude in degrees, WGS 84.
    longitude: Option<f64>,
    /// IANA zone name, e.g. `Asia/Shanghai`.
    timezone: Option<String>,
    /// `true` when the answer is a refusal rather than a location.
    error: Option<bool>,
    /// The refusal's short reason, e.g. `RateLimited`.
    reason: Option<String>,
    /// The refusal's sentence, when it sends one.
    message: Option<String>,
}

/// The one error shape this module reports: the service's host, no HTTP status — an answer did
/// arrive, it was simply unusable — and the detail.
fn upstream(service: IpService, message: impl Into<String>) -> Error {
    Error::Upstream {
        provider: service.label().to_owned(),
        status: None,
        message: message.into(),
    }
}

/// Decodes a body into `T`, blaming the service rather than the cache when the shape is not the
/// documented one: the user needs to hear which provider misbehaved.
fn decode<T: DeserializeOwned>(service: IpService, body: &Value) -> Result<T> {
    serde_json::from_value(body.clone()).map_err(|error| shape_error(service, &error))
}

/// The failure of an answer whose shape is not the documented one.
fn shape_error(service: IpService, error: &serde_json::Error) -> Error {
    upstream(
        service,
        format!("the answer is not the documented shape: {error}"),
    )
}

/// A field's text, or `None` when the answer omits it, sends `null` or sends blanks.
fn text(value: Option<impl AsRef<str>>) -> Option<String> {
    value
        .map(|value| value.as_ref().trim().to_owned())
        .filter(|value| !value.is_empty())
}

/// A required field: a missing one names the service and the field instead of taking a default.
fn required<T>(service: IpService, field: &str, value: Option<T>) -> Result<T> {
    let detail = format!("the answer does not report {field}");
    value.ok_or_else(|| upstream(service, detail))
}

/// The failure an answer's own error envelope describes, quoting the service's first non-empty
/// detail.
fn envelope_error(service: IpService, reason: Option<&str>, message: Option<&str>) -> Error {
    let detail = text(reason)
        .or_else(|| text(message))
        .unwrap_or_else(|| "no reason given".to_owned());
    upstream(service, detail)
}

/// The IANA zone of an answer, parsed; an unknown name is an error rather than a silent `UTC`.
fn zone(service: IpService, name: Option<String>) -> Result<Tz> {
    let name = required(service, "timezone", text(name))?;
    let detail = format!("the timezone {name:?} is not an IANA zone name");
    Tz::from_str(&name).map_err(|_| upstream(service, detail))
}
