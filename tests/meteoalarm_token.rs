// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The `MeteoAlarm` token conversion, exercised without a socket.
//!
//! `meteoalarm::fetch` is a best-effort alert source: a rejected token becomes an
//! `InvalidToken` (exit 6) only when the source is explicit, and a `401` from the real endpoint
//! needs a socket that the default suite may not open. The conversion itself does not: the stub
//! transport can answer `401` in-process, which is what the probe below drives.
//!
//! `meteoalarm::token` reads the process environment, and a test cannot inject that safely —
//! edition 2024 makes `std::env::set_var` unsafe and the workspace forbids unsafe code. A child
//! process is the honest seam: [`read_the_rejected_token_conversion`] re-runs this binary with the
//! token in the child's environment and [`rejected_token_probe`] performs the stub-transport call
//! there. No socket is opened.

mod common;

use chrono_tz::Tz;

use cirrocast::alerts::meteoalarm;
use cirrocast::cache::CacheMode;
use cirrocast::error::Error;
use cirrocast::http::StubReply;
use cirrocast::model::{Location, LocationSource};

/// The variable `meteoalarm::fetch` reads its bearer token from.
const TOKEN_ENV: &str = "CIRROCAST_METEOALARM_KEY";

/// The probe's location: `MeteoAlarm` filters on the country code, so it must be populated.
fn vienna() -> Location {
    Location {
        name: "Vienna".to_owned(),
        admin1: None,
        country: "Austria".to_owned(),
        country_code: Some("AT".to_owned()),
        lat: 48.20849,
        lon: 16.37208,
        tz: Tz::Europe__Vienna,
        elevation_m: None,
        population: None,
        source: LocationSource::Geocoder,
        station: None,
        named_by: None,
    }
}

/// The conversion, driven through the stub transport in a child process that carries the token.
///
/// Ignored on its own: without `CIRROCAST_METEOALARM_KEY` the source is skipped and there is
/// nothing to assert. [`read_the_rejected_token_conversion`] runs it with the variable set.
#[test]
#[ignore = "probe: read_the_rejected_token_conversion runs it with CIRROCAST_METEOALARM_KEY set"]
fn rejected_token_probe() {
    if std::env::var(TOKEN_ENV).is_err() {
        eprintln!("skipping: run by `read_the_rejected_token_conversion` with {TOKEN_ENV} set");
        return;
    }

    let run = common::ProviderRun::new(
        vec![StubReply::ok(401, "")],
        common::provider_clock(2026, 7, 15),
        CacheMode::Normal,
    );
    let error = meteoalarm::fetch(&vienna(), &run.env(), "en-US")
        .expect_err("a 401 from the stub transport must fail the fetch");
    assert!(
        matches!(
            &error,
            Error::InvalidToken { provider, var, status }
                if provider == "meteoalarm" && var == TOKEN_ENV && *status == 401
        ),
        "expected InvalidToken(meteoalarm, {TOKEN_ENV}, 401), got {error:?}"
    );
    assert_eq!(
        error.exit_code(),
        6,
        "InvalidToken maps to the missing-key code"
    );
    assert_eq!(run.calls().len(), 1, "only the country index was requested");
}

/// Runs [`rejected_token_probe`] in a child of this test binary, with the token in its environment.
///
/// The child is captured rather than inherited: `1 passed` proves the probe actually ran, so a
/// filter typo cannot make this pass vacuously.
#[test]
fn read_the_rejected_token_conversion() {
    let output = std::process::Command::new(std::env::current_exe().expect("the test binary path"))
        .args([
            "--ignored",
            "--exact",
            "rejected_token_probe",
            "--nocapture",
        ])
        .env(TOKEN_ENV, "not-a-real-token")
        .env("CIRROCAST_FORBID_NETWORK", "1")
        .output()
        .expect("the probe child process starts");
    assert!(
        output.status.success(),
        "the child probe failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("1 passed"),
        "the probe did not run: {stdout}"
    );
}
