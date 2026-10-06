// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Unit systems, conversions and display formatting.
//!
//! This is the **only** module in the crate that converts units. Every other module stores and
//! passes canonical metric/SI values (`temp_c`, `wind_kmh`, `pressure_hpa`, `distance_km`), and
//! the render layer asks this module for a string. A [`ResolvedUnits`] value is the single object
//! a renderer holds, so a second conversion point cannot appear by accident and cache entries stay
//! unit independent.
//!
//! [`UnitOverrides`] comes from the `[units]` configuration table, so the accepted spellings
//! (`c`, `kmh`, `inhg`, …) have exactly one definition: the constants in [`crate::config`].

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::config::UnitOverrides;
use crate::error::{Error, Result};

/// The unit system selected by `--units` or `defaults.units`.
///
/// The three systems differ in exactly five units, and the per-quantity `[units]` overrides can
/// replace any of them individually; [`UnitSystem::resolve`] folds both into a [`ResolvedUnits`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UnitSystem {
    /// Metric: °C, km/h, hPa, km, mm.
    Metric,
    /// United States customary: °F, mph, inHg, mi, in.
    Us,
    /// United Kingdom hybrid: °C, mph, hPa, mi, mm.
    Uk,
}

impl UnitSystem {
    /// Applies the `[units]` overrides to this system's defaults.
    ///
    /// An absent override keeps the system default. Values are already checked by
    /// [`crate::config::Config::validate`], but parsing happens here so that a caller which built
    /// the override set by hand still cannot smuggle an unusable unit into a renderer.
    pub fn resolve(self, overrides: &UnitOverrides) -> Result<ResolvedUnits> {
        let defaults = self.defaults();
        Ok(ResolvedUnits {
            temp: parse_override(overrides.temp.as_deref())?.unwrap_or(defaults.temp),
            wind: parse_override(overrides.wind.as_deref())?.unwrap_or(defaults.wind),
            pressure: parse_override(overrides.pressure.as_deref())?.unwrap_or(defaults.pressure),
            distance: parse_override(overrides.distance.as_deref())?.unwrap_or(defaults.distance),
            precip: parse_override(overrides.precip.as_deref())?.unwrap_or(defaults.precip),
        })
    }

    /// The five units this system selects when nothing is overridden.
    const fn defaults(self) -> ResolvedUnits {
        match self {
            Self::Metric => ResolvedUnits {
                temp: TempUnit::Celsius,
                wind: WindUnit::Kmh,
                pressure: PressureUnit::Hpa,
                distance: DistanceUnit::Km,
                precip: PrecipUnit::Mm,
            },
            Self::Us => ResolvedUnits {
                temp: TempUnit::Fahrenheit,
                wind: WindUnit::Mph,
                pressure: PressureUnit::Inhg,
                distance: DistanceUnit::Mi,
                precip: PrecipUnit::In,
            },
            Self::Uk => ResolvedUnits {
                temp: TempUnit::Celsius,
                wind: WindUnit::Mph,
                pressure: PressureUnit::Hpa,
                distance: DistanceUnit::Mi,
                precip: PrecipUnit::Mm,
            },
        }
    }
}

impl FromStr for UnitSystem {
    type Err = Error;

    fn from_str(input: &str) -> Result<Self> {
        match input {
            "metric" => Ok(Self::Metric),
            "us" => Ok(Self::Us),
            "uk" => Ok(Self::Uk),
            other => Err(Error::Usage(format!(
                "unknown unit system `{other}`; expected metric, us or uk"
            ))),
        }
    }
}

impl fmt::Display for UnitSystem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Metric => "metric",
            Self::Us => "us",
            Self::Uk => "uk",
        })
    }
}

/// The five units a renderer formats with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedUnits {
    /// Temperature.
    pub temp: TempUnit,
    /// Wind speed.
    pub wind: WindUnit,
    /// Pressure.
    pub pressure: PressureUnit,
    /// Distance and visibility.
    pub distance: DistanceUnit,
    /// Precipitation.
    pub precip: PrecipUnit,
}

/// Temperature unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TempUnit {
    /// Degrees Celsius (`c`).
    #[serde(rename = "c")]
    Celsius,
    /// Degrees Fahrenheit (`f`).
    #[serde(rename = "f")]
    Fahrenheit,
}

impl TempUnit {
    /// The display suffix.
    #[must_use]
    pub const fn symbol(self) -> &'static str {
        match self {
            Self::Celsius => "°C",
            Self::Fahrenheit => "°F",
        }
    }
}

impl FromStr for TempUnit {
    type Err = Error;

    fn from_str(input: &str) -> Result<Self> {
        match input {
            "c" => Ok(Self::Celsius),
            "f" => Ok(Self::Fahrenheit),
            other => Err(Error::Usage(format!(
                "unknown temperature unit `{other}`; expected c or f"
            ))),
        }
    }
}

/// Wind speed unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WindUnit {
    /// Kilometres per hour (`kmh`).
    #[serde(rename = "kmh")]
    Kmh,
    /// Miles per hour (`mph`).
    #[serde(rename = "mph")]
    Mph,
    /// Metres per second (`mps`).
    #[serde(rename = "mps")]
    Mps,
    /// Knots (`knots`).
    #[serde(rename = "knots")]
    Knots,
}

impl WindUnit {
    /// The display suffix.
    #[must_use]
    pub const fn symbol(self) -> &'static str {
        match self {
            Self::Kmh => "km/h",
            Self::Mph => "mph",
            Self::Mps => "m/s",
            Self::Knots => "kn",
        }
    }
}

impl FromStr for WindUnit {
    type Err = Error;

    fn from_str(input: &str) -> Result<Self> {
        match input {
            "kmh" => Ok(Self::Kmh),
            "mph" => Ok(Self::Mph),
            "mps" => Ok(Self::Mps),
            "knots" => Ok(Self::Knots),
            other => Err(Error::Usage(format!(
                "unknown wind unit `{other}`; expected kmh, mph, mps or knots"
            ))),
        }
    }
}

/// Pressure unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PressureUnit {
    /// Hectopascals (`hpa`).
    #[serde(rename = "hpa")]
    Hpa,
    /// Inches of mercury (`inhg`).
    #[serde(rename = "inhg")]
    Inhg,
    /// Millimetres of mercury (`mmhg`).
    #[serde(rename = "mmhg")]
    Mmhg,
}

impl PressureUnit {
    /// The display suffix.
    #[must_use]
    pub const fn symbol(self) -> &'static str {
        match self {
            Self::Hpa => "hPa",
            Self::Inhg => "inHg",
            Self::Mmhg => "mmHg",
        }
    }
}

impl FromStr for PressureUnit {
    type Err = Error;

    fn from_str(input: &str) -> Result<Self> {
        match input {
            "hpa" => Ok(Self::Hpa),
            "inhg" => Ok(Self::Inhg),
            "mmhg" => Ok(Self::Mmhg),
            other => Err(Error::Usage(format!(
                "unknown pressure unit `{other}`; expected hpa, inhg or mmhg"
            ))),
        }
    }
}

/// Distance unit, used for distances and visibility alike.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DistanceUnit {
    /// Kilometres (`km`).
    #[serde(rename = "km")]
    Km,
    /// Statute miles (`mi`).
    #[serde(rename = "mi")]
    Mi,
}

impl DistanceUnit {
    /// The display suffix.
    #[must_use]
    pub const fn symbol(self) -> &'static str {
        match self {
            Self::Km => "km",
            Self::Mi => "mi",
        }
    }
}

impl FromStr for DistanceUnit {
    type Err = Error;

    fn from_str(input: &str) -> Result<Self> {
        match input {
            "km" => Ok(Self::Km),
            "mi" => Ok(Self::Mi),
            other => Err(Error::Usage(format!(
                "unknown distance unit `{other}`; expected km or mi"
            ))),
        }
    }
}

/// Precipitation unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PrecipUnit {
    /// Millimetres (`mm`).
    #[serde(rename = "mm")]
    Mm,
    /// Inches (`in`).
    #[serde(rename = "in")]
    In,
}

impl PrecipUnit {
    /// The display suffix.
    #[must_use]
    pub const fn symbol(self) -> &'static str {
        match self {
            Self::Mm => "mm",
            Self::In => "in",
        }
    }
}

impl FromStr for PrecipUnit {
    type Err = Error;

    fn from_str(input: &str) -> Result<Self> {
        match input {
            "mm" => Ok(Self::Mm),
            "in" => Ok(Self::In),
            other => Err(Error::Usage(format!(
                "unknown precipitation unit `{other}`; expected mm or in"
            ))),
        }
    }
}

/// Parses one `[units]` override; an absent or empty value stays absent.
///
/// An empty value means "no override" everywhere else (`empty_as_none` in the configuration
/// deserialiser, the documented spellings), so a caller that builds the override set by hand must
/// not get a different answer for the same spelling than a `config.toml` does.
fn parse_override<T>(value: Option<&str>) -> Result<Option<T>>
where
    T: FromStr<Err = Error>,
{
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    value.parse::<T>().map(Some)
}

/// How a value and its unit symbol are joined.
///
/// The art table is a grid: a column is thirteen cells wide, so `0.0mm` counts. Prose-like lines
/// read better with the space (`0.0 mm`). The choice is a parameter rather than two sets of
/// formatters, so a quantity still has exactly one conversion path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnitStyle {
    /// `12 km/h`, `1013 hPa`.
    Spaced,
    /// `12km/h`, `1013hPa` — the art table, where a cell is narrow.
    Compact,
}

impl UnitStyle {
    /// What goes between the value and the symbol.
    #[must_use]
    pub const fn separator(self) -> &'static str {
        match self {
            Self::Spaced => " ",
            Self::Compact => "",
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Conversions
// ---------------------------------------------------------------------------------------------

/// Converts Celsius to Fahrenheit.
///
/// ```
/// # use cirrocast::model::units::c_to_f;
/// assert!((c_to_f(0.0) - 32.0).abs() < 1e-4);
/// assert!((c_to_f(100.0) - 212.0).abs() < 1e-3);
/// assert!((c_to_f(-40.0) + 40.0).abs() < 1e-3);
/// ```
#[must_use]
pub fn c_to_f(celsius: f32) -> f32 {
    celsius * 9.0 / 5.0 + 32.0
}

/// Converts kilometres per hour to miles per hour (1 statute mile = 1.609 344 km).
///
/// ```
/// # use cirrocast::model::units::kmh_to_mph;
/// assert!((kmh_to_mph(100.0) - 62.1371).abs() < 1e-3);
/// assert!((kmh_to_mph(16.093_44) - 10.0).abs() < 1e-4);
/// ```
#[must_use]
pub fn kmh_to_mph(kmh: f32) -> f32 {
    kmh / 1.609_344
}

/// Converts kilometres per hour to knots (1 knot = 1.852 km/h).
///
/// ```
/// # use cirrocast::model::units::kmh_to_knots;
/// assert!((kmh_to_knots(18.52) - 10.0).abs() < 1e-4);
/// ```
#[must_use]
pub fn kmh_to_knots(kmh: f32) -> f32 {
    kmh / 1.852
}

/// Converts kilometres per hour to metres per second.
///
/// ```
/// # use cirrocast::model::units::kmh_to_mps;
/// assert!((kmh_to_mps(36.0) - 10.0).abs() < 1e-4);
/// ```
#[must_use]
pub fn kmh_to_mps(kmh: f32) -> f32 {
    kmh / 3.6
}

/// Converts hectopascals to inches of mercury (1 inHg = 33.863 89 hPa).
///
/// ```
/// # use cirrocast::model::units::hpa_to_inhg;
/// assert!((hpa_to_inhg(1013.25) - 29.9213).abs() < 1e-3);
/// // The reciprocal pins the fourth digit the div-in-psi tolerance admits.
/// assert!((hpa_to_inhg(33.863_89) - 1.0).abs() < 1e-5);
/// ```
#[must_use]
pub fn hpa_to_inhg(hpa: f32) -> f32 {
    hpa / 33.863_89
}

/// Converts hectopascals to millimetres of mercury (1 mmHg = 1.333 224 hPa).
///
/// ```
/// # use cirrocast::model::units::hpa_to_mmhg;
/// assert!((hpa_to_mmhg(1013.25) - 760.0).abs() < 0.01);
/// ```
#[must_use]
pub fn hpa_to_mmhg(hpa: f32) -> f32 {
    hpa / 1.333_224
}

/// Converts kilometres to statute miles.
///
/// ```
/// # use cirrocast::model::units::km_to_mi;
/// assert!((km_to_mi(1.609_344) - 1.0).abs() < 1e-6);
/// assert!((km_to_mi(8.0) - 4.970_97).abs() < 1e-4);
/// ```
#[must_use]
pub fn km_to_mi(km: f32) -> f32 {
    km / 1.609_344
}

/// Converts millimetres to inches.
///
/// ```
/// # use cirrocast::model::units::mm_to_in;
/// assert!((mm_to_in(25.4) - 1.0).abs() < 1e-6);
/// assert!((mm_to_in(0.2) - 0.007_874).abs() < 1e-6);
/// ```
#[must_use]
pub fn mm_to_in(mm: f32) -> f32 {
    mm / 25.4
}

// ---------------------------------------------------------------------------------------------
// Rounding and formatting
// ---------------------------------------------------------------------------------------------

/// Rounds to the nearest integer, ties away from zero.
///
/// This is exactly [`f32::round`]. The scaled ties the formatters care about — `9.95 * 10` for a
/// one-decimal form, say — already land on an exact half in `f32`, so no ulp-scale correction is
/// needed. Callers pass the result through [`normalise_zero`], which maps the `-0.0` that the
/// `(-0.5, 0)` interval produces to `0.0`.
///
/// ```
/// # use cirrocast::model::units::round_half_away_from_zero;
/// assert_eq!(round_half_away_from_zero(-0.4), 0.0);
/// assert_eq!(round_half_away_from_zero(-0.6), -1.0);
/// assert_eq!(round_half_away_from_zero(9.95), 10.0);
/// assert_eq!(round_half_away_from_zero(4_194_303.0), 4_194_303.0);
/// assert_eq!(round_half_away_from_zero(8_388_608.0), 8_388_608.0);
/// ```
#[must_use]
pub fn round_half_away_from_zero(value: f32) -> f32 {
    value.round()
}

/// Formats a value as an integer, mapping a rounded `-0` to `0`.
///
/// ```
/// # use cirrocast::model::units::fmt_int;
/// assert_eq!(fmt_int(-0.4), "0");
/// assert_eq!(fmt_int(-0.6), "-1");
/// assert_eq!(fmt_int(1013.25), "1013");
/// ```
#[must_use]
pub fn fmt_int(value: f32) -> String {
    format!("{:.0}", normalise_zero(round_half_away_from_zero(value)))
}

/// Turns `-0.0` into `0.0`, so a sign rounded away never reaches the output.
///
/// Without it `-0.4` would print as `-0` (and `-0.0` in the one-decimal forms). The JSON renderer
/// calls this too: `-0.0` and `0.0` are the same number but two different documents.
#[must_use]
#[allow(clippy::float_cmp)] // comparing against both IEEE zeros exactly is the point
pub fn normalise_zero(value: f32) -> f32 {
    if value == 0.0 { 0.0 } else { value }
}

/// The `f64` spelling of [`normalise_zero`], for the air-quality readings (which the source
/// reports as decimal numbers).
#[must_use]
pub fn normalise_zero_f64(value: f64) -> f64 {
    if value == 0.0 { 0.0 } else { value }
}

/// Rounds to one decimal, ties away from zero, with `-0.0` normalised.
fn round_1dp(value: f32) -> f32 {
    normalise_zero(round_half_away_from_zero(value * 10.0) / 10.0)
}

/// Formats `value` with `decimals` decimal places, ties away from zero, `-0` normalised.
///
/// This is the template engine's numeric precision: `%.1t` and friends must round the value the
/// unit conversion produced, not the string a fixed-precision formatter already rounded, or the
/// precision could never change the output.
///
/// ```
/// # use cirrocast::model::units::fmt_decimals;
/// assert_eq!(fmt_decimals(9.95, 1), "10.0");
/// assert_eq!(fmt_decimals(-0.4, 0), "0");
/// assert_eq!(fmt_decimals(23.456, 2), "23.46");
/// ```
#[must_use]
pub fn fmt_decimals(value: f32, decimals: usize) -> String {
    let decimals = decimals.min(MAX_DECIMALS);
    let scale = 10.0_f32.powi(i32::try_from(decimals).unwrap_or(0));
    let rounded = normalise_zero(round_half_away_from_zero(value * scale) / scale);
    format!("{rounded:.decimals$}")
}

/// The most decimals [`fmt_decimals`] and the `%`-template precision may ask for.
pub const MAX_DECIMALS: usize = 6;

/// One decimal below ten (in magnitude), an integer at or above it.
///
/// The threshold is applied **after** rounding and on the magnitude, so `9.95` prints as `10`,
/// `-9.95` as `-10`, and `9.94`/`-9.94` as `9.9`/`-9.9`: a negative reading takes the same branch
/// as its positive twin.
fn fmt_small(value: f32) -> String {
    let rounded = round_1dp(value);
    if rounded.abs() < 10.0 {
        format!("{rounded:.1}")
    } else {
        fmt_int(value)
    }
}

/// Always one decimal.
fn fmt_1dp(value: f32) -> String {
    format!("{:.1}", round_1dp(value))
}

/// Always two decimals, ties away from zero.
fn fmt_2dp(value: f32) -> String {
    let rounded = normalise_zero(round_half_away_from_zero(value * 100.0) / 100.0);
    format!("{rounded:.2}")
}

/// Formats a temperature as an integer in the requested unit, e.g. `23°C`, `73°F`.
///
/// ```
/// # use cirrocast::model::units::{format_temp, TempUnit};
/// assert_eq!(format_temp(23.0, TempUnit::Celsius), "23°C");
/// assert_eq!(format_temp(23.0, TempUnit::Fahrenheit), "73°F");
/// assert_eq!(format_temp(0.0, TempUnit::Fahrenheit), "32°F");
/// assert_eq!(format_temp(-0.4, TempUnit::Celsius), "0°C");
/// assert_eq!(format_temp(-0.6, TempUnit::Celsius), "-1°C");
/// ```
#[must_use]
pub fn format_temp(celsius: f32, unit: TempUnit) -> String {
    let value = match unit {
        TempUnit::Celsius => celsius,
        TempUnit::Fahrenheit => c_to_f(celsius),
    };
    format!("{}{}", fmt_int(value), unit.symbol())
}

/// Formats a temperature with an explicit sign, e.g. `+23°C`, `-5°C`.
///
/// The sign is decided after rounding, so `-0.4` prints `+0°C` rather than a negative zero. The
/// art table and the `one-line` format use this form; the `plain` output spells temperatures
/// without a sign because it reads as prose.
///
/// ```
/// # use cirrocast::model::units::{format_temp_signed, TempUnit};
/// assert_eq!(format_temp_signed(23.0, TempUnit::Celsius), "+23°C");
/// assert_eq!(format_temp_signed(-5.2, TempUnit::Celsius), "-5°C");
/// assert_eq!(format_temp_signed(-0.4, TempUnit::Celsius), "+0°C");
/// assert_eq!(format_temp_signed(0.0, TempUnit::Fahrenheit), "+32°F");
/// ```
#[must_use]
pub fn format_temp_signed(celsius: f32, unit: TempUnit) -> String {
    format_temp_signed_prec(celsius, unit, 0)
}

/// Formats a temperature with an explicit sign and `decimals` decimal places, e.g. `+23.5°C`.
///
/// [`format_temp_signed`] is the zero-decimal spelling of this; the template engine's `%.1t`
/// precision reaches the value's own decimals through here rather than through string surgery.
///
/// ```
/// # use cirrocast::model::units::{TempUnit, format_temp_signed_prec};
/// assert_eq!(format_temp_signed_prec(23.45, TempUnit::Celsius, 1), "+23.5°C");
/// assert_eq!(format_temp_signed_prec(-0.04, TempUnit::Celsius, 1), "+0.0°C");
/// assert_eq!(format_temp_signed_prec(-5.25, TempUnit::Fahrenheit, 2), "+22.55°F");
/// ```
#[must_use]
pub fn format_temp_signed_prec(celsius: f32, unit: TempUnit, decimals: usize) -> String {
    let value = match unit {
        TempUnit::Celsius => celsius,
        TempUnit::Fahrenheit => c_to_f(celsius),
    };
    let sign = if normalise_zero(round_half_away_from_zero(value)) < 0.0 {
        "-"
    } else {
        "+"
    };
    format!(
        "{sign}{}{}",
        fmt_decimals(value.abs(), decimals),
        unit.symbol()
    )
}

/// Formats a temperature with `decimals` decimal places and no sign, e.g. `23.5°C`, `74.2°F`.
///
/// [`format_temp`] rounds to whole degrees, which is right for a forecast reading and wrong for a
/// climate normal: the averaged mean is worth one decimal, and printing it as an integer would
/// hide the precision the normal actually has.
///
/// ```
/// # use cirrocast::model::units::{TempUnit, format_temp_prec};
/// assert_eq!(format_temp_prec(23.45, TempUnit::Celsius, 1), "23.5°C");
/// assert_eq!(format_temp_prec(23.45, TempUnit::Fahrenheit, 1), "74.2°F");
/// assert_eq!(format_temp_prec(-0.04, TempUnit::Celsius, 1), "0.0°C");
/// ```
#[must_use]
pub fn format_temp_prec(celsius: f32, unit: TempUnit, decimals: usize) -> String {
    let value = match unit {
        TempUnit::Celsius => celsius,
        TempUnit::Fahrenheit => c_to_f(celsius),
    };
    format!("{}{}", fmt_decimals(value, decimals), unit.symbol())
}

/// Formats a temperature *difference* with an explicit sign and `decimals` decimals, e.g. `+1.4°C`,
/// `-0.6°F`.
///
/// A difference is not a temperature: converting one must **not** apply Fahrenheit's `+32` offset,
/// so this is deliberately not [`format_temp_signed_prec`] (which would turn a −2 K anomaly into
/// `+28.4°F`). The sign is decided after rounding, so a zero difference prints `+0.0`.
///
/// ```
/// # use cirrocast::model::units::{TempUnit, format_temp_delta};
/// assert_eq!(format_temp_delta(1.44, TempUnit::Celsius, 1), "+1.4°C");
/// assert_eq!(format_temp_delta(-0.6, TempUnit::Celsius, 1), "-0.6°C");
/// assert_eq!(format_temp_delta(-2.0, TempUnit::Fahrenheit, 1), "-3.6°F");
/// assert_eq!(format_temp_delta(0.0, TempUnit::Celsius, 1), "+0.0°C");
/// ```
#[must_use]
pub fn format_temp_delta(delta_c: f32, unit: TempUnit, decimals: usize) -> String {
    let value = match unit {
        TempUnit::Celsius => delta_c,
        TempUnit::Fahrenheit => delta_c * 9.0 / 5.0,
    };
    let sign = if normalise_zero(round_half_away_from_zero(value)) < 0.0 {
        "-"
    } else {
        "+"
    };
    format!(
        "{sign}{}{}",
        fmt_decimals(value.abs(), decimals),
        unit.symbol()
    )
}

/// Formats a wind speed, e.g. `12 km/h`, `8.3 km/h`, `5.8 mph`.
///
/// The one-decimal form is used below 10 in the target unit (after rounding), so a light breeze
/// stays readable while stronger winds stay short.
///
/// ```
/// # use cirrocast::model::units::{UnitStyle, format_wind, WindUnit};
/// assert_eq!(format_wind(12.0, WindUnit::Kmh, UnitStyle::Spaced), "12 km/h");
/// assert_eq!(format_wind(12.0, WindUnit::Kmh, UnitStyle::Compact), "12km/h");
/// assert_eq!(format_wind(8.3, WindUnit::Kmh, UnitStyle::Spaced), "8.3 km/h");
/// assert_eq!(format_wind(9.3, WindUnit::Mph, UnitStyle::Spaced), "5.8 mph");
/// assert_eq!(format_wind(9.95, WindUnit::Kmh, UnitStyle::Spaced), "10 km/h");
/// ```
#[must_use]
pub fn format_wind(kmh: f32, unit: WindUnit, style: UnitStyle) -> String {
    let value = match unit {
        WindUnit::Kmh => kmh,
        WindUnit::Mph => kmh_to_mph(kmh),
        WindUnit::Mps => kmh_to_mps(kmh),
        WindUnit::Knots => kmh_to_knots(kmh),
    };
    format!("{}{}{}", fmt_small(value), style.separator(), unit.symbol())
}

/// Formats a pressure: integer hPa, two decimals in inHg, integer in mmHg.
///
/// ```
/// # use cirrocast::model::units::{UnitStyle, format_pressure, PressureUnit};
/// assert_eq!(format_pressure(1013.25, PressureUnit::Hpa, UnitStyle::Spaced), "1013 hPa");
/// assert_eq!(format_pressure(1013.25, PressureUnit::Hpa, UnitStyle::Compact), "1013hPa");
/// assert_eq!(format_pressure(1013.25, PressureUnit::Inhg, UnitStyle::Spaced), "29.92 inHg");
/// assert_eq!(format_pressure(1013.25, PressureUnit::Mmhg, UnitStyle::Spaced), "760 mmHg");
/// ```
#[must_use]
pub fn format_pressure(hpa: f32, unit: PressureUnit, style: UnitStyle) -> String {
    let value = match unit {
        PressureUnit::Hpa => fmt_int(hpa),
        PressureUnit::Inhg => fmt_2dp(hpa_to_inhg(hpa)),
        PressureUnit::Mmhg => fmt_int(hpa_to_mmhg(hpa)),
    };
    format!("{value}{}{}", style.separator(), unit.symbol())
}

/// Formats a pressure with `decimals` decimal places in the target unit, e.g. `1013.2hPa`.
///
/// ```
/// # use cirrocast::model::units::{PressureUnit, UnitStyle, format_pressure_prec};
/// assert_eq!(
///     format_pressure_prec(1013.25, PressureUnit::Hpa, UnitStyle::Compact, 1),
///     "1013.3hPa"
/// );
/// assert_eq!(
///     format_pressure_prec(1013.25, PressureUnit::Inhg, UnitStyle::Spaced, 1),
///     "29.9 inHg"
/// );
/// ```
#[must_use]
pub fn format_pressure_prec(
    hpa: f32,
    unit: PressureUnit,
    style: UnitStyle,
    decimals: usize,
) -> String {
    let value = match unit {
        PressureUnit::Hpa => hpa,
        PressureUnit::Inhg => hpa_to_inhg(hpa),
        PressureUnit::Mmhg => hpa_to_mmhg(hpa),
    };
    format!(
        "{}{}{}",
        fmt_decimals(value, decimals),
        style.separator(),
        unit.symbol()
    )
}

/// Formats a distance, e.g. `4.2 km`, `14 km`, `2.6 mi`.
///
/// ```
/// # use cirrocast::model::units::{UnitStyle, format_distance, DistanceUnit};
/// assert_eq!(format_distance(4.2, DistanceUnit::Km, UnitStyle::Spaced), "4.2 km");
/// assert_eq!(format_distance(4.2, DistanceUnit::Km, UnitStyle::Compact), "4.2km");
/// assert_eq!(format_distance(14.0, DistanceUnit::Km, UnitStyle::Spaced), "14 km");
/// assert_eq!(format_distance(4.2, DistanceUnit::Mi, UnitStyle::Spaced), "2.6 mi");
/// ```
#[must_use]
pub fn format_distance(km: f32, unit: DistanceUnit, style: UnitStyle) -> String {
    let value = match unit {
        DistanceUnit::Km => km,
        DistanceUnit::Mi => km_to_mi(km),
    };
    format!("{}{}{}", fmt_small(value), style.separator(), unit.symbol())
}

/// Formats a visibility distance; same rule as [`format_distance`], but a separate entry point
/// because the two are rendered in different rows and may diverge later (capping, `>10 km`, …).
///
/// ```
/// # use cirrocast::model::units::{UnitStyle, format_visibility, DistanceUnit};
/// assert_eq!(format_visibility(8.0, DistanceUnit::Km, UnitStyle::Spaced), "8.0 km");
/// assert_eq!(format_visibility(8.0, DistanceUnit::Mi, UnitStyle::Spaced), "5.0 mi");
/// assert_eq!(format_visibility(20.0, DistanceUnit::Km, UnitStyle::Spaced), "20 km");
/// assert_eq!(format_visibility(10.0, DistanceUnit::Km, UnitStyle::Compact), "10km");
/// ```
#[must_use]
pub fn format_visibility(km: f32, unit: DistanceUnit, style: UnitStyle) -> String {
    format_distance(km, unit, style)
}

/// Formats a visibility distance with `decimals` decimal places in the target unit.
///
/// ```
/// # use cirrocast::model::units::{DistanceUnit, UnitStyle, format_visibility_prec};
/// assert_eq!(
///     format_visibility_prec(8.04, DistanceUnit::Km, UnitStyle::Compact, 2),
///     "8.04km"
/// );
/// ```
#[must_use]
pub fn format_visibility_prec(
    km: f32,
    unit: DistanceUnit,
    style: UnitStyle,
    decimals: usize,
) -> String {
    let value = match unit {
        DistanceUnit::Km => km,
        DistanceUnit::Mi => km_to_mi(km),
    };
    format!(
        "{}{}{}",
        fmt_decimals(value, decimals),
        style.separator(),
        unit.symbol()
    )
}

/// Formats precipitation: always one decimal in millimetres, two in inches.
///
/// ```
/// # use cirrocast::model::units::{UnitStyle, format_precip, PrecipUnit};
/// assert_eq!(format_precip(0.0, PrecipUnit::Mm, UnitStyle::Spaced), "0.0 mm");
/// assert_eq!(format_precip(0.0, PrecipUnit::Mm, UnitStyle::Compact), "0.0mm");
/// assert_eq!(format_precip(0.2, PrecipUnit::Mm, UnitStyle::Spaced), "0.2 mm");
/// assert_eq!(format_precip(0.2, PrecipUnit::In, UnitStyle::Spaced), "0.01 in");
/// ```
#[must_use]
pub fn format_precip(mm: f32, unit: PrecipUnit, style: UnitStyle) -> String {
    let value = match unit {
        PrecipUnit::Mm => fmt_1dp(mm),
        PrecipUnit::In => fmt_2dp(mm_to_in(mm)),
    };
    format!("{value}{}{}", style.separator(), unit.symbol())
}

/// Formats precipitation with `decimals` decimal places in the target unit, e.g. `0.00mm`.
///
/// ```
/// # use cirrocast::model::units::{PrecipUnit, UnitStyle, format_precip_prec};
/// assert_eq!(
///     format_precip_prec(0.2, PrecipUnit::Mm, UnitStyle::Compact, 2),
///     "0.20mm"
/// );
/// ```
#[must_use]
pub fn format_precip_prec(mm: f32, unit: PrecipUnit, style: UnitStyle, decimals: usize) -> String {
    let value = match unit {
        PrecipUnit::Mm => mm,
        PrecipUnit::In => mm_to_in(mm),
    };
    format!(
        "{}{}{}",
        fmt_decimals(value, decimals),
        style.separator(),
        unit.symbol()
    )
}

// ---------------------------------------------------------------------------------------------
// Wind direction
// ---------------------------------------------------------------------------------------------

/// The sixteen compass points, clockwise from north.
const COMPASS: [&str; 16] = [
    "N", "NNE", "NE", "ENE", "E", "ESE", "SE", "SSE", "S", "SSW", "SW", "WSW", "W", "WNW", "NW",
    "NNW",
];

/// The compass point a wind direction falls in.
///
/// Each point owns a 22.5° sector centred on it, so the boundaries sit at the 11.25° half-sector:
/// `11 → N` and `12 → NNE`, `348 → NNW` and `349 → N`. Directions at or above 360° wrap.
///
/// ```
/// # use cirrocast::model::units::compass_16;
/// assert_eq!(compass_16(0), "N");
/// assert_eq!(compass_16(95), "E");
/// assert_eq!(compass_16(180), "S");
/// assert_eq!(compass_16(270), "W");
/// assert_eq!(compass_16(360), "N");
/// ```
#[must_use]
pub fn compass_16(deg: u16) -> &'static str {
    let sector = ((usize::from(deg) % 360) * 4 + 45) / 90 % COMPASS.len();
    COMPASS[sector]
}

/// The centre angle of a compass point, the inverse of [`compass_16`].
///
/// A provider that reports a cardinal direction (`NWS`' `windDirection: "SSW"`) needs the degrees
/// the canonical model stores; the point's own 22.5° sector centre is the natural reading, so
/// `N` is `0`, `SSW` is `203` and `NNE` is `23`. The comparison is case-insensitive; an unknown
/// point (including the empty string NWS uses for a variable wind) is `None`.
///
/// ```
/// # use cirrocast::model::units::compass_degrees;
/// assert_eq!(compass_degrees("N"), Some(0));
/// assert_eq!(compass_degrees("ssw"), Some(203));
/// assert_eq!(compass_degrees(""), None);
/// ```
#[must_use]
pub fn compass_degrees(point: &str) -> Option<u16> {
    let point = point.trim();
    COMPASS
        .iter()
        .position(|candidate| candidate.eq_ignore_ascii_case(point))
        .map(|index| {
            // A sector's centre is `index * 22.5°`, rounded half up to the nearest whole degree;
            // integer maths keeps it exact. The value stays below 360, so the conversion cannot
            // fail.
            u16::try_from((index * 45).div_ceil(2) % 360).unwrap_or_default()
        })
}
