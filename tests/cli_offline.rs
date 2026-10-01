// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! End-to-end offline rendering: the recorded payloads drive the real binary.
//!
//! Each test seeds the sandbox cache with a recorded upstream body (and the geocode entry that
//! resolves `Beijing`), then runs `cirrocast --offline`. No test opens a socket, and the assertions
//! are the ones a user would make by eye: the current temperature, the condition text and the
//! credit line the data licence requires.
//!
//! The cache keys are built with the crate's own `CacheKey` constructors, so a key-shape change
//! cannot silently make these tests pass by missing the entry.

mod common;

use std::fs;
use std::path::Path;

use chrono::Utc;
use chrono_tz::Tz;
use predicates::prelude::*;

use cirrocast::cache::{CACHE_SCHEMA_VERSION, CacheKey};
use common::{Sandbox, fixture_path};

/// The coordinates the seeded geocode entry resolves `Beijing` to.
const LAT: f64 = 39.9075;
const LON: f64 = 116.39723;

/// The three forecast days the runs ask for.
const DAYS: u8 = 3;

/// Writes one cache entry with the recorded `body`.
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

/// Seeds the geocode entry that resolves `Beijing` to [`LAT`]/[`LON`].
fn seed_geocode(sandbox: &Sandbox) {
    let body = fs::read_to_string(fixture_path("geo/open_meteo_geocode_beijing.json"))
        .expect("the geocode fixture is readable");
    seed(
        sandbox,
        &CacheKey::hash("geocode", "open-meteo|beijing|10|en"),
        &body,
    );
}

/// Stores the throwaway keys the keyed backends need, `0600` as the real store does.
fn seed_keys(sandbox: &Sandbox) {
    use std::os::unix::fs::PermissionsExt as _;

    let text = "[keys]\n\
                openweathermap = \"test-key-openweathermap\"\n\
                weatherapi = \"test-key-weatherapi\"\n\
                worldweatheronline = \"test-key-worldweatheronline\"\n\
                pirateweather = \"test-key-pirateweather\"\n\
                qweather = \"test-key-qweather\"\n";
    fs::create_dir_all(
        sandbox
            .keys_file()
            .parent()
            .expect("the key file has a parent"),
    )
    .expect("the config directory");
    fs::write(sandbox.keys_file(), text).expect("keys.toml is written");
    fs::set_permissions(sandbox.keys_file(), fs::Permissions::from_mode(0o600))
        .expect("keys.toml is owner-only");
}

/// The recorded payload of `tests/fixtures/<name>`.
fn body(name: &str) -> String {
    fs::read_to_string(fixture_path(name)).expect("the fixture is readable")
}

/// Today in the location's zone: the weather key carries the location-local date.
fn today() -> chrono::NaiveDate {
    Utc::now().with_timezone(&Tz::Asia__Shanghai).date_naive()
}

/// Runs the binary offline for `provider` and returns stdout.
fn render(sandbox: &Sandbox, provider: &str) -> String {
    let assert = sandbox
        .cirrocast()
        .args([
            "--offline",
            "-p",
            provider,
            "-d",
            "3",
            "Beijing",
            "-f",
            "plain",
        ])
        .assert()
        .success();
    String::from_utf8(assert.get_output().stdout.clone()).expect("stdout is UTF-8")
}

/// Asserts the current line carries `temperature` and the credit line names the licence.
fn assert_rendered(stdout: &str, temperature: &str, credit: &str) {
    assert!(
        stdout.contains(temperature),
        "expected {temperature} in:\n{stdout}"
    );
    assert!(stdout.contains(credit), "expected {credit} in:\n{stdout}");
    assert!(stdout.contains("current:"), "no current line in:\n{stdout}");
}

#[test]
fn open_meteo_renders_from_the_recorded_payload() {
    let sandbox = Sandbox::new();
    seed_geocode(&sandbox);
    seed(
        &sandbox,
        &CacheKey::weather("open-meteo", LAT, LON, DAYS, today()),
        &body("open_meteo/forecast_beijing_2026-07-15.json"),
    );
    let stdout = render(&sandbox, "open-meteo");
    assert_rendered(&stdout, "18°C", "Data: Open-Meteo.com (CC BY 4.0)");
    assert!(stdout.contains("Clear sky"), "{stdout}");
}

#[test]
fn smhi_renders_from_the_recorded_payload() {
    let sandbox = Sandbox::new();
    seed_geocode(&sandbox);
    seed(
        &sandbox,
        &CacheKey::weather("smhi", LAT, LON, DAYS, today()),
        &body("smhi/point_stockholm_2026-09-30.json"),
    );
    let stdout = render(&sandbox, "smhi");
    assert_rendered(&stdout, "14°C", "Data: SMHI (CC BY 4.0 SE)");
    assert!(stdout.contains("Overcast"), "{stdout}");
}

#[test]
fn openweathermap_renders_from_the_recorded_payloads() {
    let sandbox = Sandbox::new();
    seed_geocode(&sandbox);
    seed_keys(&sandbox);
    let date = today();
    seed(
        &sandbox,
        &CacheKey::weather_part("openweathermap", "current", LAT, LON, DAYS, date),
        &body("owm/current.json"),
    );
    seed(
        &sandbox,
        &CacheKey::weather_part("openweathermap", "forecast", LAT, LON, DAYS, date),
        &body("owm/forecast.json"),
    );
    let stdout = render(&sandbox, "openweathermap");
    assert_rendered(
        &stdout,
        "16°C",
        "Data: OpenWeather (ODbL 1.0) — https://openweathermap.org/",
    );
    assert!(stdout.contains("Clear sky"), "{stdout}");
}

#[test]
fn weatherapi_renders_from_the_recorded_payload() {
    let sandbox = Sandbox::new();
    seed_geocode(&sandbox);
    seed_keys(&sandbox);
    seed(
        &sandbox,
        &CacheKey::weather("weatherapi", LAT, LON, DAYS, today()),
        &body("weatherapi/forecast.json"),
    );
    let stdout = render(&sandbox, "weatherapi");
    assert_rendered(
        &stdout,
        "15°C",
        "Data: WeatherAPI.com (free-tier attribution) — https://www.weatherapi.com/",
    );
    assert!(stdout.contains("Clear sky"), "{stdout}");
}

#[test]
fn worldweatheronline_renders_from_the_recorded_payload() {
    let sandbox = Sandbox::new();
    seed_geocode(&sandbox);
    seed_keys(&sandbox);
    seed(
        &sandbox,
        &CacheKey::weather("worldweatheronline", LAT, LON, DAYS, today()),
        &body("wwo/weather_ashx.json"),
    );
    let stdout = render(&sandbox, "worldweatheronline");
    assert_rendered(
        &stdout,
        "15°C",
        "Data: WorldWeatherOnline.com (free-tier attribution) — https://www.worldweatheronline.com/",
    );
    assert!(stdout.contains("Clear sky"), "{stdout}");
}

#[test]
fn pirateweather_renders_from_the_recorded_payload() {
    let sandbox = Sandbox::new();
    seed_geocode(&sandbox);
    seed_keys(&sandbox);
    seed(
        &sandbox,
        &CacheKey::weather("pirateweather", LAT, LON, DAYS, today()),
        &body("pirateweather/forecast.json"),
    );
    let stdout = render(&sandbox, "pirateweather");
    assert_rendered(
        &stdout,
        "13°C",
        "Data: Pirate Weather — https://pirateweather.net/",
    );
    assert!(stdout.contains("Overcast"), "{stdout}");
}

#[test]
fn qweather_renders_from_the_recorded_payloads() {
    let sandbox = Sandbox::new();
    seed_geocode(&sandbox);
    seed_keys(&sandbox);
    // The account host comes from the configuration; the test uses a placeholder.
    sandbox.write_config("[providers.qweather]\nhost = \"https://example.qweatherapi.com\"\n");
    let date = today();
    seed(
        &sandbox,
        &CacheKey::weather_part("qweather", "current", LAT, LON, DAYS, date),
        &body("qweather/current.json"),
    );
    seed(
        &sandbox,
        &CacheKey::weather_part("qweather", "hourly", LAT, LON, DAYS, date),
        &body("qweather/hourly.json"),
    );
    let stdout = render(&sandbox, "qweather");
    assert_rendered(
        &stdout,
        "12°C",
        "Data: QWeather — https://www.qweather.com/",
    );
    assert!(stdout.contains("Clear sky"), "{stdout}");
}

#[test]
fn no_run_output_or_cache_entry_carries_the_api_key() {
    let sandbox = Sandbox::new();
    seed_geocode(&sandbox);
    let date = today();
    seed(
        &sandbox,
        &CacheKey::weather_part("openweathermap", "current", LAT, LON, DAYS, date),
        &body("owm/current.json"),
    );
    seed(
        &sandbox,
        &CacheKey::weather_part("openweathermap", "forecast", LAT, LON, DAYS, date),
        &body("owm/forecast.json"),
    );

    // The key travels through the environment (the documented BYOK path); `-vv` is the most
    // talkative mode, so it is where a leak would first show.
    let secret = "sk-live-0123456789abcdef";
    let assert = sandbox
        .cirrocast()
        .args([
            "-vv",
            "--offline",
            "-p",
            "openweathermap",
            "-d",
            "3",
            "Beijing",
            "-f",
            "plain",
        ])
        .env("CIRROCAST_OPENWEATHERMAP_KEY", secret)
        .assert()
        .success();
    let output = assert.get_output();
    for (stream, bytes) in [("stdout", &output.stdout), ("stderr", &output.stderr)] {
        let text = String::from_utf8(bytes.clone()).expect("UTF-8 output");
        assert!(!text.contains(secret), "the key leaked to {stream}: {text}");
    }

    // Nothing written under the cache may carry it either.
    scan(&sandbox.cache_dir(), &[secret]);
    if sandbox.config_file().exists() {
        let config = fs::read_to_string(sandbox.config_file()).expect("config.toml is readable");
        assert!(!config.contains(secret), "the key leaked into config.toml");
    }
}

/// The fixtures must not carry any of the recorded API keys.
#[test]
fn no_fixture_carries_an_api_key() {
    let keys = std::env::var("HOME")
        .map(|home| std::path::PathBuf::from(home).join(".config/cirrocast/keys.toml"));
    let Ok(path) = keys else {
        return;
    };
    let Ok(text) = fs::read_to_string(&path) else {
        return;
    };
    let secrets: Vec<&str> = text
        .lines()
        .filter_map(|line| line.split_once('='))
        .map(|(_, value)| value.trim().trim_matches('"'))
        .filter(|value| value.len() >= 8)
        .collect();
    assert_ne!(secrets, Vec::<&str>::new(), "no keys to scan for");

    scan(Path::new("tests/fixtures"), &secrets);
}

/// Walks `directory`, asserting no file contains any of `secrets`.
fn scan(directory: &Path, secrets: &[&str]) {
    for entry in fs::read_dir(directory).expect("the fixtures are readable") {
        let path = entry.expect("a directory entry").path();
        if path.is_dir() {
            scan(&path, secrets);
            continue;
        }
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        for secret in secrets {
            assert!(
                !text.contains(secret),
                "{} contains an API key",
                path.display()
            );
        }
    }
}

// ---------------------------------------------------------------------------------------------
// `--offline` correctness: same output as a warm run, no socket, a hint on a miss
// ---------------------------------------------------------------------------------------------

/// A sandbox whose cache already holds the geocode answer and the Beijing forecast.
fn warm_sandbox() -> Sandbox {
    let sandbox = Sandbox::new();
    seed_geocode(&sandbox);
    seed(
        &sandbox,
        &CacheKey::weather("open-meteo", LAT, LON, DAYS, today()),
        &body("open_meteo/forecast_beijing_2026-07-15.json"),
    );
    sandbox
}

/// The plain-format forecast line for `Beijing`, with `flags` in front of the location.
fn plain_run(sandbox: &Sandbox, flags: &[&str]) -> std::process::Output {
    let mut args: Vec<&str> = flags.to_vec();
    args.extend(["-p", "open-meteo", "-d", "3", "Beijing", "-f", "plain"]);
    sandbox
        .cirrocast()
        .args(&args)
        .assert()
        .success()
        .get_output()
        .clone()
}

#[test]
fn a_warm_cache_renders_identically_with_and_without_offline() {
    let sandbox = warm_sandbox();
    let offline = plain_run(&sandbox, &["--offline"]);
    let online = plain_run(&sandbox, &[]);
    assert_eq!(offline.stdout, online.stdout, "offline changed the output");
    assert_eq!(offline.stderr, online.stderr, "offline changed the notes");
}

#[test]
fn offline_never_asks_for_a_socket_even_with_the_guard_on() {
    // The guard turns any real connection attempt into a distinct error, so a successful run
    // proves that `--offline` served everything from disk. A warm cache cannot be a false pass:
    // the run would not fail if it refetched, it would merely print fresh data.
    let sandbox = warm_sandbox();
    let guarded = sandbox
        .cirrocast()
        .args([
            "--offline",
            "-p",
            "open-meteo",
            "-d",
            "3",
            "Beijing",
            "-f",
            "plain",
        ])
        .env("CIRROCAST_FORBID_NETWORK", "1")
        .assert()
        .success();
    assert_eq!(
        guarded.get_output().stdout,
        plain_run(&sandbox, &["--offline"]).stdout
    );
}

#[test]
fn a_cold_cache_with_offline_exits_three_and_names_the_rerun() {
    let sandbox = Sandbox::new();
    sandbox
        .cirrocast()
        .args(["--offline", "Beijing", "-f", "plain"])
        .assert()
        .code(3)
        .stderr(
            predicate::str::contains("offline mode: no cached open-meteo answer for")
                .and(predicate::str::contains("rerun without `--offline`")),
        );
}

#[test]
fn only_vv_prints_request_and_cache_detail() {
    let sandbox = warm_sandbox();
    let quiet = plain_run(&sandbox, &["--offline", "-v"]);
    let chatty = plain_run(&sandbox, &["--offline", "-vv"]);
    let quiet_stderr = String::from_utf8(quiet.stderr).expect("stderr is UTF-8");
    let chatty_stderr = String::from_utf8(chatty.stderr).expect("stderr is UTF-8");

    assert!(!quiet_stderr.contains("cache: "), "{quiet_stderr}");
    assert!(
        quiet_stderr.contains("provider: open-meteo (from "),
        "-v still explains the resolved settings: {quiet_stderr}"
    );
    assert!(chatty_stderr.contains("cache: "), "{chatty_stderr}");
    assert!(chatty_stderr.contains("hit"), "{chatty_stderr}");
}

#[test]
fn offline_conflicts_name_both_flags() {
    let sandbox = Sandbox::new();
    for other in ["--no-cache", "--refresh"] {
        let assert = sandbox
            .cirrocast()
            .args(["--offline", other, "Beijing"])
            .assert()
            .code(2);
        let stderr =
            String::from_utf8(assert.get_output().stderr.clone()).expect("stderr is UTF-8");
        for flag in ["--offline", other] {
            assert!(
                stderr.contains(flag),
                "{flag} is not named in the conflict: {stderr}"
            );
        }
    }
}
