// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The candidate picker wired through the real binary (step 20).
//!
//! `--pick` is a real flag, not a test hook: it forces the prompt where the TTY check cannot, which
//! is also what makes the interaction testable without a pseudo-terminal (this repository does not
//! spawn one for tests). Every run goes through `assert_cmd` with a scripted stdin, a throwaway XDG
//! sandbox and `CIRROCAST_FORBID_NETWORK=1`; the candidates come from the recorded geocode fixture
//! and the chosen place renders from a recorded forecast, so nothing here touches a socket.

mod common;

use std::sync::Arc;
use std::time::Duration;

use assert_cmd::Command;
use chrono::Utc;
use chrono_tz::Tz;
use cirrocast::cache::{Cache, CacheKey, CacheMode, SystemClock};
use common::{Sandbox, fixture_path};

/// The three hits of the recorded ambiguous `Beijing` response, in ranking order, as the picker
/// numbers them: `(place, latitude, longitude)`.
const CANDIDATES: [(&str, f64, f64); 3] = [
    ("Beijing, Beijing Municipality, China", 39.9075, 116.39723),
    ("Beijing, Shanxi, China", 35.20917, 110.73278),
    ("Beijing, Jiangxi, China", 29.34644, 116.19873),
];

/// A sandbox that resolves `Beijing` through the recorded geocode fixture (`strategy = "network"`
/// keeps the bundled table out of the way) and whose weather cache holds the recorded forecast for
/// each candidate in `cached` — so a run can only succeed for a candidate that was seeded.
fn sandbox(cached: &[usize], extra_config: &str) -> Sandbox {
    let sandbox = Sandbox::new();
    sandbox.write_config(&format!(
        "schema_version = 2\n[geo]\nstrategy = \"network\"\n{extra_config}"
    ));
    seed(
        &sandbox,
        &CacheKey::hash("geocode", "open-meteo|beijing|10|en"),
        "geo/open_meteo_geocode_beijing_ambiguous.json",
    );
    for index in cached {
        let (_, lat, lon) = CANDIDATES[*index];
        seed(
            &sandbox,
            &CacheKey::weather("open-meteo", lat, lon, 3, today(Tz::Asia__Shanghai)),
            "open_meteo/forecast_beijing_2026-07-15.json",
        );
    }
    sandbox
}

/// Writes one cache entry holding the recorded `fixture` body.
fn seed(sandbox: &Sandbox, key: &CacheKey, fixture: &str) {
    let body = std::fs::read_to_string(fixture_path(fixture)).expect("the fixture is readable");
    let cache = Cache::with_root(
        sandbox.cache_dir(),
        CacheMode::Normal,
        Arc::new(SystemClock),
        0,
    );
    cache
        .write(key, 200, &body, Duration::from_secs(3600))
        .expect("the cache entry is written");
}

/// Today in the location's zone: the weather key carries the location-local date.
fn today(tz: Tz) -> chrono::NaiveDate {
    Utc::now().with_timezone(&tz).date_naive()
}

/// A `cirrocast` run in `sandbox`.
fn run(sandbox: &Sandbox) -> Command {
    sandbox.cirrocast()
}

/// The captured stdout of a finished run.
fn stdout(assert: &assert_cmd::assert::Assert) -> String {
    String::from_utf8(assert.get_output().stdout.clone()).expect("stdout is UTF-8")
}

/// The captured stderr of a finished run.
fn stderr(assert: &assert_cmd::assert::Assert) -> String {
    String::from_utf8(assert.get_output().stderr.clone()).expect("stderr is UTF-8")
}

#[test]
fn pick_selects_the_second_candidate_end_to_end() {
    let sandbox = sandbox(&[1], "");
    let assert = run(&sandbox)
        .args(["Beijing", "--pick", "-f", "plain", "--offline"])
        .write_stdin("2\n")
        .assert()
        .success();

    let out = stdout(&assert);
    assert!(out.contains("location: Beijing, Shanxi, China"), "{out}");

    let err = stderr(&assert);
    assert!(
        err.contains("[1] * Beijing, Beijing Municipality, China"),
        "{err}"
    );
    assert!(err.contains("[2]   Beijing, Shanxi, China"), "{err}");
    assert!(
        err.contains(
            "selected: Beijing, Shanxi, China — use @35.20917,110.73278 to skip the prompt"
        ),
        "{err}"
    );
    assert!(!err.contains("candidates for `Beijing`"), "{err}");
}

#[test]
fn quiet_keeps_the_prompt_and_the_selection_echo_but_silences_the_notes() {
    let sandbox = sandbox(&[1], "");
    let assert = run(&sandbox)
        .args(["Beijing", "--pick", "-q", "-f", "plain", "--offline"])
        .write_stdin("2\n")
        .assert()
        .success();
    let err = stderr(&assert);
    assert!(
        err.contains("choose a location [1-3, Enter=1, q=quit]: "),
        "{err}"
    );
    // The selection echo is the reproducibility affordance for a prompted choice: it is printed
    // unconditionally, so `-q` keeps it.
    assert!(
        err.contains(
            "selected: Beijing, Shanxi, China — use @35.20917,110.73278 to skip the prompt"
        ),
        "{err}"
    );
    // The commentary note is still silenced.
    assert!(!err.contains("candidates for `Beijing`"), "{err}");
}

#[test]
fn yes_takes_the_winner_without_reading_stdin() {
    // Only the winner's forecast is cached; an answer of `2` would select the second candidate and
    // fail, so a successful run proves stdin was never read.
    let sandbox = sandbox(&[0], "");
    let assert = run(&sandbox)
        .args(["Beijing", "--yes", "-f", "plain", "--offline"])
        .write_stdin("2\n")
        .assert()
        .success();
    assert!(
        stdout(&assert).contains("location: Beijing, Beijing Municipality, China"),
        "{}",
        stdout(&assert)
    );
    let err = stderr(&assert);
    assert!(err.contains("candidates for `Beijing`"), "{err}");
    assert!(
        err.contains("`--pick` to choose one, or `--yes` to keep the winner"),
        "{err}"
    );
    assert!(!err.contains("choose a location"), "{err}");
}

#[test]
fn a_non_terminal_run_takes_the_winner_and_prints_the_extended_note() {
    let sandbox = sandbox(&[0], "");
    let assert = run(&sandbox)
        .args(["Beijing", "-f", "plain", "--offline"])
        .assert()
        .success();
    assert!(stdout(&assert).contains("location: Beijing, Beijing Municipality, China"));
    let err = stderr(&assert);
    assert!(err.contains("candidates for `Beijing`"), "{err}");
    assert!(err.contains("`--pick` to choose one"), "{err}");
    assert!(!err.contains("choose a location"), "{err}");
}

#[test]
fn pick_and_yes_are_a_usage_error_together() {
    let sandbox = sandbox(&[], "");
    run(&sandbox)
        .args(["Beijing", "--pick", "--yes"])
        .assert()
        .code(2)
        .stderr(predicates::str::contains("cannot be used with"));
}

#[test]
fn the_policy_never_behaves_like_yes_from_the_environment_and_from_the_file() {
    for (sandbox, environment) in [
        (sandbox(&[0], "[location]\npick = \"never\"\n"), false),
        (sandbox(&[0], ""), true),
    ] {
        let mut command = run(&sandbox);
        if environment {
            command.env("CIRROCAST_LOCATION_PICK", "never");
        }
        let assert = command
            .args(["Beijing", "-f", "plain", "--offline"])
            .write_stdin("2\n")
            .assert()
            .success();
        assert!(
            stdout(&assert).contains("location: Beijing, Beijing Municipality, China"),
            "{}",
            stdout(&assert)
        );
        assert!(!stderr(&assert).contains("choose a location"));
    }
}

#[test]
fn an_invalid_environment_policy_is_a_configuration_error() {
    let sandbox = sandbox(&[], "");
    run(&sandbox)
        .env("CIRROCAST_LOCATION_PICK", "sometimes")
        .args(["Beijing", "-f", "plain", "--offline"])
        .assert()
        .code(4)
        .stderr(predicates::str::contains(
            "location.pick: `sometimes` is not one of auto, never",
        ));
}

#[test]
fn junk_answers_and_eof_end_the_run_with_the_documented_codes() {
    let sandbox = sandbox(&[], "");
    run(&sandbox)
        .args(["Beijing", "--pick", "--offline"])
        .write_stdin("x\ny\nz\n")
        .assert()
        .code(2)
        .stderr(predicates::str::contains(
            "enter a number 1..=3, Enter for 1, or q to quit",
        ));

    run(&sandbox)
        .args(["Beijing", "--pick", "--offline"])
        .write_stdin("")
        .assert()
        .code(5)
        .stderr(predicates::str::contains(
            "no location selected for Beijing",
        ));
}

#[test]
fn location_search_never_prompts_and_all_matches_the_picker_ranking() {
    let sandbox = sandbox(&[], "");

    // The default output stays the winner line, and a pipe is never asked anything.
    let search = run(&sandbox)
        .args(["location", "search", "Beijing", "--offline"])
        .write_stdin("2\n")
        .assert()
        .success();
    let out = stdout(&search);
    assert_eq!(out.lines().count(), 1, "{out}");
    assert!(
        out.starts_with("Beijing, Beijing Municipality, China"),
        "{out}"
    );
    assert!(!stderr(&search).contains("choose a location"));

    let all = run(&sandbox)
        .args(["location", "search", "--all", "Beijing", "--offline"])
        .assert()
        .success();
    let table = ranked_places(&stdout(&all), ". ");

    // The picker lists the same ranking; `q` gives up after printing it.
    let pick = run(&sandbox)
        .args(["Beijing", "--pick", "--offline"])
        .write_stdin("q\n")
        .assert()
        .code(5);
    let listed = ranked_places(&stderr(&pick), "] ");
    assert_eq!(listed, table, "the two lists must be the one ranking");
    assert_eq!(table.len(), CANDIDATES.len());
    assert!(table[0].starts_with(CANDIDATES[0].0), "{}", table[0]);
}

/// The place text of every ranked row, in order: everything after `number` + `separator`, with the
/// `location search --all` table's `(population <n>)` spelled the picker's way.
fn ranked_places(text: &str, separator: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| line.split_once(separator).map(|(_, rest)| rest))
        .map(|rest| {
            let rest = rest
                .strip_prefix("* ")
                .or_else(|| rest.strip_prefix("  "))
                .unwrap_or(rest);
            rest.replace(" (population ", " (pop. ")
        })
        .collect()
}

#[test]
fn a_single_candidate_never_prompts_even_with_pick() {
    let sandbox = Sandbox::new();
    sandbox.write_config("schema_version = 2\n[geo]\nstrategy = \"network\"\n");
    seed(
        &sandbox,
        &CacheKey::hash("geocode", "open-meteo|vienna|10|en"),
        "geo/open_meteo_geocode_vienna.json",
    );
    seed(
        &sandbox,
        &CacheKey::weather(
            "open-meteo",
            48.20849,
            16.37208,
            3,
            today(Tz::Europe__Vienna),
        ),
        "open_meteo/forecast_beijing_2026-07-15.json",
    );

    // Empty stdin: a prompt would read EOF and exit 5, so success proves there was none.
    let assert = run(&sandbox)
        .args(["Vienna", "--pick", "-f", "plain", "--offline"])
        .write_stdin("")
        .assert()
        .success();
    assert!(
        stdout(&assert).contains("location: Vienna, Vienna, Austria"),
        "{}",
        stdout(&assert)
    );
    assert!(!stderr(&assert).contains("choose a location"));
}
