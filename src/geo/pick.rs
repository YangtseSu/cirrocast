// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The interactive half of location resolution: one numbered list, one line of input.
//!
//! When a name resolves to several places (step 20) the ranked list is printed and the user picks
//! one — on the terminal that is the answer to "which Beijing?", while a script, `--yes` or
//! `[location] pick = "never"` keeps the ranked winner. The picker is deliberately not a TUI:
//! [`Picker`] owns no terminal state, reads through an injected [`BufRead`] and writes through an
//! injected [`Write`], so it works over ssh, a serial console or a here-doc, and its tests need no
//! pseudo-terminal.

use std::io::{BufRead, Write};
use std::sync::{Mutex, MutexGuard, PoisonError};

use crate::error::{Error, Result};
use crate::geo::location_line;
use crate::model::Location;

/// How many invalid answers in a row the prompt tolerates before giving up.
const MAX_INVALID: usize = 3;

/// Serializes prompts across a multi-location run: the slots are resolved on worker threads, and
/// two prompts must never interleave on the one stdin/stderr the process owns.
static PROMPT: Mutex<()> = Mutex::new(());

/// The lock a caller holds while a prompt is on screen.
///
/// Poisoning is ignored on purpose: the guarded state is `()`, so a panicked prompt cannot leave
/// anything inconsistent behind, and the next prompt may proceed.
pub fn prompt_lock() -> MutexGuard<'static, ()> {
    PROMPT.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A numbered candidate list plus one line of input from `input`, written to `output`.
///
/// Both streams are borrowed: the picker is built per prompt (the CLI locks stdin and stderr for
/// the duration) and owns nothing.
pub struct Picker<'a> {
    input: &'a mut dyn BufRead,
    output: &'a mut dyn Write,
}

impl<'a> Picker<'a> {
    /// Builds a picker over the two streams it may touch.
    pub fn new(input: &'a mut dyn BufRead, output: &'a mut dyn Write) -> Self {
        Self { input, output }
    }

    /// Prints the ranked list, asks which candidate to use and returns the chosen one.
    ///
    /// The winner (`candidates[0]`) is marked `*` and is what an empty line selects; a number in
    /// `1..=candidates.len()` selects that row. `q`/`Q` gives up with [`Error::LocationNotFound`]
    /// (exit code 5), EOF reads as `q`, and three invalid answers in a row are [`Error::Usage`]
    /// (exit code 2) naming the accepted input. `query` is the text the user typed and is what the
    /// give-up message names.
    pub fn choose(&mut self, query: &str, candidates: &[Location]) -> Result<Location> {
        let Some(winner) = candidates.first().cloned() else {
            // The caller only builds a picker when there is a choice to make; an empty list here
            // is a wiring bug that still has to behave like "no selection".
            return Err(no_selection(query));
        };
        for (index, candidate) in candidates.iter().enumerate() {
            let marker = if index == 0 { '*' } else { ' ' };
            writeln!(
                self.output,
                "[{}] {marker} {}",
                index + 1,
                candidate_line(candidate)
            )?;
        }
        let accepted = accepted_input(candidates.len());
        let mut invalid = 0;
        loop {
            write!(
                self.output,
                "choose a location [1-{}, Enter=1, q=quit]: ",
                candidates.len()
            )?;
            self.output.flush()?;
            let mut answer = String::new();
            if self.input.read_line(&mut answer)? == 0 {
                // EOF is a reader that went away: the same give-up as `q`.
                return Err(no_selection(query));
            }
            let answer = answer.trim();
            if answer.is_empty() {
                return Ok(winner);
            }
            if answer == "q" || answer == "Q" {
                return Err(no_selection(query));
            }
            let index = answer
                .parse::<usize>()
                .ok()
                .filter(|index| (1..=candidates.len()).contains(index));
            if let Some(chosen) = index.and_then(|index| candidates.get(index - 1)) {
                return Ok(chosen.clone());
            }
            invalid += 1;
            if invalid >= MAX_INVALID {
                return Err(Error::Usage(format!(
                    "no valid selection for `{query}` after {MAX_INVALID} answers; {accepted}"
                )));
            }
            writeln!(self.output, "invalid input; {accepted}")?;
        }
    }
}

/// The give-up error, exit code 5 like every other "no place" answer.
fn no_selection(query: &str) -> Error {
    Error::LocationNotFound(format!("no location selected for {query}"))
}

/// What the prompt accepts, spelled once so the retry note and the give-up error cannot disagree.
fn accepted_input(candidates: usize) -> String {
    format!("enter a number 1..={candidates}, Enter for 1, or q to quit")
}

/// One candidate row: the shared location header plus the population that broke ties.
///
/// The count is written `pop. <n>` rather than the `location search --all` table's
/// `(population <n>)`: this list is read at a prompt, where the place is the line that matters.
fn candidate_line(location: &Location) -> String {
    let population = location
        .population
        .map(|population| format!(" (pop. {population})"))
        .unwrap_or_default();
    format!("{}{population}", location_line(location))
}
