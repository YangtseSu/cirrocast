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
}

impl AirSource {
    /// Every source, in registry order; the fetcher dispatches over this array.
    pub const ALL: [Self; 1] = [Self::OpenMeteo];

    /// The id the JSON document and the request URLs use.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OpenMeteo => "open-meteo",
        }
    }

    /// The human-readable name, for the panel's provenance line.
    #[must_use]
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::OpenMeteo => "Open-Meteo",
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
    pub aqi_us: Option<u16>,
    /// European AQI, exactly as the source reports it.
    pub aqi_european: Option<u16>,
    /// Fine particulate matter (PM2.5) in μg/m³.
    pub pm2_5: Option<f64>,
    /// Coarse particulate matter (PM10) in μg/m³.
    pub pm10: Option<f64>,
    /// Ground-level ozone in μg/m³.
    pub o3: Option<f64>,
    /// Nitrogen dioxide in μg/m³.
    pub no2: Option<f64>,
    /// Sulphur dioxide in μg/m³.
    pub so2: Option<f64>,
    /// Carbon monoxide in μg/m³.
    pub co: Option<f64>,
    /// Pollen forecast; `None` when the source does not cover this point.
    pub pollen: Option<Pollen>,
    /// Which service answered.
    pub source: AirSource,
}

#[cfg(test)]
mod tests {
    use super::{AirQuality, AirSource, POLLEN_UNIT, POLLUTANT_UNIT};

    #[test]
    fn every_source_spells_its_own_id_and_display_name() {
        for source in AirSource::ALL {
            assert_eq!(source.as_str(), "open-meteo");
            assert_eq!(source.to_string(), source.as_str());
            assert_ne!(source.display_name(), "");
        }
        assert_eq!(POLLUTANT_UNIT, "μg/m³");
        assert_eq!(POLLEN_UNIT, "grains/m³");
    }

    #[test]
    fn a_reading_round_trips_through_serde() {
        let reading = AirQuality {
            time: chrono::DateTime::parse_from_rfc3339("2026-10-03T20:00:00+02:00")
                .expect("a valid instant"),
            aqi_us: Some(43),
            aqi_european: Some(42),
            pm2_5: Some(8.2),
            pm10: Some(13.3),
            o3: Some(38.0),
            no2: Some(27.9),
            so2: Some(3.0),
            co: Some(251.0),
            pollen: None,
            source: AirSource::OpenMeteo,
        };
        let text = serde_json::to_string(&reading).expect("the reading encodes");
        let parsed: AirQuality = serde_json::from_str(&text).expect("the reading decodes");
        assert_eq!(parsed, reading);
        assert!(text.contains("\"source\":\"open-meteo\""), "{text}");
    }
}
