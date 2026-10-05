// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Marine conditions: the wave, swell and sea-surface readings the marine panel renders.
//!
//! The type lives in the model, next to everything else a [`Report`](super::Report) carries,
//! because the renderers may read the model but never the module that fetches the data (`src/
//! provider` owns the fetching), and [`Report::marine`](super::Report) defaults to `None` so a
//! document written before the block existed still parses.
//!
//! The rules that shape the type:
//!
//! * **Canonical metric.** Wave heights are metres, periods seconds, temperatures °C and the
//!   direction degrees clockwise from north — the same single conversion point every other
//!   reading goes through, so `--units` never reaches the wire and the cache stays unit-free.
//! * **`None` means "not reported", never zero.** A calm sea still reports a height; an API that
//!   omits `swell_wave_height` did not measure it, and a zero-filled panel would invent a flat
//!   sea. The `current` block itself is required: a response without one is an upstream error,
//!   and `Report::marine` stays `None` rather than becoming an all-null block.
//! * **The sampled cell travels with the reading.** A marine API answers for the nearest sea
//!   point, which for a coastal or inland location can be tens of kilometres away; the panel
//!   names that coordinate (and its distance) instead of silently presenting sea data as the
//!   coordinate the user asked about.

use chrono::{DateTime, FixedOffset, NaiveDate};
use serde::{Deserialize, Serialize};

/// How far the sampled sea cell may sit from the requested point before the panel says so.
///
/// A coastal point usually lands within a few kilometres; a point further inland gets the nearest
/// sea cell, and presenting that as "the waves at your coordinates" without a word would be
/// wrong. The threshold is one constant so the fetcher and the renderer cannot disagree.
pub const FAR_CELL_KM: f64 = 25.0;

/// Where a marine reading came from.
///
/// An enum rather than a string, so a later source is a variant and not a schema change; the JSON
/// renderer spells the value out itself and cannot drift with a rename.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MarineSource {
    /// Open-Meteo's Marine Weather API (Copernicus Marine and DWD ICON Wave data).
    OpenMeteoMarine,
}

impl MarineSource {
    /// Every source, in registry order; the fetcher dispatches over this array.
    pub const ALL: [Self; 1] = [Self::OpenMeteoMarine];

    /// The id the JSON document and the request URLs use.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OpenMeteoMarine => "open-meteo-marine",
        }
    }

    /// The human-readable name, for the panel's provenance line.
    #[must_use]
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::OpenMeteoMarine => "Open-Meteo Marine",
        }
    }

    /// The credit the source's licence requires, in the wording the panel prints.
    #[must_use]
    pub const fn licence(self) -> &'static str {
        match self {
            Self::OpenMeteoMarine => {
                "Marine data: Open-Meteo.com (CC BY 4.0) — https://open-meteo.com/ (Copernicus \
                 Marine Service, DWD ICON Wave)"
            }
        }
    }
}

impl std::fmt::Display for MarineSource {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// One day of the marine forecast.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MarineDay {
    /// The location-local date.
    pub date: NaiveDate,
    /// Highest significant wave height that day, in metres.
    pub wave_height_max_m: Option<f64>,
    /// Longest wave period that day, in seconds.
    pub wave_period_max_s: Option<f64>,
    /// Dominant wave direction that day, degrees clockwise from north.
    pub wave_direction_dominant_deg: Option<u16>,
}

/// One marine reading for a location: the current sea state, the daily wave summary and the cell
/// the answer was sampled at.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Marine {
    /// Observation time, in the location's local offset.
    pub time: DateTime<FixedOffset>,
    /// Significant wave height in metres.
    pub wave_height_m: Option<f64>,
    /// Wave direction the swell travels *from*, degrees clockwise from north.
    pub wave_direction_deg: Option<u16>,
    /// Peak wave period in seconds.
    pub wave_period_s: Option<f64>,
    /// Swell wave height in metres (the part of the sea state that travelled in from elsewhere).
    pub swell_wave_height_m: Option<f64>,
    /// Sea surface temperature in °C.
    pub sea_surface_temp_c: Option<f64>,
    /// The daily wave summary; empty when the source answered with no daily block.
    pub days: Vec<MarineDay>,
    /// Latitude of the sea cell the answer was sampled at.
    pub sampled_lat: f64,
    /// Longitude of the sea cell the answer was sampled at.
    pub sampled_lon: f64,
    /// Distance between the requested point and the sampled cell, in kilometres.
    pub distance_km: f64,
    /// Which service answered.
    pub source: MarineSource,
}

impl Marine {
    /// Whether the sampled cell is far enough away that a panel must name it.
    #[must_use]
    pub fn sampled_cell_is_far(&self) -> bool {
        self.distance_km > FAR_CELL_KM
    }

    /// Whether the reading carries any measurement at all; an all-`None` current block is not a
    /// reading and the provider refuses it instead of attaching an empty panel.
    #[must_use]
    pub fn has_readings(&self) -> bool {
        self.wave_height_m.is_some()
            || self.wave_direction_deg.is_some()
            || self.wave_period_s.is_some()
            || self.swell_wave_height_m.is_some()
            || self.sea_surface_temp_c.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::{FAR_CELL_KM, Marine, MarineDay, MarineSource};

    fn reading(distance_km: f64) -> Marine {
        Marine {
            time: chrono::DateTime::parse_from_rfc3339("2026-10-06T20:00:00+02:00")
                .expect("a valid instant"),
            wave_height_m: Some(1.4),
            wave_direction_deg: Some(280),
            wave_period_s: Some(6.8),
            swell_wave_height_m: Some(0.9),
            sea_surface_temp_c: Some(14.2),
            days: vec![MarineDay {
                date: chrono::NaiveDate::from_ymd_opt(2026, 10, 6).expect("a valid date"),
                wave_height_max_m: Some(1.7),
                wave_period_max_s: Some(7.1),
                wave_direction_dominant_deg: Some(275),
            }],
            sampled_lat: 54.541_664,
            sampled_lon: 10.208_343,
            distance_km,
            source: MarineSource::OpenMeteoMarine,
        }
    }

    #[test]
    fn a_reading_round_trips_through_serde() {
        let value = reading(1.2);
        let text = serde_json::to_string(&value).expect("the reading encodes");
        let parsed: Marine = serde_json::from_str(&text).expect("the reading decodes");
        assert_eq!(parsed, value);
        assert!(text.contains("\"source\":\"open-meteo-marine\""), "{text}");
    }

    #[test]
    fn the_sampled_cell_is_far_only_past_the_threshold() {
        assert!(!reading(FAR_CELL_KM - 0.1).sampled_cell_is_far());
        assert!(!reading(FAR_CELL_KM).sampled_cell_is_far());
        assert!(reading(FAR_CELL_KM + 0.1).sampled_cell_is_far());
    }

    #[test]
    fn an_all_null_current_block_is_not_a_reading() {
        let mut value = reading(1.0);
        value.wave_height_m = None;
        value.wave_direction_deg = None;
        value.wave_period_s = None;
        value.swell_wave_height_m = None;
        value.sea_surface_temp_c = None;
        assert!(!value.has_readings());
        assert!(reading(1.0).has_readings());
    }

    #[test]
    fn every_source_spells_its_own_id_and_credit() {
        for source in MarineSource::ALL {
            assert_eq!(source.as_str(), "open-meteo-marine");
            assert_eq!(source.to_string(), source.as_str());
            assert_ne!(source.display_name(), "");
            assert!(source.licence().contains("CC BY 4.0"));
        }
    }
}
