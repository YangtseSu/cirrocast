// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! `MeteoAlarm`: the EUMETNET members' warning service.
//!
//! The EDR collection exposes warnings per **country**, not per point
//! (`…/collections/warnings/locations/{COUNTRY}`), so a point query is emulated: the country comes
//! from the resolved location's ISO 3166-1 alpha-2 code, each returned feature's geometry is
//! tested against the point client-side, and the surviving features' `hubLink` documents are the
//! actual CAP 1.2 payloads. `datetime` is a required query parameter, so the request always carries
//! a 24-hour window around now; the liveness filter still runs against the CAP `expires`/`ends`.
//!
//! The endpoints are protected (`401` without a token) and access is intended for `MeteoAlarm`
//! members and re-distributors, so the token is **optional BYOK** (`CIRROCAST_METEOALARM_KEY`,
//! never `keys.toml` because the service is not a weather provider): without it the source is
//! skipped with a `--verbose` note and the global aggregators still answer. A token that is
//! present but rejected fails the source, which in auto-selection mode is a note, not an error.

use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::Value;

use super::{Alert, AlertSource};
use crate::error::{Error, Result};
use crate::http::HttpRequest;
use crate::model::Location;
use crate::provider::Env;

/// The environment variable that may carry the bearer token.
const TOKEN_ENV: &str = "CIRROCAST_METEOALARM_KEY";

/// The EDR collection root; the country code is appended.
const ENDPOINT: &str = "https://api.meteoalarm.org/edr/v1/collections/warnings/locations";

/// The language every request asks for; CAP `info` blocks are picked by locale afterwards.
const LANGUAGE: &str = "en-GB";

/// The window half-width around now for both the `datetime` (sent) and `active` intervals.
const WINDOW: Duration = Duration::from_hours(24);

/// Fetches the warnings whose area geometry contains the point.
pub fn fetch(loc: &Location, env: &Env<'_>, language: &str) -> Result<Vec<Alert>> {
    let Some(token) = token() else {
        if env.verbose > 0 {
            eprintln!("alerts: meteoalarm: no {TOKEN_ENV} configured; skipping");
        }
        return Ok(Vec::new());
    };
    let Some(country) = loc.country_code.as_deref() else {
        return Ok(Vec::new());
    };
    let now: DateTime<Utc> = env.cache.clock().now().into();
    let window = format!(
        "{}/{}",
        (now - WINDOW).format("%Y-%m-%dT%H:%M:%SZ"),
        (now + WINDOW).format("%Y-%m-%dT%H:%M:%SZ")
    );
    let request = HttpRequest::get(format!("{ENDPOINT}/{}", country.to_ascii_uppercase()))
        .query("datetime", window.clone())
        .query("active", window)
        .query("language", LANGUAGE)
        .header("Authorization", format!("Bearer {token}"))
        .secret(token);
    let key = super::key(env, AlertSource::MeteoAlarm.as_str(), loc);
    let body = super::cached_text(
        env,
        AlertSource::MeteoAlarm,
        &request,
        &key,
        "country warnings",
    )
    .map_err(token_error)?;
    let features = index(&body)?;

    let mut alerts = Vec::new();
    let mut first_error: Option<Error> = None;
    for feature in features {
        if super::geometry::geojson_contains(feature.geometry.as_ref(), loc.lat, loc.lon)
            == Some(false)
        {
            continue;
        }
        match document(&feature, env, language) {
            Ok(mut found) => alerts.append(&mut found),
            Err(error) => {
                if env.verbose > 0 {
                    eprintln!("alerts: meteoalarm: {error}");
                }
                first_error.get_or_insert(error);
            }
        }
    }
    if alerts.is_empty()
        && let Some(error) = first_error
    {
        return Err(error);
    }
    Ok(alerts)
}

/// The bearer token, trimmed; empty or unset means "source skipped".
fn token() -> Option<String> {
    let value = std::env::var(TOKEN_ENV).ok()?;
    let value = value.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_owned())
    }
}

/// A rejected token names the variable, not `cirrocast key set`: the token has no key-store entry
/// (`MeteoAlarm` is not a weather provider), so the generic invalid-key advice would send the user
/// to a command that does not know the service.
fn token_error(error: Error) -> Error {
    match error {
        Error::InvalidKey { status, .. } => Error::InvalidToken {
            provider: AlertSource::MeteoAlarm.as_str().to_owned(),
            var: TOKEN_ENV.to_owned(),
            status,
        },
        other => other,
    }
}

/// Fetches and normalises one feature's `hubLink` document.
fn document(feature: &Feature, env: &Env<'_>, language: &str) -> Result<Vec<Alert>> {
    let Some(link) = feature.properties.hub_link.as_deref() else {
        return Err(super::upstream(
            AlertSource::MeteoAlarm,
            "a feature carries no hubLink",
        ));
    };
    let url = hub_url(link);
    let identifier = feature
        .properties
        .alert_id
        .as_deref()
        .unwrap_or(link)
        .to_owned();
    let key = super::document_key(AlertSource::MeteoAlarm, &identifier);
    let request = HttpRequest::get(url);
    let body = super::cached_text(env, AlertSource::MeteoAlarm, &request, &key, "CAP document")?;
    let document = super::cap::parse_cap(&body, AlertSource::MeteoAlarm)?;
    super::cap::alerts_from_cap(&document, AlertSource::MeteoAlarm, language)
}

/// One feature of the EDR collection, reduced to the fields the adapter reads.
#[derive(Debug, Deserialize)]
struct Feature {
    #[serde(default)]
    geometry: Option<Value>,
    properties: Properties,
}

/// The feature properties.
#[derive(Debug, Deserialize)]
struct Properties {
    #[serde(rename = "alertId", default)]
    alert_id: Option<String>,
    #[serde(rename = "hubLink", default)]
    hub_link: Option<String>,
}

/// The feature collection.
#[derive(Debug, Deserialize)]
struct Envelope {
    #[serde(default)]
    features: Vec<Feature>,
}

/// Decodes the EDR feature collection.
fn index(body: &str) -> Result<Vec<Feature>> {
    let envelope: Envelope = serde_json::from_str(body).map_err(|error| {
        super::upstream(
            AlertSource::MeteoAlarm,
            format!("the country warnings response does not parse as GeoJSON: {error}"),
        )
    })?;
    Ok(envelope.features)
}

/// A `hubLink` as an absolute URL: the service usually answers an absolute one, and a relative
/// path is resolved against the public API host.
fn hub_url(link: &str) -> String {
    let link = link.trim();
    if link.starts_with("http://") || link.starts_with("https://") {
        link.to_owned()
    } else {
        format!(
            "https://api.meteoalarm.org/{}",
            link.trim_start_matches('/')
        )
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{hub_url, index};

    #[test]
    fn the_index_carries_geometry_and_the_hub_link() {
        let body = json!({
            "type": "FeatureCollection",
            "features": [
                {
                    "type": "Feature",
                    "geometry": {
                        "type": "Polygon",
                        "coordinates": [[[16.0, 48.0], [16.6, 48.0], [16.6, 48.4], [16.0, 48.4], [16.0, 48.0]]]
                    },
                    "properties": {
                        "alertId": "AT-2026-001",
                        "countryCode": "AT",
                        "hubLink": "https://api.meteoalarm.org/cap/AT-2026-001.xml"
                    }
                },
                {
                    "type": "Feature",
                    "properties": { "alertId": "AT-2026-002", "hubLink": "cap/AT-2026-002.xml" }
                }
            ]
        })
        .to_string();
        let features = index(&body).expect("the index decodes");
        assert_eq!(features.len(), 2);
        assert!(features[0].geometry.is_some());
        assert!(features[1].geometry.is_none());
        assert_eq!(
            features[0].properties.hub_link.as_deref(),
            Some("https://api.meteoalarm.org/cap/AT-2026-001.xml")
        );
    }

    #[test]
    fn hub_links_are_made_absolute() {
        assert_eq!(
            hub_url("https://api.meteoalarm.org/cap/x.xml"),
            "https://api.meteoalarm.org/cap/x.xml"
        );
        assert_eq!(hub_url("cap/x.xml"), "https://api.meteoalarm.org/cap/x.xml");
        assert_eq!(
            hub_url("/cap/x.xml"),
            "https://api.meteoalarm.org/cap/x.xml"
        );
    }

    #[test]
    fn a_malformed_index_is_an_upstream_error() {
        let error = index("not json").unwrap_err();
        assert!(error.to_string().contains("meteoalarm"), "{error}");
    }
}
