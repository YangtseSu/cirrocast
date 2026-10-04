// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! `@NAME` location aliases (step 19): expansion (including nesting), the coordinate-first
//! disambiguation, cycles and the suggestion list.
//!
//! Every run goes through the real binary against a throwaway XDG sandbox. Name lookups resolve
//! from the bundled city table and coordinate aliases need no lookup at all, so nothing here
//! touches the network.

mod common;

use predicates::prelude::*;

use common::Sandbox;

/// A sandbox with the given `[locations]` table, plus an optional `location.default`.
fn sandbox_with(entries: &[(&str, &str)]) -> Sandbox {
    use std::fmt::Write as _;

    let sandbox = Sandbox::new();
    let mut document = String::from("schema_version = 2\n");
    if !entries.is_empty() {
        document.push_str("[locations]\n");
        for (name, value) in entries {
            let _ = writeln!(document, "{name} = \"{value}\"");
        }
    }
    sandbox.write_config(&document);
    sandbox
}

/// The single line `location search` prints for `argument`.
fn search(sandbox: &Sandbox, argument: &str) -> String {
    let assert = sandbox
        .cirrocast()
        .args(["location", "search", argument, "--offline"])
        .assert()
        .success();
    String::from_utf8(assert.get_output().stdout.clone()).expect("stdout is UTF-8")
}

#[test]
fn an_alias_expands_to_the_spec_it_names() {
    let sandbox = sandbox_with(&[
        ("home", "@39.9042,116.4074"),
        ("city", "Beijing"),
        ("exact", ":Beijing"),
        ("osm", "~Tsinghua"),
    ]);
    assert!(search(&sandbox, "@home").starts_with("39.9042, 116.4074 "));
    assert!(search(&sandbox, "@city").starts_with("Beijing, CN "));
    assert!(search(&sandbox, "@exact").starts_with("Beijing, CN "));
    // `~` searches need the network, which `--offline` blocks with the documented refusal; the
    // alias still *resolved* (exit 3 rather than 2).
    sandbox
        .cirrocast()
        .args(["location", "search", "@osm", "--offline"])
        .assert()
        .code(3);
}

#[test]
fn aliases_chain_through_each_other() {
    let sandbox = sandbox_with(&[
        ("home", "@39.9042,116.4074"),
        ("work", "@home"),
        ("office", "@work"),
    ]);
    for name in ["@home", "@work", "@office"] {
        assert!(
            search(&sandbox, name).starts_with("39.9042, 116.4074 "),
            "{name} expands through the chain"
        );
    }
}

#[test]
fn coordinates_win_over_an_alias_of_the_same_spelling() {
    // A `[locations]` key may literally be `39.9042,116.4074` (quoted, since a bare TOML key
    // cannot hold dots or commas); the coordinate form still wins, because the spec is recognised
    // before the table is consulted.
    let sandbox = Sandbox::new();
    sandbox.write_config("schema_version = 2\n[locations]\n\"39.9042,116.4074\" = \"Beijing\"\n");
    assert!(search(&sandbox, "@39.9042,116.4074").starts_with("39.9042, 116.4074 "));
}

#[test]
fn a_cycle_is_a_config_error_naming_the_chain() {
    let sandbox = sandbox_with(&[("home", "@work"), ("work", "@home")]);
    sandbox
        .cirrocast()
        .args(["location", "search", "@home", "--offline"])
        .assert()
        .code(4)
        .stderr(
            predicate::str::contains("locations.home: location alias cycle")
                .and(predicate::str::contains("@home -> @work -> @home")),
        );
}

#[test]
fn an_unknown_alias_suggests_the_closest_names() {
    let sandbox = sandbox_with(&[("home", "@39.9,116.4"), ("work", ":Shanghai")]);
    sandbox
        .cirrocast()
        .args(["location", "search", "@hom", "--offline"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("did you mean @home?"));

    sandbox
        .cirrocast()
        .args(["location", "search", "@zzz", "--offline"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("known aliases: home, work"));

    // No table at all is its own message, and the accepted forms stay listed.
    let bare = Sandbox::new();
    bare.cirrocast()
        .args(["location", "search", "@home", "--offline"])
        .assert()
        .code(2)
        .stderr(
            predicate::str::contains("no `[locations]` aliases are configured").and(
                predicate::str::contains("@name (an alias from [locations])"),
            ),
        );
}

#[test]
fn a_weather_run_accepts_an_alias_as_its_location() {
    let sandbox = sandbox_with(&[("home", "@39.9042,116.4074")]);
    // The alias resolves to the coordinates; with no cached forecast the run fails for the missing
    // cache entry (exit 3), which proves it got past location resolution.
    sandbox
        .cirrocast()
        .args(["@home", "--offline", "-f", "one-line"])
        .assert()
        .code(3)
        .stderr(predicate::str::contains("offline: no cached open-meteo"));
}
