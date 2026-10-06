// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The `status` probe's contract, pinned at the process boundary.
//!
//! Every scenario is offline: the sandbox exports `CIRROCAST_FORBID_NETWORK`, so a run that would
//! reach for the network fails with the documented transport error instead — which is exactly what
//! makes the placeholder path deterministic. A cache entry comes either from
//! [`common::seed_weather`] (fresh, the real clock) or from [`seed_aged`] (a hand-built envelope
//! with a chosen age, the only way to exercise `--max-age` and the stale window).

mod common;

use std::fs;

use chrono::{DateTime, Duration, Utc};
use chrono_tz::Tz;
use predicates::prelude::*;

use cirrocast::cache::{CACHE_SCHEMA_VERSION, CacheKey};
use common::{Sandbox, fixture_path, seed_weather, weather_key_dates};

/// The coordinates the fixture's cache key is built from (`@39.9,116.4`).
const LAT: f64 = 39.9;
const LON: f64 = 116.4;

/// The location argument the tests use: coordinates need no geocoder, so they resolve offline.
const LOCATION: &str = "@39.9,116.4";

/// The forecast days the probe asks for by default (`defaults.days`).
const DAYS: u8 = 3;

/// The recorded Open-Meteo body inside the committed cache fixture.
///
/// The probe's tests do not read the fixture file itself: they seed a throwaway cache with the body
/// it carries, so the entry's age is the test's choice rather than the recording's.
fn fixture_body() -> String {
    let path = fixture_path("cache/weather/open-meteo-39.90-116.40-3-2026-10-05.json");
    let text =
        fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    let envelope: serde_json::Value =
        serde_json::from_str(&text).expect("the fixture is a cache envelope");
    envelope["body"]
        .as_str()
        .expect("the envelope carries the response body")
        .to_owned()
}

/// Seeds one weather entry per candidate local date, `age` seconds old and with a `ttl` of its own.
///
/// The key dates follow the same rule as [`common::seed_weather`]: a coordinate spec keys on the
/// provisional UTC zone, and the child's own clock picks one of the three days.
fn seed_aged(sandbox: &Sandbox, age: Duration, ttl_secs: u64, body: &str) {
    let fetched_at: DateTime<Utc> = Utc::now() - age;
    for date in weather_key_dates(Tz::UTC) {
        let key = CacheKey::weather("open-meteo", LAT, LON, DAYS, date);
        let path = sandbox.cache_dir().join(key.path());
        fs::create_dir_all(path.parent().expect("the entry has a parent"))
            .expect("the cache directory");
        let envelope = serde_json::json!({
            "cache_schema_version": CACHE_SCHEMA_VERSION,
            "key": key.normalised(),
            "fetched_at": fetched_at.to_rfc3339(),
            "ttl_secs": ttl_secs,
            "status": 200,
            "body": body,
        });
        fs::write(
            &path,
            serde_json::to_string_pretty(&envelope).expect("the envelope encodes"),
        )
        .expect("the entry is written");
    }
}

/// The stdout of a run, as text, with the exit code asserted separately.
fn stdout_of(assert: &assert_cmd::assert::Assert) -> String {
    String::from_utf8(assert.get_output().stdout.clone()).expect("UTF-8 stdout")
}

#[test]
fn the_line_is_one_line_even_when_the_template_spans_lines() {
    let sandbox = Sandbox::new();
    seed_weather(
        &sandbox,
        "open-meteo",
        LAT,
        LON,
        DAYS,
        Tz::UTC,
        &fixture_body(),
    );

    // A template newline (real or escaped) and the carriage return of a file with CRLF endings
    // become spaces, and the result is trimmed: a bar sees exactly one line whatever the template
    // held.
    let assert = sandbox
        .cirrocast()
        .args([
            "status",
            "--location",
            LOCATION,
            "--offline",
            "-q",
            "--format",
            "  one\\ntwo\r\nthree  ",
        ])
        .assert()
        .code(0);
    assert_eq!(stdout_of(&assert), "one two three\n");
}

#[test]
fn a_cached_reading_renders_the_template() {
    let sandbox = Sandbox::new();
    seed_weather(
        &sandbox,
        "open-meteo",
        LAT,
        LON,
        DAYS,
        Tz::UTC,
        &fixture_body(),
    );

    // `--template` is the same flag as `--format`, and the body is the recorded answer: the
    // temperature comes from the fixture's `current` block, byte for byte.
    let assert = sandbox
        .cirrocast()
        .args([
            "status",
            "--location",
            LOCATION,
            "--offline",
            "--template",
            "%t",
        ])
        .assert()
        .code(0);
    assert_eq!(stdout_of(&assert), "+18°C\n");

    // A `[templates]` key is addressable the same way `one-line` addresses it.
    sandbox.write_config("[templates]\ncompact = \"%t\"\n");
    let assert = sandbox
        .cirrocast()
        .args([
            "status",
            "--location",
            LOCATION,
            "--offline",
            "--format",
            "@compact",
            "-q",
        ])
        .assert()
        .code(0);
    assert_eq!(stdout_of(&assert), "+18°C\n");
}

#[test]
fn a_transient_failure_becomes_the_placeholder_and_exit_zero() {
    // Nothing cached, offline: the entry is missing, which is a network-class failure.
    let cold = Sandbox::new();
    let assert = cold
        .cirrocast()
        .args(["status", "--location", LOCATION, "--offline", "-q"])
        .assert()
        .code(0);
    assert_eq!(stdout_of(&assert), "n/a\n");

    // An upstream body the decoder refuses: the answer parses as JSON but carries no `daily`
    // block, so the provider raises `Error::Upstream` without a socket.
    let upstream = Sandbox::new();
    seed_weather(
        &upstream,
        "open-meteo",
        LAT,
        LON,
        DAYS,
        Tz::UTC,
        r#"{"latitude":39.9,"longitude":116.4,"timezone":"UTC"}"#,
    );
    upstream
        .cirrocast()
        .args(["status", "--location", LOCATION, "--offline", "-q"])
        .assert()
        .code(0)
        .stdout("n/a\n")
        .stderr(predicate::str::contains("error:"));

    // A missing credential is the same class: a key-requiring backend with no key stored.
    let keyless = Sandbox::new();
    keyless.write_config("[defaults]\nprovider = \"qweather\"\n");
    keyless
        .cirrocast()
        .args(["status", "--location", LOCATION, "--offline", "-q"])
        .assert()
        .code(0)
        .stdout("n/a\n")
        .stderr(predicate::str::contains("cirrocast key set qweather"));

    // The placeholder is a choice: `--placeholder` wins over `[status] placeholder`.
    let placeholder = Sandbox::new();
    placeholder.write_config("[status]\nplaceholder = \"-\"\n");
    placeholder
        .cirrocast()
        .args(["status", "--location", LOCATION, "--offline", "-q"])
        .assert()
        .code(0)
        .stdout("-\n");
    placeholder
        .cirrocast()
        .args([
            "status",
            "--location",
            LOCATION,
            "--offline",
            "-q",
            "--placeholder",
            "offline",
        ])
        .assert()
        .code(0)
        .stdout("offline\n");
}

#[test]
fn max_age_widens_the_window_a_ttl_no_longer_covers() {
    // An entry fetched 700 seconds ago with a 600 second TTL: stale by its own TTL, and the
    // network guard makes the fetch that would follow an error, which is what makes the outcome
    // readable — a served entry prints the temperature, a miss prints the placeholder.
    let sandbox = Sandbox::new();
    seed_aged(&sandbox, Duration::seconds(700), 600, &fixture_body());

    for args in [
        vec!["status", "--location", LOCATION, "-q"],
        vec!["status", "--location", LOCATION, "-q", "--max-age", "0"],
        vec!["status", "--location", LOCATION, "-q", "--max-age", "60"],
    ] {
        let assert = sandbox.cirrocast().args(&args).assert().code(0);
        assert_eq!(stdout_of(&assert), "n/a\n", "{args:?} served a stale entry");
    }

    // 900 seconds of allowed age covers the entry, and it is served without a socket.
    let assert = sandbox
        .cirrocast()
        .args([
            "status",
            "--location",
            LOCATION,
            "-q",
            "--max-age",
            "900",
            "-f",
            "%t",
        ])
        .assert()
        .code(0);
    assert_eq!(stdout_of(&assert), "+18°C\n");
}

#[test]
fn offline_serves_a_stale_entry_and_never_opens_a_socket() {
    let sandbox = Sandbox::new();
    seed_aged(&sandbox, Duration::days(30), 600, &fixture_body());

    // A month past its TTL, `--offline` still answers: the probe never opens a socket and takes
    // what is on disk. The sandbox's network guard would turn any attempt into an error.
    let assert = sandbox
        .cirrocast()
        .args([
            "status",
            "--location",
            LOCATION,
            "--offline",
            "-q",
            "-f",
            "%t",
        ])
        .assert()
        .code(0);
    assert_eq!(stdout_of(&assert), "+18°C\n");

    // `--offline` also silences the name scope: a name that only the network geocoder knows cannot
    // be resolved, which is exit 5 territory — the probe answers with the placeholder, not a crash.
    let assert = sandbox
        .cirrocast()
        .args([
            "status",
            "--location",
            "A Name No Table Has",
            "--offline",
            "-q",
        ])
        .assert()
        .code(0);
    assert_eq!(stdout_of(&assert), "n/a\n");
}

#[test]
fn two_is_usage_for_a_bad_template_or_flag() {
    let sandbox = Sandbox::new();
    sandbox
        .cirrocast()
        .args(["status", "--location", LOCATION, "--format", "%y"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("unknown template token"));
    sandbox
        .cirrocast()
        .args(["status", "--location", LOCATION, "--color", "auto"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("auto"));
    sandbox
        .cirrocast()
        .args([
            "status",
            "--location",
            LOCATION,
            "--format",
            "%t",
            "--template",
            "%t",
        ])
        .assert()
        .code(2);
    // `status --format ''` is a template that renders nothing, which is a mistake rather than an
    // empty line.
    sandbox
        .cirrocast()
        .args(["status", "--location", LOCATION, "--format", ""])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("empty"));
}

#[test]
fn four_is_configuration_and_the_probe_never_guesses_a_location() {
    // No `--location` and no `[location] default`: the probe must not fall back to the public-IP
    // lookup, so this is a configuration problem the user has to fix.
    let unset = Sandbox::new();
    unset.cirrocast().args(["status"]).assert().code(4).stderr(
        predicate::str::contains("[location] default").and(predicate::str::contains("public-IP")),
    );

    // An unreadable configuration file is the same code, whatever the subcommand.
    let unreadable = Sandbox::new();
    unreadable.write_config("schema_version = 2\n");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(unreadable.config_file(), fs::Permissions::from_mode(0o000))
            .expect("the config file is made unreadable");
    }
    unreadable
        .cirrocast()
        .args(["status", "--location", LOCATION])
        .assert()
        .code(4);
}

#[test]
fn a_configured_default_location_is_enough() {
    // The location sources besides `--location` and `CIRROCAST_LOCATION`: the configured default.
    // The run is offline against the seeded entry, so this also proves the configured spec is
    // expanded and resolved exactly as a query's would be.
    let sandbox = Sandbox::new();
    sandbox.write_config("[location]\ndefault = \"@39.9,116.4\"\n");
    seed_weather(
        &sandbox,
        "open-meteo",
        LAT,
        LON,
        DAYS,
        Tz::UTC,
        &fixture_body(),
    );
    let assert = sandbox
        .cirrocast()
        .args(["status", "--offline", "-q", "-f", "%t"])
        .assert()
        .code(0);
    assert_eq!(stdout_of(&assert), "+18°C\n");
}

#[test]
fn the_environment_location_outranks_the_configured_one() {
    // The repository's precedence ladder, applied to the probe's location: the flag, then
    // `CIRROCAST_LOCATION` (the query's positional reads the same variable), then
    // `[location] default`. The configured default points somewhere the cache has nothing for, so
    // which tier answered is visible in the output.
    let sandbox = Sandbox::new();
    sandbox.write_config("[location]\ndefault = \"@1.0,2.0\"\n");
    seed_weather(
        &sandbox,
        "open-meteo",
        LAT,
        LON,
        DAYS,
        Tz::UTC,
        &fixture_body(),
    );

    let assert = sandbox
        .cirrocast()
        .args(["status", "--offline", "-q", "-f", "%t"])
        .assert()
        .code(0);
    assert_eq!(
        stdout_of(&assert),
        "n/a\n",
        "the configured default was used"
    );

    let assert = sandbox
        .cirrocast()
        .args(["status", "--offline", "-q", "-f", "%t"])
        .env("CIRROCAST_LOCATION", LOCATION)
        .assert()
        .code(0);
    assert_eq!(stdout_of(&assert), "+18°C\n", "the environment tier won");

    // `--location` still outranks the variable.
    let assert = sandbox
        .cirrocast()
        .args([
            "status",
            "--offline",
            "-q",
            "-f",
            "%t",
            "--location",
            "@1.0,2.0",
        ])
        .env("CIRROCAST_LOCATION", LOCATION)
        .assert()
        .code(0);
    assert_eq!(stdout_of(&assert), "n/a\n", "the flag won");
}

/// The probe names a coordinate from the bundled tables at most: it never adds a reverse request
/// to the run (step 25), because a status bar must not wait on a donated service for a display
/// name.
#[test]
fn the_probe_never_adds_a_reverse_request() {
    let sandbox = Sandbox::new();
    // A point no bundled city is within 25 km of, so the online half *would* be asked.
    let assert = sandbox
        .cirrocast()
        .args(["status", "--location", "@0,-140", "-v"])
        .assert();
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("UTF-8 stderr");
    assert!(
        !stderr.contains("nominatim"),
        "the probe must not name a coordinate over the network: {stderr}"
    );
    assert!(
        stderr.contains("no city within 25 km"),
        "the bundled-table note is what a `-v` run prints: {stderr}"
    );
}
