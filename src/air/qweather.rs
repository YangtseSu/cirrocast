// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! `QWeather`'s air-quality API: the Chinese national AQI and the local index set.
//!
//! `{host}/airquality/v1/current/{lat}/{lon}` is the endpoint this source speaks, one request per
//! reading. Its value over the keyless Open-Meteo source is the *index*: the payload carries the
//! Chinese national AQI (`cn-mee`, 环境空气质量指数 GB 3095-2012) beside `QWeather`'s own universal
//! QAQI and the point's local standard, and the CAMS data behind Open-Meteo cannot produce it.
//! Measured 2026-10-13 on the account host:
//!
//! * the envelope is `metadata` + `indexes[]` + `pollutants[]`; a mainland-China point carries
//!   exactly one index (`cn-mee`, QAQI is documented as not available there), a point elsewhere
//!   carries `qaqi` plus the local standard (`us-epa`, `eu-eea`, `jp-moe`, `sg-nea` were seen);
//! * **there is no observation time in the payload** — the reading's `time` is therefore the fetch
//!   instant, converted to the location's offset, exactly as the forecast backend stamps its own
//!   `observed_at` when upstream omits one;
//! * the concentration unit varies **per pollutant by design**: `μg/m³` for the particulates and
//!   for the Chinese/European points' gases, `mg/m³` for CO there, and `ppb`/`ppm` for the gases at
//!   the US and Japanese points. [`concentration_ug_m3`] is the whole policy, with the factors and
//!   their reference written down beside it; a unit outside that table fails the reading rather
//!   than putting a number from one scale into a panel that reads like another.
//!
//! The credential and host are the `qweather` provider's (`providers.qweather.host` and whichever
//! of the API key / JWT the key store resolves, step 27), through the same [`QWeatherAuth`] helper
//! the forecast backend and the alert source use, so all three follow the configured mode with no
//! second code path. The `401` taxonomy is the shared one: [`fetch_json`] turns it into
//! [`Error::InvalidCredential`], whose message names both `key set` forms.
//!
//! Nothing upstream text-wise is displayed: the panel derives its category word from the number
//! with its own catalog (the response's `category` is a *localised* string — `Excellent` under
//! `lang=en`, `优` under `lang=zh` — and `indexes[].code` is the stable part), so the request
//! carries no `lang` parameter.
//!
//! The response's `metadata.attributions` travel on the reading and are printed after the source's
//! credit line: `QWeather`'s attribution terms require the list shown in full and unmodified
//! wherever its air-quality data is shown (`https://dev.qweather.com/docs/terms/attribution/`).

use chrono::{DateTime, FixedOffset, Utc};
use serde::Deserialize;

use crate::auth::QWeatherAuth;
use crate::cache::CacheKey;
use crate::error::{Error, Result};
use crate::http::HttpRequest;
use crate::model::air::POLLUTANT_UNIT;
use crate::model::{AirQuality, AirSource, Location};
use crate::provider::{
    Env, JsonFetch, ProviderId, fetch_json, local_today, missing_credential, weather_ttl,
};

/// The provider id whose credential and host this source borrows.
const PROVIDER: &str = "qweather";

/// The endpoint path, after the account host.
const PATH: &str = "/airquality/v1/current";

/// Fetches one reading for `loc`.
pub fn fetch(loc: &Location, env: &Env<'_>) -> Result<AirQuality> {
    let host = host(env)?;
    let variable = ProviderId::QWeather
        .metadata()
        .key_env
        .unwrap_or("CIRROCAST_QWEATHER_KEY");
    // Step 27: the same store resolution and the same header helper the forecast backend uses.
    let credential = env
        .keys
        .credential(ProviderId::QWeather)?
        .ok_or_else(|| missing_credential(ProviderId::QWeather, variable))?;
    let auth = QWeatherAuth::resolve(&credential, env.cache.clock().now())?;

    let request = auth.apply(HttpRequest::get(format!(
        "{host}{PATH}/{:.4}/{:.4}",
        loc.lat, loc.lon
    )));
    let key = CacheKey::air(PROVIDER, loc.lat, loc.lon, local_today(env, loc.tz));
    let response: Response = fetch_json(
        env,
        loc,
        &JsonFetch {
            provider: ProviderId::QWeather,
            request,
            key,
            ttl: weather_ttl(env),
            what: "air quality",
        },
    )?;

    // The payload carries no timestamp (recorded 2026-10-13), so the fetch instant is the
    // reading's time, stamped in the location's offset like every other artifact.
    let at: DateTime<Utc> = env.cache.clock().now().into();
    decode(
        &response,
        at.with_timezone(&loc.tz).fixed_offset(),
        env.verbose,
    )
}

/// The account's API host, from `[providers.qweather].host`.
fn host(env: &Env<'_>) -> Result<String> {
    let host = env
        .config
        .providers
        .qweather
        .host
        .trim()
        .trim_end_matches('/');
    if host.is_empty() {
        return Err(Error::Config(format!(
            "air source `{PROVIDER}` needs `providers.{PROVIDER}.host` (see \
             https://console.qweather.com/)"
        )));
    }
    Ok(host.to_owned())
}

// ---------------------------------------------------------------------------------------------
// Response shape
// ---------------------------------------------------------------------------------------------

/// The response envelope, in the subset `cirrocast` consumes.
///
/// `indexes` and `pollutants` are optional-by-default rather than required: a maintenance window
/// answers with `metadata` and an empty half, which is a reading with no numbers rather than a
/// decode error, and the panel prints what is there.
#[derive(Debug, Deserialize)]
struct Response {
    #[serde(default)]
    metadata: Metadata,
    #[serde(default)]
    indexes: Vec<Index>,
    #[serde(default)]
    pollutants: Vec<Pollutant>,
}

/// The response's `metadata` object: the attribution lines the terms require shown with the data.
#[derive(Debug, Default, Deserialize)]
struct Metadata {
    #[serde(default)]
    attributions: Vec<String>,
}

/// One `indexes[]` entry: the stable `code` and the number; `category` and `color` are upstream
/// presentation (and localised), so the model derives its own word and colour instead.
#[derive(Debug, Deserialize)]
struct Index {
    /// The standard's code, e.g. `cn-mee`, `us-epa`, `qaqi`.
    code: String,
    /// The index value. QAQI is fractional (0.0–10.0); the national scales are whole numbers.
    aqi: f64,
}

/// One `pollutants[]` entry.
#[derive(Debug, Deserialize)]
struct Pollutant {
    /// The pollutant code, e.g. `pm2p5`, `no2`.
    code: String,
    /// The measured concentration with its own unit.
    concentration: Concentration,
}

/// `concentration`: the value and the unit it is expressed in.
#[derive(Debug, Deserialize)]
struct Concentration {
    /// The measured value, in `unit`.
    value: f64,
    /// The unit the value is in; see [`concentration_ug_m3`].
    unit: String,
}

// ---------------------------------------------------------------------------------------------
// Response → model
// ---------------------------------------------------------------------------------------------

/// Which of the model's fields an index code belongs to.
///
/// The three mapped codes are the ones this client can name: the Chinese national scale it was
/// written for, the US and European scales the panel already carried. Everything else `QWeather`
/// documents (`ca-eccc`, `cn-mee-1h`, `fr-atmo`, `gb-defra`, `hk-epd`, `jp-moe`, `kr-moe`,
/// `mo-smg`, `qaqi`, `sg-nea`, `sg-nea-pm1h`, `th-pcd`, `tw-me`, `tw-me-1h`, `us-epa-nc`) is
/// named in [`IGNORED_INDEXES`] and skipped with a `--verbose` note: guessing a scale's
/// breakpoints from its name is how a panel prints a plausible wrong word. The list is dated
/// 2026-10-13 (the vendor's supported-AQI table); a code that is not even in it is still only a
/// note, so a new standard appearing upstream can never break a run.
const IGNORED_INDEXES: [&str; 15] = [
    "ca-eccc",
    "cn-mee-1h",
    "fr-atmo",
    "gb-defra",
    "hk-epd",
    "jp-moe",
    "kr-moe",
    "mo-smg",
    "qaqi",
    "sg-nea",
    "sg-nea-pm1h",
    "th-pcd",
    "tw-me",
    "tw-me-1h",
    "us-epa-nc",
];

/// Turns one response into the model reading.
fn decode(response: &Response, at: DateTime<FixedOffset>, verbose: u8) -> Result<AirQuality> {
    let mut aqi_us = None;
    let mut aqi_european = None;
    let mut aqi_cn = None;
    for index in &response.indexes {
        let slot = match index.code.as_str() {
            "us-epa" => &mut aqi_us,
            "eu-eea" => &mut aqi_european,
            "cn-mee" => &mut aqi_cn,
            _ => {
                if verbose > 0 {
                    if IGNORED_INDEXES.contains(&index.code.as_str()) {
                        eprintln!(
                            "air: QWeather index `{}` has no scale in this client; ignored",
                            index.code
                        );
                    } else {
                        eprintln!(
                            "air: QWeather index code `{}` is not one this client knows; ignored",
                            index.code
                        );
                    }
                }
                continue;
            }
        };
        if slot.is_some() {
            if verbose > 0 {
                eprintln!(
                    "air: QWeather reports index `{}` more than once; the first value is shown",
                    index.code
                );
            }
            continue;
        }
        *slot = Some(whole_index(index)?);
    }

    let mut reading = AirQuality {
        time: at,
        aqi_us,
        aqi_european,
        aqi_cn,
        pm2_5: None,
        pm10: None,
        o3: None,
        no2: None,
        so2: None,
        co: None,
        pollen: None,
        source: AirSource::QWeather,
        credit: response
            .metadata
            .attributions
            .iter()
            .filter(|line| !line.trim().is_empty())
            .cloned()
            .collect(),
    };

    for pollutant in &response.pollutants {
        let Some(field) = field_of(&pollutant.code) else {
            // A code the model does not carry (nitric oxide, NMHC, a new one) is not an error:
            // the panel shows the six regulated pollutants and ignores the rest, with the note a
            // reader can act on.
            if verbose > 0 {
                eprintln!(
                    "air: QWeather reports pollutant `{}`, which this client does not model; ignored",
                    pollutant.code
                );
            }
            continue;
        };
        let value = concentration_ug_m3(
            &pollutant.code,
            pollutant.concentration.value,
            &pollutant.concentration.unit,
        )?;
        let slot = match field {
            Field::Pm2_5 => &mut reading.pm2_5,
            Field::Pm10 => &mut reading.pm10,
            Field::O3 => &mut reading.o3,
            Field::No2 => &mut reading.no2,
            Field::So2 => &mut reading.so2,
            Field::Co => &mut reading.co,
        };
        if slot.is_none() {
            *slot = Some(value);
        }
    }

    if verbose > 0
        && reading.aqi_us.is_none()
        && reading.aqi_european.is_none()
        && reading.aqi_cn.is_none()
    {
        eprintln!(
            "air: QWeather answered with no index this client maps; showing the pollutants only"
        );
    }
    Ok(reading)
}

/// The model field one pollutant code fills.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Field {
    Pm2_5,
    Pm10,
    O3,
    No2,
    So2,
    Co,
}

/// The field a pollutant code belongs to, or `None` for a code the model does not carry.
#[must_use]
fn field_of(code: &str) -> Option<Field> {
    match code {
        "pm2p5" => Some(Field::Pm2_5),
        "pm10" => Some(Field::Pm10),
        "o3" => Some(Field::O3),
        "no2" => Some(Field::No2),
        "so2" => Some(Field::So2),
        "co" => Some(Field::Co),
        _ => None,
    }
}

/// The molar volume the ppb/ppm conversion assumes: 24.45 L/mol, the ideal gas at 25 °C and
/// 101.325 kPa.
///
/// The same reference the published per-gas factors are quoted at, which is why the mass this
/// module computes for NO₂ (1.88 μg/m³ per ppb), O₃ (1.96), SO₂ (2.62) and CO (1.15) reproduces
/// them to two decimals. The alternative reference (22.4 L/mol at 0 °C) would move every converted
/// gas by ~9 %: the assumption is therefore part of the policy, not a detail, and
/// [`concentration_ug_m3`] is where it lives.
const MOLAR_VOLUME_L: f64 = 24.45;

/// A gas pollutant's molar mass in g/mol, for the ppb/ppm conversion.
///
/// The particulates are not gases: `pm2p5`/`pm10` have no ppb spelling, and a payload that
/// reports one is a mismatch rather than a conversion.
#[must_use]
fn molar_mass(code: &str) -> Option<f64> {
    match code {
        "o3" => Some(48.00),
        "no2" => Some(46.0055),
        "so2" => Some(64.066),
        "co" => Some(28.010),
        _ => None,
    }
}

/// One pollutant concentration in μg/m³, or an upstream error naming the unit this client cannot
/// convert.
///
/// The unit table is a measurement (2026-10-13), not a promise: `μg/m³` and `mg/m³` are exact
/// factors of 1000, `ppb` adds `molar mass / 24.45` and `ppm` multiplies that by 1000 again. A
/// unit outside the table — including a gas-phase unit on a particulate, or a unit the vendor
/// starts sending after this step — fails the reading, the way `src/air/open_meteo.rs` fails a
/// mismatched one.
fn concentration_ug_m3(code: &str, value: f64, unit: &str) -> Result<f64> {
    let per_ppb = |mass: f64| mass / MOLAR_VOLUME_L;
    let converted = match unit {
        POLLUTANT_UNIT => value,
        "mg/m³" => value * 1000.0,
        "ppb" => match molar_mass(code) {
            Some(mass) => value * per_ppb(mass),
            None => return Err(unconvertible(code, unit)),
        },
        "ppm" => match molar_mass(code) {
            Some(mass) => value * 1000.0 * per_ppb(mass),
            None => return Err(unconvertible(code, unit)),
        },
        _ => return Err(unconvertible(code, unit)),
    };
    if !converted.is_finite() || converted < 0.0 {
        return Err(upstream(format!(
            "pollutant `{code}` reports `{value} {unit}`, which is not a concentration"
        )));
    }
    Ok(converted)
}

/// The error for a unit [`concentration_ug_m3`] cannot turn into μg/m³.
fn unconvertible(code: &str, unit: &str) -> Error {
    upstream(format!(
        "pollutant `{code}` is reported in `{unit}`, which this client cannot convert to \
         `{POLLUTANT_UNIT}`"
    ))
}

/// The whole-number index a mapped scale reports, or an upstream error when the value cannot be
/// one.
///
/// The mapped scales are integer scales (`cn-mee`, `us-epa` and `eu-eea` answer whole numbers) on
/// the documented `0..=500` range; a fractional, negative or out-of-range value means upstream
/// changed the field's meaning, and rounding it into a category would be an invention.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn whole_index(index: &Index) -> Result<u16> {
    let rounded = index.aqi.round();
    if !rounded.is_finite()
        || (rounded - index.aqi).abs() > f64::EPSILON
        || !(0.0..=500.0).contains(&rounded)
    {
        return Err(upstream(format!(
            "index `{}` reports `{}`, which is not a whole index on the 0–500 scale",
            index.code, index.aqi
        )));
    }
    Ok(rounded as u16)
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
    use chrono::{DateTime, FixedOffset};

    use super::{Response, concentration_ug_m3, decode};

    /// The instant the decode stamps a reading with when upstream carries none.
    fn at() -> DateTime<FixedOffset> {
        DateTime::parse_from_rfc3339("2026-10-13T12:00:00+08:00").expect("a valid instant")
    }

    /// A recorded response, read from `tests/fixtures/qweather/air/`.
    fn recorded(name: &str) -> Response {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/qweather/air/",);
        let text = std::fs::read_to_string(format!("{path}{name}"))
            .unwrap_or_else(|error| panic!("{name} is readable: {error}"));
        serde_json::from_str(&text).unwrap_or_else(|error| panic!("{name} parses: {error}"))
    }

    /// A hand-written envelope: the response shape with only the fields under test.
    fn envelope(indexes: &str, pollutants: &str) -> Response {
        let json = format!(
            r#"{{"metadata":{{"tag":"t","attributions":["https://developer.qweather.com/attribution.html"]}},
                "indexes":[{indexes}],"pollutants":[{pollutants}]}}"#
        );
        serde_json::from_str(&json).expect("the envelope parses")
    }

    #[test]
    fn a_recorded_chinese_reading_carries_the_national_index_and_its_pollutants() {
        let reading = decode(&recorded("beijing-2026-10-13.json"), at(), 0)
            .expect("the recorded reading decodes");

        assert_eq!(reading.aqi_cn, Some(20));
        assert_eq!(reading.aqi_us, None, "the point carries no US standard");
        assert_eq!(reading.aqi_european, None);
        assert_eq!(reading.pm2_5, Some(3.0));
        assert_eq!(reading.pm10, Some(6.0));
        assert_eq!(reading.o3, Some(62.0));
        assert_eq!(reading.no2, Some(8.0));
        assert_eq!(reading.so2, Some(2.0));
        assert_eq!(
            reading.co,
            Some(200.0),
            "0.2 mg/m³ is 200 μg/m³, the unit the model stores"
        );
        assert_eq!(reading.pollen, None, "QWeather publishes no pollen");
        assert_eq!(reading.source, crate::model::AirSource::QWeather);
        assert_eq!(
            reading.credit,
            ["https://developer.qweather.com/attribution.html"]
        );
        assert_eq!(reading.time, at(), "the payload carries no timestamp");
    }

    #[test]
    fn a_recorded_european_reading_maps_the_local_standard_and_ignores_qaqi() {
        let reading =
            decode(&recorded("berlin-2026-10-13.json"), at(), 0).expect("the reading decodes");

        assert_eq!(
            reading.aqi_european,
            Some(1),
            "eu-eea is the European scale"
        );
        assert_eq!(reading.aqi_us, None);
        assert_eq!(reading.aqi_cn, None, "QAQI is not a scale this client maps");
        assert_eq!(reading.pm2_5, Some(8.58));
        assert_eq!(reading.co, None, "the European set has no CO");
    }

    #[test]
    fn a_point_without_a_mapped_index_still_reads_its_pollutants() {
        // Tokyo answers `qaqi` and `jp-moe`: neither is a scale this client carries, and the
        // pollutants are still worth showing.
        let reading =
            decode(&recorded("tokyo-2026-10-13.json"), at(), 0).expect("the reading decodes");

        assert_eq!(reading.aqi_us, None);
        assert_eq!(reading.aqi_european, None);
        assert_eq!(reading.aqi_cn, None);
        assert_eq!(reading.pm2_5, Some(9.0));
        assert_eq!(reading.pm10, Some(13.0));
        // 15.2 ppb of NO₂ at 25 °C.
        let expected = 15.2 * 46.0055 / 24.45;
        let no2 = reading.no2.expect("NO2 is converted, not dropped");
        assert!((no2 - expected).abs() < 1e-9, "{no2} vs {expected}");
        assert_eq!(reading.o3, None, "the Japanese set reports no ozone here");
    }

    #[test]
    fn every_documented_unit_of_every_carried_pollutant_has_a_factor() {
        // One row per convertible combination of the vendor's pollutant/unit table:
        // pm10 μg/m³; pm2p5 μg/m³; co mg/m³ | μg/m³ | ppm; no2 μg/m³ | ppb | ppm;
        // so2 μg/m³ | ppb | ppm; o3 μg/m³ | ppb | ppm.
        for (code, unit, value, expected) in [
            ("pm10", "μg/m³", 6.0, 6.0),
            ("pm2p5", "μg/m³", 3.0, 3.0),
            ("co", "mg/m³", 0.2, 200.0),
            ("co", "μg/m³", 200.0, 200.0),
            ("co", "ppm", 0.6, 0.6 * 1000.0 * 28.010 / 24.45),
            ("no2", "μg/m³", 8.0, 8.0),
            ("no2", "ppb", 16.5, 16.5 * 46.0055 / 24.45),
            ("no2", "ppm", 0.0165, 0.0165 * 1000.0 * 46.0055 / 24.45),
            ("so2", "μg/m³", 2.0, 2.0),
            ("so2", "ppb", 1.0, 64.066 / 24.45),
            ("so2", "ppm", 0.001, 64.066 / 24.45),
            ("o3", "μg/m³", 62.0, 62.0),
            ("o3", "ppb", 0.006, 0.006 * 48.00 / 24.45),
            ("o3", "ppm", 6e-6, 6e-6 * 1000.0 * 48.00 / 24.45),
        ] {
            let converted = concentration_ug_m3(code, value, unit)
                .unwrap_or_else(|error| panic!("{code} in {unit}: {error}"));
            assert!(
                (converted - expected).abs() < 1e-9,
                "{code} {value} {unit} → {converted}, expected {expected}"
            );
        }
    }

    #[test]
    fn the_published_factors_come_out_of_the_same_reference() {
        // The widely quoted per-gas factors (1 ppb → μg/m³) at 25 °C: NO₂ 1.88, O₃ 1.96, SO₂ 2.62,
        // CO 1.15. They are the check that the molar volume this module assumes is the one the
        // published numbers were computed with, so a change to it is visible here.
        for (code, expected) in [("no2", 1.88), ("o3", 1.96), ("so2", 2.62), ("co", 1.15)] {
            let factor = concentration_ug_m3(code, 1.0, "ppb").expect("a gas converts");
            assert!(
                (factor - expected).abs() < 0.005,
                "{code}: {factor} per ppb, expected about {expected}"
            );
        }
    }

    #[test]
    fn a_unit_outside_the_table_fails_the_reading() {
        for (code, unit) in [
            ("pm2_5", "ppb"),
            ("pm10", "ppm"),
            ("no2", "ppg"),
            ("o3", "mg/l"),
            ("co", "µg/m³"),
        ] {
            let error = concentration_ug_m3(code, 1.0, unit)
                .map(|_| ())
                .expect_err("the unit is not convertible");
            assert!(
                error
                    .to_string()
                    .contains(&format!("`{code}` is reported in `{unit}`")),
                "{error}"
            );
        }
        // The vendor's own spellings convert; a near miss (`µ` U+00B5 for `μ` U+03BC) does not,
        // which is the point: a silent acceptance is how a wrong number reads as a right one.
        assert!(concentration_ug_m3("pm2_5", 1.0, "μg/m³").is_ok());
        assert!(concentration_ug_m3("pm2_5", 1.0, "µg/m³").is_err());
    }

    #[test]
    fn a_fractional_or_negative_national_index_is_refused() {
        let response = envelope(r#"{"code":"cn-mee","aqi":20.5}"#, "");
        let error = decode(&response, at(), 0)
            .map(|_| ())
            .expect_err("20.5 is not a whole index");
        assert!(
            error.to_string().contains("`cn-mee` reports `20.5`"),
            "{error}"
        );

        let response = envelope(r#"{"code":"us-epa","aqi":-1}"#, "");
        assert!(decode(&response, at(), 0).is_err(), "a negative index");
    }

    #[test]
    fn an_unmapped_or_unknown_index_code_is_ignored_not_guessed() {
        let response = envelope(
            r#"{"code":"gb-defra","aqi":3},{"code":"zz-someday","aqi":7},{"code":"qaqi","aqi":6.3}"#,
            "",
        );
        let reading = decode(&response, at(), 1).expect("unmapped codes are not errors");
        assert_eq!(reading.aqi_us, None);
        assert_eq!(reading.aqi_european, None);
        assert_eq!(reading.aqi_cn, None);
    }

    #[test]
    fn the_first_value_of_a_repeated_index_wins() {
        let response = envelope(
            r#"{"code":"cn-mee","aqi":20},{"code":"cn-mee","aqi":99}"#,
            "",
        );
        let reading = decode(&response, at(), 1).expect("the reading decodes");
        assert_eq!(reading.aqi_cn, Some(20));
    }

    #[test]
    fn blank_attribution_lines_are_dropped_and_the_rest_kept_verbatim() {
        let json = r#"{"metadata":{"tag":"t","attributions":["QWeather","","   "]},
            "indexes":[{"code":"cn-mee","aqi":20}],"pollutants":[]}"#;
        let response: Response = serde_json::from_str(json).expect("the envelope parses");
        let reading = decode(&response, at(), 0).expect("the reading decodes");
        assert_eq!(reading.credit, ["QWeather"]);
    }

    #[test]
    fn an_empty_response_is_an_empty_reading_not_an_error() {
        // A maintenance window answers `metadata` with empty halves; there is simply nothing to
        // show, and the caller's failure policy never sees an error.
        let response = envelope("", "");
        let reading = decode(&response, at(), 1).expect("an empty reading decodes");
        assert_eq!(reading.aqi_us, None);
        assert_eq!(reading.pm2_5, None);
        assert_eq!(reading.time, at());
    }

    #[test]
    fn a_pollutant_the_model_does_not_carry_is_ignored() {
        // Tokyo's `no` (nitric oxide) and `nmhc` are in the vendor's pollutant list but not in the
        // model's six; the panel shows the six and a `-v` note, never a wrong field.
        let response = envelope(
            r#"{"code":"cn-mee","aqi":20}"#,
            r#"{"code":"pm2p5","concentration":{"value":3.0,"unit":"μg/m³"}},
               {"code":"no","concentration":{"value":7.0,"unit":"ppb"}},
               {"code":"nmhc","concentration":{"value":0.12,"unit":"ppmC"}}"#,
        );
        let reading = decode(&response, at(), 1).expect("unmapped pollutants are not errors");
        assert_eq!(reading.pm2_5, Some(3.0));
        assert_eq!(reading.no2, None);
    }
}
