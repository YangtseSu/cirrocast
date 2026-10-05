// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The `--date`, `--history` and `--marine` gates: every refusal happens before any request.
//!
//! These are CLI-level checks: the flags' *requests* are covered by the provider suites against
//! recorded fixtures, while what belongs here is that a usage mistake costs no traffic at all —
//! the sandbox runs every case with `CIRROCAST_FORBID_NETWORK=1`, so a case that reached a socket
//! would fail the test instead of the assertion.

mod common;

/// The exit code and stderr of one rejected invocation.
fn rejected(args: &[&str]) -> (i32, String) {
    let sandbox = common::Sandbox::new();
    let assert = sandbox.cirrocast().args(args).assert().failure();
    let output = assert.get_output();
    (
        output.status.code().expect("the process exited"),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn the_marine_source_is_supplementary_not_a_chain_entry() {
    let (code, stderr) = rejected(&[
        "-p",
        "open-meteo-marine",
        "--lat",
        "54.54",
        "--lon",
        "10.23",
    ]);
    assert_eq!(code, 2, "{stderr}");
    assert!(stderr.contains("--marine"), "{stderr}");
}

#[test]
fn an_archive_flag_needs_a_history_capable_backend() {
    let (code, stderr) = rejected(&[
        "-p",
        "met-no",
        "--date",
        "2026-09-14",
        "--lat",
        "59.91",
        "--lon",
        "10.75",
    ]);
    assert_eq!(code, 2, "{stderr}");
    assert!(stderr.contains("history"), "{stderr}");

    let (code, stderr) = rejected(&[
        "-p",
        "met-no",
        "--history",
        "3d",
        "--lat",
        "59.91",
        "--lon",
        "10.75",
    ]);
    assert_eq!(code, 2, "{stderr}");
    assert!(stderr.contains("history"), "{stderr}");
}

#[test]
fn an_archive_only_backend_refuses_a_forecast_request() {
    // `open-meteo-archive` has no forecast horizon (`max_days: 0`) and a 30 000-day archive, so
    // asking it for days is a usage error naming the two flags that work, not a silent clamp.
    let (code, stderr) = rejected(&[
        "-p",
        "open-meteo-archive",
        "--lat",
        "52.52",
        "--lon",
        "13.41",
    ]);
    assert_eq!(code, 2, "{stderr}");
    assert!(stderr.contains("archive only"), "{stderr}");
    assert!(stderr.contains("--date"), "{stderr}");
}

#[test]
fn a_window_further_back_than_the_backend_serves_is_refused() {
    // The default chain's head (`open-meteo`) carries 92 days; 1900 is far outside it, and the
    // refusal names the bound rather than sending a request that cannot be answered.
    let (code, stderr) = rejected(&["--date", "1900-01-01", "--lat", "52.52", "--lon", "13.41"]);
    assert_eq!(code, 2, "{stderr}");
    assert!(stderr.contains("92 days back"), "{stderr}");
}

#[test]
fn the_flag_values_are_parsed_before_anything_else() {
    for args in [
        vec!["--date", "14-09-2026", "--lat", "52.52", "--lon", "13.41"],
        vec!["--history", "0d", "--lat", "52.52", "--lon", "13.41"],
        vec!["--history", "40000d", "--lat", "52.52", "--lon", "13.41"],
        vec!["--history", "7x", "--lat", "52.52", "--lon", "13.41"],
        vec![
            "--days",
            "3",
            "--date",
            "2026-09-14",
            "--lat",
            "52.52",
            "--lon",
            "13.41",
        ],
        vec![
            "--days",
            "3",
            "--history",
            "7d",
            "--lat",
            "52.52",
            "--lon",
            "13.41",
        ],
        vec![
            "--date",
            "2026-09-14",
            "--history",
            "7d",
            "--lat",
            "52.52",
            "--lon",
            "13.41",
        ],
    ] {
        let (code, stderr) = rejected(&args);
        assert_eq!(code, 2, "{args:?}: {stderr}");
    }
}
