// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The location argument parse table, driven by `tests/fixtures/geo/spec-cases.tsv`.
//!
//! Every accepted spelling and every rejection is a row in that file, so adding a form means
//! adding a row instead of another test function. The fixture format is documented at its top;
//! the ranking, resolution and header-line rules are unit-tested next to the code in
//! `src/geo/mod.rs`.

use cirrocast::error::Error;
use cirrocast::geo::{LocationSpec, USAGE_FORMS};

const TABLE: &str = include_str!("fixtures/geo/spec-cases.tsv");

/// What the second column of a row expects.
#[derive(Debug, PartialEq)]
enum Expectation {
    Default,
    Fuzzy(String),
    Exact(String),
    Osm(String),
    LatLon(f64, f64),
    Alias(String),
    Error(String),
}

/// Parses the expectation column.
fn expectation(field: &str, line_number: usize, line: &str) -> Expectation {
    let (kind, payload) = field.split_once(':').unwrap_or((field, ""));
    match kind {
        "default" => Expectation::Default,
        "fuzzy" => Expectation::Fuzzy(decode(payload)),
        "exact" => Expectation::Exact(decode(payload)),
        "osm" => Expectation::Osm(decode(payload)),
        "alias" => Expectation::Alias(decode(payload)),
        "latlon" => {
            let (lat, lon) = payload
                .split_once(':')
                .unwrap_or_else(|| panic!("line {line_number}: `latlon` needs `lat:lon`: {line}"));
            Expectation::LatLon(
                lat.parse().unwrap_or_else(|error| {
                    panic!("line {line_number}: latitude `{lat}`: {error}")
                }),
                lon.parse().unwrap_or_else(|error| {
                    panic!("line {line_number}: longitude `{lon}`: {error}")
                }),
            )
        }
        "error" => Expectation::Error(decode(payload)),
        other => panic!("line {line_number}: unknown expectation `{other}`: {line}"),
    }
}

/// Undoes the fixture's escaping: `\s` → space, `\t` → tab, `\\` → backslash.
fn decode(field: &str) -> String {
    let mut decoded = String::new();
    let mut chars = field.chars();
    while let Some(character) = chars.next() {
        if character != '\\' {
            decoded.push(character);
            continue;
        }
        match chars.next() {
            Some('s') => decoded.push(' '),
            Some('t') => decoded.push('\t'),
            Some('\\') => decoded.push('\\'),
            other => panic!("unknown escape `\\{}`", other.unwrap_or('\0')),
        }
    }
    decoded
}

#[test]
fn the_parse_table_holds_and_every_rejection_points_at_the_accepted_forms() {
    let mut cases = 0_usize;
    let mut errors = 0_usize;
    for (index, line) in TABLE.lines().enumerate() {
        let line_number = index + 1;
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut columns = line.split('\t');
        let input = columns
            .next()
            .unwrap_or_else(|| panic!("line {line_number}: no input column"));
        let expected = columns
            .next()
            .unwrap_or_else(|| panic!("line {line_number}: no expectation column"));
        assert!(
            columns.next().is_none(),
            "line {line_number}: more than two columns"
        );

        let argument = (input != "<none>").then(|| decode(input));
        let expected = expectation(expected, line_number, line);
        let result = LocationSpec::parse_arg(argument.as_deref());
        cases += 1;

        match expected {
            Expectation::Default => assert_eq!(
                result.unwrap_or_else(|error| panic!("line {line_number}: {error}")),
                LocationSpec::Default,
                "line {line_number}"
            ),
            Expectation::Fuzzy(query) => assert_eq!(
                result.unwrap_or_else(|error| panic!("line {line_number}: {error}")),
                LocationSpec::Fuzzy(query),
                "line {line_number}"
            ),
            Expectation::Exact(query) => assert_eq!(
                result.unwrap_or_else(|error| panic!("line {line_number}: {error}")),
                LocationSpec::Exact(query),
                "line {line_number}"
            ),
            Expectation::Osm(query) => assert_eq!(
                result.unwrap_or_else(|error| panic!("line {line_number}: {error}")),
                LocationSpec::Osm(query),
                "line {line_number}"
            ),
            Expectation::Alias(name) => assert_eq!(
                result.unwrap_or_else(|error| panic!("line {line_number}: {error}")),
                LocationSpec::Alias(name),
                "line {line_number}"
            ),
            Expectation::LatLon(lat, lon) => assert_eq!(
                result.unwrap_or_else(|error| panic!("line {line_number}: {error}")),
                LocationSpec::LatLon(lat, lon),
                "line {line_number}"
            ),
            Expectation::Error(fragment) => {
                errors += 1;
                let error = result.expect_err(&format!(
                    "line {line_number}: expected a usage error for `{line}`"
                ));
                assert_eq!(error.exit_code(), 2, "line {line_number}: {error}");
                assert!(
                    matches!(error, Error::Usage(_)),
                    "line {line_number}: expected Error::Usage, got {error:?}"
                );
                let message = error.to_string();
                assert!(
                    message.contains(&fragment),
                    "line {line_number}: `{message}` does not contain `{fragment}`"
                );
                assert!(
                    message.contains(USAGE_FORMS),
                    "line {line_number}: `{message}` does not list the accepted forms"
                );
            }
        }
    }
    assert!(cases >= 40, "the parse table shrank to {cases} cases");
    assert!(
        errors >= 8,
        "the parse table covers only {errors} rejections"
    );
}

#[test]
fn from_str_and_parse_arg_agree() {
    let spec: LocationSpec = ":Beijing".parse().expect("an exact name parses");
    assert_eq!(spec, LocationSpec::Exact("Beijing".to_owned()));
    assert_eq!(
        LocationSpec::parse_arg(Some(":Beijing")).expect("the same input parses"),
        spec
    );
    assert_eq!(
        LocationSpec::parse_arg(None).expect("no argument"),
        LocationSpec::Default
    );
}
