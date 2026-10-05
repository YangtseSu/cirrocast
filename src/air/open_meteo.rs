// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Open-Meteo's Air Quality API: the keyless source of the air panel.
//!
//! One request per reading, asking for the two consolidated indices, the six regulated pollutants
//! and the six pollen species, in metric units — the API reports μg/m³ and grains/m³ natively and
//! the request carries no unit parameters, so the decode asserts the units it was promised and
//! fails the reading (not the run) when upstream changes them.
//!
//! What the decode encodes beyond "parse the fields":
//!
//! * `current_units` is checked field by field; a pollutant in anything but μg/m³ or a pollen
//!   count in anything but grains/m³ is [`Error::Upstream`] naming the field, the received unit
//!   and the expected one. Silently converting would put a number from one scale into an AQI
//!   panel that reads like another.
//! * `time` is a local wall clock plus the response's `utc_offset_seconds`; the zone *name* is
//!   cross-checked against the offset at that instant when chrono knows it, so a payload whose
//!   two time fields disagree does not produce a plausible-looking wrong instant. The check is
//!   done on the absolute instant, which keeps it valid across a daylight-saving fold.
//! * Pollen degrades in two steps, both `--verbose` notes: all six fields `null` means the point
//!   is outside the CAMS European domain and yields `pollen: None`; in a partially `null` block
//!   the missing species stay `None` (not measured), because the API's `null` and a measured `0.0`
//!   are different answers. Nothing is invented silently.
//! * A response without a `current` block is [`Error::Upstream`]: there is no reading to show.

use std::time::Duration;

use chrono::{DateTime, FixedOffset, NaiveDateTime, Offset as _, TimeZone as _};
use chrono_tz::Tz;
use serde::Deserialize;

use crate::air::aqi::us_beyond_index;
use crate::cache::CacheKey;
use crate::error::{Error, Result};
use crate::http::HttpRequest;
use crate::model::air::{POLLEN_UNIT, POLLUTANT_UNIT};
use crate::model::{AirQuality, AirSource, Location, Pollen};
use crate::provider::{Env, JsonFetch, ProviderId, fetch_json, local_today};

/// The provider id, as error messages spell it.
const PROVIDER: &str = "open-meteo";

/// The air-quality endpoint.
pub const BASE: &str = "https://air-quality-api.open-meteo.com/v1/air-quality";

/// The `current` variables, in the fixed order the request uses.
const CURRENT_VARIABLES: &str = "us_aqi,european_aqi,pm2_5,pm10,ozone,nitrogen_dioxide,\
sulphur_dioxide,carbon_monoxide,alder_pollen,birch_pollen,grass_pollen,mugwort_pollen,\
olive_pollen,ragweed_pollen";

/// Fetches one reading for `loc`.
pub fn fetch(loc: &Location, env: &Env<'_>) -> Result<AirQuality> {
    let key = CacheKey::air(PROVIDER, loc.lat, loc.lon, local_today(env, loc.tz));
    let request = air_request(loc);
    let ttl = Duration::from_secs(u64::from(env.config.cache.weather_ttl_secs));

    let response: AirResponse = fetch_json(
        env,
        loc,
        &JsonFetch {
            provider: ProviderId::OpenMeteo,
            request,
            key,
            ttl,
            what: "air quality",
        },
    )?;

    decode(&response, env.verbose)
}

/// The request, assembled in a fixed parameter order (the tests assert the URL verbatim).
fn air_request(loc: &Location) -> HttpRequest {
    HttpRequest::get(BASE)
        .query("latitude", format!("{:.4}", loc.lat))
        .query("longitude", format!("{:.4}", loc.lon))
        .query("current", CURRENT_VARIABLES)
        .query("timezone", "auto")
}

// ---------------------------------------------------------------------------------------------
// Response shape
// ---------------------------------------------------------------------------------------------

/// The air-quality response, in the subset `cirrocast` consumes.
///
/// Unknown fields are ignored on purpose: upstream adds variables regularly, and a new one never
/// invalidates a cached body.
#[derive(Debug, Clone, Deserialize)]
pub struct AirResponse {
    /// Offset of the location's zone at the requested instant, in seconds.
    pub utc_offset_seconds: i32,
    /// IANA zone name the timestamps are expressed in.
    pub timezone: String,
    /// The unit each `current` field is expressed in.
    pub current_units: CurrentUnits,
    /// The reading.
    #[serde(default)]
    pub current: Option<CurrentBlock>,
}

/// The `current_units` object, for the fields the decode asserts.
#[derive(Debug, Clone, Deserialize)]
pub struct CurrentUnits {
    /// Unit of the PM2.5 reading.
    pub pm2_5: String,
    /// Unit of the PM10 reading.
    pub pm10: String,
    /// Unit of the ozone reading.
    pub ozone: String,
    /// Unit of the nitrogen dioxide reading.
    pub nitrogen_dioxide: String,
    /// Unit of the sulphur dioxide reading.
    pub sulphur_dioxide: String,
    /// Unit of the carbon monoxide reading.
    pub carbon_monoxide: String,
    /// Unit of the alder pollen reading.
    pub alder_pollen: String,
    /// Unit of the birch pollen reading.
    pub birch_pollen: String,
    /// Unit of the grass pollen reading.
    pub grass_pollen: String,
    /// Unit of the mugwort pollen reading.
    pub mugwort_pollen: String,
    /// Unit of the olive pollen reading.
    pub olive_pollen: String,
    /// Unit of the ragweed pollen reading.
    pub ragweed_pollen: String,
}

/// The `current` object: every measurement optional, the timestamp the only required field.
#[derive(Debug, Clone, Deserialize)]
pub struct CurrentBlock {
    /// Observation time, local wall clock (`YYYY-MM-DDTHH:MM`).
    pub time: String,
    /// US AQI.
    #[serde(default)]
    pub us_aqi: Option<u16>,
    /// European AQI.
    #[serde(default)]
    pub european_aqi: Option<u16>,
    /// Fine particulate matter in μg/m³.
    #[serde(default)]
    pub pm2_5: Option<f64>,
    /// Coarse particulate matter in μg/m³.
    #[serde(default)]
    pub pm10: Option<f64>,
    /// Ground-level ozone in μg/m³.
    #[serde(default)]
    pub ozone: Option<f64>,
    /// Nitrogen dioxide in μg/m³.
    #[serde(default)]
    pub nitrogen_dioxide: Option<f64>,
    /// Sulphur dioxide in μg/m³.
    #[serde(default)]
    pub sulphur_dioxide: Option<f64>,
    /// Carbon monoxide in μg/m³.
    #[serde(default)]
    pub carbon_monoxide: Option<f64>,
    /// Alder pollen in grains/m³.
    #[serde(default)]
    pub alder_pollen: Option<f64>,
    /// Birch pollen in grains/m³.
    #[serde(default)]
    pub birch_pollen: Option<f64>,
    /// Grass pollen in grains/m³.
    #[serde(default)]
    pub grass_pollen: Option<f64>,
    /// Mugwort pollen in grains/m³.
    #[serde(default)]
    pub mugwort_pollen: Option<f64>,
    /// Olive pollen in grains/m³.
    #[serde(default)]
    pub olive_pollen: Option<f64>,
    /// Ragweed pollen in grains/m³.
    #[serde(default)]
    pub ragweed_pollen: Option<f64>,
}

// ---------------------------------------------------------------------------------------------
// Response → model
// ---------------------------------------------------------------------------------------------

/// Turns one response into the model reading, or an upstream error naming what does not fit.
fn decode(response: &AirResponse, verbose: u8) -> Result<AirQuality> {
    let block = response.current.as_ref().ok_or_else(|| {
        upstream("the response has no `current` block, so there is no reading to show")
    })?;

    let units = &response.current_units;
    for (field, got, expected) in [
        ("pm2_5", &units.pm2_5, POLLUTANT_UNIT),
        ("pm10", &units.pm10, POLLUTANT_UNIT),
        ("ozone", &units.ozone, POLLUTANT_UNIT),
        ("nitrogen_dioxide", &units.nitrogen_dioxide, POLLUTANT_UNIT),
        ("sulphur_dioxide", &units.sulphur_dioxide, POLLUTANT_UNIT),
        ("carbon_monoxide", &units.carbon_monoxide, POLLUTANT_UNIT),
        ("alder_pollen", &units.alder_pollen, POLLEN_UNIT),
        ("birch_pollen", &units.birch_pollen, POLLEN_UNIT),
        ("grass_pollen", &units.grass_pollen, POLLEN_UNIT),
        ("mugwort_pollen", &units.mugwort_pollen, POLLEN_UNIT),
        ("olive_pollen", &units.olive_pollen, POLLEN_UNIT),
        ("ragweed_pollen", &units.ragweed_pollen, POLLEN_UNIT),
    ] {
        if got != expected {
            return Err(upstream(format!(
                "`{field}` is reported in `{got}`, expected `{expected}`"
            )));
        }
    }

    // Validate the reading before any `-v` note: the instant is the only thing that can refuse a
    // payload, and a note about a panel the run then does not produce would be a lie.
    let time = instant(&block.time, response)?;
    let pollen = pollen_of(block, verbose);

    if verbose > 0
        && let Some(index) = block.us_aqi
        && us_beyond_index(index)
    {
        eprintln!("air: US AQI {index} is above the documented 500 scale; showing it as hazardous");
    }

    Ok(AirQuality {
        time,
        aqi_us: block.us_aqi,
        aqi_european: block.european_aqi,
        pm2_5: block.pm2_5,
        pm10: block.pm10,
        o3: block.ozone,
        no2: block.nitrogen_dioxide,
        so2: block.sulphur_dioxide,
        co: block.carbon_monoxide,
        pollen,
        source: AirSource::OpenMeteo,
    })
}

/// The reading's instant: the local wall clock plus the response's own UTC offset, cross-checked
/// against the zone name when chrono knows it.
fn instant(text: &str, response: &AirResponse) -> Result<DateTime<FixedOffset>> {
    let naive = NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M")
        .map_err(|error| upstream(format!("`{text}` is not a local date and time: {error}")))?;
    let offset = FixedOffset::east_opt(response.utc_offset_seconds).ok_or_else(|| {
        upstream(format!(
            "the response's UTC offset ({}s) is out of range",
            response.utc_offset_seconds
        ))
    })?;
    let at = offset
        .from_local_datetime(&naive)
        .single()
        .ok_or_else(|| upstream(format!("`{text}` is not representable at offset {offset}")))?;

    // The zone name is the documentation of the offset, not the authority: a name chrono does not
    // know (or a zone that changed since the response was cached) must not cost the reader a
    // correct instant. When it does parse, the offset it implies at that instant has to match, or
    // the payload contradicts itself.
    if let Ok(tz) = response.timezone.parse::<Tz>() {
        let implied = at.with_timezone(&tz).offset().fix().local_minus_utc();
        if implied != response.utc_offset_seconds {
            return Err(upstream(format!(
                "`{}` is at UTC{:+03}:00 at {text}, but the response says UTC{:+03}:00",
                response.timezone,
                implied / 3600,
                response.utc_offset_seconds / 3600
            )));
        }
    }
    Ok(at)
}

/// The pollen forecast, with the two documented degradations.
fn pollen_of(block: &CurrentBlock, verbose: u8) -> Option<Pollen> {
    let values = [
        block.alder_pollen,
        block.birch_pollen,
        block.grass_pollen,
        block.mugwort_pollen,
        block.olive_pollen,
        block.ragweed_pollen,
    ];
    let missing = values.iter().filter(|value| value.is_none()).count();
    if missing == values.len() {
        if verbose > 0 {
            eprintln!("air: pollen forecast is not covered here (CAMS European domain only)");
        }
        return None;
    }
    if missing > 0 && verbose > 0 {
        eprintln!(
            "air: {missing} of {} pollen fields are not covered here; those species read as not measured",
            values.len()
        );
    }
    let [alder, birch, grass, mugwort, olive, ragweed] = values;
    Some(Pollen {
        alder,
        birch,
        grass,
        mugwort,
        olive,
        ragweed,
    })
}

/// An upstream error of this source.
fn upstream(message: impl Into<String>) -> Error {
    Error::Upstream {
        provider: PROVIDER.to_owned(),
        status: None,
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::{AirResponse, CurrentBlock, decode, pollen_of};

    fn block(json: &str) -> CurrentBlock {
        serde_json::from_str(json).expect("the block parses")
    }

    /// The standard unit envelope, so a test only writes the fields it cares about.
    fn envelope(current: &str) -> AirResponse {
        let json = format!(
            r#"{{"utc_offset_seconds":7200,"timezone":"Europe/Berlin","current_units":{{
                "pm2_5":"μg/m³","pm10":"μg/m³","ozone":"μg/m³","nitrogen_dioxide":"μg/m³",
                "sulphur_dioxide":"μg/m³","carbon_monoxide":"μg/m³","alder_pollen":"grains/m³",
                "birch_pollen":"grains/m³","grass_pollen":"grains/m³","mugwort_pollen":"grains/m³",
                "olive_pollen":"grains/m³","ragweed_pollen":"grains/m³"}},
                "current":{current}}}"#
        );
        serde_json::from_str(&json).expect("the envelope parses")
    }

    #[test]
    fn a_partially_null_pollen_block_keeps_the_missing_species_unmeasured() {
        let block = block(
            r#"{"time":"2026-10-03T20:00","alder_pollen":1.5,"birch_pollen":null,
                "grass_pollen":2.0,"mugwort_pollen":null,"olive_pollen":0.0,
                "ragweed_pollen":3.5}"#,
        );
        let pollen = pollen_of(&block, 0).expect("a partial block is a forecast");
        assert_eq!(pollen.alder, Some(1.5));
        assert_eq!(pollen.birch, None, "not measured, never a measured zero");
        assert_eq!(pollen.grass, Some(2.0));
        assert_eq!(pollen.mugwort, None);
        assert_eq!(pollen.olive, Some(0.0), "a reported zero stays a zero");
        assert_eq!(pollen.ragweed, Some(3.5));
    }

    #[test]
    fn an_all_null_pollen_block_is_no_coverage() {
        let block = block(r#"{"time":"2026-10-03T20:00"}"#);
        assert_eq!(pollen_of(&block, 0), None);
    }

    #[test]
    fn a_missing_current_block_is_an_upstream_error() {
        let text = r#"{"utc_offset_seconds":0,"timezone":"UTC","current_units":{
            "pm2_5":"μg/m³","pm10":"μg/m³","ozone":"μg/m³","nitrogen_dioxide":"μg/m³",
            "sulphur_dioxide":"μg/m³","carbon_monoxide":"μg/m³","alder_pollen":"grains/m³",
            "birch_pollen":"grains/m³","grass_pollen":"grains/m³","mugwort_pollen":"grains/m³",
            "olive_pollen":"grains/m³","ragweed_pollen":"grains/m³"}}"#;
        let response: AirResponse = serde_json::from_str(text).expect("the envelope parses");
        let error = decode(&response, 0)
            .map(|_| ())
            .expect_err("there is no reading");
        assert!(error.to_string().contains("no `current` block"), "{error}");
    }

    #[test]
    fn a_unit_mismatch_names_the_field_and_the_received_unit() {
        let text = r#"{"utc_offset_seconds":0,"timezone":"UTC","current_units":{
            "pm2_5":"mg/m³","pm10":"μg/m³","ozone":"μg/m³","nitrogen_dioxide":"μg/m³",
            "sulphur_dioxide":"μg/m³","carbon_monoxide":"μg/m³","alder_pollen":"grains/m³",
            "birch_pollen":"grains/m³","grass_pollen":"grains/m³","mugwort_pollen":"grains/m³",
            "olive_pollen":"grains/m³","ragweed_pollen":"grains/m³"},"current":{"time":"2026-10-03T20:00"}}"#;
        let response: AirResponse = serde_json::from_str(text).expect("the envelope parses");
        let error = decode(&response, 0)
            .map(|_| ())
            .expect_err("the unit is not the expected one");
        assert!(
            error.to_string().contains("`pm2_5` is reported in `mg/m³`"),
            "{error}"
        );
        assert!(error.to_string().contains("expected `μg/m³`"), "{error}");
    }

    #[test]
    fn the_zone_name_is_cross_checked_against_the_offset() {
        let response = envelope(r#"{"time":"2026-10-03T20:00","us_aqi":43}"#);
        let reading = decode(&response, 0).expect("offset and zone agree");
        assert_eq!(reading.time.to_rfc3339(), "2026-10-03T20:00:00+02:00");

        let mut wrong = envelope(r#"{"time":"2026-10-03T20:00","us_aqi":43}"#);
        wrong.utc_offset_seconds = 0;
        let error = decode(&wrong, 0).map(|_| ()).expect_err("they disagree");
        assert!(error.to_string().contains("Europe/Berlin"), "{error}");
    }
}
