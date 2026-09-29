// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! End to end tests of the CLI surface, driven through the real binary.

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
