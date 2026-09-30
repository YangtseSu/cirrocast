// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Prints the same canonical values through the three unit systems, side by side.
//!
//! The developer's eyeball for `src/model/units.rs` before any renderer exists:
//!
//! ```text
//! cargo run --example units_demo
//! ```
//!
//! It is an example on purpose: nothing here ships in the release binary.

use cirrocast::config::UnitOverrides;
use cirrocast::error::Result;
use cirrocast::model::units::{
    ResolvedUnits, UnitStyle, UnitSystem, format_distance, format_precip, format_pressure,
    format_temp, format_visibility, format_wind,
};

/// Which formatter a demo row exercises.
#[derive(Clone, Copy)]
enum Quantity {
    /// Temperature.
    Temp,
    /// Wind speed.
    Wind,
    /// Pressure.
    Pressure,
    /// Distance.
    Distance,
    /// Visibility.
    Visibility,
    /// Precipitation.
    Precip,
}

impl Quantity {
    /// Formats `value` (canonical metric/SI) with the matching formatter of `units`.
    fn format(self, value: f32, units: ResolvedUnits) -> String {
        match self {
            Self::Temp => format_temp(value, units.temp),
            Self::Wind => format_wind(value, units.wind, UnitStyle::Spaced),
            Self::Pressure => format_pressure(value, units.pressure, UnitStyle::Spaced),
            Self::Distance => format_distance(value, units.distance, UnitStyle::Spaced),
            Self::Visibility => format_visibility(value, units.distance, UnitStyle::Spaced),
            Self::Precip => format_precip(value, units.precip, UnitStyle::Spaced),
        }
    }
}

/// `(canonical reading, quantity, canonical value)`, including every rounding edge the formatters
/// promise: the `-0.4 °C → 0°C` and `0 °C → 32°F` cases and the `< 10` one-decimal rule.
const CASES: [(&str, Quantity, f32); 13] = [
    ("temp 23.0 °C", Quantity::Temp, 23.0),
    ("temp 0.0 °C", Quantity::Temp, 0.0),
    ("temp -0.4 °C", Quantity::Temp, -0.4),
    ("temp -0.6 °C", Quantity::Temp, -0.6),
    ("wind 5.4 km/h", Quantity::Wind, 5.4),
    ("wind 9.95 km/h", Quantity::Wind, 9.95),
    ("wind 12.0 km/h", Quantity::Wind, 12.0),
    ("pressure 1013.25 hPa", Quantity::Pressure, 1013.25),
    ("precip 0.2 mm", Quantity::Precip, 0.2),
    ("precip 0.0 mm", Quantity::Precip, 0.0),
    ("distance 4.2 km", Quantity::Distance, 4.2),
    ("visibility 8.0 km", Quantity::Visibility, 8.0),
    ("visibility 20.0 km", Quantity::Visibility, 20.0),
];

fn main() -> Result<()> {
    let overrides = UnitOverrides::default();
    let metric = UnitSystem::Metric.resolve(&overrides)?;
    let us = UnitSystem::Us.resolve(&overrides)?;
    let uk = UnitSystem::Uk.resolve(&overrides)?;

    println!(
        "{:<20} {:<11} {:<11} {:<11}",
        "quantity", "metric", "us", "uk"
    );
    for (label, quantity, value) in CASES {
        println!(
            "{label:<20} {:<11} {:<11} {:<11}",
            quantity.format(value, metric),
            quantity.format(value, us),
            quantity.format(value, uk),
        );
    }
    Ok(())
}
