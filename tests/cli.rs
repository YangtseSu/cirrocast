// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! End to end tests of the CLI surface, driven through the real binary.

mod common;

use assert_cmd::Command;
use predicates::prelude::*;

/// Every provider id the registry knows, in `provider list` order.
const PROVIDER_IDS: [&str; 8] = [
    "open-meteo",
    "openweathermap",
    "weatherapi",
    "worldweatheronline",
    "pirateweather",
    "qweather",
    "smhi",
    "metar",
];

fn cirrocast() -> Command {
    Command::cargo_bin("cirrocast").expect("the cirrocast binary is built by cargo test")
}

/// The table row whose first column is `id`.
fn row<'a>(table: &'a str, id: &str) -> &'a str {
    table
        .lines()
        .find(|line| line.starts_with(id))
        .unwrap_or_else(|| panic!("no `{id}` row in:\n{table}"))
}

#[test]
fn version_follows_the_scripting_contract() {
    cirrocast()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::eq(format!(
            "cirrocast {}\n",
            env!("CARGO_PKG_VERSION")
        )));
}

#[test]
fn help_exits_successfully() {
    cirrocast().arg("--help").assert().success().stdout(
        predicate::str::contains("Usage: cirrocast")
            .and(predicate::str::contains("provider"))
            .and(predicate::str::contains("config")),
    );
}

#[test]
fn provider_list_shows_every_provider_and_its_key_requirement() {
    let assert = cirrocast().args(["provider", "list"]).assert().success();
    let stdout =
        String::from_utf8(assert.get_output().stdout.clone()).expect("stdout should be UTF-8");

    for id in PROVIDER_IDS {
        assert!(stdout.contains(id), "`{id}` missing from:\n{stdout}");
    }

    let columns = |id: &str| -> Vec<String> {
        row(&stdout, id)
            .split_whitespace()
            .map(str::to_owned)
            .collect()
    };

    assert!(
        columns("open-meteo").iter().any(|cell| cell == "none"),
        "open-meteo should be keyless:\n{}",
        row(&stdout, "open-meteo")
    );
    assert!(
        columns("qweather").contains(&"CIRROCAST_QWEATHER_KEY".to_owned()),
        "qweather should name its env var:\n{}",
        row(&stdout, "qweather")
    );
}

#[test]
fn provider_info_rejects_an_unknown_id_as_a_usage_error() {
    cirrocast()
        .args(["provider", "info", "nope"])
        .assert()
        .code(2)
        .stderr(
            predicate::str::contains("error:")
                .and(predicate::str::contains("unknown provider `nope`")),
        );
}

#[test]
fn config_path_prints_the_xdg_config_file() {
    let config_home = tempfile::tempdir().expect("tempdir");

    cirrocast()
        .env("XDG_CONFIG_HOME", config_home.path())
        .env("XDG_CONFIG_DIRS", config_home.path().join("system"))
        .args(["config", "path"])
        .assert()
        .success()
        .stdout(predicate::eq(format!(
            "{}\n",
            config_home.path().join("cirrocast/config.toml").display()
        )));
}

#[test]
fn config_show_applies_the_same_environment_overrides_as_config_get() {
    let sandbox = common::Sandbox::new();
    sandbox.write_config(
        "schema_version = 1\n\
         [defaults]\n\
         format = \"plain\"\n",
    );

    let get = |sandbox: &common::Sandbox, args: &[&str]| -> String {
        let assert = sandbox
            .cirrocast()
            .env("CIRROCAST_FORMAT", "json")
            .args(args)
            .assert()
            .success();
        String::from_utf8(assert.get_output().stdout.clone()).expect("UTF-8 output")
    };

    // The variable outranks the file for both subcommands, so they cannot disagree.
    assert_eq!(
        get(&sandbox, &["config", "get", "defaults.format"]).trim(),
        "json"
    );
    let shown = get(&sandbox, &["config", "show"]);
    assert!(
        shown.contains("format = \"json\""),
        "`config show` is the effective configuration:\n{shown}"
    );

    // With no variable, the file's value is what both report.
    let assert = sandbox
        .cirrocast()
        .args(["config", "show"])
        .assert()
        .success();
    let shown = String::from_utf8(assert.get_output().stdout.clone()).expect("UTF-8 output");
    assert!(shown.contains("format = \"plain\""), "{shown}");
}

#[test]
fn config_edit_rejects_an_unknown_key_like_config_validate_does() {
    use std::os::unix::fs::PermissionsExt;

    let sandbox = common::Sandbox::new();
    // The editor only has to leave a document behind: `config edit` validates whatever is on disk
    // after the editor exits, and an unknown key is the problem `validate` exists to catch.
    let editor = sandbox.home().join("editor.sh");
    std::fs::write(
        &editor,
        "#!/bin/sh\nprintf 'schema_version = 1\\n[defaults]\\nnope = true\\n' > \"$1\"\n",
    )
    .expect("the editor script is written");
    let mut permissions = std::fs::metadata(&editor)
        .expect("the script exists")
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&editor, permissions).expect("the script is executable");

    sandbox
        .cirrocast()
        .env("VISUAL", &editor)
        .env("EDITOR", &editor)
        .args(["config", "edit"])
        .assert()
        .code(4)
        .stderr(predicate::str::contains(
            "unknown config key `defaults.nope`",
        ));
}

#[test]
fn location_search_resolves_coordinates_without_touching_the_network() {
    let sandbox = common::Sandbox::new();
    sandbox
        .cirrocast()
        .args(["location", "search", "@39.9042,116.4074"])
        .assert()
        .success()
        .stdout(predicate::eq(
            "39.9042, 116.4074 <timezone resolved at fetch time>\n",
        ))
        .stderr(predicate::eq(""));
}

#[test]
fn location_search_reports_usage_errors_with_exit_code_two() {
    let sandbox = common::Sandbox::new();
    sandbox
        .cirrocast()
        .args(["location", "search", "@91,0"])
        .assert()
        .code(2)
        .stdout(predicate::eq(""))
        .stderr(
            predicate::str::contains("latitude 91 is out of range -90..=90").and(
                predicate::str::contains(
                    "accepted forms: Beijing | :Beijing | ~Tsinghua | @39.9042,116.4074",
                ),
            ),
        );

    sandbox
        .cirrocast()
        .args(["location", "search", "B"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("is too short"));
}

#[test]
fn location_search_help_and_error_messages_share_the_accepted_forms() {
    let sandbox = common::Sandbox::new();
    sandbox
        .cirrocast()
        .args(["location", "search", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "accepted forms: Beijing | :Beijing | ~Tsinghua | @39.9042,116.4074",
        ));
}

#[test]
fn cache_stat_separates_expired_entries_from_valid_ones() {
    let sandbox = common::Sandbox::new();
    let weather = sandbox.cache_dir().join("weather");
    std::fs::create_dir_all(&weather).expect("the cache directory is writable");

    let entry = |key: &str, fetched_at: &str, ttl_secs: u64| {
        format!(
            r#"{{"cache_schema_version":1,"key":"{key}","fetched_at":"{fetched_at}","ttl_secs":{ttl_secs},"status":200,"body":"{{}}"}}"#
        )
    };
    let now = chrono::Utc::now();
    std::fs::write(
        weather.join("fresh.json"),
        entry("weather|fresh", &now.to_rfc3339(), 600),
    )
    .expect("the fresh entry is written");
    std::fs::write(
        weather.join("stale.json"),
        entry(
            "weather|stale",
            &(now - chrono::Duration::days(30)).to_rfc3339(),
            600,
        ),
    )
    .expect("the stale entry is written");

    sandbox
        .cirrocast()
        .args(["cache", "stat"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("weather      2 entries")
                .and(predicate::str::contains("1 expired"))
                .and(predicate::str::contains("geocode      0 entries")),
        );
}

#[test]
fn cache_stat_and_clean_report_an_empty_cache() {
    let sandbox = common::Sandbox::new();
    sandbox
        .cirrocast()
        .args(["cache", "stat"])
        .assert()
        .success()
        .stdout(predicate::eq(
            "weather      0 entries       0 B\ngeocode      0 entries       0 B\nip           0 entries       0 B\nstation      0 entries       0 B\nalerts       0 entries       0 B\n",
        ));

    sandbox
        .cirrocast()
        .args(["cache", "clean"])
        .assert()
        .success()
        .stdout(predicate::eq("removed 0 expired entries\n"));

    sandbox
        .cirrocast()
        .args(["cache", "clean", "--all"])
        .assert()
        .success()
        .stdout(predicate::eq("removed 0 entries\n"));

    sandbox
        .cirrocast()
        .args(["cache", "clean", "--all", "--offline"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains(
            "offline mode: cache writes are disabled",
        ));
}

#[test]
fn cache_mode_flags_are_mutually_exclusive() {
    let sandbox = common::Sandbox::new();
    sandbox
        .cirrocast()
        .args([
            "location",
            "search",
            "@39.9,116.4",
            "--no-cache",
            "--offline",
        ])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("cannot be used with"));
}

/// The whole pipeline over a pre-seeded cache entry: no request leaves the machine, and the output
/// is the one a fetched report produces.
#[test]
fn a_coordinate_query_renders_a_report_from_the_cache() {
    use std::sync::Arc;
    use std::time::Duration;

    use cirrocast::cache::{Cache, CacheKey, CacheMode, SystemClock};

    let sandbox = common::Sandbox::new();
    let body = std::fs::read_to_string(common::fixture_path(
        "open_meteo/forecast_beijing_2026-07-15.json",
    ))
    .expect("the fixture is readable");

    // The key the CLI computes for `@39.9042,116.4074 --days 3`: the location is coordinates, so
    // its zone is still UTC when the key is built and `local_today` is today's UTC date.
    let today = chrono::Utc::now().date_naive();
    let key = CacheKey::weather("open-meteo", 39.9042, 116.4074, 3, today);
    let cache = Cache::with_root(
        sandbox.cache_dir(),
        CacheMode::Normal,
        Arc::new(SystemClock),
        0,
    );
    cache
        .write(&key, 200, &body, Duration::from_secs(600))
        .expect("the cache entry is written");

    let assert = sandbox
        .cirrocast()
        .args(["@39.9042,116.4074", "--format", "plain", "--offline"])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("UTF-8 output");

    // The response's zone replaces the provisional UTC of a coordinate location, and the report is
    // the fixture's first day aggregated into the canonical four parts.
    assert!(stdout.contains("Asia/Shanghai"), "{stdout}");
    assert!(
        stdout.contains(
            "current: Clear sky 18°C (feels 13°C) wind 13km/h NW humidity 11% precip 0.0mm pressure 1021hPa visibility 17km"
        ),
        "{stdout}"
    );
    assert!(
        stdout.contains("updated: 2026-09-30T19:30:00+08:00"),
        "{stdout}"
    );
    assert!(
        stdout.contains("day 2026-07-15: Morning") && stdout.contains(" | Noon "),
        "{stdout}"
    );
    // The four part labels are catalog words now, so they are spelled as the catalog spells them.
    for label in ["Morning", "Noon", "Evening", "Night"] {
        assert!(stdout.contains(label), "`{label}` missing from:\n{stdout}");
    }
    assert!(
        stdout.contains("Data: Open-Meteo.com (CC BY 4.0)"),
        "{stdout}"
    );
    assert!(
        stdout.contains("attribution: open-meteo https://api.open-meteo.com/v1/forecast"),
        "{stdout}"
    );
}

#[test]
fn an_unknown_provider_is_a_usage_error() {
    let sandbox = common::Sandbox::new();
    sandbox
        .cirrocast()
        .args(["-p", "open-meteo,does-not-exist", "@39.9,116.4"])
        .assert()
        .code(2)
        .stderr(
            predicate::str::contains("error:").and(predicate::str::contains(
                "unknown provider `does-not-exist`",
            )),
        );
}

#[test]
fn metar_is_selectable_and_reaches_the_fetch() {
    let sandbox = common::Sandbox::new();
    // `metar` has a backend now, so this run gets past selection and fails on the offline cache
    // miss (exit 3) rather than on a usage error: the station table answers the metadata without a
    // request, and the observation is what is missing.
    sandbox
        .cirrocast()
        .args(["-p", "metar", "--station", "ZBAA", "--offline"])
        .assert()
        .code(3)
        .stderr(predicate::str::contains("offline: no cached metar"));
}

#[test]
fn an_unknown_format_is_a_usage_error() {
    let sandbox = common::Sandbox::new();

    // Checked before any request is sent, so this stays offline.
    sandbox
        .cirrocast()
        .args(["@39.9,116.4", "--format", "yaml"])
        .assert()
        .code(2)
        .stderr(
            predicate::str::contains("invalid value 'yaml'").and(predicate::str::contains(
                "possible values: art-table, one-line, plain, json, dumb",
            )),
        );
}
