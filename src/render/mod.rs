// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The render layer: the only place a unit is ever converted.
//!
//! A renderer reads the canonical [`Report`] — metric/SI, WMO conditions, location-local times —
//! and turns it into the bytes a user sees. Nothing else in the crate formats a value for display,
//! which is why the cache never has to care which [`UnitSystem`] a user prefers.
//!
//! Step 06 introduces the trait and the shape of [`RenderContext`]; step 07 adds the art table, the
//! colour palette, the dumb format and the width/colour resolution, and step 08 the stable JSON
//! document. They extend this module rather than redefine it, so a renderer written today keeps
//! working.

pub mod plain;

use chrono::{DateTime, FixedOffset};
use chrono_tz::Tz;
use clap::ValueEnum;

use crate::error::{Error, Result};
use crate::model::Report;
use crate::model::units::ResolvedUnits;

/// Whether colour may be used.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ColorMode {
    /// Colour when stdout is a terminal that supports it.
    Auto,
    /// Colour even when piping.
    Always,
    /// Never colour.
    Never,
}

/// What the terminal itself supports.
///
/// Split from [`ColorMode`] because the two answer different questions: `ColorMode` is what the
/// user asked for, `TermCaps` what the terminal can do. `dumb` is `TERM=dumb` (or an unset
/// `TERM`), where box drawing and colour are not worth attempting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TermCaps {
    /// Whether stdout is a terminal.
    pub is_tty: bool,
    /// Whether the terminal is believed to support colour.
    pub color: bool,
    /// Whether the terminal is a dumb one.
    pub dumb: bool,
}

/// Everything a renderer may look at beyond the report itself.
///
/// `now` and `tz` are injected rather than read from the clock inside a renderer, so a snapshot
/// test cannot drift with the wall clock.
#[derive(Debug, Clone, Copy)]
pub struct RenderContext {
    /// The units to convert into.
    pub units: ResolvedUnits,
    /// Whether colour is permitted.
    pub color: ColorMode,
    /// Maximum line width in columns.
    pub width: usize,
    /// Terminal capabilities.
    pub term: TermCaps,
    /// The current instant, at the location's offset.
    pub now: DateTime<FixedOffset>,
    /// The location's time zone.
    pub tz: Tz,
}

/// A renderer turns one report into the text the CLI prints.
pub trait Renderer {
    /// Renders `report`. The returned text has no trailing newline; the caller decides how to
    /// terminate the last line.
    fn render(&self, report: &Report, ctx: &RenderContext) -> Result<String>;
}

/// The output formats.
///
/// Only `plain` exists so far; step 07 adds `art-table`, `one-line` and `dumb`, step 08 `json`. The
/// enum is a `clap::ValueEnum`, so `--format` rejects an unknown spelling with exit code 2 before
/// any work happens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Format {
    /// Box-free, colour-free, pipe-friendly lines.
    Plain,
}

impl Format {
    /// The format's command line spelling.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Plain => "plain",
        }
    }

    /// Parses a format name that did **not** come from the command line — `defaults.format` and
    /// `CIRROCAST_FORMAT`.
    ///
    /// Clap already validates `--format`, so this is the path a configuration file takes, and it
    /// is where the formats that are planned but not implemented get their own message instead of
    /// "unknown format".
    pub fn from_name(name: &str) -> Result<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "plain" => Ok(Self::Plain),
            "art-table" | "one-line" | "json" | "dumb" => Err(Error::Usage(format!(
                "format `{name}` is not implemented yet; `--format plain` is available"
            ))),
            other => Err(Error::Usage(format!(
                "unknown format `{other}`; known formats: art-table, one-line, plain, json, dumb"
            ))),
        }
    }

    /// The renderer for this format.
    pub fn renderer(self) -> Result<Box<dyn Renderer>> {
        match self {
            Self::Plain => Ok(Box::new(plain::Plain)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Format;

    #[test]
    fn the_config_path_reports_planned_formats_as_such() {
        assert_eq!(
            Format::from_name("plain").expect("plain works"),
            Format::Plain
        );
        assert_eq!(
            Format::from_name(" PLAIN ").expect("trimmed"),
            Format::Plain
        );

        let planned = Format::from_name("art-table").expect_err("art-table arrives in step 07");
        assert_eq!(planned.exit_code(), 2);
        assert!(planned.to_string().contains("not implemented yet"));

        let unknown = Format::from_name("yaml").expect_err("never a format");
        assert!(unknown.to_string().contains("unknown format `yaml`"));
    }
}
