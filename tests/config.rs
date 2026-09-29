// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! End to end tests of `cirrocast config …` and of the configuration precedence rules.

mod common;

use std::fs;

use assert_cmd::Command;
use common::Sandbox;
use predicates::prelude::*;

/// Every key of `KEY_TABLE` with a value that passes validation — the table is the source of truth
/// for the *set* of keys, so this list is checked against it below.
const PROBES: [(&str, &str); 23] = [
    ("schema_version", "1"),
    ("defaults.provider", "smhi"),
    ("defaults.format", "json"),
    ("defaults.units", "us"),
    ("defaults.days", "7"),
    ("defaults.language", "zh-CN"),
    ("location.default", "@39.9,116.4"),
    ("units.temp", "f"),
    ("units.wind", "mph"),
    ("units.pressure", "inhg"),
    ("units.distance", "mi"),
    ("units.precip", "in"),
    ("network.timeout_secs", "30"),
    ("network.retries", "5"),
    ("network.proxy", "http://127.0.0.1:8080"),
    ("cache.enabled", "false"),
    ("cache.weather_ttl_secs", "900"),
    ("cache.ip_ttl_secs", "60"),
    ("cache.geocode_ttl_secs", "60"),
    ("render.color", "never"),
    ("render.width", "100"),
    ("providers.metar.station", "ZBAA"),
    ("providers.qweather.host", "https://api.qweather.com"),
];

fn get(sandbox: &Sandbox, key: &str) -> Command {
    let mut command = sandbox.cirrocast();
    command.args(["config", "get", key]);
    command
}

#[test]
fn config_path_points_at_the_file_and_creates_nothing() {
    let sandbox = Sandbox::new();

    sandbox
        .cirrocast()
        .args(["config", "path"])
        .assert()
        .success()
        .stdout(predicate::eq(format!(
            "{}\n",
            sandbox.config_file().display()
        )));

    assert!(
        !sandbox.config_dir().exists(),
        "`config path` must not create the directory"
    );
    assert!(!sandbox.config_file().exists());
}

#[test]
fn defaults_apply_when_no_file_exists() {
    let sandbox = Sandbox::new();

    for (key, expected) in [
        ("defaults.provider", "open-meteo"),
        ("defaults.format", "art-table"),
        ("defaults.units", "metric"),
        ("defaults.days", "3"),
        ("defaults.language", "auto"),
        ("units.temp", ""),
        ("network.timeout_secs", "15"),
        ("render.color", "auto"),
        ("cache.enabled", "true"),
    ] {
        get(&sandbox, key)
            .assert()
            .success()
            .stdout(predicate::eq(format!("{expected}\n")));
    }

    sandbox
        .cirrocast()
        .args(["config", "validate"])
        .assert()
        .success()
        .stdout(predicate::eq(format!(
            "ok: {}\n",
            sandbox.config_file().display()
        )));
}

#[test]
fn a_malformed_file_reports_its_path_line_and_column() {
    let sandbox = Sandbox::new();
    sandbox.install_fixture("config/bad-syntax.toml");

    get(&sandbox, "defaults.days")
        .assert()
        .code(4)
        .stderr(predicate::str::contains("error:").and(predicate::str::contains("config.toml:5:")));
}

#[test]
fn a_future_schema_is_rejected_with_the_supported_version() {
    let sandbox = Sandbox::new();
    sandbox.install_fixture("config/future-schema.toml");

    sandbox
        .cirrocast()
        .args(["config", "validate"])
        .assert()
        .code(4)
        .stderr(predicate::str::contains(
            "config written by a newer cirrocast (schema_version 99, supported 1)",
        ));
}

#[test]
fn init_writes_the_default_document_once_and_refuses_to_clobber_it() {
    let sandbox = Sandbox::new();

    sandbox
        .cirrocast()
        .args(["config", "init"])
        .assert()
        .success()
        .stdout(predicate::str::contains("wrote"));
    assert!(sandbox.config_file().is_file());

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = fs::metadata(sandbox.config_file())
            .expect("the file exists")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o644, "config.toml is world readable by design");
    }

    sandbox
        .cirrocast()
        .args(["config", "init"])
        .assert()
        .code(4)
        .stderr(predicate::str::contains("exists; pass --force"));

    sandbox
        .cirrocast()
        .args(["config", "init", "--force"])
        .assert()
        .success();
    common::assert_no_temporary_files(&sandbox.config_dir());
}

#[test]
fn set_round_trips_through_get_and_writes_the_canonical_document() {
    let sandbox = Sandbox::new();

    sandbox
        .cirrocast()
        .args(["config", "set", "defaults.days", "5"])
        .assert()
        .success();
    get(&sandbox, "defaults.days")
        .assert()
        .success()
        .stdout(predicate::eq("5\n"));
    get(&sandbox, "units.temp")
        .assert()
        .success()
        .stdout(predicate::eq("\n"));

    let written: toml::Value =
        toml::from_str(&fs::read_to_string(sandbox.config_file()).expect("the file is readable"))
            .expect("the written document parses");
    let expected: toml::Value = toml::from_str(&common::fixture("config/expected-after-set.toml"))
        .expect("the fixture parses");
    assert_eq!(
        written, expected,
        "`config set` writes the canonical document"
    );
    common::assert_no_temporary_files(&sandbox.config_dir());
}

#[test]
fn every_key_round_trips_through_the_cli() {
    let mut known: Vec<&str> = cirrocast::config::KEY_TABLE
        .iter()
        .map(|spec| spec.name)
        .collect();
    known.sort_unstable();
    let mut probed: Vec<&str> = PROBES.iter().map(|(key, _)| *key).collect();
    probed.sort_unstable();
    assert_eq!(probed, known, "every KEY_TABLE row needs a probe value");

    let sandbox = Sandbox::new();
    for (key, value) in PROBES {
        sandbox
            .cirrocast()
            .args(["config", "set", key, value])
            .assert()
            .success();
        get(&sandbox, key)
            .assert()
            .success()
            .stdout(predicate::eq(format!("{value}\n")));
    }
}

#[test]
fn a_stored_value_can_be_cleared_again() {
    let sandbox = Sandbox::new();

    sandbox
        .cirrocast()
        .args(["config", "set", "units.temp", "f"])
        .assert()
        .success();
    get(&sandbox, "units.temp")
        .assert()
        .success()
        .stdout(predicate::eq("f\n"));

    sandbox
        .cirrocast()
        .args(["config", "set", "units.temp", ""])
        .assert()
        .success();
    get(&sandbox, "units.temp")
        .assert()
        .success()
        .stdout(predicate::eq("\n"));
}

#[test]
fn rejected_values_are_named_and_never_written() {
    let sandbox = Sandbox::new();

    sandbox
        .cirrocast()
        .args(["config", "set", "defaults.days", "99"])
        .assert()
        .code(4)
        .stderr(predicate::str::contains(
            "defaults.days: 99 is out of range 0..=14",
        ));
    sandbox
        .cirrocast()
        .args(["config", "set", "render.color", "sometimes"])
        .assert()
        .code(4)
        .stderr(predicate::str::contains(
            "render.color: `sometimes` is not one of auto, always, never",
        ));
    sandbox
        .cirrocast()
        .args(["config", "set", "defaults.pvd", "x"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains(
            "unknown config key `defaults.pvd`",
        ));
    get(&sandbox, "keys.openweathermap")
        .assert()
        .code(2)
        .stderr(predicate::str::contains("keys.openweathermap"));

    assert!(
        !sandbox.config_file().exists(),
        "a rejected `config set` must not write the file"
    );
}

#[test]
fn the_environment_overrides_the_file_for_get() {
    let sandbox = Sandbox::new();

    sandbox
        .cirrocast()
        .args(["config", "set", "defaults.days", "5"])
        .assert()
        .success();
    get(&sandbox, "defaults.days")
        .assert()
        .success()
        .stdout(predicate::eq("5\n"));

    sandbox
        .cirrocast()
        .env("CIRROCAST_DAYS", "7")
        .args(["config", "get", "defaults.days"])
        .assert()
        .success()
        .stdout(predicate::eq("7\n"));
}

#[test]
fn show_prints_the_effective_document() {
    let sandbox = Sandbox::new();
    sandbox
        .cirrocast()
        .args(["config", "set", "defaults.days", "5"])
        .assert()
        .success();

    let assert = sandbox
        .cirrocast()
        .args(["config", "show"])
        .assert()
        .success();
    let stdout =
        String::from_utf8(assert.get_output().stdout.clone()).expect("show prints UTF-8 TOML");
    let document: toml::Value = toml::from_str(&stdout).expect("show prints parsable TOML");

    assert_eq!(
        document
            .get("schema_version")
            .and_then(toml::Value::as_integer),
        Some(1)
    );
    assert_eq!(
        document
            .get("defaults")
            .and_then(|defaults| defaults.get("days"))
            .and_then(toml::Value::as_integer),
        Some(5)
    );
}

#[test]
fn edit_runs_the_editor_and_revalidates_the_result() {
    let sandbox = Sandbox::new();

    sandbox
        .cirrocast()
        .env("VISUAL", "true")
        .args(["config", "edit"])
        .assert()
        .success();
    assert!(
        sandbox.config_file().is_file(),
        "edit writes the default document first"
    );

    let script = sandbox.home().join("bad-editor.sh");
    fs::write(
        &script,
        "#!/bin/sh\nprintf '[defaults]\\ndays = 99\\n' > \"$1\"\n",
    )
    .expect("the editor script is written");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755))
            .expect("the editor script is executable");
    }

    sandbox
        .cirrocast()
        .env("VISUAL", &script)
        .args(["config", "edit"])
        .assert()
        .code(4)
        .stderr(predicate::str::contains(
            "defaults.days: 99 is out of range 0..=14",
        ));

    sandbox
        .cirrocast()
        .env("VISUAL", "/nonexistent-editor-for-cirrocast-tests")
        .args(["config", "edit"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("cannot run"));
}
