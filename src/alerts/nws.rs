// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The US National Weather Service alert feed: CAP documents in `GeoJSON` clothing.
//!
//! `https://api.weather.gov/alerts/active?point=<lat>,<lon>` answers a `GeoJSON` `FeatureCollection`
//! whose `features[].properties` carry the CAP alert fields directly (`NWS` is a CAP-native
//! publisher; the envelope is `GeoJSON`, the payload is not). The adapter reads those properties and
//! normalises them, so no CAP XML parser is involved for this source.
//!
//! Two endpoint quirks are worth knowing:
//!
//! * `limit` is not sent: the endpoint rejects unknown parameters with `400`, so there is no
//!   client-side paging to do;
//! * the request must carry a descriptive `User-Agent` (the `NWS` usage policy), which every
//!   request in this crate already does through [`crate::http::UA`].
//!
//! `status = Test` and `messageType = Cancel` records are dropped: neither is a warning in force.

use serde::Deserialize;

use super::cap::instant;
use super::{Alert, AlertSource, Certainty, Severity, Urgency};
use crate::error::Result;
use crate::http::HttpRequest;
use crate::model::Location;
use crate::provider::Env;

/// The active-alerts endpoint.
const ENDPOINT: &str = "https://api.weather.gov/alerts/active";

/// Fetches the active warnings for the point.
pub fn fetch(loc: &Location, env: &Env<'_>, _language: &str) -> Result<Vec<Alert>> {
    let request = HttpRequest::get(ENDPOINT)
        .query("point", format!("{:.4},{:.4}", loc.lat, loc.lon))
        .header("Accept", "application/geo+json");
    let key = super::key(env, AlertSource::Nws.as_str(), loc);
    let body = super::cached_text(env, AlertSource::Nws, &request, &key, "active alerts")?;
    decode(&body)
}

/// The recorded envelope.
#[derive(Debug, Deserialize)]
struct Envelope {
    #[serde(default)]
    features: Vec<Feature>,
}

/// One `GeoJSON` feature.
#[derive(Debug, Deserialize)]
struct Feature {
    properties: Props,
}

/// The CAP fields `NWS` repeats inside `properties`.
#[derive(Debug, Deserialize)]
struct Props {
    #[serde(default)]
    id: String,
    #[serde(default)]
    event: String,
    #[serde(default)]
    severity: Option<String>,
    #[serde(default)]
    urgency: Option<String>,
    #[serde(default)]
    certainty: Option<String>,
    #[serde(default)]
    onset: Option<String>,
    #[serde(default)]
    expires: Option<String>,
    #[serde(default)]
    ends: Option<String>,
    #[serde(rename = "areaDesc", default)]
    area_desc: Option<String>,
    #[serde(default)]
    headline: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    instruction: Option<String>,
    #[serde(rename = "senderName", default)]
    sender_name: Option<String>,
    #[serde(rename = "messageType", default)]
    message_type: Option<String>,
    #[serde(default)]
    status: Option<String>,
}

/// Decodes one `alerts/active` envelope.
///
/// A body that does not parse is an upstream error; an individual record that cannot be named
/// (`event` empty) is skipped rather than failing the feed.
pub fn decode(body: &str) -> Result<Vec<Alert>> {
    let envelope: Envelope = serde_json::from_str(body).map_err(|error| {
        super::upstream(
            AlertSource::Nws,
            format!("the active-alerts response does not parse as GeoJSON: {error}"),
        )
    })?;
    Ok(envelope
        .features
        .into_iter()
        .filter_map(|feature| feature.properties.into_alert())
        .collect())
}

impl Props {
    /// One record as the model, or `None` when it is not a warning in force.
    fn into_alert(self) -> Option<Alert> {
        if self.message_type.as_deref().is_some_and(is_cancel)
            || self.status.as_deref().is_some_and(is_test)
        {
            return None;
        }
        let event = self.event.trim();
        if event.is_empty() {
            return None;
        }
        let onset = self.onset.as_deref().and_then(instant);
        let area_desc = self.area_desc.unwrap_or_default();
        let id = if self.id.trim().is_empty() {
            format!("nws|{event}|{}", self.onset.as_deref().unwrap_or_default())
        } else {
            self.id.trim().to_owned()
        };
        Some(Alert {
            id,
            source: AlertSource::Nws,
            event: event.to_owned(),
            severity: Severity::from_cap(self.severity.as_deref().unwrap_or_default()),
            urgency: Urgency::from_cap(self.urgency.as_deref().unwrap_or_default()),
            certainty: Certainty::from_cap(self.certainty.as_deref().unwrap_or_default()),
            onset,
            expires: self.expires.as_deref().and_then(instant),
            ends: self.ends.as_deref().and_then(instant),
            areas: split_areas(&area_desc),
            headline: self
                .headline
                .filter(|text| !text.trim().is_empty())
                .unwrap_or_else(|| event.to_owned()),
            description: non_empty(self.description),
            instruction: non_empty(self.instruction),
            sender: non_empty(self.sender_name),
        })
    }
}

/// Whether a `messageType` cancels an alert.
fn is_cancel(value: &str) -> bool {
    value.eq_ignore_ascii_case("cancel")
}

/// Whether a `status` marks a test message.
fn is_test(value: &str) -> bool {
    value.eq_ignore_ascii_case("test")
}

/// `Some(trimmed)` unless the text is missing or blank.
fn non_empty(value: Option<String>) -> Option<String> {
    value.filter(|text| !text.trim().is_empty())
}

/// `NWS` joins affected areas with `; ` (`Navarro, TX; ...`); an empty string means no area.
fn split_areas(area_desc: &str) -> Vec<String> {
    let mut areas = Vec::new();
    for area in area_desc.split(';') {
        let area = area.trim();
        if !area.is_empty() && !areas.iter().any(|seen: &String| seen == area) {
            areas.push(area.to_owned());
        }
    }
    areas
}

#[cfg(test)]
mod tests {
    use super::decode;
    use crate::model::{AlertSource, Certainty, Severity, Urgency};

    /// A hand-trimmed `NWS` envelope: one live tornado warning and one cancellation.
    const ENVELOPE: &str = r#"{
      "type": "FeatureCollection",
      "features": [
        {
          "properties": {
            "@type": "wx:Alert",
            "id": "urn:oid:2.49.0.1.840.0.tornado.001",
            "areaDesc": "Cleveland, OK; McClain, OK",
            "sent": "2026-10-03T07:37:00-05:00",
            "onset": "2026-10-03T07:37:00-05:00",
            "expires": "2026-10-03T08:15:00-05:00",
            "ends": "2026-10-03T08:15:00-05:00",
            "status": "Actual",
            "messageType": "Alert",
            "severity": "Extreme",
            "certainty": "Observed",
            "urgency": "Immediate",
            "event": "Tornado Warning",
            "sender": "w-nws.webmaster@noaa.gov",
            "senderName": "NWS Norman OK",
            "headline": "Tornado Warning issued October 3 at 7:37AM CDT until October 3 at 8:15AM CDT by NWS Norman OK",
            "description": "At 737 AM CDT, a confirmed tornado was located near Norman.",
            "instruction": "Take shelter now."
          }
        },
        {
          "properties": {
            "id": "urn:oid:2.49.0.1.840.0.tornado.000",
            "areaDesc": "Cleveland, OK",
            "event": "Tornado Warning",
            "messageType": "Cancel",
            "severity": "Extreme",
            "status": "Actual"
          }
        },
        {
          "properties": {
            "id": "urn:oid:2.49.0.1.840.0.test.001",
            "areaDesc": "Nowhere",
            "event": "Test Warning",
            "messageType": "Alert",
            "status": "Test",
            "severity": "Minor"
          }
        }
      ]
    }"#;

    #[test]
    fn a_live_warning_maps_every_field() {
        let alerts = decode(ENVELOPE).expect("the envelope decodes");
        assert_eq!(alerts.len(), 1);
        let alert = &alerts[0];
        assert_eq!(alert.source, AlertSource::Nws);
        assert_eq!(alert.event, "Tornado Warning");
        assert_eq!(alert.severity, Severity::Extreme);
        assert_eq!(alert.urgency, Urgency::Immediate);
        assert_eq!(alert.certainty, Certainty::Observed);
        assert_eq!(alert.areas, ["Cleveland, OK", "McClain, OK"]);
        assert_eq!(alert.sender.as_deref(), Some("NWS Norman OK"));
        assert!(alert.headline.contains("Tornado Warning"));
        assert!(alert.description.is_some());
        assert_eq!(alert.instruction.as_deref(), Some("Take shelter now."));
        assert!(alert.ends.is_some());
    }

    #[test]
    fn cancellations_and_tests_are_dropped() {
        let alerts = decode(ENVELOPE).expect("the envelope decodes");
        assert!(alerts.iter().all(|alert| alert.event != "Test Warning"));
        assert_eq!(
            alerts
                .iter()
                .filter(|alert| alert.id.contains("tornado.000"))
                .count(),
            0,
            "a cancelled warning is not in force"
        );
    }

    #[test]
    fn an_empty_feature_list_is_an_empty_result() {
        let alerts = decode(r#"{"type": "FeatureCollection", "features": []}"#)
            .expect("the envelope decodes");
        assert_eq!(alerts.len(), 0);
    }

    #[test]
    fn a_malformed_body_is_an_upstream_error() {
        let error = decode("<html>not json</html>").unwrap_err();
        assert!(error.to_string().contains("nws"), "{error}");
    }
}
