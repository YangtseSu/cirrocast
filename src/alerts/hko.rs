// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The Hong Kong Observatory's keyless warning summary and detail documents.
//!
//! `HKO` publishes JSON rather than CAP: `dataType=warnsum` answers an object keyed by warning code
//! (`{}` when nothing is active) and `dataType=warningInfo` the detail document with the statement
//! text. The adapter builds one alert per summary entry, mapping `HKO`'s codes (`WRAIN`, `WTCSGNL`,
//! `WHOT`, …) and sub-types (`Amber`/`Red`/`Black`, `T1`…`T10`) to CAP `event` and `severity`
//! through the explicit table below; an unknown code keeps the Observatory's own name and an
//! unknown severity rather than guessing.
//!
//! `actionCode = CANCEL` entries are dropped (a cancellation removes a warning). `lang` follows
//! the requested locale: `tc` for a Chinese run, `en` otherwise.
//!
//! Credit: `Warnings by the Hong Kong Observatory`.

use std::collections::HashMap;

use serde::Deserialize;
use serde_json::Value;

use super::cap::instant;
use super::{Alert, AlertSource, Certainty, Severity, Urgency};
use crate::error::Result;
use crate::http::HttpRequest;
use crate::model::Location;
use crate::provider::Env;

/// The Observatory's open-data endpoint.
const ENDPOINT: &str = "https://data.weather.gov.hk/weatherAPI/opendata/weather.php";

/// Fetches the active warnings.
pub fn fetch(loc: &Location, env: &Env<'_>, language: &str) -> Result<Vec<Alert>> {
    let lang = if language.to_ascii_lowercase().starts_with("zh") {
        "tc"
    } else {
        "en"
    };
    let summary_request = HttpRequest::get(ENDPOINT)
        .query("dataType", "warnsum")
        .query("lang", lang);
    let summary_key = super::language_key(env, "hko-warnsum", loc, lang);
    let summary_body = super::cached_text(
        env,
        AlertSource::Hko,
        &summary_request,
        &summary_key,
        "warning summary",
    )?;
    let summary: HashMap<String, Summary> =
        serde_json::from_str(&summary_body).map_err(|error| {
            super::upstream(
                AlertSource::Hko,
                format!("the warning summary does not parse as JSON: {error}"),
            )
        })?;
    if summary.is_empty() {
        return Ok(Vec::new());
    }

    let info_request = HttpRequest::get(ENDPOINT)
        .query("dataType", "warningInfo")
        .query("lang", lang);
    let info_key = super::language_key(env, "hko-warninginfo", loc, lang);
    let info_body = match super::cached_text(
        env,
        AlertSource::Hko,
        &info_request,
        &info_key,
        "warning details",
    ) {
        Ok(body) => body,
        Err(error) => {
            if env.verbose > 0 {
                eprintln!("alerts: hko: {error}; continuing without statement text");
            }
            String::new()
        }
    };
    let details = details(&info_body);

    let mut alerts: Vec<Alert> = summary
        .into_iter()
        .filter_map(|(code, entry)| entry.into_alert(&code, details.get(&code).map(String::as_str)))
        .collect();
    alerts.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(alerts)
}

/// One `warnsum` entry.
#[derive(Debug, Deserialize)]
struct Summary {
    #[serde(default)]
    name: Option<String>,
    #[serde(rename = "actionCode", default)]
    action_code: Option<String>,
    #[serde(rename = "issueTime", default)]
    issue_time: Option<String>,
    #[serde(rename = "updateTime", default)]
    update_time: Option<String>,
    #[serde(rename = "expireTime", default)]
    expire_time: Option<String>,
    /// The sub-type: `T8`, `Amber`, `Red`, …; the API sends a string, but a number is accepted.
    #[serde(rename = "type", default)]
    kind: Option<Value>,
}

impl Summary {
    /// One entry as the model, or `None` when it is a cancellation.
    fn into_alert(self, code: &str, contents: Option<&str>) -> Option<Alert> {
        if self
            .action_code
            .as_deref()
            .is_some_and(|action| action.eq_ignore_ascii_case("cancel"))
        {
            return None;
        }
        let kind = self.kind.as_ref().and_then(kind_text);
        let (event, severity) = event_and_severity(code, kind.as_deref(), self.name.as_deref());
        let description = contents
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_owned);
        Some(Alert {
            id: format!("hko-{}", code.trim().to_ascii_lowercase()),
            source: AlertSource::Hko,
            event: event.clone(),
            severity,
            urgency: Urgency::Immediate,
            certainty: Certainty::Likely,
            onset: self
                .issue_time
                .as_deref()
                .and_then(instant)
                .or_else(|| self.update_time.as_deref().and_then(instant)),
            expires: self.expire_time.as_deref().and_then(instant),
            ends: None,
            areas: vec!["Hong Kong".to_owned()],
            headline: self
                .name
                .filter(|name| !name.trim().is_empty())
                .unwrap_or(event),
            description,
            instruction: None,
            sender: Some("Hong Kong Observatory".to_owned()),
            credit: Vec::new(),
        })
    }
}

/// A `type` field as text, whether the API spelled it as a string or a number.
fn kind_text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

/// The CAP event name and severity for an `HKO` warning code and sub-type.
#[must_use]
fn event_and_severity(code: &str, kind: Option<&str>, name: Option<&str>) -> (String, Severity) {
    let kind = kind.unwrap_or_default();
    let (event, severity) = match code.trim().to_ascii_uppercase().as_str() {
        "WRAIN" => (
            "Rainstorm Warning",
            match kind {
                "Red" => Severity::Severe,
                "Black" => Severity::Extreme,
                _ => Severity::Moderate,
            },
        ),
        "WTCSGNL" => (
            "Tropical Cyclone Warning Signal",
            match kind {
                "T1" => Severity::Minor,
                "T8" | "T9" => Severity::Severe,
                "T10" => Severity::Extreme,
                _ => Severity::Moderate,
            },
        ),
        "WTCPRE8" => (
            "Pre-No. 8 Tropical Cyclone Warning Signal",
            Severity::Moderate,
        ),
        "WMSGNL" => ("Strong Monsoon Signal", Severity::Moderate),
        "WTS" => ("Thunderstorm Warning", Severity::Moderate),
        "WHOT" => ("Very Hot Weather Warning", Severity::Moderate),
        "WCOLD" => ("Cold Weather Warning", Severity::Moderate),
        "WFROST" => ("Frost Warning", Severity::Moderate),
        "WFL" => ("Flooding Warning", Severity::Moderate),
        "WL" => ("Landslip Warning", Severity::Moderate),
        "WFIRE" => (
            "Fire Danger Warning",
            match kind {
                "Red" => Severity::Severe,
                _ => Severity::Moderate,
            },
        ),
        _ => (name.unwrap_or(code), Severity::Unknown),
    };
    (event.to_owned(), severity)
}

/// The `warningInfo` statement text per warning code.
///
/// The document is a `details` array whose entries carry `warningStatementCode` and a `contents`
/// array; a body that does not parse yields no text rather than failing the warnings themselves.
#[must_use]
fn details(body: &str) -> HashMap<String, String> {
    let mut details = HashMap::new();
    let Ok(value) = serde_json::from_str::<Value>(body) else {
        return details;
    };
    let Some(entries) = value.get("details").and_then(Value::as_array) else {
        return details;
    };
    for entry in entries {
        let code = entry
            .get("warningStatementCode")
            .or_else(|| entry.get("code"))
            .and_then(Value::as_str)
            .map(str::trim);
        let Some(code) = code else {
            continue;
        };
        let text = entry
            .get("contents")
            .and_then(Value::as_array)
            .map(|lines| {
                lines
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::trim)
                    .filter(|line| !line.is_empty())
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default();
        if !text.is_empty() {
            details.insert(code.to_owned(), text);
        }
    }
    details
}

#[cfg(test)]
mod tests {
    use super::{Summary, details, event_and_severity};
    use crate::model::{AlertSource, Severity};

    #[test]
    fn the_code_table_maps_subtypes_to_severity() {
        assert_eq!(
            event_and_severity("WRAIN", Some("Black"), None),
            ("Rainstorm Warning".to_owned(), Severity::Extreme)
        );
        assert_eq!(
            event_and_severity("WTCSGNL", Some("T8"), None),
            (
                "Tropical Cyclone Warning Signal".to_owned(),
                Severity::Severe
            )
        );
        assert_eq!(
            event_and_severity("WTCSGNL", Some("T1"), None),
            (
                "Tropical Cyclone Warning Signal".to_owned(),
                Severity::Minor
            )
        );
        assert_eq!(
            event_and_severity("WHOT", None, None),
            ("Very Hot Weather Warning".to_owned(), Severity::Moderate)
        );
        // An unknown code keeps the Observatory's own name and an unknown severity.
        assert_eq!(
            event_and_severity("WXYZ", None, Some("Meteor Shower Warning")),
            ("Meteor Shower Warning".to_owned(), Severity::Unknown)
        );
    }

    #[test]
    fn a_summary_entry_becomes_an_alert_and_a_cancellation_does_not() {
        let entry: Summary = serde_json::from_str(
            r#"{"name":"Tropical Cyclone Warning Signal","code":"WTCSGNL","actionCode":"ISSUE",
                "issueTime":"2026-10-03T09:45:00+08:00","updateTime":"2026-10-03T09:45:00+08:00",
                "expireTime":"2026-10-03T18:00:00+08:00","type":"T8"}"#,
        )
        .expect("the summary entry decodes");
        let alert = entry
            .into_alert("WTCSGNL", Some("The No. 8 signal is in force."))
            .expect("an issued warning is an alert");
        assert_eq!(alert.source, AlertSource::Hko);
        assert_eq!(alert.id, "hko-wtcsgnl");
        assert_eq!(alert.severity, Severity::Severe);
        assert_eq!(alert.areas, ["Hong Kong"]);
        assert_eq!(alert.sender.as_deref(), Some("Hong Kong Observatory"));
        assert!(alert.description.is_some());

        let cancelled: Summary = serde_json::from_str(
            r#"{"name":"Rainstorm Warning","actionCode":"CANCEL","type":"Amber"}"#,
        )
        .expect("the entry decodes");
        assert!(cancelled.into_alert("WRAIN", None).is_none());
    }

    #[test]
    fn statement_text_is_extracted_per_code() {
        let body = r#"{"details":[
            {"warningStatementCode":"WTS","contents":["Thunderstorms are expected.","Take care."]},
            {"warningStatementCode":"WRAIN","contents":[]},
            {"contents":["no code"]}
        ]}"#;
        let details = details(body);
        assert_eq!(
            details.get("WTS").map(String::as_str),
            Some("Thunderstorms are expected.\nTake care.")
        );
        assert!(!details.contains_key("WRAIN"));
        assert_eq!(details.len(), 1);
    }
}
