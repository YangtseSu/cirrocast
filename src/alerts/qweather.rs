// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! `QWeather`'s weather-alert API: the China source, reusing the forecast provider's credential.
//!
//! `{host}/weatheralert/v1/current/{lat}/{lon}` is the endpoint this source speaks. The v7
//! spelling it used first — `/weatheralert/v7/alert/now?location=<lon>,<lat>` — is retired: the
//! account host answers it with `404` and an empty body (observed 2026-10-06, re-verified
//! 2026-10-07) while the v1 path answers `200` with the documented `metadata`/`alerts` envelope.
//! The payload carries CAP's severity, urgency and certainty value sets verbatim, so the mapping is
//! direct, and `messageType.code` says whether an entry is an initial `alert`, an `update` of
//! earlier ones, or a `cancel` of them.
//!
//! The credential and host are the `qweather` provider's (`providers.qweather.host` and whichever
//! of the API key / JWT the key store resolves, step 27); the source is only selected when that
//! provider is on the chain. The header comes from the same `QWeatherAuth` helper the forecast
//! backend uses, so both follow the configured authentication mode with no second code path.
//!
//! Two rules are easy to undo by accident:
//!
//! * **A `cancel` entry is dropped, not shown.** The endpoint answers the *active* set, and a
//!   cancellation entry only names the ids it supersedes — alerts this client never displayed.
//! * **The cache key carries the API generation** ([`CACHE_SOURCE`]). The key is one entry per
//!   source, place and UTC hour, so a body the retired v7 path left behind in the same hour must
//!   never reach this decoder: it has `warning[]`, not `alerts[]`.
//!
//! The response's `metadata.attributions` — the issuing agency plus a standing disclaimer — travel
//! on every decoded alert and are printed by [`crate::alerts::credits`] beside the registry's own
//! credit line: `QWeather`'s attribution terms require the list shown in full and unmodified wherever
//! its warning data is shown (`https://dev.qweather.com/docs/terms/attribution/`).

use serde::Deserialize;

use super::cap::instant;
use super::{Alert, AlertSource, Certainty, Severity, Urgency};
use crate::auth::QWeatherAuth;
use crate::error::{Error, Result};
use crate::http::HttpRequest;
use crate::model::Location;
use crate::provider::{Env, ProviderId};

/// The provider id whose credential and host this source borrows.
const PROVIDER: &str = "qweather";

/// The cache-key discriminator: the source id plus the API generation it speaks.
const CACHE_SOURCE: &str = "qweather-v1";

/// Fetches the active warnings for the point.
pub fn fetch(loc: &Location, env: &Env<'_>, _language: &str) -> Result<Vec<Alert>> {
    let host = env
        .config
        .providers
        .qweather
        .host
        .trim()
        .trim_end_matches('/');
    if host.is_empty() {
        return Err(Error::Config(format!(
            "alert source `qweather` needs `providers.{PROVIDER}.host` (see \
             https://console.qweather.com/)"
        )));
    }
    let variable = ProviderId::QWeather
        .metadata()
        .key_env
        .unwrap_or("CIRROCAST_QWEATHER_KEY");
    // Step 27: the same store resolution and the same header helper the forecast backend uses, so
    // the alert request follows the configured mode with no second code path.
    let credential = env
        .keys
        .credential(ProviderId::QWeather)?
        .ok_or_else(|| crate::provider::missing_credential(ProviderId::QWeather, variable))?;
    let auth = QWeatherAuth::resolve(&credential, env.cache.clock().now())?;

    let request = auth.apply(HttpRequest::get(format!(
        "{host}/weatheralert/v1/current/{:.4}/{:.4}",
        loc.lat, loc.lon
    )));
    let cache_key = super::key(env, CACHE_SOURCE, loc);
    let body = super::cached_text(
        env,
        AlertSource::QWeather,
        &request,
        &cache_key,
        "active alerts",
    )?;
    decode(&body)
}

/// The response envelope: `metadata` (`tag`, `attributions`, `zeroResult`) plus the active alerts.
///
/// `alerts` is required: it is always present — empty when nothing is in force, which
/// `metadata.zeroResult` merely repeats — so a payload of another shape (the retired v7 `warning[]`
/// body, say) becomes an error instead of a silently empty answer. `metadata.attributions` is the
/// list `QWeather`'s attribution terms require displayed in full and unmodified with the data; it
/// travels on every alert of the response and is printed by [`crate::alerts::credits`].
#[derive(Debug, Deserialize)]
struct Response {
    #[serde(default)]
    metadata: Metadata,
    alerts: Vec<Entry>,
}

/// The response's `metadata` object, in the subset this source consumes.
#[derive(Debug, Default, Deserialize)]
struct Metadata {
    /// The attribution lines the terms require shown with the data, verbatim.
    #[serde(default)]
    attributions: Vec<String>,
}

/// One `alerts[]` entry.
#[derive(Debug, Deserialize)]
struct Entry {
    #[serde(default)]
    id: String,
    #[serde(rename = "senderName", default)]
    sender_name: Option<String>,
    #[serde(rename = "issuedTime", default)]
    issued_time: Option<String>,
    #[serde(rename = "messageType", default)]
    message_type: Option<MessageType>,
    #[serde(rename = "eventType", default)]
    event_type: Option<EventType>,
    #[serde(default)]
    urgency: Option<String>,
    #[serde(default)]
    severity: Option<String>,
    #[serde(default)]
    certainty: Option<String>,
    #[serde(rename = "effectiveTime", default)]
    effective_time: Option<String>,
    #[serde(rename = "onsetTime", default)]
    onset_time: Option<String>,
    #[serde(rename = "expireTime", default)]
    expire_time: Option<String>,
    #[serde(default)]
    headline: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    instruction: Option<String>,
    #[serde(default)]
    color: Option<Color>,
}

/// `messageType`: the entry's nature (`alert`, `update`, `cancel`) and the ids it supersedes.
#[derive(Debug, Deserialize)]
struct MessageType {
    #[serde(default)]
    code: Option<String>,
}

/// `eventType`: the event's name and `QWeather`'s own event code.
#[derive(Debug, Deserialize)]
struct EventType {
    #[serde(default)]
    name: Option<String>,
}

/// The nested colour object: the palette entry for the event.
#[derive(Debug, Deserialize)]
struct Color {
    #[serde(default)]
    code: Option<String>,
}

impl Entry {
    /// Whether the entry cancels earlier alerts rather than describing an active one.
    fn is_cancellation(&self) -> bool {
        self.message_type
            .as_ref()
            .and_then(|kind| kind.code.as_deref())
            .is_some_and(|code| code.trim().eq_ignore_ascii_case("cancel"))
    }

    /// The event name: `eventType.name`, else the headline; `None` when both are empty.
    fn event_name(&self) -> Option<&str> {
        self.event_type
            .as_ref()
            .and_then(|event| event.name.as_deref())
            .or(self.headline.as_deref())
            .map(str::trim)
            .filter(|text| !text.is_empty())
    }

    /// The headline, when the payload carries one.
    fn headline(&self) -> Option<&str> {
        self.headline
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty())
    }
}

/// Decodes one alert response.
///
/// The endpoint answers `200` with `{"metadata":{…},"alerts":[]}` when nothing is active, which is
/// an empty list and not an error; a body of another shape is an upstream error.
pub fn decode(body: &str) -> Result<Vec<Alert>> {
    let response: Response = serde_json::from_str(body).map_err(|error| {
        super::upstream(
            AlertSource::QWeather,
            format!("the alert response does not parse as JSON: {error}"),
        )
    })?;
    let mut alerts = Vec::new();
    // The response's attribution lines are the same for every alert it carries: they are the
    // publisher's, not the event's, and the terms ask for them shown in full wherever the data is.
    let credit: Vec<String> = response
        .metadata
        .attributions
        .into_iter()
        .filter(|line| !line.trim().is_empty())
        .collect();
    for entry in response.alerts {
        if entry.is_cancellation() {
            continue;
        }
        let Some(event) = entry.event_name() else {
            continue;
        };
        let id = if entry.id.trim().is_empty() {
            format!(
                "qweather|{event}|{}",
                entry.issued_time.as_deref().unwrap_or_default()
            )
        } else {
            entry.id.trim().to_owned()
        };
        alerts.push(Alert {
            id,
            source: AlertSource::QWeather,
            event: event.to_owned(),
            severity: severity_of(&entry),
            urgency: Urgency::from_cap(entry.urgency.as_deref().unwrap_or_default()),
            certainty: Certainty::from_cap(entry.certainty.as_deref().unwrap_or_default()),
            onset: entry
                .onset_time
                .as_deref()
                .or(entry.effective_time.as_deref())
                .or(entry.issued_time.as_deref())
                .and_then(instant),
            expires: entry.expire_time.as_deref().and_then(instant),
            ends: None,
            areas: Vec::new(),
            headline: entry.headline().unwrap_or(event).to_owned(),
            description: entry
                .description
                .as_deref()
                .map(strip_html)
                .filter(|text| non_empty(text)),
            instruction: entry
                .instruction
                .as_deref()
                .map(strip_html)
                .filter(|text| non_empty(text)),
            sender: entry
                .sender_name
                .clone()
                .filter(|text| !text.trim().is_empty()),
            credit: credit.clone(),
        });
    }
    Ok(alerts)
}

/// The severity: the explicit CAP field, else the warning colour when the field is absent.
fn severity_of(entry: &Entry) -> Severity {
    if let Some(severity) = entry
        .severity
        .as_deref()
        .filter(|text| !text.trim().is_empty())
    {
        return Severity::from_cap(severity);
    }
    let colour = entry
        .color
        .as_ref()
        .and_then(|color| color.code.as_deref())
        .unwrap_or_default();
    severity_of_colour(colour)
}

/// The Chinese warning-colour ladder — blue < yellow < orange < red — for an entry whose `severity`
/// field is absent.
///
/// The v1 palette is wider (white, gray, green, blue, yellow, amber, orange, red, purple, black)
/// and the ordering of the colours beyond the ladder is not documented, so only its four rungs are
/// mapped: guessing a severity from `amber` or `purple` would be an invention, and
/// [`Severity::Unknown`] is the honest answer.
fn severity_of_colour(code: &str) -> Severity {
    match code.trim().to_ascii_lowercase().as_str() {
        "blue" => Severity::Minor,
        "yellow" => Severity::Moderate,
        "orange" => Severity::Severe,
        "red" => Severity::Extreme,
        _ => Severity::Unknown,
    }
}

/// `true` when the text has non-whitespace content.
fn non_empty(text: &str) -> bool {
    !text.trim().is_empty()
}

/// Strips the simple HTML tags `QWeather` puts into alert text (`<br/>`, `<p>`), so the model
/// never carries markup into the banner. Entities are left as-is: the API sends plain text with
/// occasional tags, not a full HTML document.
fn strip_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_tag = false;
    for character in text.chars() {
        match character {
            '<' => in_tag = true,
            '>' => {
                in_tag = false;
                out.push(' ');
            }
            _ if !in_tag => out.push(character),
            _ => {}
        }
    }
    out.trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::decode;
    use crate::model::{AlertSource, Certainty, Severity, Urgency};

    /// A payload in the v1 shape: a full triple with markup in the description, an entry whose
    /// severity is only in the colour, and a cancellation.
    const RESPONSE: &str = r#"{
      "metadata": {
        "tag": "163d1fcc",
        "attributions": ["北京市气象台", "当前预警数据可能存在延迟或信息过时，以官方数据发布为准。"],
        "zeroResult": false
      },
      "alerts": [
        {
          "id": "20261003120001",
          "senderName": "北京市气象台",
          "issuedTime": "2026-10-03T12:00+08:00",
          "messageType": { "code": "alert", "supersedes": [] },
          "eventType": { "name": "暴雨", "code": "1003" },
          "urgency": "immediate",
          "severity": "severe",
          "certainty": "likely",
          "icon": "1003",
          "color": { "code": "orange", "red": 255, "green": 165, "blue": 0, "alpha": 1 },
          "effectiveTime": "2026-10-03T12:00+08:00",
          "onsetTime": "2026-10-03T12:00+08:00",
          "expireTime": "2026-10-03T20:00+08:00",
          "headline": "北京市气象台发布暴雨橙色预警",
          "description": "预计未来6小时<BR/>降雨量将达50毫米以上。",
          "criteria": "3小时降雨量将达50毫米以上。",
          "responseTypes": [],
          "instruction": "1. 政府及相关部门按照职责做好防暴雨准备工作。\n2. 学校、幼儿园采取适当措施。"
        },
        {
          "id": "20261003110002",
          "senderName": "北京市气象台",
          "issuedTime": "2026-10-03T11:00+08:00",
          "messageType": { "code": "alert", "supersedes": [] },
          "eventType": { "name": "大风", "code": "1006" },
          "urgency": null,
          "severity": null,
          "certainty": null,
          "color": { "code": "blue" },
          "effectiveTime": "2026-10-03T11:00+08:00",
          "onsetTime": null,
          "expireTime": "2026-10-03T23:00+08:00",
          "headline": "北京市气象台发布大风蓝色预警",
          "description": null
        },
        {
          "id": "20261003070003",
          "senderName": "北京市气象台",
          "issuedTime": "2026-10-03T07:00+08:00",
          "messageType": { "code": "cancel", "supersedes": ["20261003060000"] },
          "eventType": { "name": "高温", "code": "1001" },
          "severity": "minor",
          "headline": "北京市气象台解除高温黄色预警",
          "expireTime": "2026-10-03T08:00+08:00"
        }
      ]
    }"#;

    /// The zero-result envelope, recorded from the live service on 2026-10-07.
    const ZERO: &str = r#"{"metadata":{"tag":"08c9a146a2e4a1fb851fb073aafebeaaeb3a9f3942ed393778b10d9708497be1","zeroResult":true},"alerts":[]}"#;

    #[test]
    fn an_alert_maps_its_severity_triple_and_strips_markup() {
        let alerts = decode(RESPONSE).expect("the response decodes");
        assert_eq!(alerts.len(), 2, "the cancellation is dropped");
        let first = &alerts[0];
        assert_eq!(first.source, AlertSource::QWeather);
        assert_eq!(first.event, "暴雨");
        assert_eq!(first.severity, Severity::Severe);
        assert_eq!(first.urgency, Urgency::Immediate);
        assert_eq!(first.certainty, Certainty::Likely);
        assert_eq!(first.sender.as_deref(), Some("北京市气象台"));
        assert_eq!(first.headline, "北京市气象台发布暴雨橙色预警");
        let description = first.description.as_deref().unwrap_or_default();
        assert!(!description.contains('<'), "{description}");
        assert!(description.contains("50毫米"), "{description}");
        assert!(
            first
                .instruction
                .as_deref()
                .unwrap_or_default()
                .contains("防暴雨"),
            "{:?}",
            first.instruction
        );
        assert_eq!(
            first.expires.expect("an expiry").to_rfc3339(),
            "2026-10-03T20:00:00+08:00"
        );
    }

    #[test]
    fn the_colour_supplies_the_severity_when_the_field_is_absent() {
        let alerts = decode(RESPONSE).expect("the response decodes");
        let second = &alerts[1];
        assert_eq!(second.severity, Severity::Minor, "blue is the lowest rung");
        assert_eq!(
            second.urgency,
            Urgency::Unknown,
            "a null urgency is unknown"
        );
        assert_eq!(second.certainty, Certainty::Unknown);
        assert_eq!(second.description, None);
        assert_eq!(
            second
                .onset
                .expect("the effective time stands in for a null onset")
                .to_rfc3339(),
            "2026-10-03T11:00:00+08:00"
        );
    }

    #[test]
    fn an_event_falls_back_to_the_headline_and_an_empty_list_is_fine() {
        let fallback = r#"{"alerts":[{"headline":"某地解除大风预警","severity":"minor"}]}"#;
        let alerts = decode(fallback).expect("the response decodes");
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].event, "某地解除大风预警");
        assert_eq!(alerts[0].id, "qweather|某地解除大风预警|");

        assert_eq!(
            decode(ZERO).expect("zero results decode").len(),
            0,
            "the recorded zero-result envelope is an empty list, not an error"
        );
    }

    #[test]
    fn the_response_attributions_travel_with_every_alert() {
        let alerts = decode(RESPONSE).expect("the response decodes");
        let expected = [
            "北京市气象台",
            "当前预警数据可能存在延迟或信息过时，以官方数据发布为准。",
        ];
        for alert in &alerts {
            assert_eq!(alert.credit, expected, "verbatim, on every alert");
        }
        // The recorded zero-result envelope carries no attributions of its own.
        assert_eq!(decode(ZERO).expect("zero results decode").len(), 0);
    }

    #[test]
    fn a_payload_of_another_shape_is_an_error() {
        // The retired v7 body: `warning[]` with no `alerts`, which must not read as "no warnings".
        let error = decode(r#"{"code":"200","warning":[]}"#).unwrap_err();
        assert!(error.to_string().contains("does not parse"), "{error}");
        assert_eq!(error.exit_code(), 3);
    }
}
