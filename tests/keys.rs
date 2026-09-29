// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! End to end tests of `cirrocast key …`: storage, mode discipline and masking.

mod common;

use std::fs;

use common::Sandbox;
use predicates::prelude::*;

/// A key long enough to be masked as `abcd…yz`.
const SECRET: &str = "sk-test-abcdef123456";

#[test]
fn key_set_reads_the_secret_from_stdin_and_stores_it_masked() {
    let sandbox = Sandbox::new();

    sandbox
        .cirrocast()
        .args(["key", "set", "openweathermap"])
        .write_stdin(format!("{SECRET}\n"))
        .assert()
        .success();

    let stored = fs::read_to_string(sandbox.keys_file()).expect("keys.toml is readable");
    assert!(stored.contains(SECRET), "the secret is stored verbatim");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = fs::metadata(sandbox.keys_file())
            .expect("the file exists")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "keys.toml must be owner-only");
    }

    let assert = sandbox.cirrocast().args(["key", "list"]).assert().success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("UTF-8 output");
    assert!(stdout.contains("openweathermap"), "{stdout}");
    assert!(stdout.contains("sk-t…56"), "{stdout}");
    assert!(stdout.contains("(file)"), "{stdout}");
    assert!(
        !stdout.contains(SECRET),
        "the full secret must never be printed:\n{stdout}"
    );
    common::assert_no_temporary_files(&sandbox.config_dir());
}

#[test]
fn an_environment_key_wins_over_the_file_and_is_marked() {
    let sandbox = Sandbox::new();
    sandbox
        .cirrocast()
        .args(["key", "set", "openweathermap"])
        .write_stdin("sk-file-abcdef123456\n")
        .assert()
        .success();

    let assert = sandbox
        .cirrocast()
        .env("CIRROCAST_OPENWEATHERMAP_KEY", "sk-env-abcdef123456")
        .args(["key", "list"])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("UTF-8 output");
    assert!(stdout.contains("sk-e…56"), "{stdout}");
    assert!(stdout.contains("(env)"), "{stdout}");
}

#[test]
fn a_group_or_world_readable_key_file_is_refused() {
    let sandbox = Sandbox::new();
    sandbox
        .cirrocast()
        .args(["key", "set", "qweather"])
        .write_stdin("a-secret-value\n")
        .assert()
        .success();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(sandbox.keys_file(), fs::Permissions::from_mode(0o644))
            .expect("chmod succeeds");

        sandbox
            .cirrocast()
            .args(["key", "list"])
            .assert()
            .code(4)
            .stderr(
                predicate::str::contains("mode 0644")
                    .and(predicate::str::contains("chmod 600"))
                    .and(predicate::str::contains("keys.toml")),
            );

        sandbox
            .cirrocast()
            .args(["key", "set", "qweather"])
            .write_stdin("another-secret\n")
            .assert()
            .code(4)
            .stderr(predicate::str::contains("chmod 600"));
    }
}

#[test]
fn key_rm_removes_the_stored_key() {
    let sandbox = Sandbox::new();
    sandbox
        .cirrocast()
        .args(["key", "set", "qweather"])
        .write_stdin("a-secret-value\n")
        .assert()
        .success();

    sandbox
        .cirrocast()
        .args(["key", "rm", "qweather"])
        .assert()
        .success()
        .stdout(predicate::str::contains("removed qweather API key"));

    let assert = sandbox.cirrocast().args(["key", "list"]).assert().success();
    assert!(
        assert.get_output().stdout.is_empty(),
        "no keys are configured any more"
    );

    sandbox
        .cirrocast()
        .args(["key", "rm", "qweather"])
        .assert()
        .success()
        .stdout(predicate::str::contains("no qweather API key stored"));
}

#[test]
fn unknown_and_keyless_providers_are_usage_errors() {
    let sandbox = Sandbox::new();

    sandbox
        .cirrocast()
        .args(["key", "set", "nope"])
        .write_stdin("a-secret-value\n")
        .assert()
        .code(2)
        .stderr(predicate::str::contains("unknown provider `nope`"));

    sandbox
        .cirrocast()
        .args(["key", "set", "smhi"])
        .write_stdin("a-secret-value\n")
        .assert()
        .code(2)
        .stderr(predicate::str::contains(
            "provider `smhi` does not use an API key",
        ));

    sandbox
        .cirrocast()
        .args(["key", "rm", "nope"])
        .assert()
        .code(2);
}

#[test]
fn an_empty_secret_never_reaches_the_key_file() {
    let sandbox = Sandbox::new();

    sandbox
        .cirrocast()
        .args(["key", "set", "qweather"])
        .write_stdin("\n")
        .assert()
        .code(2)
        .stderr(predicate::str::contains("no API key given"));

    sandbox
        .cirrocast()
        .args(["key", "set", "qweather", "--stdin"])
        .write_stdin("   \n")
        .assert()
        .code(2);

    assert!(!sandbox.keys_file().exists(), "nothing was written");
}

#[test]
fn secrets_cannot_be_smuggled_through_config_set() {
    let sandbox = Sandbox::new();

    sandbox
        .cirrocast()
        .args(["config", "set", "keys.openweathermap", "secret"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("unknown config key"));
    assert!(!sandbox.config_file().exists());
}
