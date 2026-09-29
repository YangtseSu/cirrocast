// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Shared helpers for the CLI integration tests: a throwaway XDG sandbox.

// Each test binary uses a different subset of these helpers.
#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};

use assert_cmd::Command;

/// Every `CIRROCAST_*` override variable, cleared for the child process so that the developer's
/// shell cannot influence a test.
const OVERRIDE_VARS: [&str; 7] = [
    "CIRROCAST_PROVIDER",
    "CIRROCAST_FORMAT",
    "CIRROCAST_UNITS",
    "CIRROCAST_DAYS",
    "CIRROCAST_LANG",
    "CIRROCAST_LOCATION",
    "CIRROCAST_TIMEOUT",
];

/// A throwaway XDG environment: config, cache and data all point into a temporary directory, and
/// the system wide config path points at an empty sibling.
pub struct Sandbox {
    home: tempfile::TempDir,
}

impl Sandbox {
    /// Creates an empty sandbox.
    pub fn new() -> Self {
        let home = tempfile::tempdir().expect("a temporary directory");
        fs::create_dir_all(home.path().join("system")).expect("the system config directory");
        Self { home }
    }

    /// The sandbox root.
    pub fn home(&self) -> &Path {
        self.home.path()
    }

    /// `$XDG_CONFIG_HOME/cirrocast`.
    pub fn config_dir(&self) -> PathBuf {
        self.home.path().join("config/cirrocast")
    }

    /// The user configuration file.
    pub fn config_file(&self) -> PathBuf {
        self.config_dir().join("config.toml")
    }

    /// The API key file.
    pub fn keys_file(&self) -> PathBuf {
        self.config_dir().join("keys.toml")
    }

    /// A `cirrocast` invocation wired to the sandbox and to a clean environment.
    pub fn cirrocast(&self) -> Command {
        let mut command = Command::cargo_bin("cirrocast").expect("the binary is built by cargo");
        command
            .env("XDG_CONFIG_HOME", self.home.path().join("config"))
            .env("XDG_CONFIG_DIRS", self.home.path().join("system"))
            .env("XDG_CACHE_HOME", self.home.path().join("cache"))
            .env("XDG_DATA_HOME", self.home.path().join("data"));
        for name in OVERRIDE_VARS {
            command.env_remove(name);
        }
        command
    }

    /// Writes `text` as the user configuration file.
    pub fn write_config(&self, text: &str) {
        fs::create_dir_all(self.config_dir()).expect("the config directory");
        fs::write(self.config_file(), text).expect("the config file is written");
    }

    /// Copies `tests/fixtures/config/<name>` into place as `config.toml`.
    pub fn install_fixture(&self, name: &str) {
        self.write_config(&fixture(name));
    }
}

/// The contents of `tests/fixtures/<name>`.
pub fn fixture(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

/// Asserts that `directory` holds no leftover `.<file>.tmp.<pid>` from the atomic writer.
pub fn assert_no_temporary_files(directory: &Path) {
    let leftovers: Vec<String> = fs::read_dir(directory)
        .expect("the directory is readable")
        .map(|entry| {
            entry
                .expect("a directory entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .filter(|name| name.contains(".tmp."))
        .collect();
    assert!(leftovers.is_empty(), "temporary files left: {leftovers:?}");
}
