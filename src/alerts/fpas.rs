// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The FOSS Public Alert Server: a keyless, self-hostable worldwide aggregator.
//!
//! `FPAS` answers a bounding box first — `GET /alert/area?min_lat=…` returns a JSON array of alert
//! UUIDs — and each UUID resolves to a CAP 1.2 document (`GET /alert/<uuid>`, which the public
//! instance answers with a `301` to the file under `/cap/alerts/…`; the HTTP client follows it).
//! `[alerts] fpas_url` points at a self-hosted instance and defaults to the public
//! `https://alerts.kde.org`.
//!
//! Unlike WMO `SWIC`, `FPAS` runs no point-in-polygon test, so three filters live here:
//!
//! * `category != Met` documents are dropped (the server federates civil-protection feeds too);
//! * `msgType = Cancel` documents are dropped;
//! * a document whose area polygon/circle does not contain the point is dropped — CAP areas may be
//!   polygons or circles, and a document with no testable area is kept rather than guessed away.
//!
//! A UUID whose document fails to fetch or parse is skipped with a `--verbose` note; the source
//! only fails when every document it saw did.
//!
//! Credit: `Warnings via the FOSS Public Alert Server (alerts.kde.org)`.

use super::geometry::{cap_circle_contains, cap_polygon_contains};
use super::{Alert, AlertSource, cap};
use crate::error::{Error, Result};
use crate::http::HttpRequest;
use crate::model::Location;
use crate::provider::Env;

/// The public instance used when `[alerts] fpas_url` is empty.
const DEFAULT_URL: &str = "https://alerts.kde.org";

/// The half-width of the bounding box, in degrees, around the requested point.
const BOX: f64 = 0.5;

/// Fetches the warnings whose areas contain the point.
pub fn fetch(loc: &Location, env: &Env<'_>, language: &str) -> Result<Vec<Alert>> {
    let base = base_url(&env.config.alerts.fpas_url);
    let request = HttpRequest::get(format!("{base}/alert/area"))
        .query("min_lat", format!("{:.4}", loc.lat - BOX))
        .query("max_lat", format!("{:.4}", loc.lat + BOX))
        .query("min_lon", format!("{:.4}", loc.lon - BOX))
        .query("max_lon", format!("{:.4}", loc.lon + BOX));
    let key = super::key(env, AlertSource::Fpas.as_str(), loc);
    let body = super::cached_text(env, AlertSource::Fpas, &request, &key, "area alerts")?;
    let identifiers: Vec<String> = serde_json::from_str(&body).map_err(|error| {
        super::upstream(
            AlertSource::Fpas,
            format!("the area response does not parse as a UUID list: {error}"),
        )
    })?;

    let mut alerts = Vec::new();
    let mut first_error: Option<Error> = None;
    for identifier in identifiers {
        match document(&base, &identifier, loc, env, language) {
            Ok(mut found) => alerts.append(&mut found),
            Err(error) => {
                if env.verbose > 0 {
                    eprintln!("alerts: fpas: {error}");
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

/// The configured instance, or the public default.
#[must_use]
pub fn base_url(configured: &str) -> String {
    let configured = configured.trim().trim_end_matches('/');
    if configured.is_empty() {
        DEFAULT_URL.to_owned()
    } else {
        configured.to_owned()
    }
}

/// The instance label for the credit line: the host of the configured URL, or the public host.
#[must_use]
pub fn instance_label(configured: &str) -> String {
    let base = base_url(configured);
    let host = base
        .split_once("://")
        .map_or(base.as_str(), |(_, rest)| rest)
        .split('/')
        .next()
        .unwrap_or(base.as_str());
    host.to_owned()
}

/// Fetches one UUID's CAP document and applies the client-side filters.
fn document(
    base: &str,
    identifier: &str,
    loc: &Location,
    env: &Env<'_>,
    language: &str,
) -> Result<Vec<Alert>> {
    let key = super::document_key(AlertSource::Fpas, identifier);
    let request = HttpRequest::get(format!("{base}/alert/{identifier}"));
    let body = super::cached_text(env, AlertSource::Fpas, &request, &key, "CAP document")?;
    let document = cap::parse_cap(&body, AlertSource::Fpas)?;
    if !is_met(&document) {
        return Ok(Vec::new());
    }
    if !area_contains(&document, loc.lat, loc.lon) {
        return Ok(Vec::new());
    }
    cap::alerts_from_cap(&document, AlertSource::Fpas, language)
}

/// Whether the document carries a meteorological category.
///
/// A document whose `info` blocks carry no category at all is kept: "no category" is not "not
/// meteorological", and dropping it would hide a warning.
#[must_use]
fn is_met(document: &cap::CapDocument) -> bool {
    let mut any = false;
    for info in &document.infos {
        for category in &info.categories {
            any = true;
            if category.trim().eq_ignore_ascii_case("met") {
                return true;
            }
        }
    }
    !any
}

/// Whether the document's areas contain the point.
///
/// `true` when no area carries a testable polygon or circle: an area the tool cannot test is not
/// evidence against the warning.
#[must_use]
fn area_contains(document: &cap::CapDocument, lat: f64, lon: f64) -> bool {
    let mut testable = false;
    for info in &document.infos {
        for area in &info.areas {
            for polygon in &area.polygons {
                if let Some(contains) = cap_polygon_contains(polygon, lat, lon) {
                    testable = true;
                    if contains {
                        return true;
                    }
                }
            }
            for circle in &area.circles {
                if let Some(contains) = cap_circle_contains(circle, lat, lon) {
                    testable = true;
                    if contains {
                        return true;
                    }
                }
            }
        }
    }
    !testable
}

#[cfg(test)]
mod tests {
    use super::{area_contains, is_met};
    use crate::alerts::cap::parse_cap;
    use crate::model::AlertSource;

    fn parse(xml: &str) -> crate::alerts::cap::CapDocument {
        parse_cap(xml, AlertSource::Fpas).expect("the document parses")
    }

    #[test]
    fn only_meteorological_documents_survive() {
        let met = parse(
            r"<alert><identifier>a</identifier><info><category>Met</category><event>gale</event></info></alert>",
        );
        assert!(is_met(&met));
        let geo = parse(
            r"<alert><identifier>b</identifier><info><category>Geo</category><event>quake</event></info></alert>",
        );
        assert!(!is_met(&geo));
        let none = parse(r"<alert><identifier>c</identifier><info><event>x</event></info></alert>");
        assert!(is_met(&none), "an absent category is not a rejection");
    }

    #[test]
    fn a_polygon_that_misses_the_point_drops_the_document() {
        let inside = parse(
            r"<alert><identifier>a</identifier>
              <info><event>gale</event>
                <area><areaDesc>box</areaDesc>
                  <polygon>39.0,116.0 40.0,116.0 40.0,117.0 39.0,117.0 39.0,116.0</polygon>
                </area>
              </info></alert>",
        );
        assert!(area_contains(&inside, 39.5, 116.5));
        assert!(!area_contains(&inside, 41.0, 116.5));

        let circle = parse(
            r"<alert><identifier>b</identifier>
              <info><event>gale</event><area><areaDesc>c</areaDesc><circle>39.9,116.4 10</circle></area></info>
            </alert>",
        );
        assert!(area_contains(&circle, 39.95, 116.4));
        assert!(!area_contains(&circle, 35.0, 116.4));

        let untestable = parse(
            r"<alert><identifier>c</identifier><info><event>gale</event><area><areaDesc>nowhere</areaDesc></area></info></alert>",
        );
        assert!(area_contains(&untestable, 0.0, 0.0));
    }
}
