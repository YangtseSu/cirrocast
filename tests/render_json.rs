// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The `json` format: one snapshot, the complete key set, and the unit-independence promise.
//!
//! The snapshot is the hand-reviewed document; the key-set test is what makes a rename or a
//! removal fail loudly, because a reviewer skimming a diff would not notice `min_c` becoming
//! `temp_min_c`. Both exist for the same reason the module doc states the stability promise:
//! consumers are scripts, and a silently renamed key breaks them.

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

/// Every key path the schema documents, in `serde_json`'s own spelling (`[]` = array element).
const EXPECTED_KEYS: [&str; 89] = [
    "schema_version",
    "location",
    "location.name",
    "location.admin1",
    "location.country",
    "location.country_code",
    "location.lat",
    "location.lon",
    "location.timezone",
    "location.elevation_m",
    "location.source",
    "current",
    "current.time",
    "current.condition",
    "current.condition.code",
    "current.condition.text",
    "current.temp_c",
    "current.feels_like_c",
    "current.humidity_pct",
    "current.precip_mm",
    "current.pressure_hpa",
    "current.visibility_km",
    "current.wind_kmh",
    "current.wind_dir_deg",
    "current.wind_gust_kmh",
    "current.cloud_cover_pct",
    "current.uv_index",
    "current.is_day",
    "days",
    "days[].date",
    "days[].parts",
    "days[].sunrise",
    "days[].sunset",
    "days[].min_c",
    "days[].max_c",
    "days[].parts.morning",
    "days[].parts.morning.condition",
    "days[].parts.morning.condition.code",
    "days[].parts.morning.condition.text",
    "days[].parts.morning.temp_c",
    "days[].parts.morning.feels_like_c",
    "days[].parts.morning.precip_mm",
    "days[].parts.morning.precip_prob_pct",
    "days[].parts.morning.humidity_pct",
    "days[].parts.morning.visibility_km",
    "days[].parts.morning.wind_kmh",
    "days[].parts.morning.wind_dir_deg",
    "days[].parts.noon",
    "days[].parts.noon.condition",
    "days[].parts.noon.condition.code",
    "days[].parts.noon.condition.text",
    "days[].parts.noon.temp_c",
    "days[].parts.noon.feels_like_c",
    "days[].parts.noon.precip_mm",
    "days[].parts.noon.precip_prob_pct",
    "days[].parts.noon.humidity_pct",
    "days[].parts.noon.visibility_km",
    "days[].parts.noon.wind_kmh",
    "days[].parts.noon.wind_dir_deg",
    "days[].parts.evening",
    "days[].parts.evening.condition",
    "days[].parts.evening.condition.code",
    "days[].parts.evening.condition.text",
    "days[].parts.evening.temp_c",
    "days[].parts.evening.feels_like_c",
    "days[].parts.evening.precip_mm",
    "days[].parts.evening.precip_prob_pct",
    "days[].parts.evening.humidity_pct",
    "days[].parts.evening.visibility_km",
    "days[].parts.evening.wind_kmh",
    "days[].parts.evening.wind_dir_deg",
    "days[].parts.night",
    "days[].parts.night.condition",
    "days[].parts.night.condition.code",
    "days[].parts.night.condition.text",
    "days[].parts.night.temp_c",
    "days[].parts.night.feels_like_c",
    "days[].parts.night.precip_mm",
    "days[].parts.night.precip_prob_pct",
    "days[].parts.night.humidity_pct",
    "days[].parts.night.visibility_km",
    "days[].parts.night.wind_kmh",
    "days[].parts.night.wind_dir_deg",
    "attribution",
    "attribution.provider",
    "attribution.url",
    "attribution.notice",
    "attribution.location_notice",
    "attribution.retrieved_at",
];

/// The fixture every case renders.
fn report() -> Report {
    common::fixture_report("beijing-1d.json")
}

/// Renders `report` in `units`, at the fixture's own observation time.
fn render(report: &Report, units: UnitSystem) -> String {
    let i18n = english();
    let ctx = RenderContext {
        units: units
            .resolve(&UnitOverrides::default())
            .expect("the default overrides resolve"),
        color: ColorMode::Never,
        width: 80,
        term: TermCaps::default(),
        now: common::fixture_now(report),
        tz: report.location.tz,
        lang: i18n.lang(),
        i18n: &i18n,
    };
    Json.render(report, &ctx)
        .expect("the fixture renders as JSON")
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
fn every_documented_key_is_present() {
    let mut paths = BTreeSet::new();
    key_paths(
        &document("beijing-1d.json", UnitSystem::Metric),
        "",
        &mut paths,
    );
    let expected: BTreeSet<String> = EXPECTED_KEYS.into_iter().map(str::to_owned).collect();

    let missing: Vec<&String> = expected.difference(&paths).collect();
    let extra: Vec<&String> = paths.difference(&expected).collect();
    assert!(missing.is_empty(), "keys disappeared: {missing:?}");
    assert!(
        extra.is_empty(),
        "undocumented keys appeared (add them to the schema doc): {extra:?}"
    );
}

#[test]
fn a_missing_value_is_null_and_never_an_omitted_key() {
    // `current-only.json` has no days, and its `current` has no gust; both keep their keys.
    let document = document("current-only.json", UnitSystem::Metric);
    assert!(document["current"]["wind_gust_kmh"].is_null());
    assert!(document["days"].as_array().expect("an array").is_empty());
    assert!(document["days"].is_array());

    // A coordinate location has no admin1 and no country code, and they are null, not absent.
    let mut report = report();
    report.location.admin1 = None;
    report.location.country_code = None;
    report.location.elevation_m = None;
    let document: Value =
        serde_json::from_str(&render(&report, UnitSystem::Metric)).expect("valid JSON");
    assert!(document["location"]["admin1"].is_null());
    assert!(document["location"]["country_code"].is_null());
    assert!(document["location"]["elevation_m"].is_null());
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
