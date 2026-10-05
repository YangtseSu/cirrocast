// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The exit-code contract, one deterministic scenario per binding code.
//!
//! A script reads the code, not the prose, so every scenario here pins the code *and* the part of
//! the message that names the offending value — never a whole sentence, so rewording stays cheap
//! while "which value was wrong?" stays asserted. Every scenario is offline: the network guard is
//! exported for the runs that would otherwise reach out, and the cache is seeded through the
//! crate's own key constructors where a run needs data.
//!
//! This file is the one end-to-end driver of the 0–6 table. The `Error` variants that share a code
//! are pinned where the process boundary can reach them: code 3 is driven both as [`Error::Network`]
//! (the guard) and as [`Error::Upstream`] (a cached answer the decoder refuses), and code 6 as
//! [`Error::MissingKey`] (through `qweather`). `InvalidKey`/`InvalidToken` also map to 6, but their
//! only process-boundary path is a live `401` from an upstream — the alert sources that use a token
//! degrade to a note by design — so the conversion is pinned at the stub-transport level in
//! `tests/meteoalarm_token.rs` instead of by opening a socket here.

mod common;

use std::fs;

use chrono::Utc;
use chrono_tz::Tz;
use predicates::prelude::*;

use cirrocast::cache::{CACHE_SCHEMA_VERSION, CacheKey};
use common::{Sandbox, seed_weather};

/// The coordinates of the seeded weather cache entry.
const LAT: f64 = 39.9042;
const LON: f64 = 116.4074;

/// The location argument those coordinates spell.
const LOCATION: &str = "@39.9042,116.4074";

/// The forecast days the seeded entry is keyed by.
const DAYS: u8 = 3;

/// Writes one fresh cache entry for `key`, the way a previous run would have left it.
fn seed(sandbox: &Sandbox, key: &CacheKey, body: &str) {
    let path = sandbox.cache_dir().join(key.path());
    fs::create_dir_all(path.parent().expect("the entry has a parent"))
        .expect("the cache directory");
    let envelope = serde_json::json!({
        "cache_schema_version": CACHE_SCHEMA_VERSION,
        "key": key.normalised(),
        "fetched_at": Utc::now().to_rfc3339(),
        "ttl_secs": 600,
        "status": 200,
        "body": body,
    });
    fs::write(
        &path,
        serde_json::to_string_pretty(&envelope).expect("the envelope encodes"),
    )
    .expect("the entry is written");
}

#[test]
fn zero_is_success() {
    let sandbox = Sandbox::new();
    sandbox
        .cirrocast()
        .arg("--version")
        .assert()
        .code(0)
        .stdout(predicate::str::starts_with("cirrocast "));
}

#[test]
fn two_is_usage_and_names_the_rejected_value() {
    let sandbox = Sandbox::new();
    for (args, value) in [
        (vec!["--days", "99", "@39.9,116.4"], "99"),
        (vec!["--format", "yaml", "@39.9,116.4"], "yaml"),
        (vec!["--station", "12"], "12"),
    ] {
        let assert = sandbox.cirrocast().args(&args).assert().code(2);
        let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("UTF-8 stderr");
        assert!(
            stderr.contains(value),
            "{args:?} does not name `{value}`: {stderr}"
        );
    }

    // The range and the accepted values are part of the contract, not incidental prose.
    sandbox
        .cirrocast()
        .args(["--days", "99", "@39.9,116.4"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("0..=14"));
    sandbox
        .cirrocast()
        .args(["--format", "yaml", "@39.9,116.4"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains(
            "formats: art-table, one-line, plain, json, dumb",
        ));
}

#[test]
fn four_is_configuration_state_on_disk() {
    // An unreadable configuration file.
    let sandbox = Sandbox::new();
    sandbox.write_config("schema_version = 1\n");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(sandbox.config_file(), fs::Permissions::from_mode(0o000))
            .expect("the config file is made unreadable");
    }
    let assert = sandbox
        .cirrocast()
        .args(["@39.9,116.4", "--offline"])
        .assert()
        .code(4);
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("UTF-8 stderr");
    assert!(
        stderr.contains(&sandbox.config_file().display().to_string()),
        "the error does not name the file: {stderr}"
    );

    // A key file group/other can read is refused rather than used.
    let sandbox = Sandbox::new();
    fs::create_dir_all(
        sandbox
            .keys_file()
            .parent()
            .expect("the key file has a parent"),
    )
    .expect("the config directory");
    fs::write(
        sandbox.keys_file(),
        "[keys]\nopenweathermap = \"fake-key-abcd\"\n",
    )
    .expect("the key file is written");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(sandbox.keys_file(), fs::Permissions::from_mode(0o644))
            .expect("the key file is made group readable");
    }
    sandbox
        .cirrocast()
        .args(["key", "list"])
        .assert()
        .code(4)
        .stderr(
            predicate::str::contains("chmod 600").and(predicate::str::contains(
                sandbox.keys_file().display().to_string(),
            )),
        );
}

#[test]
fn six_is_a_missing_key_and_names_both_ways_to_store_one() {
    let sandbox = Sandbox::new();
    sandbox
        .cirrocast()
        .args(["-p", "qweather", "@39.9,116.4"])
        .env("CIRROCAST_FORBID_NETWORK", "1")
        .assert()
        .code(6)
        .stderr(
            predicate::str::contains("cirrocast key set qweather")
                .and(predicate::str::contains("CIRROCAST_QWEATHER_KEY")),
        );
}

#[test]
fn three_is_a_blocked_fetch_or_a_failing_upstream() {
    // Network: `--refresh` skips the cache, so the run must fetch; the guard turns the attempt into
    // the documented network failure before a socket exists.
    let sandbox = Sandbox::new();
    sandbox
        .cirrocast()
        .args(["--refresh", "@39.9,116.4", "-f", "plain"])
        .env("CIRROCAST_FORBID_NETWORK", "1")
        .assert()
        .code(3)
        .stderr(predicate::str::contains("CIRROCAST_FORBID_NETWORK"));

    // Upstream: a cached body that parses as a forecast envelope but carries no `daily` block is
    // refused by the provider's own decoder. Serving it from the cache needs no socket, which is
    // what lets `Error::Upstream` be pinned at the process boundary deterministically.
    let upstream = Sandbox::new();
    seed_weather(
        &upstream,
        "open-meteo",
        LAT,
        LON,
        DAYS,
        Tz::UTC,
        r#"{"latitude":39.9042,"longitude":116.4074,"utc_offset_seconds":0,"timezone":"UTC"}"#,
    );
    upstream
        .cirrocast()
        .args([
            LOCATION,
            "-p",
            "open-meteo",
            "-d",
            "3",
            "--offline",
            "-f",
            "plain",
        ])
        .assert()
        .code(3)
        .stderr(predicate::str::contains(
            "the response has no `daily` block",
        ));
}

#[test]
fn one_is_a_generic_failure() {
    let sandbox = Sandbox::new();
    // `config edit` hands the file to an editor; a missing one fails the spawn, which is the generic
    // code (1). Both variables are pinned because `VISUAL` outranks `EDITOR` and the developer's
    // shell has one of them set.
    let missing = sandbox.home().join("no-such-editor");
    sandbox
        .cirrocast()
        .args(["config", "edit"])
        .env("VISUAL", &missing)
        .env("EDITOR", &missing)
        .assert()
        .code(1)
        .stderr(predicate::str::contains("cannot run"));
}

#[test]
fn five_is_an_unknown_station() {
    let sandbox = Sandbox::new();
    // The station metadata endpoint answered "no such station" once; the cached empty array is
    // what the backend reads, so the run stays offline and deterministic.
    seed(&sandbox, &CacheKey::station("ZZZZ"), "[]");
    sandbox
        .cirrocast()
        .args(["--station", "ZZZZ", "--offline", "-f", "plain"])
        .assert()
        .code(5)
        .stderr(
            predicate::str::contains("unknown station `ZZZZ`")
                .and(predicate::str::contains("cirrocast -p metar")),
        );
}
