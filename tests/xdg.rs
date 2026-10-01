// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The XDG contract: the CLI writes nothing outside `$XDG_{CONFIG,CACHE,DATA}_HOME/cirrocast`.
//!
//! `HOME` is pointed at the same throwaway tree as the three XDG variables, so a stray write to
//! `~/.config`, `~/.cache` or `~/.local/share` cannot hide: the whole tree is walked afterwards
//! and every entry that exists must sit under one of the three `cirrocast` directories. Reads are
//! checked separately: a configuration in `XDG_CONFIG_DIRS` is honoured when the user directory
//! has none.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use predicates::prelude::*;

use common::Sandbox;

/// Every path below `root`, directories and files alike, as a relative path.
fn collect(root: &Path, prefix: &Path, into: &mut Vec<PathBuf>) {
    let entries = fs::read_dir(root).expect("the tree is readable");
    for entry in entries {
        let path = entry.expect("a directory entry").path();
        let relative = path
            .strip_prefix(prefix)
            .expect("every entry is below the root")
            .to_path_buf();
        if path.is_dir() {
            collect(&path, prefix, into);
        }
        into.push(relative);
    }
}

/// The directories that may exist: the three XDG bases, the harness's empty `system/` sibling that
/// `XDG_CONFIG_DIRS` points at, and anything inside the `cirrocast/` directory of each.
const ALLOWED: [&str; 4] = ["config", "cache", "data", "system"];

/// Whether a relative path is inside one of the allowed trees.
fn allowed(relative: &Path) -> bool {
    let text = relative.to_string_lossy();
    if ALLOWED.contains(&text.as_ref()) {
        return true;
    }
    ["config", "cache", "data"].iter().any(|base| {
        let root = format!("{base}/cirrocast");
        text == root || text.starts_with(&format!("{root}/"))
    })
}

#[test]
fn every_write_stays_inside_the_xdg_tree() {
    let sandbox = Sandbox::new();

    // A run that writes both documents: the config file and the key store.
    sandbox
        .cirrocast()
        .args(["config", "init"])
        .assert()
        .success();
    sandbox
        .cirrocast()
        .args(["key", "set", "openweathermap"])
        .write_stdin("fake-key-for-the-xdg-audit")
        .assert()
        .success();

    // Runs that would write a cache entry if the guard let them through.
    sandbox
        .cirrocast()
        .args(["@39.9,116.4", "-f", "plain"])
        .env("CIRROCAST_FORBID_NETWORK", "1")
        .assert()
        .code(3);
    sandbox
        .cirrocast()
        .args(["location", "search", "@39.9,116.4"])
        .assert()
        .success();

    let mut paths = Vec::new();
    collect(sandbox.home(), sandbox.home(), &mut paths);
    for relative in &paths {
        assert!(
            allowed(relative),
            "the run created {} outside the cirrocast directories",
            relative.display()
        );
    }
    assert!(
        paths
            .iter()
            .any(|path| path.ends_with("config/cirrocast/config.toml")),
        "the config file was not created at all"
    );
}

#[test]
fn the_system_config_directory_is_honoured_for_reads() {
    let sandbox = Sandbox::new();
    let system = sandbox.home().join("system/cirrocast/config.toml");
    fs::create_dir_all(system.parent().expect("the system directory"))
        .expect("the system tree is created");
    fs::write(&system, "schema_version = 1\n[defaults]\ndays = 7\n")
        .expect("the system config is written");

    sandbox
        .cirrocast()
        .args(["config", "get", "defaults.days"])
        .assert()
        .success()
        .stdout(predicate::eq("7\n"));

    // And the user file still wins when it exists.
    sandbox.write_config("schema_version = 1\n[defaults]\ndays = 2\n");
    sandbox
        .cirrocast()
        .args(["config", "get", "defaults.days"])
        .assert()
        .success()
        .stdout(predicate::eq("2\n"));
}

#[test]
fn offline_and_no_cache_create_no_cache_directory() {
    for flag in ["--offline", "--no-cache"] {
        let sandbox = Sandbox::new();
        sandbox
            .cirrocast()
            .args([flag, "@39.9,116.4", "-f", "plain"])
            .env("CIRROCAST_FORBID_NETWORK", "1")
            .assert()
            .code(3);
        assert!(
            !sandbox.cache_dir().exists(),
            "{flag} created {}",
            sandbox.cache_dir().display()
        );
    }
}
