// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The WMO Severe Weather Information Centre: the keyless worldwide aggregator (operated by `HKO`).
//!
//! Two steps, as the service is designed:
//!
//! 1. a WFS `GetFeature` whose CQL filter runs the point-in-polygon test **server-side**
//!    (`INTERSECTS(wkb_geometry,POINT(<lat> <lon>)) AND row_type NEQ 'BOUNDARY'`), answered as
//!    `GeoJSON` — each surviving feature carries the `capurl` of its CAP 1.2 document;
//! 2. one CAP document fetch per feature under `/v2/cap-alerts/<capurl>`, parsed by the shared
//!    reader.
//!
//! The WFS index is cached under `alerts/wmo-<lat>-<lon>-<hour>` (it changes with the hour), each
//! CAP document per identifier, so a repeated run is served from disk. A document that fails to
//! fetch or parse is skipped with a `--verbose` note: the aggregator federates 130+ agencies, and
//! one bad publisher must not hide the others' warnings.
//!
//! Credit: `Warnings by the WMO Severe Weather Information Centre (severeweather.wmo.int),
//! © the issuing agencies`.

use serde::Deserialize;

use super::{Alert, AlertSource};
use crate::error::{Error, Result};
use crate::http::HttpRequest;
use crate::model::Location;
use crate::provider::Env;

/// The WFS endpoint that answers the point query.
const ENDPOINT: &str = "https://severeweather.wmo.int/f/wfs";

/// The CAP document root.
const CAP_ROOT: &str = "https://severeweather.wmo.int/v2/cap-alerts";

/// Fetches the warnings whose area contains the point.
pub fn fetch(loc: &Location, env: &Env<'_>, language: &str) -> Result<Vec<Alert>> {
    let filter = format!(
        "INTERSECTS(wkb_geometry,POINT({:.4} {:.4})) AND row_type NEQ 'BOUNDARY'",
        loc.lat, loc.lon
    );
    let request = HttpRequest::get(ENDPOINT)
        .query("request", "GetFeature")
        .query("version", "1.1.0")
        .query("outputFormat", "json")
        .query("typeName", "local_postgis:postgis_geojsons")
        .query("cql_filter", filter);
    // The plan names this key `alerts/wmo-…`, not `alerts/wmoswic-…`.
    let key = super::key(env, "wmo", loc);
    let body = super::cached_text(env, AlertSource::WmoSwic, &request, &key, "warning index")?;
    let features = index(&body)?;

    let mut alerts = Vec::new();
    let mut first_error: Option<Error> = None;
    for feature in features {
        let Some(link) = feature.properties.cap_url() else {
            if env.verbose > 0 {
                eprintln!("alerts: wmoswic: a feature carries no capurl; skipping");
            }
            continue;
        };
        match document(
            &link,
            feature.properties.identifier.as_deref(),
            env,
            language,
        ) {
            Ok(mut found) => alerts.append(&mut found),
            Err(error) => {
                if env.verbose > 0 {
                    eprintln!("alerts: wmoswic: {error}");
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

/// Fetches and normalises one CAP document.
fn document(
    link: &str,
    identifier: Option<&str>,
    env: &Env<'_>,
    language: &str,
) -> Result<Vec<Alert>> {
    let url = cap_url(link)?;
    let key = super::document_key(AlertSource::WmoSwic, identifier.unwrap_or(link));
    let request = HttpRequest::get(url);
    let body = super::cached_text(env, AlertSource::WmoSwic, &request, &key, "CAP document")?;
    let document = super::cap::parse_cap(&body, AlertSource::WmoSwic)?;
    super::cap::alerts_from_cap(&document, AlertSource::WmoSwic, language)
}

/// A `capurl` resolved against the WMO `CAP_ROOT`.
///
/// A relative path is joined, and an absolute link is accepted only on the WMO host itself; any
/// other host or a non-https scheme is an upstream error (see `super::document_link`).
fn cap_url(link: &str) -> Result<String> {
    super::document_link(CAP_ROOT, link, AlertSource::WmoSwic)
}

/// The WFS feature collection.
#[derive(Debug, Deserialize)]
struct Envelope {
    #[serde(default)]
    features: Vec<Feature>,
}

/// One feature of the index.
#[derive(Debug, Deserialize)]
struct Feature {
    properties: Properties,
}

/// The properties the adapter needs.
#[derive(Debug, Deserialize)]
struct Properties {
    #[serde(default)]
    capurl: Option<String>,
    #[serde(default)]
    rlink: Option<String>,
    #[serde(default)]
    identifier: Option<String>,
}

impl Properties {
    /// The CAP document link: `capurl` first, `rlink` (the alternate-language document) as the
    /// fallback; `None` when the feature carries neither.
    fn cap_url(&self) -> Option<String> {
        [self.capurl.as_deref(), self.rlink.as_deref()]
            .into_iter()
            .flatten()
            .map(str::trim)
            .find(|link| !link.is_empty())
            .map(str::to_owned)
    }
}

/// Decodes the WFS index.
fn index(body: &str) -> Result<Vec<Feature>> {
    let envelope: Envelope = serde_json::from_str(body).map_err(|error| {
        super::upstream(
            AlertSource::WmoSwic,
            format!("the warning index does not parse as GeoJSON: {error}"),
        )
    })?;
    Ok(envelope.features)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{cap_url, index};

    #[test]
    fn the_index_exposes_capurl_and_falls_back_to_rlink() {
        let body = json!({
            "type": "FeatureCollection",
            "features": [
                {
                    "type": "Feature",
                    "geometry": null,
                    "properties": {
                        "identifier": "20261003-1",
                        "capurl": "20261003T120000Z_xx_1.xml",
                        "rlink": "20261003T120000Z_xx_1_fr.xml"
                    }
                },
                {
                    "type": "Feature",
                    "properties": { "rlink": "20261003T120000Z_xx_2.xml" }
                },
                { "type": "Feature", "properties": {} }
            ]
        })
        .to_string();
        let features = index(&body).expect("the index decodes");
        assert_eq!(features.len(), 3);
        assert_eq!(
            features[0].properties.cap_url().as_deref(),
            Some("20261003T120000Z_xx_1.xml")
        );
        assert_eq!(
            features[1].properties.cap_url().as_deref(),
            Some("20261003T120000Z_xx_2.xml")
        );
        assert_eq!(features[2].properties.cap_url(), None);
    }

    #[test]
    fn cap_urls_are_made_absolute_and_stay_on_the_wmo_host() {
        assert_eq!(
            cap_url("20261003T120000Z_xx_1.xml").expect("a relative link"),
            "https://severeweather.wmo.int/v2/cap-alerts/20261003T120000Z_xx_1.xml"
        );
        assert_eq!(
            cap_url("https://severeweather.wmo.int/v2/cap-alerts/x.xml").expect("an on-host link"),
            "https://severeweather.wmo.int/v2/cap-alerts/x.xml"
        );
    }

    #[test]
    fn off_host_and_cleartext_cap_urls_are_refused() {
        let off_host = cap_url("https://elsewhere.example/x.xml").unwrap_err();
        assert!(off_host.to_string().contains("wmoswic"), "{off_host}");
        assert!(cap_url("http://127.0.0.1/x.xml").is_err());
        assert!(cap_url("http://severeweather.wmo.int/x.xml").is_err());
    }

    #[test]
    fn a_malformed_index_is_an_upstream_error() {
        let error = index("<html>not json</html>").unwrap_err();
        assert!(error.to_string().contains("wmoswic"), "{error}");
    }
}
