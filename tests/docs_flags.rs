// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The flag reference cannot drift from the binary.
//!
//! Three checks: `docs/reference/flags.txt` is exactly the set of long flags the whole help
//! surface (the top-level page and every subcommand's, recursively) names, one per line sorted and
//! without duplicates; every `` `--flag` `` token a Markdown document under `docs/` or the README
//! spells exists in that set; and every flag in the file is documented somewhere. A flag added or
//! removed without touching the reference fails the first check, a typo or a stale mention fails
//! the second, and a flag nobody wrote down fails the third.
//!
//! Tokens that belong to a *different* tool the documents quote (cargo, the bench harness, lychee,
//! `man(1)`) are neither cirrocast flags nor drift; they are listed in [`FOREIGN`] with the tool
//! that owns them, so a new external tool in the prose is a deliberate line here rather than a
//! silent pass.

mod common;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use common::Sandbox;

/// The committed flag reference: one long flag per line, sorted.
const FLAGS_FILE: &str = "docs/reference/flags.txt";

/// Long flags the documentation quotes from other tools, `(flag, tool)`.
const FOREIGN: [(&str, &str); 7] = [
    ("--release", "cargo"),
    ("--locked", "cargo"),
    ("--path", "cargo install"),
    ("--no-default-features", "cargo test"),
    ("--warmup", "hyperfine"),
    ("--cold-ms", "scripts/bench/cold.sh"),
    ("--free-only", "scripts/bench/run.sh"),
];

/// A path inside the repository.
fn repository_file(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(name)
}

/// Runs one help page and returns its text, with `COLUMNS` removed so the width is fixed.
fn help_page(sandbox: &Sandbox, path: &[String]) -> String {
    let mut command = sandbox.cirrocast();
    command.env_remove("COLUMNS").args(path).arg("--help");
    let output = command.output().expect("the help page runs");
    assert!(
        output.status.success(),
        "`cirrocast {} --help` failed: {}",
        path.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("the help page is UTF-8")
}

/// The subcommands a help page lists, in listing order.
fn subcommands(help: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut listing = false;
    for line in help.lines() {
        if line == "Commands:" {
            listing = true;
            continue;
        }
        if !listing {
            continue;
        }
        let Some(entry) = line.strip_prefix("  ") else {
            break;
        };
        let Some((name, _)) = entry.split_once(char::is_whitespace) else {
            continue;
        };
        if name != "help"
            && name.starts_with(|character: char| character.is_ascii_lowercase())
            && name.chars().all(|character| {
                character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
            })
        {
            names.push(name.to_owned());
        }
    }
    names
}

/// Every `--long-flag` token a help page spells, at the top level or in the prose under a flag.
fn long_flags(help: &str) -> BTreeSet<String> {
    let mut flags = BTreeSet::new();
    let mut rest = help;
    while let Some(found) = rest.find("--") {
        let after = &rest[found + 2..];
        let name: String = after
            .chars()
            .take_while(|character| {
                character.is_ascii_lowercase() || character.is_ascii_digit() || *character == '-'
            })
            .collect();
        if name.starts_with(|character: char| character.is_ascii_lowercase()) {
            flags.insert(format!("--{}", name.trim_end_matches('-')));
        }
        rest = after;
    }
    flags
}

/// The whole help surface's flag set: the top-level page, then every subcommand page, recursively.
fn binary_flags(sandbox: &Sandbox) -> BTreeSet<String> {
    let mut flags = BTreeSet::new();
    let mut queue = vec![Vec::<String>::new()];
    let mut visited = BTreeSet::new();
    while let Some(path) = queue.pop() {
        if !visited.insert(path.clone()) {
            continue;
        }
        let help = help_page(sandbox, &path);
        flags.extend(long_flags(&help));
        for name in subcommands(&help) {
            let mut child = path.clone();
            child.push(name);
            queue.push(child);
        }
    }
    flags
}

/// Every Markdown file the documentation scan covers: `docs/*.md` and the README.
///
/// The plan set (`docs/plans/`) and the dated reviews (`docs/reviews/`) are deliberately out of
/// scope: they are development records, and a plan step may name a flag that has not shipped or a
/// flag of another tool without either being drift in a document a user acts on.
fn markdown_files() -> Vec<PathBuf> {
    let mut files = vec![repository_file("README.md")];
    let directory = repository_file("docs");
    let entries = match std::fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(error) => panic!("{} is readable: {error}", directory.display()),
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() && path.extension().is_some_and(|extension| extension == "md") {
            files.push(path);
        }
    }
    files
}

/// The `` `--flag` `` tokens a document spells; a token that joins several flags with `/`
/// (`` `--pick`/`--yes` ``) counts as each of them, and a token that carries a value
/// (`` `--limit N` ``, `` `--placeholder <TEXT>` ``) counts as the flag.
fn backticked_flags(text: &str) -> BTreeSet<String> {
    let mut flags = BTreeSet::new();
    let mut rest = text;
    while let Some(opening) = rest.find('`') {
        let after = &rest[opening + 1..];
        let Some(closing) = after.find('`') else {
            break;
        };
        for token in after[..closing].split('/') {
            let Some(name) = token.split_whitespace().next() else {
                continue;
            };
            if name.starts_with("--")
                && name[2..].chars().all(|character| {
                    character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
                })
            {
                flags.insert(name.to_owned());
            }
        }
        rest = &after[closing + 1..];
    }
    flags
}

/// The flags `docs/reference/flags.txt` lists, as written.
fn committed_flags() -> Vec<String> {
    let text = std::fs::read_to_string(repository_file(FLAGS_FILE))
        .unwrap_or_else(|error| panic!("{FLAGS_FILE} is readable: {error}"));
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect()
}

#[test]
fn flags_file_lists_exactly_the_binary_flags() {
    let sandbox = Sandbox::new();
    let binary = binary_flags(&sandbox);
    let committed: BTreeSet<String> = committed_flags().into_iter().collect();
    let missing: Vec<&String> = binary.difference(&committed).collect();
    let extra: Vec<&String> = committed.difference(&binary).collect();
    assert!(
        missing.is_empty() && extra.is_empty(),
        "{FLAGS_FILE} is stale: missing {missing:?}, no longer accepted {extra:?}"
    );
}

#[test]
fn flags_file_is_sorted_and_duplicate_free() {
    let flags = committed_flags();
    let unique: BTreeSet<&String> = flags.iter().collect();
    assert_eq!(flags.len(), unique.len(), "{FLAGS_FILE} lists a flag twice");
    let mut sorted = flags.clone();
    sorted.sort();
    assert_eq!(flags, sorted, "{FLAGS_FILE} is not sorted");
}

#[test]
fn documents_only_name_flags_the_binary_accepts() {
    let sandbox = Sandbox::new();
    let known = binary_flags(&sandbox);
    let foreign: BTreeSet<&str> = FOREIGN.iter().map(|(flag, _)| *flag).collect();
    let mut unknown: BTreeSet<String> = BTreeSet::new();
    let mut places: Vec<String> = Vec::new();
    for file in markdown_files() {
        let text = std::fs::read_to_string(&file)
            .unwrap_or_else(|error| panic!("{} is readable: {error}", file.display()));
        for flag in backticked_flags(&text) {
            if !known.contains(&flag) && !foreign.contains(flag.as_str()) {
                unknown.insert(flag.clone());
                let relative = file
                    .strip_prefix(repository_file(""))
                    .unwrap_or(&file)
                    .display()
                    .to_string();
                places.push(format!("{relative}: {flag}"));
            }
        }
    }
    assert!(
        unknown.is_empty(),
        "documentation names flags the binary does not accept: {places:?}; if a token belongs to \
         another tool, add it to FOREIGN"
    );
}

#[test]
fn every_flag_is_documented() {
    let flags = committed_flags();
    let mut documented: BTreeSet<String> = BTreeSet::new();
    for file in markdown_files() {
        let text = std::fs::read_to_string(&file)
            .unwrap_or_else(|error| panic!("{} is readable: {error}", file.display()));
        documented.extend(backticked_flags(&text));
    }
    let undocumented: Vec<&String> = flags
        .iter()
        .filter(|flag| !documented.contains(*flag))
        .collect();
    assert!(
        undocumented.is_empty(),
        "these flags appear in no Markdown document: {undocumented:?}"
    );
}
