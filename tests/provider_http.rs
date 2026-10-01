// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The shared provider HTTP helper: `fetch_json` and the policy it centralises.
//!
//! Every case is scripted through `StubTransport`, so no test here opens a socket: what is asserted
//! is the error taxonomy (a `401` is the user's to fix, a `403`/`429`/`5xx` is the chain's to fall
//! through), the cache behaviour, and the rule that a credential never reaches an error message or
//! a cache envelope.

mod common;

use std::time::Duration;

use chrono::NaiveDate;

use cirrocast::cache::{CacheKey, CacheMode};
use cirrocast::error::Error;
use cirrocast::http::{HttpRequest, StubReply, TransportError};
use cirrocast::provider::{JsonFetch, ProviderId, fetch_json};

use common::{ProviderRun, provider_clock};

/// The credential the redaction tests look for; long enough to be unmistakable.
const SECRET: &str = "sk-live-0123456789abcdef";

/// The cache key one fetch in these tests uses.
fn key(part: &str) -> CacheKey {
    let date = NaiveDate::from_ymd_opt(2026, 9, 30).expect("a valid date");
    CacheKey::weather_part("openweathermap", part, 39.9042, 116.4074, 5, date)
}

/// The location the requests in these tests are for; what the offline miss message spells out.
fn place() -> cirrocast::model::Location {
    common::fixture_location("beijing")
}

/// The request one fetch in these tests uses: the key travels in a query parameter, as it does for
/// `OpenWeatherMap`, `WeatherAPI` and `WWO`.
fn request() -> HttpRequest {
    HttpRequest::get("https://api.example.invalid/data/2.5/weather")
        .query("lat", "39.9042")
        .query("appid", SECRET)
        .secret(SECRET)
}

/// One fetch through the helper, returning what it decoded (or failed with).
fn run(
    replies: Vec<StubReply>,
    mode: CacheMode,
    part: &str,
) -> (ProviderRun, Result<serde_json::Value, Error>) {
    let run = ProviderRun::new(replies, provider_clock(2026, 9, 30), mode);
    let result = fetch_json(
        &run.env(),
        &place(),
        &JsonFetch {
            provider: ProviderId::OpenWeatherMap,
            request: request(),
            key: key(part),
            ttl: Duration::from_secs(600),
            what: "current",
        },
    );
    (run, result)
}

#[test]
fn a_401_is_an_invalid_key_error_that_names_the_fix() {
    let (_run, result) = run(
        vec![StubReply::status(
            401,
            Vec::new(),
            "{\"cod\":401,\"message\":\"Invalid API key\"}",
        )],
        CacheMode::Normal,
        "current",
    );
    let error = result.expect_err("a rejected key is an error");
    assert_eq!(error.exit_code(), 6);
    let text = error.to_string();
    assert!(text.contains("openweathermap"), "{text}");
    assert!(text.contains("cirrocast key set openweathermap"), "{text}");
    assert!(!text.contains(SECRET), "the key leaked: {text}");
}

#[test]
fn a_403_stays_an_upstream_error_so_a_chain_can_fall_through() {
    let (_run, result) = run(
        vec![StubReply::status(
            403,
            Vec::new(),
            "{\"error\":{\"code\":2007,\"message\":\"API key has exceeded calls per month quota.\"}}",
        )],
        CacheMode::Normal,
        "current",
    );
    let error = result.expect_err("a quota refusal is an error");
    assert_eq!(error.exit_code(), 3);
    let text = error.to_string();
    assert!(text.contains("exceeded calls per month quota"), "{text}");
    assert!(!text.contains(SECRET), "the key leaked: {text}");
}

#[test]
fn a_429_and_a_500_keep_the_upstream_taxonomy() {
    for status in [429, 500] {
        let (_run, result) = run(
            vec![StubReply::ok(status, "{\"message\":\"slow down\"}")],
            CacheMode::Normal,
            "current",
        );
        let error = result.expect_err("a transient refusal is an error");
        assert_eq!(error.exit_code(), 3, "wrong code for HTTP {status}");
        assert!(error.to_string().contains("slow down"), "{error}");
    }
}

#[test]
fn a_malformed_body_names_the_provider_not_the_cache() {
    let (run, result) = run(
        vec![StubReply::ok(200, "<html>not json</html>")],
        CacheMode::Normal,
        "current",
    );
    let error = result.expect_err("an unparsable body is an error");
    assert_eq!(error.exit_code(), 3);
    let text = error.to_string();
    assert!(text.contains("openweathermap"), "{text}");
    assert!(!text.contains("provider cache"), "{text}");
    assert_eq!(run.calls().len(), 1);
}

#[test]
fn a_second_fetch_is_served_from_the_cache() {
    let (run, first) = run(
        vec![StubReply::ok(200, "{\"temp\":21}")],
        CacheMode::Normal,
        "current",
    );
    assert_eq!(first.expect("the fixture parses")["temp"], 21);

    let again: serde_json::Value = fetch_json(
        &run.env(),
        &place(),
        &JsonFetch {
            provider: ProviderId::OpenWeatherMap,
            request: request(),
            key: key("current"),
            ttl: Duration::from_secs(600),
            what: "current",
        },
    )
    .expect("the cached body parses");
    assert_eq!(again["temp"], 21);
    assert_eq!(run.calls().len(), 1, "the cache must not refetch");
}

#[test]
fn offline_never_touches_the_network_and_names_the_missing_entry() {
    let (run, result) = run(vec![], CacheMode::Offline, "current");
    let error = result.expect_err("a miss in offline mode is an error");
    assert_eq!(error.exit_code(), 3);
    let text = error.to_string();
    assert!(
        text.contains(
            "offline mode: no cached openweathermap answer for Beijing (39.90, 116.41) at"
        ),
        "{text}"
    );
    assert!(text.contains("openweathermap-current-"), "{text}");
    assert!(text.contains("rerun without `--offline`"), "{text}");
    assert_eq!(
        run.calls(),
        Vec::<HttpRequest>::new(),
        "offline mode must not fetch"
    );
}

#[test]
fn a_transport_failure_reports_the_redacted_url() {
    let (_run, result) = run(
        vec![StubReply::err(TransportError::Timeout)],
        CacheMode::Normal,
        "current",
    );
    let error = result.expect_err("a timeout is an error");
    assert_eq!(error.exit_code(), 3);
    let text = error.to_string();
    assert!(text.contains("appid=***"), "{text}");
    assert!(!text.contains(SECRET), "the key leaked: {text}");
}

#[test]
fn the_cache_envelope_never_stores_the_key() {
    let (run, result) = run(
        vec![StubReply::ok(200, "{\"temp\":21}")],
        CacheMode::Normal,
        "current",
    );
    result.expect("the fixture parses");

    let path = run.cache().entry_path(&key("current"));
    let entry = std::fs::read_to_string(&path).expect("the entry is written");
    assert!(!entry.contains(SECRET), "the key leaked into {path:?}");

    // A hashed key derived from the request text stores the redacted spelling, never the secret.
    let hashed = CacheKey::hash("weather", &request().redacted_normalized());
    let run = ProviderRun::new(
        vec![StubReply::ok(200, "{}")],
        provider_clock(2026, 9, 30),
        CacheMode::Normal,
    );
    fetch_json::<serde_json::Value>(
        &run.env(),
        &place(),
        &JsonFetch {
            provider: ProviderId::OpenWeatherMap,
            request: request(),
            key: hashed.clone(),
            ttl: Duration::from_secs(600),
            what: "current",
        },
    )
    .expect("the fixture parses");
    let entry = std::fs::read_to_string(run.cache().entry_path(&hashed))
        .expect("the hashed entry is written");
    assert!(
        !entry.contains(SECRET),
        "the key leaked into the hashed envelope"
    );
    assert!(
        entry.contains("***"),
        "the envelope keeps the redacted text: {entry}"
    );
}
