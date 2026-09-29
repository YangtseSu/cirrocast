// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The canonical condition table and the daylight saving policy of `resolve_local`.
//!
//! Both are driven by fixtures where a fixture can express the expectation
//! (`tests/fixtures/model/wmo4677-*.tsv|txt`); the DST cases are real 2026 transitions in
//! `Europe/Berlin` and `Antarctica/Troll`.

mod common;

use std::collections::BTreeSet;
use std::str::FromStr;

use chrono::{NaiveDate, NaiveDateTime, Offset as _, TimeZone as _, Utc};
use chrono_tz::Tz;

use cirrocast::error::Error;
use cirrocast::model::{Condition, resolve_local};

/// The codes the known-code fixture describes.
fn described_codes() -> BTreeSet<u8> {
    let text = common::fixture("model/wmo4677-known.tsv");
    let mut codes = BTreeSet::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let raw = line.split('\t').next().unwrap_or_default();
        let code: u8 = raw
            .parse()
            .unwrap_or_else(|error| panic!("{line}: {error}"));
        codes.insert(code);
    }
    codes
}

/// A local wall clock reading.
fn at(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> NaiveDateTime {
    NaiveDate::from_ymd_opt(year, month, day)
        .and_then(|date| date.and_hms_opt(hour, minute, 0))
        .unwrap_or_else(|| panic!("{year}-{month}-{day} {hour}:{minute} is a valid local time"))
}

/// An IANA zone from the bundled database.
fn zone(name: &str) -> Tz {
    Tz::from_str(name).unwrap_or_else(|error| panic!("{name}: {error}"))
}

#[test]
fn described_codes_match_the_fixture() {
    let text = common::fixture("model/wmo4677-known.tsv");
    let mut rows = 0;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let columns: Vec<&str> = line.split('\t').collect();
        assert_eq!(columns.len(), 8, "malformed fixture row: {line}");

        let code: u8 = columns[0]
            .parse()
            .unwrap_or_else(|error| panic!("{line}: {error}"));
        let condition = Condition::from_u8(code);
        assert!(condition.is_known(), "{line}");
        assert_eq!(condition.code(), code, "{line}");
        assert_eq!(condition.i18n_key(), columns[1], "{line}");
        assert_eq!(condition.description_en(), columns[2], "{line}");
        assert_eq!(condition.is_precipitation(), columns[3] == "true", "{line}");
        assert_eq!(condition.is_fog(), columns[4] == "true", "{line}");
        assert_eq!(condition.is_thunder(), columns[5] == "true", "{line}");
        assert_eq!(condition.art_key(), columns[6], "{line}");
        let rank: u8 = columns[7]
            .parse()
            .unwrap_or_else(|error| panic!("{line}: {error}"));
        assert_eq!(condition.severity_rank(), rank, "{line}");
        rows += 1;
    }
    assert_eq!(rows, 29, "every table row must have a fixture row");
}

#[test]
fn every_byte_is_total_and_only_described_codes_are_known() {
    let known = described_codes();
    for code in 0..=u8::MAX {
        let condition = Condition::from_u8(code);
        assert_eq!(condition.code(), code, "from_u8 must round-trip {code}");
        assert_eq!(condition.is_known(), known.contains(&code), "{code}");

        if !known.contains(&code) {
            assert_eq!(condition.i18n_key(), "cond.unknown", "{code}");
            assert_eq!(condition.description_en(), "Unknown", "{code}");
            assert_eq!(condition.art_key(), "unknown", "{code}");
            assert_eq!(condition.severity_rank(), 0, "{code}");
            assert!(!condition.is_precipitation(), "{code}");
            assert!(!condition.is_fog(), "{code}");
            assert!(!condition.is_thunder(), "{code}");
            assert!(!condition.is_clear(), "{code}");
        }
    }
}

#[test]
fn the_unknown_list_stays_unknown() {
    let text = common::fixture("model/wmo4677-unknown.txt");
    let mut rows = 0;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let code: u8 = line
            .parse()
            .unwrap_or_else(|error| panic!("{line}: {error}"));
        let condition = Condition::from_u8(code);
        assert!(!condition.is_known(), "{code} must stay unknown");
        assert_eq!(condition.code(), code);
        assert_eq!(condition.description_en(), "Unknown");
        assert_eq!(condition.art_key(), "unknown");
        assert_eq!(condition.severity_rank(), 0);
        rows += 1;
    }
    assert!(rows >= 10, "the unknown-code list lost rows: {rows}");
}

#[test]
fn only_the_clear_codes_are_clear() {
    assert!(Condition::from_u8(0).is_clear());
    assert!(Condition::from_u8(1).is_clear());
    for code in [2_u8, 3, 45, 48, 61, 80, 95] {
        assert!(!Condition::from_u8(code).is_clear(), "{code}");
    }
}

#[test]
fn resolve_local_uses_an_unambiguous_time_as_is() {
    let tz = zone("Europe/Berlin");
    let naive = at(2026, 6, 15, 12, 0);
    let resolved = resolve_local(tz, naive).expect("midday in June happens exactly once");
    assert_eq!(resolved.naive_local(), naive);
    assert_eq!(resolved.offset().fix().local_minus_utc(), 2 * 3600);
    assert_eq!(
        resolved.with_timezone(&Utc),
        Utc.with_ymd_and_hms(2026, 6, 15, 10, 0, 0).unwrap()
    );
}

#[test]
fn resolve_local_moves_out_of_the_spring_forward_gap() {
    // Europe/Berlin jumps 02:00 → 03:00 on 2026-03-29, so 02:30 never happens locally.
    let tz = zone("Europe/Berlin");
    let naive = at(2026, 3, 29, 2, 30);
    let resolved = resolve_local(tz, naive).expect("the gap retries one hour later");
    assert_eq!(resolved.naive_local(), at(2026, 3, 29, 3, 30));
    assert_eq!(resolved.offset().fix().local_minus_utc(), 2 * 3600);
    assert_eq!(
        resolved.with_timezone(&Utc),
        Utc.with_ymd_and_hms(2026, 3, 29, 1, 30, 0).unwrap()
    );
}

#[test]
fn resolve_local_prefers_the_earliest_offset_when_ambiguous() {
    // Europe/Berlin falls back 03:00 → 02:00 on 2026-10-25, so 02:30 happens twice; the first
    // occurrence is still on summer time (UTC+2).
    let tz = zone("Europe/Berlin");
    let naive = at(2026, 10, 25, 2, 30);
    let resolved = resolve_local(tz, naive).expect("the repeated hour resolves to the first pass");
    assert_eq!(resolved.naive_local(), naive);
    assert_eq!(resolved.offset().fix().local_minus_utc(), 2 * 3600);
    assert_eq!(
        resolved.with_timezone(&Utc),
        Utc.with_ymd_and_hms(2026, 10, 25, 0, 30, 0).unwrap()
    );
}

#[test]
fn resolve_local_fails_when_the_gap_swallows_the_retry() {
    // Antarctica/Troll jumps two hours (01:00 → 03:00 local) on the last Sunday of March, so even
    // `naive + 1h` is still inside the gap and the wall clock reading cannot be placed at all.
    let tz = zone("Antarctica/Troll");
    let naive = at(2026, 3, 29, 1, 30);
    let error = resolve_local(tz, naive).expect_err("the two hour gap swallows the retry");
    assert!(matches!(error, Error::Upstream { .. }), "{error}");
    assert_eq!(error.exit_code(), 3);
    let message = error.to_string();
    assert!(
        message.contains("does not exist in Antarctica/Troll"),
        "{message}"
    );
}
