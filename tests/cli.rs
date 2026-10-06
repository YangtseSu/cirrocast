// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! End to end tests of the CLI surface, driven through the real binary.

mod common;

use predicates::prelude::*;

/// Every provider id the registry knows, in `provider list` order.
///
/// Read from the registry rather than pinned as a list: the test's job is "the table prints every
/// row the binary knows", and a hand-kept copy would go stale silently (it did, when steps 23 and
/// 24 added six backends).
fn provider_ids() -> Vec<&'static str> {
    cirrocast::provider::ProviderId::all()
        .iter()
        .map(cirrocast::provider::ProviderId::as_str)
        .collect()
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
    let sandbox = common::Sandbox::new();
    sandbox
        .cirrocast()
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
    let sandbox = common::Sandbox::new();
    sandbox.cirrocast().arg("--help").assert().success().stdout(
        predicate::str::contains("Usage: cirrocast")
            .and(predicate::str::contains("provider"))
            .and(predicate::str::contains("config")),
    );
}

#[test]
fn help_stays_inside_the_line_budget() {
    // Step 21's budget is "`--help` under 225 lines" (200 until step 23 added the three archive
    // and marine flags). The count depends on how clap wraps the
    // epilogue, and clap reads `COLUMNS` even when stdout is a pipe, so the width is pinned here
    // rather than inherited from whoever runs the tests.
    let sandbox = common::Sandbox::new();
    let assert = sandbox
        .cirrocast()
        .env("COLUMNS", "100")
        .arg("--help")
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("UTF-8 output");
    let lines = stdout.lines().count();
    assert!(
        lines < 225,
        "--help is {lines} lines (the budget is under 225)"
    );
}

#[test]
fn provider_list_shows_every_provider_and_its_key_requirement() {
    let sandbox = common::Sandbox::new();
    let assert = sandbox
        .cirrocast()
        .args(["provider", "list"])
        .assert()
        .success();
    let stdout =
        String::from_utf8(assert.get_output().stdout.clone()).expect("stdout should be UTF-8");

    for id in provider_ids() {
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

    // The network class (step 24): keyless rows are free, commercial BYOK services are not.
    assert!(
        columns("open-meteo").contains(&"free".to_owned()),
        "open-meteo should be free:\n{}",
        row(&stdout, "open-meteo")
    );
    assert!(
        columns("qweather").contains(&"nonfree".to_owned()),
        "qweather should be nonfree:\n{}",
        row(&stdout, "qweather")
    );

    // `provider info` prints the same metadata, plus the history/marine rows.
    let assert = sandbox
        .cirrocast()
        .args(["provider", "info", "open-meteo"])
        .assert()
        .success();
    let info = String::from_utf8(assert.get_output().stdout.clone()).expect("UTF-8 output");
    for needle in ["network:     free", "history days:92", "marine:      no"] {
        assert!(info.contains(needle), "{needle:?} missing from:\n{info}");
    }
}

#[test]
fn provider_info_rejects_an_unknown_id_as_a_usage_error() {
    let sandbox = common::Sandbox::new();
    sandbox
        .cirrocast()
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
    let sandbox = common::Sandbox::new();
    sandbox
        .cirrocast()
        .args(["config", "path"])
        .assert()
        .success()
        .stdout(predicate::eq(format!(
            "{}\n",
            sandbox.config_file().display()
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
    // The coordinate is named from the bundled tables (step 25) — the sandbox forbids the network,
    // so the run can only succeed by reading them — and the name's credit is the only stderr line.
    sandbox
        .cirrocast()
        .args(["location", "search", "@39.9042,116.4074"])
        .assert()
        .success()
        .stdout(predicate::eq(
            "Beijing, China (39.90, 116.41) <timezone resolved at fetch time>\n",
        ))
        .stderr(predicate::eq(
            "Location data by GeoNames (CC BY 4.0) — https://www.geonames.org/\n",
        ));
}

#[test]
fn location_search_reports_usage_errors_with_exit_code_two() {
    let sandbox = common::Sandbox::new();
    // `@91,0` is not a coordinate pair (91 is out of latitude range), so it is read as an alias —
    // and an unknown one, which is a usage error naming the forms that would have been accepted.
    sandbox
        .cirrocast()
        .args(["location", "search", "@91,0"])
        .assert()
        .code(2)
        .stdout(predicate::eq(""))
        .stderr(
            predicate::str::contains("unknown location alias `@91,0`")
                .and(predicate::str::contains("accepted forms: Beijing")),
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
            "weather      0 entries       0 B\ngeocode      0 entries       0 B\nip           0 entries       0 B\nstation      0 entries       0 B\nalerts       0 entries       0 B\ngrid         0 entries       0 B\nnormals      0 entries       0 B\nratelimit    0 entries       0 B\ngeo          0 entries       0 B\n",
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
    let sandbox = common::Sandbox::new();
    let body = std::fs::read_to_string(common::fixture_path(
        "open_meteo/forecast_beijing_2026-07-15.json",
    ))
    .expect("the fixture is readable");

    // The key the CLI computes for `@39.9042,116.4074 --days 3`: the location is coordinates, so
    // its zone is still UTC when the key is built and `local_today` is today's UTC date. The seed
    // covers the date on either side of local midnight so the pair cannot race the clock.
    common::seed_weather(
        &sandbox,
        "open-meteo",
        39.9042,
        116.4074,
        3,
        chrono_tz::Tz::UTC,
        &body,
    );

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
        stdout.contains("Data: Open-Meteo.com (CC BY 4.0) — https://open-meteo.com/"),
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

    // Checked before any request is sent, so this stays offline. The message lists the three
    // namespaces a format name can address: the built-in formats, the built-in presets and the
    // user's `[templates]` keys.
    sandbox
        .cirrocast()
        .args(["@39.9,116.4", "--format", "yaml"])
        .assert()
        .code(2)
        .stderr(
            predicate::str::contains("unknown format `yaml`")
                .and(predicate::str::contains(
                    "formats: art-table, one-line, plain, json, dumb",
                ))
                .and(predicate::str::contains(
                    "presets: default, short, minimal, full",
                )),
        );
}

/// A build without the bundled city table refuses `location update-data` with a message naming the
/// missing feature, rather than failing to compile or silently doing nothing. It only exists in the
/// `--no-default-features` build, which CI's `cargo test` job runs as its second command (review
/// §3.14 / nit 13).
#[cfg(not(feature = "offline-geo"))]
#[test]
fn a_build_without_the_city_table_refuses_update_data() {
    let sandbox = common::Sandbox::new();
    sandbox
        .cirrocast()
        .args(["location", "update-data"])
        .assert()
        .code(4)
        .stderr(predicate::str::contains(
            "this build has no offline city table (the `offline-geo` feature is off); nothing to install",
        ));
}
