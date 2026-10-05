// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! CLI regression tests for the 2026-10-05 review (§3.2, §3.4, §3.5, §4.5 #45/#46/#58).
//!
//! Every run is offline: the sandbox sets `CIRROCAST_FORBID_NETWORK=1`, and the rendering runs
//! replay cache entries seeded from the recorded fixtures, so a test that reaches for the network
//! fails loudly instead of silently depending on it. Keys are built with the crate's own
//! `CacheKey` constructors and seeded for the local day and the next, so a run crossing local
//! midnight cannot miss the entry.

mod common;

use std::fs;

use chrono::Utc;
use chrono_tz::Tz;
use predicates::prelude::*;

use cirrocast::cache::{CACHE_SCHEMA_VERSION, CacheKey};
use common::{Sandbox, fixture_path};

/// The coordinates the seeded geocode entry resolves `Beijing` to.
const LAT: f64 = 39.9075;
/// The longitude half of the pair.
const LON: f64 = 116.39723;
/// The number of forecast days the rendering runs ask for.
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

/// Today in the location's zone: the weather key carries the location-local date.
fn today() -> chrono::NaiveDate {
    Utc::now().with_timezone(&Tz::Asia__Shanghai).date_naive()
}

/// Seeds the geocode entry that resolves `Beijing`.
fn seed_geocode(sandbox: &Sandbox) {
    let body = fs::read_to_string(fixture_path("geo/open_meteo_geocode_beijing.json"))
        .expect("the geocode fixture is readable");
    seed(
        sandbox,
        &CacheKey::hash("geocode", "open-meteo|beijing|10|en"),
        &body,
    );
}

/// Seeds the Open-Meteo forecast entry for the local day and the next.
fn seed_forecast(sandbox: &Sandbox) {
    let body = fs::read_to_string(fixture_path("open_meteo/forecast_beijing_2026-07-15.json"))
        .expect("the forecast fixture is readable");
    let day = today();
    for offset in [chrono::Days::new(0), chrono::Days::new(1)] {
        seed(
            sandbox,
            &CacheKey::weather(
                "open-meteo",
                LAT,
                LON,
                DAYS,
                day.checked_add_days(offset).expect("the date arithmetic"),
            ),
            &body,
        );
    }
}

/// A sandbox with the geocode and forecast entries seeded.
fn ready_sandbox() -> Sandbox {
    let sandbox = Sandbox::new();
    seed_geocode(&sandbox);
    seed_forecast(&sandbox);
    sandbox
}

#[test]
fn alerts_sources_rejects_auto_mixed_with_an_explicit_id() {
    // §3.2: the validator accepted `auto,fpas`, then every run failed with a usage error naming
    // `auto` as an unknown source. The validator must reject the mix, naming the key.
    let sandbox = Sandbox::new();
    sandbox.write_config("[alerts]\nsources = [\"auto\", \"fpas\"]\n");
    sandbox
        .cirrocast()
        .args(["config", "validate"])
        .assert()
        .code(4)
        .stderr(predicate::str::contains("alerts.sources"));
}

#[test]
fn alerts_sources_still_accepts_auto_alone() {
    let sandbox = Sandbox::new();
    sandbox.write_config("[alerts]\nsources = [\"auto\"]\n");
    sandbox
        .cirrocast()
        .args(["config", "validate"])
        .assert()
        .success()
        .stdout(predicate::str::starts_with("ok: "));
}

#[test]
fn alerts_sources_accepts_the_wired_visualcrossing_id() {
    // §3.4 originally: `visualcrossing` was a legal shape the runtime always rejected. Step 23
    // wired it with its provider, so a config list naming it now validates and must run.
    let sandbox = Sandbox::new();
    sandbox.write_config("[alerts]\nsources = [\"visualcrossing\"]\n");
    sandbox
        .cirrocast()
        .args(["config", "validate"])
        .assert()
        .success()
        .stdout(predicate::str::starts_with("ok: "));
}

#[test]
fn alerts_from_visualcrossing_needs_its_provider() {
    // Step 23 wired the `visualcrossing` alert source, and its warnings travel in the forecast
    // payload, so naming the source without its provider is a usage error rather than a silent
    // "no warnings" answer.
    let sandbox = Sandbox::new();
    sandbox
        .cirrocast()
        .args([
            "--alerts-from",
            "visualcrossing",
            "--lat",
            "38.97",
            "--lon",
            "-77.35",
        ])
        .assert()
        .code(2)
        .stderr(
            predicate::str::contains("--provider visualcrossing")
                .and(predicate::str::contains("not wired up yet").not()),
        );
}

#[test]
fn an_empty_argument_uses_the_configured_default_location() {
    // §3.5: `Some("")` used to short-circuit `location.default` and query the public IP. An empty
    // positional must mean "absent", so the configured default resolves — and does so from the
    // cache, with no IP request.
    let sandbox = ready_sandbox();
    sandbox.write_config("[location]\ndefault = \"Beijing\"\n");
    sandbox
        .cirrocast()
        .args(["", "-d", "3", "--offline"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Beijing"));
}

#[test]
fn an_empty_argument_with_ip_is_not_a_usage_error() {
    // §3.5: `cirrocast "" --ip` reported "a location argument cannot be combined with `--ip`".
    // With the empty argument treated as absent, `--ip` is a legal location request; offline it
    // fails as a network error (exit 3), never as usage (exit 2).
    let sandbox = Sandbox::new();
    let output = sandbox
        .cirrocast()
        .args(["", "--ip", "--offline"])
        .output()
        .expect("the binary runs");
    assert_ne!(
        output.status.code(),
        Some(2),
        "an empty argument plus --ip must not be a usage error; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn a_template_file_with_a_trailing_newline_prints_exactly_one_line() {
    // §4.5 #58: `--template-file -` kept the template's trailing newline and appended another, so
    // a one-line template emitted two lines.
    let sandbox = ready_sandbox();
    let assert = sandbox
        .cirrocast()
        .args([
            "Beijing",
            "-d",
            "3",
            "--offline",
            "-f",
            "one-line",
            "--template-file",
            "-",
        ])
        .write_stdin("%l\n")
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("stdout is UTF-8");
    assert_eq!(
        stdout.trim_end_matches('\n'),
        "Beijing",
        "one rendered line, no duplicated template"
    );
    assert_eq!(stdout.lines().count(), 1, "no doubled newline: {stdout:?}");
}

#[test]
fn the_dumb_format_prints_no_escapes_even_with_color_always() {
    // §4.5 #45: `--format dumb` forces the mono palette, so an explicit `--color always` must not
    // paint it.
    let sandbox = ready_sandbox();
    let output = sandbox
        .cirrocast()
        .args([
            "Beijing",
            "-d",
            "3",
            "--offline",
            "-f",
            "dumb",
            "--color",
            "always",
        ])
        .output()
        .expect("the binary runs");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains('\u{1b}'),
        "the dumb format must emit no escapes:\n{stdout}"
    );
}

#[test]
fn color_always_under_a_dumb_terminal_folds_to_sixteen() {
    // §4.5 #46: `--color always` under `TERM=dumb` used to upgrade `Mono` to the 256-colour
    // palette. An explicit request must still produce escapes, but folded down to the sixteen
    // ANSI colours: never a `38;5;`/`48;5;` sequence.
    let sandbox = ready_sandbox();
    let output = sandbox
        .cirrocast()
        .env("TERM", "dumb")
        .args(["Beijing", "-d", "3", "--offline", "--color", "always"])
        .output()
        .expect("the binary runs");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains('\u{1b}'),
        "an explicit `--color always` emits escapes even on a dumb terminal:\n{stdout}"
    );
    // A folded palette is still spelled `38;5;N`, so the fold is proved by the index: a dumb
    // terminal must never see an index above the sixteen ANSI colours.
    for (start, _) in stdout.match_indices("38;5;") {
        let number: u32 = stdout[start + 5..]
            .chars()
            .take_while(char::is_ascii_digit)
            .collect::<String>()
            .parse()
            .expect("a colour index follows the sequence prefix");
        assert!(
            number <= 15,
            "256-colour index {number} reached a dumb terminal:\n{stdout}"
        );
    }
}

#[test]
fn the_dumb_format_says_why_under_verbose() {
    // §4.5 #45: the discarded `--color always` is now explained under `-v`.
    let sandbox = ready_sandbox();
    let output = sandbox
        .cirrocast()
        .args([
            "Beijing",
            "-d",
            "3",
            "--offline",
            "-f",
            "dumb",
            "--color",
            "always",
            "-v",
        ])
        .output()
        .expect("the binary runs");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--format dumb"),
        "the verbose note names the format:\n{stderr}"
    );
}
