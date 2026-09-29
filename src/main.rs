// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Thin entry point: parse the command line, run it, turn failures into an exit code.

use std::error::Error as StdError;
use std::process::ExitCode;

use clap::Parser;

use cirrocast::cli::Cli;

fn main() -> ExitCode {
    let cli = Cli::parse();

    match cli.run() {
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
