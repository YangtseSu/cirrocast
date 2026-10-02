// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The `json` format: one snapshot, the doc-checked key set, and the unit-independence promise.
//!
//! The snapshot is the hand-reviewed document. The key set is *not* a second copy in this file:
//! it is read out of the machine-checked index in `docs/schema.md`, so a rename, a removal, a
//! retyping or an undocumented addition fails this suite instead of only drifting the document
//! away from the code. Both exist for the same reason the module doc states the stability
//! promise: consumers are scripts, and a silently renamed key breaks them.

mod common;

use std::collections::BTreeSet;

use serde_json::Value;

use cirrocast::config::UnitOverrides;
use cirrocast::i18n::{I18n, LanguageRequest};
use cirrocast::model::Report;
use cirrocast::model::units::UnitSystem;
use cirrocast::render::json::{Json, SCHEMA_VERSION};
use cirrocast::render::{ColorMode, RenderContext, Renderer, TermCaps};

/// The English catalog, loaded the way the CLI loads an unconfigured run.
fn english() -> I18n {
    I18n::load(&LanguageRequest::Auto, |_| None)
}

/// One row of the machine-checked key index in `docs/schema.md`.
struct DocumentedKey {
    /// Key path, `[]` marking an array element.
    path: String,
    /// The JSON type the document promises (`integer`, `number`, `string`, `boolean`, `object`,
    /// `array`).
    json_type: String,
    /// Whether the value may be `null`; every key is always present.
    nullable: bool,
}

/// Reads the key index out of `docs/schema.md`.
///
/// The document is the single source of the key list: a change to the renderer that is not made
/// in the document fails the suite below, and a change to the document that is not made in the
/// renderer fails it just the same. Parsing is deliberately strict — a row that does not have the
/// documented shape panics with its line number.
fn documented_keys() -> Vec<DocumentedKey> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/schema.md");
    let text = std::fs::read_to_string(&path).expect("docs/schema.md is readable");
    let body = text
        .split_once("<!-- schema-key-index:begin -->")
        .and_then(|(_, rest)| rest.split_once("<!-- schema-key-index:end -->"))
        .map(|(body, _)| body)
        .expect("docs/schema.md carries the machine-checked key index");

    let mut keys = Vec::new();
    for (index, line) in body.lines().enumerate() {
        let line = line.trim();
        if !line.starts_with("| `") {
            continue;
        }
        let cells: Vec<&str> = line.trim_matches('|').split('|').map(str::trim).collect();
        let fail = |why: &str| -> ! {
            panic!("docs/schema.md key index line {} {why}: {line}", index + 1)
        };
        if cells.len() != 5 {
            fail("does not have five columns");
        }
        let path = cells[0]
            .strip_prefix('`')
            .and_then(|cell| cell.strip_suffix('`'))
            .unwrap_or_else(|| fail("starts with something that is not a backticked key"));
        let json_type = cells[1];
        let nullable = match cells[3] {
            "yes" => true,
            "no" => false,
            _ => fail("does not say `yes` or `no` in its Null column"),
        };
        keys.push(DocumentedKey {
            path: path.to_owned(),
            json_type: json_type.to_owned(),
            nullable,
        });
    }
    assert!(
        keys.len() > 50,
        "the key index parsed as {} rows; the markers or the table shape changed",
        keys.len()
    );
    keys
}

/// The JSON type name of a value, in the vocabulary of the key index.
fn json_type(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(number) => {
            if number.is_i64() || number.is_u64() {
                "integer"
            } else {
                "number"
            }
        }
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// The value a documented key path points at, the first element of an array for `[]`.
fn value_at<'a>(document: &'a Value, path: &str) -> Option<&'a Value> {
    let mut current = document;
    for segment in path.split('.') {
        current = match segment.strip_suffix("[]") {
            Some(name) => current.get(name)?.as_array()?.first()?,
            None => current.get(segment)?,
        };
    }
    Some(current)
}

/// The fixture every case renders.
fn report() -> Report {
    common::fixture_report("beijing-1d.json")
}

/// Renders `report` in `units`, at the fixture's own observation time.
fn render(report: &Report, units: UnitSystem) -> String {
    render_at(report, units, 80)
}

/// Renders `report` in `units` at `width`, which the `json` format promises to ignore.
fn render_at(report: &Report, units: UnitSystem, width: usize) -> String {
    let i18n = english();
    let ctx = RenderContext {
        units: units
            .resolve(&UnitOverrides::default())
            .expect("the default overrides resolve"),
        color: ColorMode::Never,
        width,
        term: TermCaps::default(),
        now: common::fixture_now(report),
        tz: report.location.tz,
        lang: i18n.lang(),
        i18n: &i18n,
    };
    Json.render(report, &ctx)
        .expect("the fixture renders as JSON")
}

/// Asserts that `key` of the object `value` is present **and** null.
///
/// `Value`'s index operator returns `Null` for a missing key, so a bare `is_null()` passes whether
/// the renderer wrote the documented `null` or dropped the key the schema promises.
fn assert_null_present(value: &Value, key: &str, what: &str) {
    let object = value
        .as_object()
        .unwrap_or_else(|| panic!("{what} is not an object"));
    assert!(
        object.contains_key(key),
        "{what}.{key} is an omitted key, not the documented null"
    );
    assert!(object[key].is_null(), "{what}.{key} is not null");
}

/// The document of a fixture.
fn document(name: &str, units: UnitSystem) -> Value {
    let report = common::fixture_report(name);
    serde_json::from_str(&render(&report, units)).expect("the renderer emits valid JSON")
}

/// Every key path in `value`, with `[]` marking an array element.
fn key_paths(value: &Value, prefix: &str, out: &mut BTreeSet<String>) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                out.insert(path.clone());
                key_paths(child, &path, out);
            }
        }
        Value::Array(items) => {
            for item in items {
                key_paths(item, &format!("{prefix}[]"), out);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

#[test]
fn the_document_is_exactly_this() {
    insta::with_settings!({ prepend_module_to_snapshot => false }, {
        insta::assert_snapshot!("json_beijing_1d", render(&report(), UnitSystem::Metric));
    });
}

#[test]
fn the_schema_version_leads_the_document_and_is_the_documented_one() {
    let text = render(&report(), UnitSystem::Metric);
    assert_eq!(SCHEMA_VERSION, 1);
    assert!(
        text.starts_with("{\n  \"schema_version\": 1,"),
        "a consumer peeks at the first key to check the version: {text}"
    );
}

#[test]
fn the_rendered_keys_are_the_documented_ones_with_the_documented_types() {
    let documented = documented_keys();
    let document = document("beijing-1d.json", UnitSystem::Metric);

    let mut paths = BTreeSet::new();
    key_paths(&document, "", &mut paths);
    let expected: BTreeSet<String> = documented.iter().map(|key| key.path.clone()).collect();

    let missing: Vec<&String> = expected.difference(&paths).collect();
    let extra: Vec<&String> = paths.difference(&expected).collect();
    assert_eq!(missing, Vec::<&String>::new(), "keys disappeared");
    assert_eq!(
        extra,
        Vec::<&String>::new(),
        "undocumented keys appeared (add them to docs/schema.md)"
    );

    for key in &documented {
        let value = value_at(&document, &key.path)
            .unwrap_or_else(|| panic!("`{}` is documented but the fixture has no value", key.path));
        if value.is_null() {
            assert!(
                key.nullable,
                "`{}` is documented as never null but rendered as null",
                key.path
            );
            continue;
        }
        assert_eq!(
            json_type(value),
            key.json_type,
            "`{}` is documented as {} but rendered as {}",
            key.path,
            key.json_type,
            json_type(value)
        );
    }
}

#[test]
fn a_missing_value_is_null_and_never_an_omitted_key() {
    // `current-only.json` has no days, and its `current` has no gust; both keep their keys.
    let document = document("current-only.json", UnitSystem::Metric);
    assert_null_present(&document["current"], "wind_gust_kmh", "current");
    assert_eq!(document["days"], serde_json::json!([]));

    // A coordinate location has no admin1 and no country code, and they are null, not absent.
    let mut report = report();
    report.location.admin1 = None;
    report.location.country_code = None;
    report.location.elevation_m = None;
    let document: Value =
        serde_json::from_str(&render(&report, UnitSystem::Metric)).expect("valid JSON");
    assert_null_present(&document["location"], "admin1", "location");
    assert_null_present(&document["location"], "country_code", "location");
    assert_null_present(&document["location"], "elevation_m", "location");
    assert_eq!(document["location"]["country"], "China");
}

#[test]
fn the_unit_system_does_not_change_a_single_byte() {
    let report = report();
    let metric = render(&report, UnitSystem::Metric);
    for units in [UnitSystem::Us, UnitSystem::Uk] {
        assert_eq!(
            render(&report, units),
            metric,
            "JSON is canonical metric; --units must not leak into it"
        );
    }
}

#[test]
fn the_width_does_not_change_the_output() {
    let report = report();
    let metric = render(&report, UnitSystem::Metric);
    assert_eq!(render_at(&report, UnitSystem::Metric, 20), metric);
    assert_eq!(render_at(&report, UnitSystem::Metric, 200), metric);
}

#[test]
fn a_negative_zero_is_written_as_a_positive_one() {
    let mut report = report();
    let current = report
        .current
        .as_mut()
        .expect("the fixture carries current conditions");
    for value in [
        &mut current.temp_c,
        &mut current.precip_mm,
        &mut current.wind_kmh,
    ] {
        *value = -0.0;
    }
    for day in &mut report.days {
        day.temp_min_c = -0.0;
        for part in &mut day.parts {
            part.temp_c = -0.0;
            part.precip_mm = -0.0;
        }
    }

    let text = render(&report, UnitSystem::Metric);
    assert!(
        !text.contains("-0.0"),
        "a sign rounded to zero reached the document:\n{text}"
    );
    let document: Value = serde_json::from_str(&text).expect("valid JSON");
    assert_eq!(document["current"]["temp_c"], 0.0);
    assert_eq!(document["days"][0]["parts"]["morning"]["precip_mm"], 0.0);
}

#[test]
fn the_days_ascend_from_the_first_day_the_report_carries() {
    let document = document("beijing-3d-day.json", UnitSystem::Metric);
    let dates: Vec<&str> = document["days"]
        .as_array()
        .expect("an array")
        .iter()
        .map(|day| day["date"].as_str().expect("a date string"))
        .collect();
    assert_eq!(dates, ["2026-09-30", "2026-10-01", "2026-10-02"]);
}
