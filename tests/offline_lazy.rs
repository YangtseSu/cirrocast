// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The bundled city table must cost nothing until a name is actually looked up.
//!
//! This lives in its own test binary on purpose: `index_loaded()` reports process-global state, so
//! a file that also runs searches would make the assertion depend on test scheduling. Here nothing
//! but `--version` and the clap command builder runs, which is exactly what `cirrocast --version`
//! does before exiting.

#![cfg(feature = "offline-geo")]

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use clap::CommandFactory as _;

use cirrocast::cli::Cli;

#[test]
fn version_never_materialises_the_table_and_stays_fast() {
    assert!(
        !cirrocast::geo::offline::index_loaded(),
        "the index was decoded before anything asked for a name"
    );

    // The library-level twin of `--version`: building the command is the first thing `main` does.
    let command = Cli::command();
    assert!(command.get_version().is_some());
    assert!(
        !cirrocast::geo::offline::index_loaded(),
        "building the clap command decoded the index"
    );

    // Five real process starts; the median is the number the step file pins (< 50 ms).
    let mut timings = Vec::new();
    for _ in 0..5 {
        let start = Instant::now();
        let status = Command::new(env!("CARGO_BIN_EXE_cirrocast"))
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .expect("the binary runs");
        assert!(status.success());
        timings.push(start.elapsed());
    }
    timings.sort_unstable();
    let median = timings[timings.len() / 2];
    assert!(
        median < Duration::from_millis(50),
        "median `--version` run took {median:?} (all: {timings:?})"
    );
}
