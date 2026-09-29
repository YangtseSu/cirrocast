// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Unit resolution, conversion and formatting.
//!
//! The bulk of the coverage is `tests/fixtures/model/units-cases.tsv`: one row per conversion and
//! rounding edge, checked against the public formatters, so the expectations can be reviewed
//! without reading Rust. The remaining tests cover what a row cannot express — system defaults,
//! overrides, rejected values and the compass sectors.

mod common;

use cirrocast::config::UnitOverrides;
use cirrocast::error::Error;
use cirrocast::model::units::{
    DistanceUnit, PrecipUnit, PressureUnit, ResolvedUnits, TempUnit, UnitSystem, WindUnit,
    compass_16, fmt_int, format_distance, format_precip, format_pressure, format_temp,
    format_visibility, format_wind, round_half_away_from_zero,
};

/// The units a fixture row's `unit spec` column selects.
fn units_for(quantity: &str, spec: &str) -> ResolvedUnits {
    let system = match spec {
        "metric" => Some(UnitSystem::Metric),
        "us" => Some(UnitSystem::Us),
        "uk" => Some(UnitSystem::Uk),
        _ => None,
    };
    if let Some(system) = system {
        return system_defaults(system);
    }

    let mut overrides = UnitOverrides::default();
    match quantity {
        "temp" => overrides.temp = Some(spec.to_owned()),
        "wind" => overrides.wind = Some(spec.to_owned()),
        "pressure" => overrides.pressure = Some(spec.to_owned()),
        "distance" | "visibility" => overrides.distance = Some(spec.to_owned()),
        "precip" => overrides.precip = Some(spec.to_owned()),
        other => panic!("unknown quantity `{other}` in the fixture"),
    }
    UnitSystem::Metric
        .resolve(&overrides)
        .expect("the fixture only uses valid unit tokens")
}

/// The five units a system selects with no overrides.
fn system_defaults(system: UnitSystem) -> ResolvedUnits {
    system
        .resolve(&UnitOverrides::default())
        .expect("the built-in defaults resolve")
}

/// Formats `value` the way the fixture row's quantity demands.
fn render(quantity: &str, value: f32, units: ResolvedUnits) -> String {
    match quantity {
        "temp" => format_temp(value, units.temp),
        "wind" => format_wind(value, units.wind),
        "pressure" => format_pressure(value, units.pressure),
        "distance" => format_distance(value, units.distance),
        "visibility" => format_visibility(value, units.distance),
        "precip" => format_precip(value, units.precip),
        other => panic!("unknown quantity `{other}` in the fixture"),
    }
}

#[test]
fn fixture_rows_reproduce() {
    let text = common::fixture("model/units-cases.tsv");
    let mut cases = 0;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let columns: Vec<&str> = line.split('\t').collect();
        assert_eq!(columns.len(), 4, "malformed fixture row: {line}");
        let (quantity, raw_value, spec, expected) =
            (columns[0], columns[1], columns[2], columns[3]);
        let value: f32 = raw_value
            .parse()
            .unwrap_or_else(|error| panic!("{raw_value}: {error}"));
        let units = units_for(quantity, spec);
        assert_eq!(render(quantity, value, units), expected, "{line}");
        cases += 1;
    }
    assert!(cases >= 80, "the fixture lost rows: {cases}");
}

#[test]
fn systems_resolve_their_documented_defaults() {
    assert_eq!(
        system_defaults(UnitSystem::Metric),
        ResolvedUnits {
            temp: TempUnit::Celsius,
            wind: WindUnit::Kmh,
            pressure: PressureUnit::Hpa,
            distance: DistanceUnit::Km,
            precip: PrecipUnit::Mm,
        }
    );
    assert_eq!(
        system_defaults(UnitSystem::Us),
        ResolvedUnits {
            temp: TempUnit::Fahrenheit,
            wind: WindUnit::Mph,
            pressure: PressureUnit::Inhg,
            distance: DistanceUnit::Mi,
            precip: PrecipUnit::In,
        }
    );
    assert_eq!(
        system_defaults(UnitSystem::Uk),
        ResolvedUnits {
            temp: TempUnit::Celsius,
            wind: WindUnit::Mph,
            pressure: PressureUnit::Hpa,
            distance: DistanceUnit::Mi,
            precip: PrecipUnit::Mm,
        }
    );
}

#[test]
fn overrides_replace_every_system_default() {
    let overrides = UnitOverrides {
        temp: Some("f".to_owned()),
        wind: Some("knots".to_owned()),
        pressure: Some("mmhg".to_owned()),
        distance: Some("mi".to_owned()),
        precip: Some("in".to_owned()),
    };
    let units = UnitSystem::Metric
        .resolve(&overrides)
        .expect("all five overrides are valid");

    assert_eq!(units.temp, TempUnit::Fahrenheit);
    assert_eq!(units.wind, WindUnit::Knots);
    assert_eq!(units.pressure, PressureUnit::Mmhg);
    assert_eq!(units.distance, DistanceUnit::Mi);
    assert_eq!(units.precip, PrecipUnit::In);

    // The overrides must reach the output, not just the struct.
    assert_eq!(format_temp(23.0, units.temp), "73°F");
    assert_eq!(format_wind(18.52, units.wind), "10 kn");
    assert_eq!(format_pressure(1013.25, units.pressure), "760 mmHg");
    assert_eq!(format_distance(4.2, units.distance), "2.6 mi");
    assert_eq!(format_precip(0.2, units.precip), "0.01 in");
}

#[test]
fn invalid_overrides_are_rejected() {
    let cases = [
        UnitOverrides {
            temp: Some("kelvin".to_owned()),
            ..UnitOverrides::default()
        },
        UnitOverrides {
            wind: Some("beaufort".to_owned()),
            ..UnitOverrides::default()
        },
        UnitOverrides {
            pressure: Some("bar".to_owned()),
            ..UnitOverrides::default()
        },
        UnitOverrides {
            distance: Some("nautical-mile".to_owned()),
            ..UnitOverrides::default()
        },
        UnitOverrides {
            precip: Some("cm".to_owned()),
            ..UnitOverrides::default()
        },
    ];

    for overrides in cases {
        let error = UnitSystem::Metric
            .resolve(&overrides)
            .expect_err("an unknown unit must not resolve");
        assert!(matches!(error, Error::Usage(_)), "{error}");
        assert_eq!(error.exit_code(), 2);
    }
}

#[test]
fn unit_systems_parse_from_their_config_spelling() {
    for (input, expected) in [
        ("metric", UnitSystem::Metric),
        ("us", UnitSystem::Us),
        ("uk", UnitSystem::Uk),
    ] {
        let parsed = input
            .parse::<UnitSystem>()
            .unwrap_or_else(|error| panic!("{input}: {error}"));
        assert_eq!(parsed, expected);
        // `Display` writes back exactly the spelling `FromStr` accepts.
        let round_trip = parsed
            .to_string()
            .parse::<UnitSystem>()
            .unwrap_or_else(|error| panic!("{parsed}: {error}"));
        assert_eq!(round_trip, expected);
    }

    for input in ["", "Metric", "imperial", "si"] {
        let error = input
            .parse::<UnitSystem>()
            .expect_err("only the three documented spellings parse");
        assert_eq!(error.exit_code(), 2, "{input}");
    }
}

#[test]
fn compass_points_use_the_half_sector_boundaries() {
    // Each point owns a 22.5° sector centred on it, so N spans 348.75..360 and 0..11.25. With
    // whole degrees as the input, the observable boundaries sit between 11|12 and 348|349.
    assert_eq!(compass_16(0), "N");
    assert_eq!(compass_16(11), "N");
    assert_eq!(compass_16(12), "NNE");
    assert_eq!(compass_16(33), "NNE");
    assert_eq!(compass_16(34), "NE");
    assert_eq!(compass_16(90), "E");
    assert_eq!(compass_16(180), "S");
    assert_eq!(compass_16(270), "W");
    assert_eq!(compass_16(348), "NNW");
    assert_eq!(compass_16(349), "N");
    assert_eq!(compass_16(359), "N");
    assert_eq!(compass_16(360), "N");

    // Anything past a full turn wraps, so a sloppy upstream value cannot index out of the table.
    assert_eq!(compass_16(721), "N");
    assert_eq!(compass_16(360 + 95), compass_16(95));
    assert_eq!(compass_16(u16::MAX), compass_16(u16::MAX % 360));
}

#[test]
fn integer_rounding_never_prints_a_negative_zero() {
    assert_eq!(fmt_int(-0.4), "0");
    assert_eq!(fmt_int(-0.6), "-1");
    assert_eq!(fmt_int(0.0), "0");
    assert_eq!(fmt_int(-0.0), "0");
    assert_eq!(fmt_int(23.4), "23");
    assert_eq!(fmt_int(23.5), "24");
    assert_eq!(fmt_int(-23.5), "-24");

    assert_rounded(round_half_away_from_zero(-0.4), 0.0);
    assert_rounded(round_half_away_from_zero(-0.6), -1.0);
    assert_rounded(round_half_away_from_zero(0.0), 0.0);
    assert_rounded(round_half_away_from_zero(-0.0), 0.0);
    assert_rounded(round_half_away_from_zero(2.5), 3.0);
    assert_rounded(round_half_away_from_zero(-2.5), -3.0);
    assert_rounded(round_half_away_from_zero(9.95), 10.0);
    assert_rounded(round_half_away_from_zero(-9.95), -10.0);
}

/// Exact float comparison: these cases are about values that must land on one specific number
/// (and, for the zeros, about the sign), so a tolerance would hide the bug under test.
#[allow(clippy::float_cmp)]
fn assert_rounded(actual: f32, expected: f32) {
    assert_eq!(actual, expected);
}
