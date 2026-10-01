// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The `CIRROCAST_FORBID_NETWORK` guard, proved from the outside.
//!
//! `src/http.rs` reads the variable once and refuses every non-loopback request before DNS or
//! connect. The first test exercises that refusal against a real upstream host (there is no stub a
//! separate process could be pointed at), so a silently broken guard cannot pass on a machine with
//! network; the second runs the CLI in a network namespace without external connectivity, so a
//! guarded `--offline` run is proven not to need DNS or a socket at all.

mod common;

use std::fs;
use std::process::Command;

use chrono::Utc;

use cirrocast::cache::{CACHE_SCHEMA_VERSION, CacheKey};
use common::{Sandbox, fixture_path};

/// Writes one fresh cache entry for `key`.
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
fn a_cold_cache_request_is_refused_before_it_reaches_the_network() {
    let sandbox = Sandbox::new();
    let assert = sandbox
        .cirrocast()
        .args(["Beijing", "-f", "plain"])
        .env("CIRROCAST_FORBID_NETWORK", "1")
        .assert()
        .code(3);
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("UTF-8 stderr");
    assert!(
        stderr.contains("CIRROCAST_FORBID_NETWORK"),
        "the guard message is missing: {stderr}"
    );
    assert!(
        !sandbox.cache_dir().exists(),
        "a blocked run must not leave cache entries behind"
    );
}

/// The binary under test, as cargo built it for this test run.
const BINARY: &str = env!("CARGO_BIN_EXE_cirrocast");

#[cfg(target_os = "linux")]
#[test]
fn a_guarded_offline_run_works_without_any_external_network() {
    // `unshare -rn` maps the current user to root in a new network namespace with only loopback:
    // if the run needed DNS or a socket to a real host, it would fail here and nowhere else.
    if !Command::new("unshare")
        .args(["-rn", "true"])
        .status()
        .is_ok_and(|status| status.success())
    {
        eprintln!("skipping: this kernel does not allow unprivileged network namespaces");
        return;
    }

    let sandbox = Sandbox::new();
    seed(
        &sandbox,
        &CacheKey::hash("geocode", "open-meteo|beijing|10|en"),
        &fs::read_to_string(fixture_path("geo/open_meteo_geocode_beijing.json"))
            .expect("the geocode fixture is readable"),
    );
    seed(
        &sandbox,
        &CacheKey::weather(
            "open-meteo",
            39.9075,
            116.39723,
            3,
            Utc::now()
                .with_timezone(&chrono_tz::Tz::Asia__Shanghai)
                .date_naive(),
        ),
        &fs::read_to_string(fixture_path("open_meteo/forecast_beijing_2026-07-15.json"))
            .expect("the forecast fixture is readable"),
    );

    let output = Command::new("unshare")
        .args(["-rn", BINARY, "--offline", "Beijing", "-f", "plain"])
        .env("CIRROCAST_FORBID_NETWORK", "1")
        .env("XDG_CONFIG_HOME", sandbox.home().join("config"))
        .env("XDG_CONFIG_DIRS", sandbox.home().join("system"))
        .env("XDG_CACHE_HOME", sandbox.home().join("cache"))
        .env("XDG_DATA_HOME", sandbox.home().join("data"))
        .env("LC_ALL", "C.UTF-8")
        .output()
        .expect("unshare runs");
    assert!(
        output.status.success(),
        "the guarded offline run failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout is UTF-8");
    assert!(stdout.contains("18°C"), "{stdout}");
}
