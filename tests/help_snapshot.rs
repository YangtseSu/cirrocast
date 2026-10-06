// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The generated documents: the long `--help` (the canonical flag reference) and the man page.
//!
//! Both are compared byte for byte with the committed files, so a flag change cannot ship without
//! the two documents moving in the same commit. They are regenerated on purpose, never by the
//! normal test run:
//!
//! ```text
//! cargo test --test help_snapshot -- --ignored regenerate_help
//! cargo test --test help_snapshot -- --ignored regenerate_man
//! ```

mod common;

use std::path::{Path, PathBuf};

use common::Sandbox;

/// The committed long help, byte for byte.
const HELP_FILE: &str = "docs/reference/help-long.txt";

/// The committed man page, byte for byte.
const MAN_FILE: &str = "man/cirrocast.1";

/// A path inside the repository.
fn repository_file(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(name)
}

/// The bytes a `cirrocast` run writes to stdout, with `COLUMNS` removed: the help width must not
/// depend on the developer's terminal.
fn produced(args: &[&str]) -> Vec<u8> {
    let sandbox = Sandbox::new();
    let output = sandbox
        .cirrocast()
        .env_remove("COLUMNS")
        .args(args)
        .output()
        .expect("the binary runs");
    assert!(
        output.status.success(),
        "`cirrocast {}` failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

/// Reads a committed document, with the path in the failure message.
fn committed(name: &str) -> Vec<u8> {
    std::fs::read(repository_file(name))
        .unwrap_or_else(|error| panic!("{name} is readable: {error}"))
}

/// Compares a generated document with its committed copy.
fn assert_matches(name: &str, args: &[&str], regenerate: &str) {
    let produced = produced(args);
    let committed = committed(name);
    if produced != committed {
        let produced_lines = String::from_utf8_lossy(&produced).lines().count();
        let committed_lines = String::from_utf8_lossy(&committed).lines().count();
        panic!(
            "{name} is stale: the binary prints {produced_lines} lines, the file has \
             {committed_lines}. Regenerate it with `cargo test --test help_snapshot -- --ignored \
             {regenerate}` and review the diff."
        );
    }
}

#[test]
fn long_help_matches_the_committed_snapshot() {
    assert_matches(HELP_FILE, &["--help"], "regenerate_help");
}

#[test]
fn man_page_matches_the_committed_snapshot() {
    assert_matches(MAN_FILE, &["man"], "regenerate_man");
}

/// Regenerates `docs/reference/help-long.txt` from the binary.
///
/// Manual, like every snapshot update: the diff is the review, and CI never runs it.
#[test]
#[ignore = "regenerates docs/reference/help-long.txt; run with `cargo test --test help_snapshot -- --ignored regenerate_help`"]
fn regenerate_help() {
    let file = repository_file(HELP_FILE);
    std::fs::write(&file, produced(&["--help"])).expect("the snapshot is written");
    eprintln!("wrote {}", file.display());
}

/// Regenerates `man/cirrocast.1` from the binary, byte for byte.
#[test]
#[ignore = "regenerates man/cirrocast.1; run with `cargo test --test help_snapshot -- --ignored regenerate_man`"]
fn regenerate_man() {
    let file = repository_file(MAN_FILE);
    std::fs::write(&file, produced(&["man"])).expect("the man page is written");
    eprintln!("wrote {}", file.display());
}
