// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Visual Crossing's alert payload: the timeline response's own `alerts[]` array.
//!
//! `{timeline}/<lat>,<lon>?unitGroup=metric&include=alerts&key=<KEY>` answers the same array the
//! forecast request carries, so this source is a thin wrapper: it fetches with the `visualcrossing`
//! provider's credential and hands the body to that provider's decoder
//! ([`crate::provider::visualcrossing::alerts_from_response`]), which keeps the provider's
//! report-carried warnings and this registry source from drifting apart.
//!
//! The source is bound to the provider — it needs its key — so `auto` selects it only when the
//! `visualcrossing` backend is on the weather chain; `--alerts-from visualcrossing` names it
//! directly.

use super::{Alert, AlertSource};
use crate::error::{Error, Result};
use crate::model::Location;
use crate::provider::visualcrossing::{TimelineResponse, alerts_from_response, alerts_request};
use crate::provider::{Env, ProviderId};

/// The provider id whose credential this source borrows.
const PROVIDER: &str = "visualcrossing";

/// Fetches the active and upcoming warnings for the point.
pub fn fetch(loc: &Location, env: &Env<'_>, _language: &str) -> Result<Vec<Alert>> {
    let variable = ProviderId::VisualCrossing
        .metadata()
        .key_env
        .unwrap_or("CIRROCAST_VISUALCROSSING_KEY");
    let key = env.keys.get(PROVIDER)?.ok_or_else(|| Error::MissingKey {
        provider: PROVIDER.to_owned(),
        env: variable.to_owned(),
    })?;

    let request = alerts_request(loc, &key);
    let cache_key = super::key(env, AlertSource::VisualCrossing.as_str(), loc);
    let body = super::cached_text(
        env,
        AlertSource::VisualCrossing,
        &request,
        &cache_key,
        "active alerts",
    )?;
    let response: TimelineResponse = serde_json::from_str(&body).map_err(|error| {
        super::upstream(
            AlertSource::VisualCrossing,
            format!("the alert response does not parse as JSON: {error}"),
        )
    })?;
    Ok(alerts_from_response(&response, loc, env))
}
