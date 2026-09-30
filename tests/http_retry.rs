// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The retry policy, the backoff schedule and the error mapping.
//!
//! Every case is scripted through `StubTransport` and timed through `FakeClock`, so no test opens
//! a socket and none of them sleeps: the waits the client asks for are asserted, not waited out.

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use cirrocast::error::Error;
use cirrocast::http::{HttpClient, HttpRequest, StubReply, StubTransport, TransportError, UA};

/// A request the tests reuse.
fn request() -> HttpRequest {
    HttpRequest::get("https://example.invalid/v1/search").query("name", "Beijing")
}

/// A client over a scripted transport, with a clock that records waits, plus the transport handle.
fn harness(
    replies: Vec<StubReply>,
) -> (
    HttpClient,
    Arc<StubTransport>,
    Arc<cirrocast::cache::FakeClock>,
) {
    let transport = Arc::new(StubTransport::new(replies));
    let clock = Arc::new(cirrocast::cache::FakeClock::new(SystemTime::UNIX_EPOCH));
    let client = HttpClient::new(Box::new(Arc::clone(&transport)), 3, clock.clone(), 0);
    (client, transport, clock)
}

/// The recorded 400 body of the Open-Meteo error envelope.
fn error_envelope() -> String {
    std::fs::read_to_string("tests/fixtures/http/open_meteo_error_invalid_param.json")
        .expect("the recorded error fixture is readable")
}

#[test]
fn transient_failures_retry_with_exponential_backoff() {
    let (client, transport, clock) = harness(vec![
        StubReply::err(TransportError::Timeout),
        StubReply::err(TransportError::Timeout),
        StubReply::ok(200, "{}"),
    ]);
    let response = client.send(&request()).expect("the third attempt succeeds");
    assert_eq!(response.status(), 200);
    assert_eq!(transport.calls().len(), 3);
    assert_eq!(
        clock.sleeps(),
        vec![Duration::from_millis(500), Duration::from_secs(1)]
    );
}

#[test]
fn exhausted_retries_name_the_request_and_the_attempt_count() {
    let (client, transport, clock) = harness(vec![
        StubReply::err(TransportError::Timeout),
        StubReply::err(TransportError::Reset),
        StubReply::err(TransportError::Dns("no such host".to_owned())),
    ]);
    let error = client.send(&request()).expect_err("all attempts fail");
    assert_eq!(error.exit_code(), 3);
    let message = error.to_string();
    assert!(
        message.contains(
            "network error: GET https://example.invalid/v1/search?name=Beijing failed after 3 attempts: DNS failure: no such host"
        ),
        "{message}"
    );
    assert_eq!(transport.calls().len(), 3);
    assert_eq!(clock.sleeps().len(), 2);
}

#[test]
fn server_errors_retry_and_client_errors_do_not() {
    let (client, transport, clock) = harness(vec![
        StubReply::ok(500, "upstream exploded"),
        StubReply::ok(200, "{}"),
    ]);
    let response = client
        .send(&request())
        .expect("the retried request succeeds");
    assert_eq!(response.status(), 200);
    assert_eq!(transport.calls().len(), 2);
    assert_eq!(clock.sleeps(), vec![Duration::from_millis(500)]);

    let (client, transport, clock) = harness(vec![StubReply::ok(400, error_envelope())]);
    let error = client.send(&request()).expect_err("a 400 is final");
    assert_eq!(transport.calls().len(), 1);
    assert!(clock.sleeps().is_empty());
    match error {
        Error::Upstream {
            provider,
            status,
            message,
        } => {
            assert_eq!(provider, "example.invalid");
            assert_eq!(status, Some(400));
            assert_eq!(
                message,
                "Latitude must be in range of -90 to 90°. Given: 91.0."
            );
        }
        other => panic!("expected an upstream error, got {other:?}"),
    }
}

#[test]
fn retry_after_replaces_the_backoff_and_is_clamped() {
    let (client, _, clock) = harness(vec![
        StubReply::status(
            429,
            vec![("Retry-After".to_owned(), "7".to_owned())],
            "slow down",
        ),
        StubReply::ok(200, "{}"),
    ]);
    client.send(&request()).expect("the retry succeeds");
    assert_eq!(clock.sleeps(), vec![Duration::from_secs(7)]);

    let (client, _, clock) = harness(vec![
        StubReply::status(
            503,
            vec![("Retry-After".to_owned(), "3600".to_owned())],
            "later",
        ),
        StubReply::ok(200, "{}"),
    ]);
    client.send(&request()).expect("the retry succeeds");
    assert_eq!(clock.sleeps(), vec![Duration::from_secs(60)]);
}

#[test]
fn a_retryable_status_that_survives_every_attempt_is_an_upstream_error() {
    let (client, transport, clock) = harness(vec![
        StubReply::ok(500, "one"),
        StubReply::ok(500, "two"),
        StubReply::ok(502, "three"),
    ]);
    let error = client.send(&request()).expect_err("three 5xx are final");
    assert_eq!(transport.calls().len(), 3);
    assert_eq!(clock.sleeps().len(), 2);
    match error {
        Error::Upstream {
            status, message, ..
        } => {
            assert_eq!(status, Some(502));
            assert_eq!(message, "three");
        }
        other => panic!("expected an upstream error, got {other:?}"),
    }
}

#[test]
fn permanent_transport_failures_do_not_retry() {
    let (client, transport, clock) = harness(vec![StubReply::err(TransportError::Tls(
        "certificate has expired".to_owned(),
    ))]);
    let error = client.send(&request()).expect_err("a TLS failure is final");
    assert_eq!(
        error.to_string(),
        "network error: GET https://example.invalid/v1/search?name=Beijing failed: TLS failure: certificate has expired"
    );
    assert_eq!(transport.calls().len(), 1);
    assert!(clock.sleeps().is_empty());
}

#[test]
fn requests_reach_the_transport_verbatim() {
    let (client, transport, _) = harness(vec![StubReply::ok(200, "{}")]);
    let request = request().query("count", "10").header("user-agent", UA);
    client.send(&request).expect("the request is answered");
    let sent = transport.calls();
    assert_eq!(sent.len(), 1);
    assert_eq!(
        sent[0].full_url(),
        "https://example.invalid/v1/search?name=Beijing&count=10"
    );
    assert_eq!(sent[0].query_pairs().len(), 2);
    assert_eq!(
        sent[0].headers(),
        [("user-agent".to_owned(), UA.to_owned())]
    );
}

#[test]
fn the_user_agent_identifies_the_tool_and_a_contact() {
    assert!(UA.starts_with("cirrocast/"), "{UA}");
    assert!(
        UA.ends_with("(+https://github.com/YangtseSu/cirrocast)"),
        "{UA}"
    );
}
