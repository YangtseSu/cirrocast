// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! `QWeather`'s weather-alert API: the China source, reusing the forecast provider's credential.
//!
//! `{host}/weatheralert/v7/alert/now?location=<lon>,<lat>` answers the same
//! severity/urgency/certainty triple as CAP, so the mapping is direct. The credential and host are
//! the `qweather` provider's (`providers.qweather.host` and whichever of the API key / JWT the key
//! store resolves, step 27); the source is only selected when that provider is on the chain. The
//! header comes from the same `QWeatherAuth` helper the forecast backend uses, so both follow the
//! configured authentication mode with no second code path.
//!
//! Two documented v7 revisions differ in where the severity colour lives (`severity` versus
//! `color.code`, plus a `severityColor` spelling in older payloads), so all three are accepted in
//! that order.

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

    let request = auth.apply(
        HttpRequest::get(format!("{host}/weatheralert/v7/alert/now"))
            .query("location", format!("{:.4},{:.4}", loc.lon, loc.lat)),
    );
    let cache_key = super::key(env, AlertSource::QWeather.as_str(), loc);
    let body = super::cached_text(
        env,
        AlertSource::QWeather,
        &request,
        &cache_key,
        "active alerts",
    )
    .map_err(not_served)?;
    decode(&body)
}

/// Names the product when the account host does not serve the warning endpoint at all.
///
/// `QWeather`'s Weather Alert service is a separate subscription (and the older `/v7/warning/now`
/// path is retired, answering `403 Deprecated`): an account with weather data but no alert product
/// answers `404` with an empty body here — observed 2026-10-06 — which would otherwise read as a
/// bare status with nothing after it. Every other error keeps its taxonomy.
fn not_served(error: Error) -> Error {
    match error {
        Error::Upstream {
            status: Some(404), ..
        } => Error::Upstream {
            provider: PROVIDER.to_owned(),
            status: Some(404),
            message: "the account host does not serve `/weatheralert/v7/alert/now`; the Weather \
                      Alert service is a separate subscription (check the console), or drop \
                      `qweather` from `[alerts] sources`"
                .to_owned(),
        },
        other => other,
    }
}

/// The response envelope.
#[derive(Debug, Deserialize)]
struct Response {
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    warning: Vec<Warning>,
}

/// One `warning[]` entry.
#[derive(Debug, Deserialize)]
struct Warning {
    #[serde(default)]
    id: String,
    #[serde(default)]
    sender: Option<String>,
    #[serde(rename = "pubTime", default)]
    pub_time: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(rename = "startTime", default)]
    start_time: Option<String>,
    #[serde(rename = "endTime", default)]
    end_time: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    severity: Option<String>,
    #[serde(default)]
    urgency: Option<String>,
    #[serde(default)]
    certainty: Option<String>,
    #[serde(rename = "typeName", default)]
    type_name: Option<String>,
    #[serde(default)]
    text: Option<String>,
    #[serde(rename = "severityColor", default)]
    severity_color: Option<String>,
    #[serde(default)]
    color: Option<Color>,
}

/// The nested colour object newer revisions use.
#[derive(Debug, Deserialize)]
struct Color {
    #[serde(default)]
    code: Option<String>,
}

/// Decodes one alert response.
///
/// A non-`200` body code is an upstream error (except `401`, which is a key problem); the
/// endpoint answers `200` with `{"code":"200","warning":[]}` when nothing is active.
pub fn decode(body: &str) -> Result<Vec<Alert>> {
    let response: Response = serde_json::from_str(body).map_err(|error| {
        super::upstream(
            AlertSource::QWeather,
            format!("the alert response does not parse as JSON: {error}"),
        )
    })?;
    match response.code.as_deref() {
        None | Some("200") => {}
        Some("401") => {
            return Err(Error::InvalidCredential {
                provider: PROVIDER.to_owned(),
                status: 401,
            });
        }
        Some(code) => {
            return Err(super::upstream(
                AlertSource::QWeather,
                format!("the alert API answered code {code}"),
            ));
        }
    }
    let mut alerts = Vec::new();
    for warning in response.warning {
        if warning
            .status
            .as_deref()
            .is_some_and(|status| status.eq_ignore_ascii_case("cancel"))
        {
            continue;
        }
        let event = warning
            .type_name
            .as_deref()
            .or(warning.title.as_deref())
            .map(str::trim)
            .filter(|text| !text.is_empty());
        let Some(event) = event else {
            continue;
        };
        let title = warning.title.as_deref().map(str::trim).unwrap_or_default();
        let id = if warning.id.trim().is_empty() {
            format!(
                "qweather|{event}|{}",
                warning.start_time.as_deref().unwrap_or_default()
            )
        } else {
            warning.id.trim().to_owned()
        };
        alerts.push(Alert {
            id,
            source: AlertSource::QWeather,
            event: event.to_owned(),
            severity: severity_of(&warning),
            urgency: Urgency::from_cap(warning.urgency.as_deref().unwrap_or_default()),
            certainty: Certainty::from_cap(warning.certainty.as_deref().unwrap_or_default()),
            onset: warning
                .start_time
                .as_deref()
                .or(warning.pub_time.as_deref())
                .and_then(instant),
            expires: warning.end_time.as_deref().and_then(instant),
            ends: None,
            areas: Vec::new(),
            headline: if title.is_empty() {
                event.to_owned()
            } else {
                title.to_owned()
            },
            description: warning
                .text
                .as_deref()
                .map(strip_html)
                .filter(|text| non_empty(text)),
            instruction: None,
            sender: warning
                .sender
                .clone()
                .filter(|text| !text.trim().is_empty()),
        });
    }
    Ok(alerts)
}

/// The severity, from the explicit field else the colour code.
fn severity_of(warning: &Warning) -> Severity {
    if let Some(severity) = warning.severity.as_deref()
        && !severity.trim().is_empty()
    {
        return Severity::from_cap(severity);
    }
    let colour = warning
        .color
        .as_ref()
        .and_then(|color| color.code.as_deref())
        .or(warning.severity_color.as_deref())
        .unwrap_or_default();
    severity_of_colour(colour)
}

/// The Chinese warning colour ladder: blue < yellow < orange < red.
fn severity_of_colour(code: &str) -> Severity {
    match code.trim().to_ascii_lowercase().as_str() {
        "blue" | "蓝" => Severity::Minor,
        "yellow" | "黄" => Severity::Moderate,
        "orange" | "橙" => Severity::Severe,
        "red" | "红" => Severity::Extreme,
        _ => Severity::Unknown,
    }
}

/// `true` when the text has non-whitespace content.
fn non_empty(text: &str) -> bool {
    !text.trim().is_empty()
}

/// Strips the simple HTML tags `QWeather` puts into warning text (`<br/>`, `<p>`), so the model
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
    use crate::error::Error;
    use crate::model::{AlertSource, Severity, Urgency};

    /// Hand-written fixtures for the two documented revisions plus a cancellation.
    const RESPONSE: &str = r#"{
      "code": "200",
      "updateTime": "2026-10-03T13:10+08:00",
      "warning": [
        {
          "id": "20261003120001",
          "sender": "北京市气象台",
          "pubTime": "2026-10-03T12:00+08:00",
          "title": "北京市气象台发布暴雨橙色预警",
          "startTime": "2026-10-03T12:00+08:00",
          "endTime": "2026-10-03T20:00+08:00",
          "status": "Active",
          "severity": "Severe",
          "urgency": "Immediate",
          "certainty": "Likely",
          "typeName": "暴雨",
          "text": "预计未来6小时<BR/>降雨量将达50毫米以上。",
          "color": { "code": "Orange" }
        },
        {
          "id": "20261003090002",
          "title": "大风蓝色预警",
          "startTime": "2026-10-03T09:00+08:00",
          "endTime": "2026-10-03T21:00+08:00",
          "status": "Active",
          "typeName": "大风",
          "severityColor": "Blue"
        },
        {
          "id": "20261003070003",
          "title": "高温黄色预警",
          "status": "Cancel",
          "typeName": "高温"
        }
      ]
    }"#;

    #[test]
    fn a_warning_maps_its_severity_triple_and_strips_markup() {
        let alerts = decode(RESPONSE).expect("the response decodes");
        assert_eq!(alerts.len(), 2, "the cancellation is dropped");
        let first = &alerts[0];
        assert_eq!(first.source, AlertSource::QWeather);
        assert_eq!(first.event, "暴雨");
        assert_eq!(first.severity, Severity::Severe);
        assert_eq!(first.urgency, Urgency::Immediate);
        assert_eq!(first.certainty, crate::model::Certainty::Likely);
        assert_eq!(first.sender.as_deref(), Some("北京市气象台"));
        assert!(
            !first
                .description
                .as_deref()
                .unwrap_or_default()
                .contains('<')
        );
        assert!(
            first
                .description
                .as_deref()
                .unwrap_or_default()
                .contains("50毫米")
        );
    }

    #[test]
    fn the_colour_code_supplies_the_severity_when_the_field_is_absent() {
        let alerts = decode(RESPONSE).expect("the response decodes");
        assert_eq!(alerts[1].severity, Severity::Minor);
    }

    #[test]
    fn an_empty_warning_list_is_fine_and_an_error_code_is_not() {
        assert_eq!(
            decode(r#"{"code":"200","warning":[]}"#)
                .expect("an empty list decodes")
                .len(),
            0
        );
        let error = decode(r#"{"code":"401"}"#).unwrap_err();
        assert!(error.to_string().contains("401"), "{error}");
        let error = decode(r#"{"code":"402"}"#).unwrap_err();
        assert!(error.to_string().contains("402"), "{error}");
    }

    #[test]
    fn a_404_names_the_alert_product_instead_of_a_bare_status() {
        // The account host used on 2026-10-06 answers the warning endpoint with `404` and an empty
        // body when the Weather Alert service is not part of the subscription.
        let error = super::not_served(Error::Upstream {
            provider: "kn76xbc7j5.re.qweatherapi.com".to_owned(),
            status: Some(404),
            message: String::new(),
        });
        assert_eq!(error.exit_code(), 3, "the alert panel is best-effort");
        let text = error.to_string();
        assert!(text.contains("Weather Alert service"), "{text}");
        assert!(text.contains("[alerts] sources"), "{text}");

        // Every other error keeps its taxonomy.
        let other = super::not_served(Error::Upstream {
            provider: "host".to_owned(),
            status: Some(500),
            message: "boom".to_owned(),
        });
        assert!(other.to_string().contains("boom"), "{other}");
    }
}
