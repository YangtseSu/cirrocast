// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The canonical `Report` document under hostile or version-shifted input.
//!
//! No production path deserialises a `Report` today — the cache stores raw upstream bodies — but
//! the type carries `Deserialize` and is the contract any future reader of a stored report must
//! rely on, so the invariants a renderer could otherwise contradict are pinned here.

mod common;

use cirrocast::model::Report;
use serde_json::{Value, json};

/// The Beijing fixture as an editable JSON document.
fn report_document() -> Value {
    serde_json::from_str(&common::fixture("report/beijing-1d.json")).expect("the fixture is JSON")
}

#[test]
fn a_shuffled_parts_array_is_rejected() {
    let mut document = report_document();
    document["days"][0]["parts"]
        .as_array_mut()
        .expect("the parts are an array")
        .swap(0, 1);

    let error = serde_json::from_value::<Report>(document)
        .expect_err("parts out of DayPartKind::ALL order must not load");
    assert!(
        error
            .to_string()
            .contains("day parts must be in Morning, Noon, Evening, Night order"),
        "{error}"
    );
}

#[test]
fn a_valid_parts_array_still_loads_in_order() {
    let report: Report = serde_json::from_value(report_document()).expect("the fixture loads");
    for (index, part) in report.days[0].parts.iter().enumerate() {
        assert_eq!(
            part.kind.index(),
            index,
            "parts must stay in DayPartKind::ALL order"
        );
        assert_eq!(
            report.days[0].part(part.kind).kind,
            part.kind,
            "the accessor must agree with the array position"
        );
    }
}

#[test]
fn hostile_field_types_yield_typed_errors_not_panics() {
    let mut document = report_document();
    document["current"]["temp_c"] = json!("18");
    let error = serde_json::from_value::<Report>(document).expect_err("a string is not a number");
    assert!(
        error.to_string().contains("invalid type: string"),
        "{error}"
    );

    let mut document = report_document();
    document["current"]["humidity_pct"] = json!(300);
    let error = serde_json::from_value::<Report>(document).expect_err("300 is not a percent");
    assert!(error.to_string().contains("invalid value"), "{error}");
}

#[test]
fn an_undescribed_condition_code_loads_as_unknown() {
    let mut document = report_document();
    document["current"]["weather"] = json!(250);
    let report: Report = serde_json::from_value(document).expect("a u8 code always loads");
    let current = report.current.expect("the fixture has current conditions");
    assert_eq!(current.weather.code(), 250);
    assert!(!current.weather.is_known(), "250 has no CODES row");
}
