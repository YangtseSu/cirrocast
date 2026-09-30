// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Thin entry point: parse the command line, run it, turn failures into an exit code.
//!
//! The parse happens in two steps on purpose: [`clap::Command::get_matches`] keeps the
//! [`clap::ArgMatches`] around, and the provenance of every setting is read from it before the
//! typed [`Cli`] value is built. Without that, an environment value and a command line value are
//! indistinguishable by the time the run starts, and the precedence rules could only be guessed.

use std::error::Error as StdError;
use std::process::ExitCode;

use clap::{CommandFactory as _, FromArgMatches as _};

use cirrocast::cli::{Cli, Sources};

fn main() -> ExitCode {
    let matches = Cli::command().get_matches();
    let sources = Sources::read(&matches);
    let cli = Cli::from_arg_matches(&matches).unwrap_or_else(|error| error.exit());

    match cli.run(&sources) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            if cli.verbose > 0 {
                print_causes(error.source());
            }
            ExitCode::from(error.exit_code())
        }
    }
}

/// Walks the error cause chain, one `caused by:` line per level. Errors that carry no source
/// (every `String` payload variant) print nothing.
fn print_causes(mut source: Option<&(dyn StdError + 'static)>) {
    while let Some(cause) = source {
        eprintln!("caused by: {cause}");
        source = cause.source();
    }
}
