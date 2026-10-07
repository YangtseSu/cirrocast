// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The canonical alert model: CAP v1.2's severity triple plus the fields every source can fill.
//!
//! Alerts arrive from six upstream shapes — `NWS` `GeoJSON`, `MeteoAlarm`'s EDR features, `QWeather`'s
//! JSON, WMO SWIC and FPAS CAP 1.2 documents, and the Hong Kong Observatory's JSON warning
//! summary — and all of them land here. The renderers, the severity ordering and the dedup logic
//! therefore have exactly one vocabulary to work with: the CAP v1.2 value sets, which every source
//! either carries natively or can be mapped into.
//!
//! Two rules that the rest of the crate depends on:
//!
//! * **Severity is ordered**, `Unknown < Minor < Moderate < Severe < Extreme`, so "strongest
//!   first" and the `--severity` threshold are one comparison rather than a match arm per format.
//! * **Liveness is `coalesce(ends, expires)`**, never `expires` alone: a live `NWS` message refreshed
//!   by an update carries an `expires` that is the *message* validity (it can lie before `onset`),
//!   while `ends` is when the event itself ends. [`Alert::effective_end`] is the single accessor
//!   for that rule.

use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, FixedOffset};
use serde::{Deserialize, Serialize};

use crate::error::Error;

/// CAP's `severity` value set, ordered from least to most severe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// The source did not report a severity, or reported a spelling this build does not know.
    #[default]
    Unknown,
    /// Minor: some inconvenient weather.
    Minor,
    /// Moderate: some hazardous weather.
    Moderate,
    /// Severe: significant threat to life or property.
    Severe,
    /// Extreme: extraordinary threat to life or property.
    Extreme,
}

impl Severity {
    /// Every variant, weakest first, for `--severity` help and error messages.
    pub const ALL: [Self; 5] = [
        Self::Unknown,
        Self::Minor,
        Self::Moderate,
        Self::Severe,
        Self::Extreme,
    ];

    /// The lowercase spelling used by `--severity`, the JSON document and the config key.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Minor => "minor",
            Self::Moderate => "moderate",
            Self::Severe => "severe",
            Self::Extreme => "extreme",
        }
    }

    /// The CAP spelling: `Unknown`, `Minor`, …; anything else maps to [`Severity::Unknown`].
    #[must_use]
    pub fn from_cap(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "minor" => Self::Minor,
            "moderate" => Self::Moderate,
            "severe" => Self::Severe,
            "extreme" => Self::Extreme,
            _ => Self::Unknown,
        }
    }
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Severity {
    type Err = Error;

    /// Case-insensitive, for `--severity` and `[alerts] severity_threshold`.
    fn from_str(input: &str) -> Result<Self, Self::Err> {
        let normalized = input.trim().to_ascii_lowercase();
        Self::ALL
            .into_iter()
            .find(|severity| severity.as_str() == normalized)
            .ok_or_else(|| {
                Error::Usage(format!(
                    "unknown severity `{input}`; levels: {}",
                    Self::ALL.map(Self::as_str).join(", ")
                ))
            })
    }
}

/// CAP's `urgency` value set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Urgency {
    /// The source did not report an urgency, or reported a spelling this build does not know.
    #[default]
    Unknown,
    /// Past: the event has passed.
    Past,
    /// Future: the event is expected but not imminent.
    Future,
    /// Expected: the event is expected (likely within the next hour).
    Expected,
    /// Immediate: the event is occurring or imminent.
    Immediate,
}

impl Urgency {
    /// The lowercase spelling used by the JSON document.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Past => "past",
            Self::Future => "future",
            Self::Expected => "expected",
            Self::Immediate => "immediate",
        }
    }

    /// The CAP spelling; anything else maps to [`Urgency::Unknown`].
    #[must_use]
    pub fn from_cap(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "past" => Self::Past,
            "future" => Self::Future,
            "expected" => Self::Expected,
            "immediate" => Self::Immediate,
            _ => Self::Unknown,
        }
    }
}

/// CAP's `certainty` value set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Certainty {
    /// The source did not report a certainty, or reported a spelling this build does not know.
    #[default]
    Unknown,
    /// Unobserved: not detected, but possible.
    Unobserved,
    /// Possible: possible but not likely.
    Possible,
    /// Unlikely: not expected to occur.
    Unlikely,
    /// Likely: likely to occur.
    Likely,
    /// Observed: the event has been observed.
    Observed,
}

impl Certainty {
    /// The lowercase spelling used by the JSON document.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Unobserved => "unobserved",
            Self::Possible => "possible",
            Self::Unlikely => "unlikely",
            Self::Likely => "likely",
            Self::Observed => "observed",
        }
    }

    /// The CAP spelling; anything else maps to [`Certainty::Unknown`].
    #[must_use]
    pub fn from_cap(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "unobserved" => Self::Unobserved,
            "possible" => Self::Possible,
            "unlikely" => Self::Unlikely,
            "likely" => Self::Likely,
            "observed" => Self::Observed,
            _ => Self::Unknown,
        }
    }
}

/// Where one alert came from.
///
/// Sources are their own registry, independent of the weather provider chain: only `QWeather` and
/// `VisualCrossing` are tied to a provider (they need its credential and host), and both are pulled
/// in only when that provider is on the chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AlertSource {
    /// The US National Weather Service `api.weather.gov` alerts endpoint.
    Nws,
    /// The `MeteoAlarm` EDR service (EUMETNET members; optional bearer token).
    MeteoAlarm,
    /// `QWeather`'s weather-alert API (China; reuses the forecast provider's credential).
    QWeather,
    /// The Hong Kong Observatory's warning summary and detail documents.
    Hko,
    /// The WMO Severe Weather Information Centre (global; operated by HKO).
    WmoSwic,
    /// The FOSS Public Alert Server (global; self-hostable).
    Fpas,
    /// Visual Crossing's alert payload (wired in step 23 with its provider).
    VisualCrossing,
}

impl AlertSource {
    /// Every source in registry order: the national services first, the global aggregators last.
    ///
    /// The order is what the banner and the `--alerts-from` error message present, so it is part of
    /// the user-visible contract rather than an implementation detail.
    pub const ALL: [Self; 7] = [
        Self::Nws,
        Self::MeteoAlarm,
        Self::QWeather,
        Self::Hko,
        Self::WmoSwic,
        Self::Fpas,
        Self::VisualCrossing,
    ];

    /// The command line and JSON spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Nws => "nws",
            Self::MeteoAlarm => "meteoalarm",
            Self::QWeather => "qweather",
            Self::Hko => "hko",
            Self::WmoSwic => "wmoswic",
            Self::Fpas => "fpas",
            Self::VisualCrossing => "visualcrossing",
        }
    }

    /// The i18n key name for the source's display name.
    #[must_use]
    pub const fn i18n_suffix(self) -> &'static str {
        self.as_str()
    }

    /// Whether the source answers anywhere on Earth.
    #[must_use]
    pub const fn is_global(self) -> bool {
        matches!(self, Self::WmoSwic | Self::Fpas | Self::VisualCrossing)
    }

    /// The provider whose credential and host this source needs, if any.
    ///
    /// `None` means the source fetches independently of the weather chain; `Some(id)` means it is
    /// selected only when that provider is on the chain.
    #[must_use]
    pub const fn provider(self) -> Option<&'static str> {
        match self {
            Self::QWeather => Some("qweather"),
            Self::VisualCrossing => Some("visualcrossing"),
            _ => None,
        }
    }

    /// Whether this source is responsible for `loc`, judged by the resolved country code.
    ///
    /// A location resolved from raw coordinates carries no country code, and then the two services
    /// whose territory is a compact, well-known rectangle fall back to a bounding box (`QWeather`
    /// over mainland China — two rectangles that follow the Himalayan frontier — and HKO over Hong
    /// Kong, close enough to exclude Shenzhen); every other national service answers `false` for a
    /// coordinate — guessing a jurisdiction from a bounding box is how a reader ends up with a
    /// warning for the wrong country — and only the global aggregators cover it.
    #[must_use]
    pub fn covers(self, loc: &super::Location) -> bool {
        if self.is_global() {
            return true;
        }
        let Some(code) = loc.country_code.as_deref() else {
            return match self {
                Self::QWeather => covers_mainland_china(loc),
                Self::Hko => in_box(loc, 113.85, 114.45, 22.15, 22.50),
                _ => false,
            };
        };
        let code = code.trim().to_ascii_uppercase();
        match self {
            Self::Nws => matches!(code.as_str(), "US" | "PR" | "VI" | "GU" | "MP"),
            Self::Hko => code == "HK",
            Self::QWeather => code == "CN",
            Self::MeteoAlarm => METEOALARM_COUNTRIES.contains(&code.as_str()),
            // Global sources returned above.
            Self::WmoSwic | Self::Fpas | Self::VisualCrossing => true,
        }
    }
}

/// The ISO 3166-1 alpha-2 codes of the EUMETNET members `MeteoAlarm` serves.
///
/// `MeteoAlarm`'s coverage is country-grained because the EDR collection exposes no point query:
/// a point is tested against the returned feature geometry client-side. The list is the set of
/// participating countries (including the EEA and the non-EU members that publish through it);
/// a country that is run by an agency outside EUMETNET is deliberately absent.
const METEOALARM_COUNTRIES: [&str; 38] = [
    "AT", "BE", "BG", "CH", "CY", "CZ", "DE", "DK", "EE", "ES", "FI", "FR", "GB", "GR", "HR", "HU",
    "IE", "IL", "IS", "IT", "LT", "LU", "LV", "MD", "ME", "MK", "MT", "NL", "NO", "PL", "PT", "RO",
    "RS", "SE", "SI", "SK", "TR", "UA",
];

/// Whether `loc` lies inside a lon/lat rectangle; the coordinate fallback of [`AlertSource::covers`].
fn in_box(loc: &super::Location, min_lon: f64, max_lon: f64, min_lat: f64, max_lat: f64) -> bool {
    (min_lon..=max_lon).contains(&loc.lon) && (min_lat..=max_lat).contains(&loc.lat)
}

/// Whether a coordinate-only location lies over mainland China, for the `QWeather` fallback.
///
/// Two rectangles rather than one bounding box: a single box would sweep in the northern Indian
/// plains (Delhi is at 77.21 °E, 28.61 °N), so the western band stops at the Himalayan frontier
/// (32 °N) while the eastern band starts where that frontier has fallen south — at 80 °E —
/// keeping Lhasa, Kunming, Guangzhou and the coast inside and Delhi and Amritsar outside.
fn covers_mainland_china(loc: &super::Location) -> bool {
    in_box(loc, 73.0, 80.0, 32.0, 54.0) || in_box(loc, 80.0, 135.0, 18.0, 54.0)
}

impl fmt::Display for AlertSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for AlertSource {
    type Err = Error;

    /// Case-insensitive and tolerant of `-`/`_`, like the provider ids: `meteo-alarm`, `MeteoAlarm`
    /// and `meteoalarm` all name the same source.
    fn from_str(input: &str) -> Result<Self, Self::Err> {
        let normalized: String = input
            .chars()
            .filter(|c| *c != '-' && *c != '_' && !c.is_whitespace())
            .flat_map(char::to_lowercase)
            .collect();
        Self::ALL
            .into_iter()
            .find(|source| source.as_str() == normalized)
            .ok_or_else(|| {
                Error::Usage(format!(
                    "unknown alert source `{input}`; known sources: {}",
                    Self::ALL.map(Self::as_str).join(", ")
                ))
            })
    }
}

/// One severe-weather warning, normalised to the CAP v1.2 shape.
///
/// A field a source does not carry stays [`None`] or empty rather than being invented; the
/// renderers print what is there and omit what is not.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Alert {
    /// The source's own identifier, used for the first dedup pass.
    pub id: String,
    /// Which source reported this alert.
    pub source: AlertSource,
    /// The event name, e.g. `Tornado Warning` or `gale`.
    pub event: String,
    /// CAP severity.
    pub severity: Severity,
    /// CAP urgency.
    pub urgency: Urgency,
    /// CAP certainty.
    pub certainty: Certainty,
    /// When the event starts, at its own offset.
    pub onset: Option<DateTime<FixedOffset>>,
    /// The message's validity end (CAP `expires`).
    pub expires: Option<DateTime<FixedOffset>>,
    /// When the event itself ends (CAP `ends`); preferred over `expires` for liveness.
    pub ends: Option<DateTime<FixedOffset>>,
    /// The affected areas, de-duplicated across `info` blocks.
    pub areas: Vec<String>,
    /// A one-line summary; falls back to [`Alert::event`] when the source carries none.
    pub headline: String,
    /// The full description, when the source carries one.
    pub description: Option<String>,
    /// What the reader is told to do, when the source carries instructions.
    pub instruction: Option<String>,
    /// The issuing agency, when reported.
    pub sender: Option<String>,
    /// The attribution lines the source's terms require displayed with its data, verbatim.
    ///
    /// `QWeather` publishes them as `metadata.attributions` (the v7 spelling was `refer.sources`)
    /// and its attribution terms demand they be shown in full, unmodified, wherever its warning or
    /// air-quality data is shown; [`crate::alerts::credits`] prints them beside the registry's own
    /// credit line. Empty for sources whose terms ask for nothing beyond that line.
    #[serde(default)]
    pub credit: Vec<String>,
}

impl Alert {
    /// The instant this alert stops being live: `ends` when present, else `expires`.
    ///
    /// `None` means "no reported end" — the alert is shown rather than hidden on a guess.
    #[must_use]
    pub fn effective_end(&self) -> Option<DateTime<FixedOffset>> {
        self.ends.or(self.expires)
    }

    /// Whether this alert is still live at `now`.
    #[must_use]
    pub fn is_live_at(&self, now: DateTime<FixedOffset>) -> bool {
        self.effective_end().is_none_or(|end| end > now)
    }
}

#[cfg(test)]
mod tests {
    use super::{AlertSource, Certainty, Severity, Urgency};

    #[test]
    fn severity_is_ordered_from_unknown_to_extreme() {
        assert!(Severity::Unknown < Severity::Minor);
        assert!(Severity::Minor < Severity::Moderate);
        assert!(Severity::Moderate < Severity::Severe);
        assert!(Severity::Severe < Severity::Extreme);
    }

    #[test]
    fn severity_parses_case_insensitively_and_rejects_the_rest() {
        assert_eq!("Severe".parse::<Severity>().unwrap(), Severity::Severe);
        assert_eq!(" minor ".parse::<Severity>().unwrap(), Severity::Minor);
        let error = "catastrophic".parse::<Severity>().unwrap_err();
        assert!(error.to_string().contains("unknown severity"), "{error}");
        assert!(error.to_string().contains("extreme"), "{error}");
    }

    #[test]
    fn cap_spellings_map_and_unknown_values_are_unknown() {
        assert_eq!(Severity::from_cap("Extreme"), Severity::Extreme);
        assert_eq!(Severity::from_cap(""), Severity::Unknown);
        assert_eq!(Urgency::from_cap("Immediate"), Urgency::Immediate);
        assert_eq!(Certainty::from_cap("Observed"), Certainty::Observed);
        assert_eq!(Certainty::from_cap("wat"), Certainty::Unknown);
    }

    #[test]
    fn alert_sources_parse_tolerantly_and_keep_registry_order() {
        for source in AlertSource::ALL {
            assert_eq!(source.as_str().parse::<AlertSource>().unwrap(), source);
        }
        assert_eq!(
            "Meteo-Alarm".parse::<AlertSource>().unwrap(),
            AlertSource::MeteoAlarm
        );
        assert_eq!(
            "wmo_swic".parse::<AlertSource>().unwrap(),
            AlertSource::WmoSwic
        );
        let error = "acme".parse::<AlertSource>().unwrap_err();
        assert!(error.to_string().contains("known sources"), "{error}");
    }

    #[test]
    fn coordinate_only_coverage_boxes_exclude_the_neighbours() {
        use chrono_tz::Tz;

        use super::super::{Location, LocationSource};

        let at = |lat: f64, lon: f64| Location {
            name: String::new(),
            admin1: None,
            country: String::new(),
            country_code: None,
            lat,
            lon,
            tz: Tz::Asia__Shanghai,
            elevation_m: None,
            population: None,
            source: LocationSource::Coordinates,
            station: None,
            named_by: None,
        };

        // Delhi and Amritsar are south of the Himalayan frontier: `qweather` must not claim them.
        for (lat, lon) in [(28.61, 77.21), (31.63, 74.87)] {
            assert!(!AlertSource::QWeather.covers(&at(lat, lon)), "{lat},{lon}");
        }
        // Chinese cities, including the far west, are still covered.
        for (lat, lon) in [
            (39.90, 116.40),
            (29.65, 91.12),
            (25.04, 102.71),
            (23.13, 113.26),
            (39.47, 75.99),
            (43.83, 87.62),
        ] {
            assert!(AlertSource::QWeather.covers(&at(lat, lon)), "{lat},{lon}");
        }

        // Shenzhen (22.54 °N) sits just north of Hong Kong: `hko` must not claim it.
        assert!(!AlertSource::Hko.covers(&at(22.54, 114.06)));
        // Hong Kong's own coordinates are still covered.
        for (lat, lon) in [(22.30, 114.17), (22.32, 114.17), (22.20, 114.03)] {
            assert!(AlertSource::Hko.covers(&at(lat, lon)), "{lat},{lon}");
        }
    }
}
