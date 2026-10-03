// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! `QWeather`'s weather-alert API: the China source, reusing the forecast provider's credential.
//!
//! `{host}/weatheralert/v7/alert/now?location=<lon>,<lat>` answers the same
//! severity/urgency/certainty triple as CAP, so the mapping is direct. The credential and host are
//! the `qweather` provider's (`X-QW-Api-Key` and `providers.qweather.host`); the source is only
//! selected when that provider is on the chain. Step 25 switches the header to the JWT resolver in
//! its own commit; this adapter calls the same accessors then.
//!
//! Two documented v7 revisions differ in where the severity colour lives (`severity` versus
//! `color.code`, plus a `severityColor` spelling in older payloads), so all three are accepted in
//! that order.

use serde::Deserialize;

use super::cap::instant;
use super::{Alert, AlertSource, Certainty, Severity, Urgency};
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
             https://console.qweather.com/setting)"
        )));
    }
    let variable = ProviderId::QWeather
        .metadata()
        .key_env
        .unwrap_or("CIRROCAST_QWEATHER_KEY");
    let key = env.keys.get(PROVIDER)?.ok_or_else(|| Error::MissingKey {
        provider: PROVIDER.to_owned(),
        env: variable.to_owned(),
    })?;

    let request = HttpRequest::get(format!("{host}/weatheralert/v7/alert/now"))
        .query("location", format!("{:.4},{:.4}", loc.lon, loc.lat))
        .header("X-QW-Api-Key", &key)
        .secret(&key);
    let cache_key = super::key(env, AlertSource::QWeather.as_str(), loc);
    let body = super::cached_text(
        env,
        AlertSource::QWeather,
        &request,
        &cache_key,
        "active alerts",
    )?;
    decode(&body)
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
            return Err(Error::InvalidKey {
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
}
