// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The CLI contract: the flag matrix, the conflict rules, the precedence tiers and the exit codes.
//!
//! Everything here runs the real binary against a throwaway XDG sandbox. The weather cache is
//! seeded with a recorded response and every run is `--offline`, so no test touches the network —
//! a flag that would have to fetch is asserted through its *validation*, which happens before any
//! request is made.

mod common;

use std::sync::Arc;
use std::time::Duration;

use assert_cmd::Command;
use cirrocast::cache::{Cache, CacheKey, CacheMode, SystemClock};
use predicates::prelude::*;

use common::Sandbox;

/// The coordinates the seeded cache entries are keyed by.
const LAT: f64 = 39.9042;
const LON: f64 = 116.4074;

/// The location argument those coordinates spell.
const LOCATION: &str = "@39.9042,116.4074";

/// A sandbox whose weather cache holds the `days`-day fixture for [`LOCATION`].
fn seeded(days: u8) -> Sandbox {
    let sandbox = Sandbox::new();
    let body = std::fs::read_to_string(common::fixture_path(
        "open_meteo/forecast_beijing_2026-07-15.json",
    ))
    .expect("the fixture is readable");
    // A coordinate location still carries the provisional UTC zone when the key is built, so the
    // day is today's UTC date — the same rule the CLI follows. The seed spans local midnight so the
    // seed/execute pair cannot race it.
    common::seed_weather(
        &sandbox,
        "open-meteo",
        LAT,
        LON,
        days,
        chrono_tz::Tz::UTC,
        &body,
    );
    sandbox
}

/// A weather run against the seeded cache, in the given mode.
fn run(sandbox: &Sandbox, args: &[&str]) -> Command {
    let mut command = sandbox.cirrocast();
    command.args(args).arg("--offline");
    command
}

#[test]
fn every_flag_is_accepted_in_both_spellings() {
    let sandbox = seeded(3);

    // Each case: the flag, a fragment stdout must carry, and a fragment stderr must carry. Only
    // the flags whose effect the rendered output cannot show (a *resolution* the renderer does not
    // print) add `-v`; the note that then names the flag's tier is the observable effect. Cases
    // whose effect is entirely in the output keep an empty stderr fragment.
    let cases: [(&[&str], &str, &str); 20] = [
        (
            &["-p", "open-meteo", "-v"],
            "Weather report:",
            "provider: open-meteo (from the command line)",
        ),
        (
            &["--provider", "open-meteo", "-v"],
            "Weather report:",
            "provider: open-meteo (from the command line)",
        ),
        (&["-f", "plain"], "location: ", ""),
        (&["--format", "plain"], "location: ", ""),
        (&["-f", "json"], "\"schema_version\": 2", ""),
        (&["--format", "json"], "\"schema_version\": 2", ""),
        (&["-f", "one-line"], "Clear sky", ""),
        (&["--format", "one-line", "--template", "@short"], "*o*", ""),
        (&["-f", "dumb"], "+18C", ""),
        (
            &["-d", "3", "-v"],
            "Weather report:",
            "days: 3 (from the command line)",
        ),
        (
            &["--days", "3", "-v"],
            "Weather report:",
            "days: 3 (from the command line)",
        ),
        (
            &["-u", "metric", "-v"],
            "Weather report:",
            "units: metric (from the command line)",
        ),
        (
            &["--units", "us", "-f", "plain"],
            "current: Clear sky 65°F",
            "",
        ),
        (
            &["--lang", "en-US", "-v"],
            "Weather report:",
            "language: en-US (from the command line)",
        ),
        (
            &["--lang", "auto", "-v"],
            "Weather report:",
            "language: auto (from the command line)",
        ),
        (
            &["--timeout", "30", "-v"],
            "Weather report:",
            "timeout: 30s (from the command line)",
        ),
        (
            &["--color", "never", "-v"],
            "Weather report:",
            "color: never",
        ),
        (
            &["--width", "60", "-v"],
            "Weather report:",
            "width: 60 columns",
        ),
        (&["-v"], "Weather report:", "provider: "),
        (&["--verbose"], "Weather report:", "provider: "),
    ];

    for (args, expected, note) in cases {
        let assert = run(&sandbox, args).arg(LOCATION).assert().success();
        let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("UTF-8 output");
        let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("UTF-8 stderr");
        assert!(stdout.contains(expected), "{args:?}:\n{stdout}");
        assert!(stderr.contains(note), "{args:?} (stderr):\n{stderr}");
    }

    // `-q`/`--quiet` suppress the one note a clean run can print: the fallback warning for an
    // unsupported language. The same run without the flag carries it, so the flag's effect — not
    // just its acceptance — is what is asserted.
    let warning = sandbox
        .cirrocast()
        .args(["--lang", "de-DE", "--offline"])
        .arg(LOCATION)
        .assert()
        .success();
    let stderr = String::from_utf8(warning.get_output().stderr.clone()).expect("UTF-8 stderr");
    assert!(stderr.contains("unsupported language"), "{stderr}");

    for flag in ["-q", "--quiet"] {
        let assert = run(&sandbox, &[flag, "--lang", "de-DE"])
            .arg(LOCATION)
            .assert()
            .success();
        let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("UTF-8 stderr");
        assert!(
            !stderr.contains("unsupported language"),
            "{flag} did not silence the note:\n{stderr}"
        );
    }
}

#[test]
fn latitude_and_longitude_replace_the_location_argument() {
    let sandbox = seeded(3);
    let assert = sandbox
        .cirrocast()
        .args(["--lat", "39.9042", "--lon", "116.4074", "--offline"])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("UTF-8 output");
    assert!(
        stdout.contains("39.9042, 116.4074"),
        "the coordinates are the location:\n{stdout}"
    );
}

#[test]
fn coordinate_flags_are_reported_as_the_command_line_source() {
    let sandbox = seeded(3);
    let assert = sandbox
        .cirrocast()
        .args(["-v", "--lat", "39.9042", "--lon", "116.4074", "--offline"])
        .assert()
        .success();
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("UTF-8 stderr");
    assert!(
        stderr.contains("location: @39.9042,116.4074 (from the command line)"),
        "the coordinates come from the flag that supplied them: {stderr}"
    );
}

#[test]
fn color_and_width_reach_the_renderer() {
    let sandbox = seeded(3);

    let plain = sandbox
        .cirrocast()
        .args([LOCATION, "--offline", "--color", "always"])
        .assert()
        .success();
    let stdout = String::from_utf8(plain.get_output().stdout.clone()).expect("UTF-8 output");
    assert!(
        stdout.contains('\u{1b}'),
        "`always` emits escapes even into a pipe"
    );

    let narrow = sandbox
        .cirrocast()
        .args([LOCATION, "--offline", "--width", "60"])
        .assert()
        .success();
    let stdout = String::from_utf8(narrow.get_output().stdout.clone()).expect("UTF-8 output");
    for line in stdout.lines() {
        let width = line.chars().count();
        assert!(
            width <= 60,
            "line of {width} columns in a 60 column run: {line:?}"
        );
    }

    // Raising a too-small explicit width is a note, not a failure.
    let raised = sandbox
        .cirrocast()
        .args([LOCATION, "--offline", "--width", "10", "-v"])
        .assert()
        .success();
    let stderr = String::from_utf8(raised.get_output().stderr.clone()).expect("UTF-8 stderr");
    assert!(
        stderr.contains("10 columns is below the 20 column minimum"),
        "{stderr}"
    );
}

#[test]
fn the_colour_environment_ladder_decides_when_the_flag_is_auto() {
    let sandbox = seeded(3);

    // Every run asks for `auto`, so the decision is the environment's. Both halves of the ladder
    // are cleared unless the case sets one, so the ambient shell cannot influence the result.
    let render = |signals: &[(&str, &str)]| -> String {
        let mut command = sandbox.cirrocast();
        command.args([
            LOCATION,
            "--offline",
            "--format",
            "art-table",
            "--color",
            "auto",
        ]);
        for name in ["NO_COLOR", "CLICOLOR_FORCE"] {
            command.env_remove(name);
        }
        for &(name, value) in signals {
            command.env(name, value);
        }
        let assert = command.assert().success();
        String::from_utf8(assert.get_output().stdout.clone()).expect("UTF-8 output")
    };

    // Neither signal: stdout is a pipe, so `auto` resolves to no colour.
    let plain = render(&[]);
    assert!(
        !plain.contains('\u{1b}'),
        "a pipe with neither signal must not colour: {plain:?}"
    );

    // CLICOLOR_FORCE outranks the tty check.
    let forced = render(&[("CLICOLOR_FORCE", "1")]);
    assert!(
        forced.contains('\u{1b}'),
        "CLICOLOR_FORCE must force colour even into a pipe"
    );

    // NO_COLOR disables; an empty value is still present, which is what the convention keys on.
    for value in ["1", ""] {
        let denied = render(&[("NO_COLOR", value)]);
        assert!(
            !denied.contains('\u{1b}'),
            "NO_COLOR={value:?} must disable colour: {denied:?}"
        );
    }
}

#[test]
fn every_conflict_rule_exits_two_with_its_message() {
    let sandbox = Sandbox::new();

    // (arguments, the stderr fragment that identifies the rule)
    let cases: [(&[&str], &str); 11] = [
        (&["--lat", "39.9", LOCATION], "--lon"),
        (&["--lon", "116.4", LOCATION], "--lat"),
        (
            &["--lat", "39.9", "--lon", "116.4", "--ip"],
            "cannot be used with",
        ),
        (&["--station", "ZBAA", "--ip"], "cannot be used with"),
        (
            &[LOCATION, "--ip"],
            "a location argument cannot be combined with --ip",
        ),
        (
            &[LOCATION, "--lat", "39.9", "--lon", "116.4"],
            "a location argument cannot be combined with --lat/--lon",
        ),
        (
            &[LOCATION, "--station", "ZBAA"],
            "a location argument cannot be combined with --station",
        ),
        (&["-q", "-v", LOCATION], "cannot be used with"),
        (
            &[LOCATION, "--no-cache", "--offline"],
            "cannot be used with",
        ),
        (
            &[LOCATION, "--no-cache", "--refresh"],
            "cannot be used with",
        ),
        (&[LOCATION, "--refresh", "--offline"], "cannot be used with"),
    ];

    for (args, fragment) in cases {
        sandbox
            .cirrocast()
            .args(args)
            .assert()
            .code(2)
            .stderr(predicate::str::contains(fragment));
    }
}

#[test]
fn a_station_needs_the_station_backend() {
    let sandbox = Sandbox::new();

    // An explicit chain without a station-capable entry is a usage error naming both flags, and it
    // is decided before any request is sent.
    sandbox
        .cirrocast()
        .args(["--station", "ZBAA", "-p", "open-meteo"])
        .assert()
        .code(2)
        .stderr(
            predicate::str::contains("--station ZBAA needs a station-capable provider")
                .and(predicate::str::contains("--provider metar")),
        );

    // A chain that *contains* `metar` passes the rule, wherever the entry sits.
    sandbox
        .cirrocast()
        .args(["--station", "ZBAA", "-p", "open-meteo,metar", "--offline"])
        .assert()
        .code(3)
        .stderr(predicate::str::contains("offline:"));

    // Without `--provider`, `--station` prepends `metar` to the configured chain; the same offline
    // miss proves the run reached the station backend rather than failing on the flag combination.
    sandbox
        .cirrocast()
        .args(["--station", "ZBAA", "--offline"])
        .assert()
        .code(3)
        .stderr(predicate::str::contains(
            "offline: no cached metar current observation for station ZBAA at weather/metar-ZBAA-current.json",
        ));

    // `auto` gains `metar` for a station run; the station backend is tried first.
    sandbox
        .cirrocast()
        .args(["--station", "ZBAA", "-p", "auto", "--offline"])
        .assert()
        .code(3)
        .stderr(predicate::str::contains("metar-ZBAA-current.json"));
}

#[test]
fn a_station_identifier_must_be_an_icao_code() {
    let sandbox = Sandbox::new();

    // Three letters (an IATA code) and five digits (a WMO number) are other vocabularies.
    for value in ["12", "JFK", "KJFKX", "1234", "K-1"] {
        sandbox
            .cirrocast()
            .args(["--station", value, "-p", "metar"])
            .assert()
            .code(2)
            .stderr(
                predicate::str::contains("is not an ICAO station identifier")
                    .and(predicate::str::contains("--station EGLL")),
            );
    }

    // A lower case identifier is normalised, not rejected: the offline miss names the upper case
    // spelling, which is what the cache key and the upstream request use.
    sandbox
        .cirrocast()
        .args(["--station", "kjfk", "-p", "metar", "--offline"])
        .assert()
        .code(3)
        .stderr(predicate::str::contains("metar-KJFK-current.json"));
}

#[test]
fn a_template_belongs_to_one_line() {
    let sandbox = Sandbox::new();

    for format in ["plain", "json", "art-table", "dumb"] {
        sandbox
            .cirrocast()
            .args([LOCATION, "-f", format, "--template", "%c"])
            .assert()
            .code(2)
            .stderr(
                predicate::str::contains("`--template` requires `--format one-line`")
                    .and(predicate::str::contains(format)),
            );
    }

    sandbox
        .cirrocast()
        .args([LOCATION, "-f", "one-line", "--template", "@nope"])
        .assert()
        .code(2)
        .stderr(
            predicate::str::contains("unknown one-line preset `@nope`")
                .and(predicate::str::contains("@default")),
        );

    sandbox
        .cirrocast()
        .args([LOCATION, "-f", "one-line", "--template="])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("template is empty"));
}

#[test]
fn an_unknown_template_token_is_a_usage_error() {
    let sandbox = seeded(3);

    // The CLI refuses a typo (exit 2, before any request); the compat surface that serves the same
    // token table keeps the literal, so the engine's passthrough stays covered by the library
    // tests.
    run(&sandbox, &["-f", "one-line", "--template", "%y %c", "-v"])
        .arg(LOCATION)
        .assert()
        .code(2)
        .stderr(
            predicate::str::contains("unknown template token `%y` at position 1").and(
                predicate::str::contains("known tokens: cCxtfHLwhpPeuUmMvldDTZzSsAq"),
            ),
        );

    // A bare `-f one-line` still renders the default preset: the gate only fires for a template
    // that really carries the unknown token.
    run(&sandbox, &["-f", "one-line", "--template", "@short"])
        .arg(LOCATION)
        .assert()
        .success();
}

#[test]
fn one_line_credits_travel_on_stderr() {
    let sandbox = seeded(3);
    let assert = run(&sandbox, &["-f", "one-line", "--template", "@short"])
        .arg(LOCATION)
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("UTF-8 output");
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("UTF-8 stderr");
    assert_eq!(stdout.lines().count(), 1, "one line by contract: {stdout}");
    assert!(!stdout.contains("Data:"), "{stdout}");
    assert!(
        stderr.contains("Data: Open-Meteo.com (CC BY 4.0) — https://open-meteo.com/"),
        "the licence credit goes to stderr: {stderr}"
    );
}

#[test]
fn the_precedence_tiers_pick_the_documented_winner() {
    let sandbox = seeded(3);
    let tier = |args: &[&str], env: Option<(&str, &str)>| -> String {
        let mut command = run(&sandbox, args);
        if let Some((name, value)) = env {
            command.env(name, value);
        }
        let assert = command.arg(LOCATION).assert().success();
        String::from_utf8(assert.get_output().stderr.clone()).expect("UTF-8 stderr")
    };

    // Built-in default.
    let stderr = tier(&["-f", "plain", "-v"], None);
    assert!(
        stderr.contains("units: metric (from the config or the built-in default)"),
        "{stderr}"
    );

    // Configuration file.
    sandbox.write_config("[defaults]\nunits = \"us\"\n");
    let stderr = tier(&["-f", "plain", "-v"], None);
    assert!(
        stderr.contains("units: us (from the config or the built-in default)"),
        "{stderr}"
    );
    let assert = run(&sandbox, &["-f", "plain"])
        .arg(LOCATION)
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("UTF-8 output");
    assert!(
        stdout.contains("current: Clear sky 65°F"),
        "the config wins over the built-in: {stdout}"
    );

    // The environment wins over the configuration.
    let stderr = tier(&["-f", "plain", "-v"], Some(("CIRROCAST_UNITS", "uk")));
    assert!(
        stderr.contains("units: uk (from the environment)"),
        "{stderr}"
    );
    let assert = run(&sandbox, &["-f", "plain"])
        .env("CIRROCAST_UNITS", "uk")
        .arg(LOCATION)
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("UTF-8 output");
    assert!(
        stdout.contains("current: Clear sky 18°C"),
        "uk is Celsius with mph winds: {stdout}"
    );

    // The flag wins over the environment.
    let stderr = tier(
        &["-f", "plain", "-u", "metric", "-v"],
        Some(("CIRROCAST_UNITS", "uk")),
    );
    assert!(
        stderr.contains("units: metric (from the command line)"),
        "{stderr}"
    );

    // The provider is reported with its own tier, even though only one backend is implemented.
    let stderr = tier(&["-f", "plain", "-p", "open-meteo", "-v"], None);
    assert!(
        stderr.contains("provider: open-meteo (from the command line)"),
        "{stderr}"
    );
    let stderr = tier(
        &["-f", "plain", "-v"],
        Some(("CIRROCAST_PROVIDER", "open-meteo")),
    );
    assert!(
        stderr.contains("provider: open-meteo (from the environment)"),
        "{stderr}"
    );
}

#[test]
fn an_environment_location_is_overridden_by_a_flag_not_treated_as_a_conflict() {
    let sandbox = seeded(3);
    let assert = sandbox
        .cirrocast()
        .env("CIRROCAST_LOCATION", "Beijing")
        .args(["--lat", "39.9042", "--lon", "116.4074", "--offline"])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("UTF-8 output");
    assert!(stdout.contains("39.9042, 116.4074"), "{stdout}");
}

#[test]
fn quiet_suppresses_the_notes_but_not_the_output() {
    let sandbox = seed_geocode_ambiguous();

    let assert = sandbox
        .cirrocast()
        .args(["location", "search", "Beijing", "--offline"])
        .assert()
        .success();
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("UTF-8 stderr");
    assert!(stderr.contains("candidates for `Beijing`"), "{stderr}");

    let quiet = sandbox
        .cirrocast()
        .args(["location", "search", "Beijing", "--offline", "-q"])
        .assert()
        .success();
    let stderr = String::from_utf8(quiet.get_output().stderr.clone()).expect("UTF-8 stderr");
    assert!(!stderr.contains("candidates for `Beijing`"), "{stderr}");
    assert_eq!(
        assert.get_output().stdout.clone(),
        quiet.get_output().stdout.clone(),
        "the answer itself is not a note"
    );
}

#[test]
fn verbose_still_lists_every_ranked_candidate() {
    // The listing shares `Resolved`'s candidate list with the picker and `location search --all`;
    // this pins that `-v` prints the whole ranking the resolution built.
    let sandbox = seed_geocode_ambiguous();
    let assert = sandbox
        .cirrocast()
        .args(["location", "search", "Beijing", "--offline", "-v"])
        .assert()
        .success();
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("UTF-8 stderr");
    assert!(stderr.contains("location: candidate 1/3"), "{stderr}");
    assert!(stderr.contains("location: candidate 3/3"), "{stderr}");
}

/// A sandbox whose geocode cache answers `Beijing` with the recorded ambiguous response.
fn seed_geocode_ambiguous() -> Sandbox {
    let sandbox = Sandbox::new();
    let body = common::fixture("geo/open_meteo_geocode_beijing_ambiguous.json");
    let key = CacheKey::hash("geocode", "open-meteo|beijing|10|en");
    let cache = Cache::with_root(
        sandbox.cache_dir(),
        CacheMode::Normal,
        Arc::new(SystemClock),
        0,
    );
    cache
        .write(&key, 200, &body, Duration::from_secs(600))
        .expect("the cache entry is written");
    sandbox
}

#[test]
fn completions_and_the_man_page_come_from_the_same_command() {
    let sandbox = Sandbox::new();

    for shell in ["bash", "zsh", "fish", "elvish", "powershell"] {
        let assert = sandbox
            .cirrocast()
            .args(["completion", shell])
            .assert()
            .success();
        let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("UTF-8 output");
        assert_ne!(stdout, "", "{shell} produced nothing");
        assert!(
            stdout.contains("cirrocast"),
            "{shell} does not mention the program"
        );
    }

    sandbox
        .cirrocast()
        .args(["completion", "bash", "--bin-name", "weather"])
        .assert()
        .success()
        .stdout(predicate::str::contains("weather"));

    sandbox
        .cirrocast()
        .args(["man"])
        .assert()
        .success()
        .stdout(predicate::str::contains(".TH cirrocast 1"));

    sandbox
        .cirrocast()
        .args(["man", "--bin-name", "weather"])
        .assert()
        .success()
        .stdout(predicate::str::contains(".TH weather 1"));
}

#[test]
fn help_and_version_read_no_state_and_the_man_page_reuses_the_help_text() {
    let sandbox = Sandbox::new();
    // A configuration file no other command could survive, and a world-readable key file: if
    // `--help` or `--version` loaded either, they would fail or print a warning.
    sandbox.install_fixture("config/bad-syntax.toml");
    std::fs::create_dir_all(
        sandbox
            .keys_file()
            .parent()
            .expect("the key file has a parent"),
    )
    .expect("the config directory");
    std::fs::write(sandbox.keys_file(), "[keys]\n").expect("the key file is written");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(sandbox.keys_file(), std::fs::Permissions::from_mode(0o644))
            .expect("the key file is group readable");
    }

    for flag in ["--help", "--version"] {
        let assert = sandbox.cirrocast().arg(flag).assert().success();
        let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("UTF-8 stderr");
        assert_eq!(stderr, "", "{flag} wrote to stderr");
    }
    assert!(
        !sandbox.cache_dir().exists(),
        "a help screen must not create the cache"
    );

    // The man page is rendered from the same clap command, so it carries the same two tables.
    sandbox.cirrocast().arg("man").assert().success().stdout(
        predicate::str::contains("CONFIG PRECEDENCE (highest first)")
            .and(predicate::str::contains("EXIT CODES"))
            .and(predicate::str::contains("missing or invalid API key")),
    );
}

#[test]
fn the_help_documents_the_flag_matrix_and_both_tables() {
    let sandbox = Sandbox::new();
    let assert = sandbox.cirrocast().arg("--help").assert().success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("UTF-8 output");

    for fragment in [
        "CONFIG PRECEDENCE (highest first)",
        "EXIT CODES",
        "ONE-LINE TOKENS",
        "--ip",
        "--lat <DEG>",
        "--lon <DEG>",
        "--station <ICAO>",
        "--template <TEMPLATE>",
        "--color <WHEN>",
        "--width <COLS>",
        "--no-cache",
        "--refresh",
        "--offline",
        "--timeout <SECS>",
    ] {
        assert!(
            stdout.contains(fragment),
            "`{fragment}` missing from --help"
        );
    }

    // The precedence table names the positional argument for the location, not a `--location`
    // flag: `assert_cmd` runs the real binary, and `cirrocast --location Beijing` is rejected.
    assert!(
        stdout.contains("LOCATION  CIRROCAST_LOCATION"),
        "the row names the argument"
    );
    assert!(
        !stdout.contains("--location"),
        "no `--location` flag exists to advertise"
    );

    // `--bin-name` lives on the two document subcommands, so it is documented where it is used.
    for command in ["completion", "man"] {
        sandbox
            .cirrocast()
            .args([command, "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("--bin-name <NAME>"));
    }

    // The preset table in `--help` is a copy of the one `--template` accepts; this is the guard
    // against the two drifting apart.
    for (name, template) in cirrocast::template::PRESETS {
        assert!(
            stdout.contains(&format!("@{name}")),
            "preset `@{name}` missing"
        );
        assert!(
            stdout.contains(template),
            "template `{template}` missing from --help"
        );
    }
}
