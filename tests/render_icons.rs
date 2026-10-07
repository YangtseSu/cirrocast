// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The `--icons` chain, end to end: the flag/environment/file tiers, the per-glyph resolution the
//! binary really renders, the ASCII forcing, and the two contracts that must not move — `%x` stays
//! 7-bit and `plain`/`json` carry no art at all.
//!
//! Everything runs the real binary against a throwaway XDG sandbox whose weather cache is seeded
//! with a recorded response, so no test touches the network.

mod common;

use assert_cmd::Command;
use common::Sandbox;
use predicates::prelude::*;
use unicode_width::UnicodeWidthStr;

/// The coordinates the seeded cache entries are keyed by.
const LAT: f64 = 39.9042;
const LON: f64 = 116.4074;

/// The location argument those coordinates spell.
const LOCATION: &str = "@39.9042,116.4074";

/// A sandbox whose weather cache holds the three-day fixture for [`LOCATION`].
fn seeded() -> Sandbox {
    let sandbox = Sandbox::new();
    let body = std::fs::read_to_string(common::fixture_path(
        "open_meteo/forecast_beijing_2026-07-15.json",
    ))
    .expect("the fixture is readable");
    common::seed_weather(
        &sandbox,
        "open-meteo",
        LAT,
        LON,
        3,
        chrono_tz::Tz::UTC,
        &body,
    );
    sandbox
}

/// A weather run against the seeded cache, under a terminal that can draw everything.
fn run(sandbox: &Sandbox, args: &[&str]) -> Command {
    let mut command = sandbox.cirrocast();
    command
        .env("TERM", "xterm-256color")
        .args(args)
        .arg(LOCATION)
        .arg("--offline");
    command
}

/// The stdout of a run that must succeed.
fn stdout(sandbox: &Sandbox, args: &[&str]) -> String {
    let output = run(sandbox, args).output().expect("the binary runs");
    assert!(
        output.status.success(),
        "`cirrocast {}` failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("the output is UTF-8")
}

/// Whether the text draws at least one glyph of the emoji set: the symbols and pictographs block,
/// the supplementary pictographs, and the two additions the corpus uses.
fn has_emoji(text: &str) -> bool {
    text.chars().any(|character| {
        matches!(
            character,
            '\u{2600}'..='\u{27bf}' | '\u{1f300}'..='\u{1f5ff}' | '\u{1f900}'..='\u{1f9ff}'
        )
    })
}

/// Whether the text draws at least one Weather Icons glyph (the Nerd Fonts private-use block).
fn has_nerd(text: &str) -> bool {
    text.chars()
        .any(|character| ('\u{e300}'..='\u{e3e3}').contains(&character))
}

#[test]
fn the_flag_the_environment_and_the_file_resolve_in_that_order() {
    // The file tier: `[render] icons`.
    let sandbox = seeded();
    sandbox.write_config("[render]\nicons = \"emoji\"\n");
    assert!(has_emoji(&stdout(&sandbox, &[])));

    // The environment beats the file.
    let mut command = run(&sandbox, &[]);
    command.env("CIRROCAST_ICONS", "nerd");
    let output = command.output().expect("the binary runs");
    let text = String::from_utf8(output.stdout).expect("UTF-8");
    assert!(has_nerd(&text) && !has_emoji(&text), "the environment wins");

    // The flag beats the environment.
    let mut command = run(&sandbox, &["--icons", "emoji"]);
    command.env("CIRROCAST_ICONS", "nerd");
    let output = command.output().expect("the binary runs");
    let text = String::from_utf8(output.stdout).expect("UTF-8");
    assert!(has_emoji(&text) && !has_nerd(&text), "the flag wins");

    // And with nothing configured anywhere, the default stays the hand-drawn blocks: no icon
    // glyph at all, and the sun the blocks draw in the current cell.
    let sandbox = seeded();
    let blocks = stdout(&sandbox, &[]);
    assert!(!has_emoji(&blocks) && !has_nerd(&blocks));
    assert!(
        blocks.contains('\u{2502}'),
        "the blocks still draw their box: {blocks}"
    );
}

#[test]
fn an_unusable_chain_is_a_usage_error_naming_the_three_sets() {
    let sandbox = seeded();
    for flag in ["wat", "emoji,", "", "nerd,,emoji"] {
        let assert = run(&sandbox, &["--icons", flag]).assert().failure();
        let output = assert.get_output().clone();
        assert_eq!(output.status.code(), Some(2), "--icons {flag:?}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("blocks, emoji, nerd"),
            "--icons {flag:?}: {stderr}"
        );
    }

    let mut command = run(&sandbox, &[]);
    command.env("CIRROCAST_ICONS", "wat");
    let output = command.output().expect("the binary runs");
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("`wat` is not an icon set"), "{stderr}");

    // A chain in the file is validated like the flag, and fails as a configuration error — even
    // when the flag would have overridden it, because the document is loaded first.
    sandbox.write_config("[render]\nicons = \"wat\"\n");
    let output = run(&sandbox, &["--icons", "emoji"])
        .output()
        .expect("the binary runs");
    assert_eq!(output.status.code(), Some(4));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("render.icons"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn an_ascii_run_draws_the_blocks_whatever_the_chain_says() {
    let sandbox = seeded();
    // `--format dumb` is ASCII by construction; `TERM=dumb` asks for it through the terminal.
    for (args, dumb_terminal) in [
        (vec!["--format", "dumb", "--icons", "emoji"], false),
        (vec!["--icons", "nerd"], true),
        (vec!["--icons", "emoji,nerd"], true),
    ] {
        let mut command = run(&sandbox, &args);
        if dumb_terminal {
            command.env("TERM", "dumb");
        }
        let output = command.output().expect("the binary runs");
        let text = String::from_utf8(output.stdout).expect("UTF-8");
        assert!(
            text.is_ascii(),
            "{args:?} must not draw an icon set in ASCII: {}",
            text.lines().nth(3).unwrap_or("")
        );

        // `-v` says so once, next to the existing charset note.
        let mut command = run(&sandbox, &args);
        command.arg("-v");
        if dumb_terminal {
            command.env("TERM", "dumb");
        }
        let output = command.output().expect("the binary runs");
        let notes = String::from_utf8_lossy(&output.stderr);
        assert!(
            notes.contains("the icon sets need a UTF-8 terminal; drawing the blocks"),
            "{args:?}: {notes}"
        );
    }
}

#[test]
fn percent_c_follows_the_chain_and_percent_x_stays_seven_bit() {
    let sandbox = seeded();
    let emoji = stdout(
        &sandbox,
        &[
            "--icons",
            "emoji",
            "-f",
            "one-line",
            "--template",
            "%l %c %t",
        ],
    );
    assert!(has_emoji(&emoji), "`%c` draws the emoji: {emoji}");

    let plain = stdout(
        &sandbox,
        &["--icons", "emoji", "-f", "one-line", "--template", "%x"],
    );
    assert!(plain.trim().is_ascii(), "`%x` stays 7-bit: {plain}");

    // `%x` is exactly what the blocks' `%c` prints, whatever the chain says.
    let blocks = stdout(&sandbox, &["-f", "one-line", "--template", "%c"]);
    assert_eq!(plain, blocks);
}

#[test]
fn the_nerd_corpus_prints_the_weather_icons_codepoints() {
    let sandbox = seeded();
    let text = stdout(&sandbox, &["--icons", "nerd"]);
    let used: Vec<char> = text
        .chars()
        .filter(|character| ('\u{e000}'..='\u{f8ff}').contains(character))
        .collect();
    assert!(!used.is_empty(), "no private-use glyph in\n{text}");
    for character in used {
        assert!(
            ('\u{e300}'..='\u{e3e3}').contains(&character),
            "{character:?} leaves the Weather Icons block"
        );
    }
}

#[test]
fn the_table_keeps_its_borders_aligned_in_every_set() {
    let sandbox = seeded();
    for chain in ["blocks", "emoji", "nerd", "nerd,emoji"] {
        let text = stdout(&sandbox, &["--icons", chain]);
        // Every line that reaches a border is as wide as the first one: an icon set changes what a
        // cell contains, never the geometry of the table.
        let widths: Vec<usize> = text
            .lines()
            .filter(|line| line.starts_with(['\u{250c}', '\u{2502}', '\u{251c}', '\u{2514}']))
            .map(UnicodeWidthStr::width)
            .collect();
        assert!(widths.len() > 10, "{chain}: {} border lines", widths.len());
        let first = widths[0];
        assert!(
            widths.iter().all(|width| *width == first),
            "{chain}: border widths {widths:?}"
        );

        // The stacked narrow layout below 60 columns keeps every line inside the width.
        let narrow = stdout(&sandbox, &["--icons", chain, "--width", "40"]);
        assert!(
            narrow
                .lines()
                .all(|line| UnicodeWidthStr::width(line) <= 40),
            "{chain}: a stacked line is wider than 40 columns:\n{narrow}"
        );
    }
}

#[test]
fn the_moon_surfaces_follow_the_chain() {
    let sandbox = seeded();
    let emoji = stdout(&sandbox, &["--icons", "emoji", "-f", "moon"]);
    assert!(
        emoji
            .chars()
            .any(|character| ('\u{1f311}'..='\u{1f318}').contains(&character)),
        "the moon view draws an emoji phase: {emoji}"
    );
    let nerd = stdout(&sandbox, &["--icons", "nerd", "-f", "moon"]);
    assert!(
        nerd.chars()
            .any(|character| ('\u{e38d}'..='\u{e3a8}').contains(&character)),
        "the moon view draws a Weather Icons phase: {nerd}"
    );
    let blocks = stdout(&sandbox, &["-f", "moon"]);
    assert!(
        blocks.contains('\u{2588}') || blocks.contains('\u{2591}'),
        "the default view still draws the disc: {blocks}"
    );

    // `%m` follows the same chain.
    let percent_m = stdout(
        &sandbox,
        &["--icons", "emoji", "-f", "one-line", "--template", "%m"],
    );
    assert!(
        percent_m
            .chars()
            .any(|character| ('\u{1f311}'..='\u{1f318}').contains(&character)),
        "{percent_m}"
    );
}

/// The text without the decode-time stamps: each run stamps `retrieved_at` with its own instant,
/// so that one line is dropped before two runs are compared.
fn without_stamps(text: &str) -> String {
    text.lines()
        .filter(|line| !line.contains("retrieved_at"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn the_data_formats_carry_no_art() {
    let sandbox = seeded();
    for format in ["json", "plain"] {
        let blocks = without_stamps(&stdout(&sandbox, &["-f", format]));
        let emoji = without_stamps(&stdout(&sandbox, &["--icons", "emoji", "-f", format]));
        let nerd = without_stamps(&stdout(&sandbox, &["--icons", "nerd", "-f", format]));
        assert_eq!(blocks, emoji, "{format} must not change with --icons");
        assert_eq!(blocks, nerd, "{format} must not change with --icons");
    }
}

#[test]
fn the_config_key_round_trips_and_validates() {
    let sandbox = seeded();
    sandbox
        .cirrocast()
        .args(["config", "set", "render.icons", "nerd,emoji"])
        .assert()
        .success();
    sandbox
        .cirrocast()
        .args(["config", "get", "render.icons"])
        .assert()
        .success()
        .stdout(predicate::eq("nerd,emoji\n"));
    sandbox
        .cirrocast()
        .args(["config", "validate"])
        .assert()
        .success();
    // The configured chain is what a run then draws, Nerd Font first.
    assert!(has_nerd(&stdout(&sandbox, &[])));

    sandbox
        .cirrocast()
        .args(["config", "set", "render.icons", "wat"])
        .assert()
        .failure()
        .code(4)
        .stderr(predicate::str::contains("render.icons"));
}

#[test]
fn the_verbose_note_names_the_resolved_chain() {
    let sandbox = seeded();
    let output = run(&sandbox, &["--icons", "nerd,emoji", "-v"])
        .output()
        .expect("the binary runs");
    let notes = String::from_utf8_lossy(&output.stderr);
    assert!(
        notes.contains("icons: nerd,emoji,blocks"),
        "the note states the ladder: {notes}"
    );
}

#[test]
fn the_status_probe_honours_the_configured_chain() {
    // The probe has no `--icons` of its own — its flag surface is the template and the colour, and
    // its settings come from the file like every other render setting there — but `%c` follows the
    // chain the file names, so the probe and a query cannot disagree about what `%c` draws.
    let sandbox = seeded();
    sandbox.write_config("[render]\nicons = \"emoji\"\n");
    let output = sandbox
        .cirrocast()
        .env("TERM", "xterm-256color")
        .args([
            "status",
            "--location",
            LOCATION,
            "--format",
            "%c",
            "--offline",
            "-q",
        ])
        .output()
        .expect("the binary runs");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).expect("UTF-8");
    assert!(has_emoji(&text), "the probe draws the emoji: {text:?}");
}
