// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The JSON schemas are the contract a consumer writes against, so they are validated two ways:
//!
//! * the frozen `docs/schema/json-v1.json` against `tests/fixtures/json/v1-detroit.json`, the
//!   document v1.0.0 printed (the file itself is never regenerated — it is the compatibility
//!   promise for consumers holding v1 documents);
//! * the current `docs/schema/json-v2.json` against output the shipped binary produces in this
//!   test: one location (a plain object), two locations (an array of reports) and three with one
//!   slot whose cache entry is missing (an array carrying an `error` object).
//!
//! The runs are fixture-backed and `--offline`, so the test never touches the network and the
//! document's shape does not depend on the weather.

mod common;

use std::path::{Path, PathBuf};

use assert_cmd::Command;
use common::Sandbox;
use serde_json::Value;

/// The frozen v1 schema and the document v1.0.0 printed for Detroit.
const V1_SCHEMA: &str = "docs/schema/json-v1.json";
const V1_FIXTURE: &str = "tests/fixtures/json/v1-detroit.json";

/// The current schema.
const V2_SCHEMA: &str = "docs/schema/json-v2.json";

/// The seeded locations: two real ones and a point whose entry is deliberately absent.
const BEIJING: (f64, f64) = (39.9042, 116.4074);
const SHANGHAI: (f64, f64) = (31.2304, 121.4737);
const UNSHED: (f64, f64) = (0.0, 0.0);

/// The fixture every seeded cache entry holds.
const FORECAST: &str = "open_meteo/forecast_beijing_2026-07-15.json";

/// The forecast days the seeded cache entries were built for.
const DAYS: u8 = 3;

/// A path inside the repository.
fn repository_file(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(name)
}

/// A committed JSON document, parsed.
fn json_file(name: &str) -> Value {
    let text = std::fs::read_to_string(repository_file(name))
        .unwrap_or_else(|error| panic!("{name} is readable: {error}"));
    serde_json::from_str(&text).unwrap_or_else(|error| panic!("{name} parses: {error}"))
}

/// Asserts `document` matches `schema`.
///
/// A failure is diagnosed through the schema's own branches (`$defs/report` for an object,
/// `$defs/report`/`$defs/error` per slot for an array): the top-level `oneOf` error quotes the whole
/// instance, which tells a reader nothing, while the branch names the key that mismatched.
fn validate(schema: &Value, document: &Value, context: &str) {
    let validator = jsonschema::validator_for(schema).expect("the committed schema compiles");
    if validator.is_valid(document) {
        return;
    }
    let mut errors = Vec::new();
    match document {
        Value::Object(_) => push_errors(
            &branch_validator(schema, "report"),
            document,
            "",
            &mut errors,
        ),
        Value::Array(slots) => {
            let report = branch_validator(schema, "report");
            let error = branch_validator(schema, "error");
            for (index, slot) in slots.iter().enumerate() {
                let branch = if slot.get("error").is_some() {
                    &error
                } else {
                    &report
                };
                push_errors(branch, slot, &format!("[{index}]"), &mut errors);
            }
        }
        _ => errors.push("the document is neither an object nor an array".to_owned()),
    }
    assert!(
        !errors.is_empty(),
        "{context} does not match the schema, and no branch diagnosed why"
    );
    panic!(
        "{context} does not match the schema:\n{}",
        errors.join("\n")
    );
}

/// One `$defs` branch of a schema as a validator of its own, so a failing document can be
/// diagnosed through the branch a consumer reads (`report`, or `error` for a failed slot).
fn branch_validator(schema: &Value, name: &str) -> jsonschema::Validator {
    let defs = schema.get("$defs").cloned().unwrap_or(Value::Null);
    let wrapper = serde_json::json!({
        "$ref": format!("#/$defs/{name}"),
        "$defs": defs,
    });
    jsonschema::validator_for(&wrapper).expect("the schema branch compiles")
}

/// Pushes every error one schema branch has for `instance`, prefixed with its position.
fn push_errors(
    branch: &jsonschema::Validator,
    instance: &Value,
    prefix: &str,
    errors: &mut Vec<String>,
) {
    for error in branch.iter_errors(instance) {
        let message: String = error.to_string().chars().take(300).collect();
        errors.push(format!("{prefix}{}: {message}", error.instance_path()));
    }
}

/// A sandbox whose weather cache holds [`FORECAST`] for every coordinate in `coordinates`.
///
/// The same seeding the CLI's own tests use: the key is built from the coordinate, the day count
/// and the location-local date, and a coordinate location carries the provisional UTC zone at key
/// time.
fn seeded(coordinates: &[(f64, f64)]) -> Sandbox {
    let sandbox = Sandbox::new();
    let body = std::fs::read_to_string(common::fixture_path(FORECAST))
        .expect("the forecast fixture is readable");
    for (lat, lon) in coordinates {
        common::seed_weather(
            &sandbox,
            "open-meteo",
            *lat,
            *lon,
            DAYS,
            chrono_tz::Tz::UTC,
            &body,
        );
    }
    sandbox
}

/// The `@lat,lon` argument for a coordinate, the spelling the CLI parses back to the same numbers.
fn coordinate((lat, lon): (f64, f64)) -> String {
    format!("@{lat},{lon}")
}

/// Runs `--format json` for `locations` against the sandbox, offline, and returns the parsed
/// stdout with the exit code.
fn run_json(sandbox: &Sandbox, locations: &[&str]) -> (Value, i32) {
    let mut command: Command = sandbox.cirrocast();
    command
        .arg("--format")
        .arg("json")
        .arg("--lang")
        .arg("en-US")
        .arg("--offline");
    for location in locations {
        command.arg(location);
    }
    let output = command.output().expect("the run completes");
    let stdout = String::from_utf8(output.stdout).expect("stdout is UTF-8");
    let document = serde_json::from_str(&stdout)
        .unwrap_or_else(|error| panic!("stdout is one JSON document: {error}\n{stdout}"));
    (document, output.status.code().unwrap_or(-1))
}

#[test]
fn the_committed_v1_fixture_matches_the_frozen_v1_schema() {
    validate(
        &json_file(V1_SCHEMA),
        &json_file(V1_FIXTURE),
        "the v1.0.0 fixture",
    );
}

#[test]
fn one_location_is_an_object_that_matches_the_v2_schema() {
    let sandbox = seeded(&[BEIJING]);
    let (document, code) = run_json(&sandbox, &[&coordinate(BEIJING)]);
    assert_eq!(code, 0, "the seeded run succeeds");
    assert!(document.is_object(), "one location is a plain object");
    assert_eq!(document["schema_version"], 2);
    validate(&json_file(V2_SCHEMA), &document, "a one-location run");
}

#[test]
fn two_locations_are_an_array_that_matches_the_v2_schema() {
    let sandbox = seeded(&[BEIJING, SHANGHAI]);
    let (document, code) = run_json(&sandbox, &[&coordinate(BEIJING), &coordinate(SHANGHAI)]);
    assert_eq!(code, 0, "the seeded runs succeed");
    assert_eq!(document.as_array().map(Vec::len), Some(2));
    validate(&json_file(V2_SCHEMA), &document, "a two-location run");
}

#[test]
fn a_failed_slot_is_an_error_object_the_v2_schema_accepts() {
    let sandbox = seeded(&[BEIJING, SHANGHAI]);
    let (document, code) = run_json(
        &sandbox,
        &[
            &coordinate(BEIJING),
            &coordinate(SHANGHAI),
            &coordinate(UNSHED),
        ],
    );
    assert_ne!(code, 0, "the missing slot maps to an exit code");
    let slots = document
        .as_array()
        .expect("three locations are an array even when one fails");
    assert_eq!(slots.len(), 3, "the failed slot keeps its place");
    assert!(
        slots[2].get("error").is_some(),
        "the last slot is an error document: {}",
        slots[2]
    );
    assert_eq!(
        slots[2]["error"]["code"], code,
        "the slot carries the process exit code"
    );
    validate(
        &json_file(V2_SCHEMA),
        &document,
        "a run with one failed slot",
    );
}
