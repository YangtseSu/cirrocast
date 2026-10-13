// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Air quality: the pollutant, pollen and index readings the air panel renders.
//!
//! The types live in the model, next to everything else a [`Report`](super::Report) carries,
//! because the renderers may read the model but never the module that fetches the data
//! (`src/air` owns the fetching, exactly as `src/alerts` owns the warnings).
//!
//! The rules that shape the type:
//!
//! * **No unit conversion.** Pollutants are stored in μg/m³ and pollen in grams/m³ exactly as the
//!   source reports them. The contract's single conversion point converts *weather* readings,
//!   because a user may prefer °F or mph; an AQI category is not unit dependent, and mixing
//!   μg/ft³ into a category scale would be meaningless. `--units` therefore leaves these numbers
//!   alone (documented in the units table of step 03).
//! * **Indices stay raw.** `aqi_us` and `aqi_european` are the source's own numbers; the category
//!   is derived for display by [`crate::air::aqi::AqiCategory`], never stored.
//! * **`None` means "not reported", never zero.** The only pollen forecast is the CAMS European
//!   domain: a point outside it carries `pollen: None`, a covered point with no pollen in the air
//!   carries `Some(Pollen)` with measured zeros, and a species the source did not report inside a
//!   covered block keeps that member `None` in turn. The representations must not collapse into
//!   one, because "0 grains" and "not measured" are different answers.

use chrono::{DateTime, FixedOffset};
use serde::{Deserialize, Serialize};

/// The unit every [`AirQuality`] pollutant value is in.
pub const POLLUTANT_UNIT: &str = "μg/m³";

/// The unit every [`Pollen`] value is in.
pub const POLLEN_UNIT: &str = "grains/m³";

/// A pollen forecast in grains/m³, one field per species the source models.
///
/// A member is `Some(0.0)` when the source measured no pollen of that species and `None` when it
/// did not report the species at all: `0 grains` and `not measured` are different answers, so a
/// partial block keeps its missing members `None` and the renderers omit them rather than
/// inventing a zero. A block the source does not cover at all stays [`AirQuality::pollen`]'s
/// `None`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Pollen {
    /// Alder pollen; `None` when the source did not report it.
    pub alder: Option<f64>,
    /// Birch pollen; `None` when the source did not report it.
    pub birch: Option<f64>,
    /// Grass pollen; `None` when the source did not report it.
    pub grass: Option<f64>,
    /// Mugwort pollen; `None` when the source did not report it.
    pub mugwort: Option<f64>,
    /// Olive pollen; `None` when the source did not report it.
    pub olive: Option<f64>,
    /// Ragweed pollen; `None` when the source did not report it.
    pub ragweed: Option<f64>,
}

impl Pollen {
    /// The species ids, in the display order of [`Self::values`]; the renderer pairs them with
    /// their catalog labels by position.
    pub const SPECIES: [&'static str; 6] =
        ["alder", "birch", "grass", "mugwort", "olive", "ragweed"];

    /// The six readings, in [`Self::SPECIES`] order; a `None` member was not measured.
    #[must_use]
    pub fn values(&self) -> [Option<f64>; 6] {
        [
            self.alder,
            self.birch,
            self.grass,
            self.mugwort,
            self.olive,
            self.ragweed,
        ]
    }
}

/// Where an air-quality reading came from.
///
/// An enum rather than a string, so a later source (step 23's candidates) is a variant, not a
/// schema change; the JSON renderer spells the value out itself and cannot drift with a rename.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AirSource {
    /// Open-Meteo's Air Quality API (CAMS ENSEMBLE data).
    OpenMeteo,
    /// `QWeather`'s air-quality API (step 29): the Chinese national AQI and the local index set.
    QWeather,
}

impl AirSource {
    /// Every source, in registry order; the fetcher dispatches over this array.
    pub const ALL: [Self; 2] = [Self::OpenMeteo, Self::QWeather];

    /// The id the JSON document and the request URLs use.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OpenMeteo => "open-meteo",
            Self::QWeather => "qweather",
        }
    }

    /// The human-readable name, for the panel's provenance line.
    #[must_use]
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::OpenMeteo => "Open-Meteo",
            Self::QWeather => "QWeather",
        }
    }

    /// Parses a source name, case-insensitively.
    ///
    /// The `--air-source` flag and the `[air] source` validator go through here, so the two cannot
    /// disagree about a spelling. `auto` is *not* a source and is parsed by
    /// [`crate::air::AirChoice`], which owns the selection rule.
    pub fn from_name(name: &str) -> crate::error::Result<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "open-meteo" => Ok(Self::OpenMeteo),
            "qweather" => Ok(Self::QWeather),
            other => Err(crate::error::Error::Usage(format!(
                "unknown air-quality source `{other}`; known sources: open-meteo, qweather"
            ))),
        }
    }
}

impl std::fmt::Display for AirSource {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// One air-quality reading for a location: the two indices, the six regulated pollutants and,
/// where the source covers it, the pollen forecast.
///
/// Every measurement is optional because the source may omit one field; the *reading* itself
/// (`time`, `source`) is always present, and a response without a `current` block is an upstream
/// error rather than an empty reading.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AirQuality {
    /// Observation time, in the location's local offset.
    pub time: DateTime<FixedOffset>,
    /// US AQI, exactly as the source reports it (`0..=500`; above 500 is off the scale).
    #[serde(default)]
    pub aqi_us: Option<u16>,
    /// European AQI, exactly as the source reports it.
    #[serde(default)]
    pub aqi_european: Option<u16>,
    /// The Chinese national AQI (GB 3095-2012, `cn-mee`), exactly as the source reports it.
    ///
    /// A field of its own rather than a second reading of [`Self::aqi_us`]: the two scales share
    /// neither breakpoints nor words, and `QWeather` publishes the national index only for mainland
    /// China (its own QAQI is explicitly not available there). `None` outside that territory or
    /// when the source does not carry the index.
    #[serde(default)]
    pub aqi_cn: Option<u16>,
    /// Fine particulate matter (PM2.5) in μg/m³.
    #[serde(default)]
    pub pm2_5: Option<f64>,
    /// Coarse particulate matter (PM10) in μg/m³.
    #[serde(default)]
    pub pm10: Option<f64>,
    /// Ground-level ozone in μg/m³.
    #[serde(default)]
    pub o3: Option<f64>,
    /// Nitrogen dioxide in μg/m³.
    #[serde(default)]
    pub no2: Option<f64>,
    /// Sulphur dioxide in μg/m³.
    #[serde(default)]
    pub so2: Option<f64>,
    /// Carbon monoxide in μg/m³.
    #[serde(default)]
    pub co: Option<f64>,
    /// Pollen forecast; `None` when the source does not cover this point.
    #[serde(default)]
    pub pollen: Option<Pollen>,
    /// Which service answered.
    pub source: AirSource,
    /// The attribution lines the source's terms require displayed with its data, verbatim.
    ///
    /// `QWeather` publishes them as `metadata.attributions` and its attribution terms demand they
    /// be shown in full, unmodified, wherever its air-quality data is shown; the air panel and
    /// `plain` print them after the source's own credit line, and `--format json` carries them in
    /// the `air` object. Empty for sources whose terms ask for nothing beyond that line.
    #[serde(default)]
    pub credit: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::{AirQuality, AirSource, POLLEN_UNIT, POLLUTANT_UNIT};

    #[test]
    fn every_source_spells_its_own_id_and_display_name() {
        let mut ids = std::collections::HashSet::new();
        for source in AirSource::ALL {
            assert!(ids.insert(source.as_str()), "{source:?}");
            assert_eq!(source.to_string(), source.as_str());
            assert_ne!(source.display_name(), "");
            assert_eq!(
                AirSource::from_name(source.as_str()).expect("its own spelling parses"),
                source
            );
        }
        assert_eq!(AirSource::QWeather.as_str(), "qweather");
        assert_eq!(POLLUTANT_UNIT, "μg/m³");
        assert_eq!(POLLEN_UNIT, "grains/m³");
    }

    #[test]
    fn an_unknown_source_name_names_the_known_ones() {
        let error = AirSource::from_name(" Quest ").expect_err("never a source");
        assert!(error.to_string().contains("known sources"), "{error}");
    }

    #[test]
    fn a_reading_round_trips_through_serde() {
        let reading = AirQuality {
            time: chrono::DateTime::parse_from_rfc3339("2026-10-03T20:00:00+02:00")
                .expect("a valid instant"),
            aqi_us: Some(43),
            aqi_european: Some(42),
            aqi_cn: Some(20),
            pm2_5: Some(8.2),
            pm10: Some(13.3),
            o3: Some(38.0),
            no2: Some(27.9),
            so2: Some(3.0),
            co: Some(251.0),
            pollen: None,
            source: AirSource::OpenMeteo,
            credit: vec!["https://developer.qweather.com/attribution.html".to_owned()],
        };
        let text = serde_json::to_string(&reading).expect("the reading encodes");
        let parsed: AirQuality = serde_json::from_str(&text).expect("the reading decodes");
        assert_eq!(parsed, reading);
        assert!(text.contains("\"source\":\"open-meteo\""), "{text}");
    }

    #[test]
    fn a_document_written_before_the_cn_reading_still_parses() {
        // Step 29 added `aqi_cn` and `credit` to the reading; a document (or cache envelope) that
        // predates them must decode rather than fail, with the new halves absent.
        let text = r#"{"time":"2026-10-03T20:00:00+02:00","aqi_us":43,"aqi_european":42,
            "pm2_5":8.2,"pm10":13.3,"o3":38.0,"no2":27.9,"so2":3.0,"co":251.0,
            "pollen":null,"source":"open-meteo"}"#;
        let reading: AirQuality = serde_json::from_str(text).expect("the reading decodes");
        assert_eq!(reading.aqi_cn, None);
        assert_eq!(reading.credit, [] as [std::string::String; 0]);
    }
}
