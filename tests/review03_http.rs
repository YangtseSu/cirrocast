// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! HTTP-level regression test for the redirect policy (§3.10).
//!
//! `UreqTransport` is built with `max_redirects(0)`, so a `3xx` reaches the caller as a plain
//! response instead of being followed (which would have forwarded a custom credential header to the
//! `Location` host). This test pins the caller-side half of that contract — a `3xx` is a typed
//! [`Error::Upstream`], not retried and not decoded — through `StubTransport`. The agent setting
//! itself is a `ureq` configuration and cannot be observed without putting a response on the wire,
//! which the testing policy forbids; see the task report.

use std::sync::Arc;
use std::time::SystemTime;

use cirrocast::cache::FakeClock;
use cirrocast::error::Error;
use cirrocast::http::{HttpClient, HttpRequest, StubReply, StubTransport};

#[test]
fn a_redirect_response_is_a_typed_upstream_error() {
    let transport = Arc::new(StubTransport::new(vec![StubReply::status(
        302,
        vec![(
            "Location".to_owned(),
            "https://elsewhere.example/v1/current".to_owned(),
        )],
        "moved",
    )]));
    let client = HttpClient::new(
        Box::new(Arc::clone(&transport)),
        0,
        Arc::new(FakeClock::new(SystemTime::UNIX_EPOCH)),
        0,
    );

    let error = client
        .send(&HttpRequest::get("https://account.example/v1/current"))
        .expect_err("a 3xx must not be followed or decoded by the client");
    if let Error::Upstream { status, .. } = &error {
        assert_eq!(*status, Some(302));
    } else {
        panic!("expected an upstream error, got {error:?}");
    }
    assert_eq!(transport.calls().len(), 1, "a 3xx is not retried");
}
