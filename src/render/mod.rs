// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The render layer: the only place a unit is ever converted.
//!
//! A renderer reads the canonical [`Report`] — metric/SI, WMO conditions, location-local times —
//! and turns it into the bytes a user sees. Nothing else in the crate formats a value for display,
//! which is why the cache never has to care which [`UnitSystem`](crate::model::units::UnitSystem) a
//! user prefers.
//!
//! This module also owns the three questions a renderer must not answer for itself, because
//! answering them twice is how output becomes untestable:
//!
//! * **Width** — [`resolve_width`] picks `--width`, then `COLUMNS`, then the terminal's own size,
//!   then 80 columns, and never returns less than [`MIN_WIDTH`] columns.
//! * **Colour** — [`resolve_color`] turns the requested [`ColorMode`] plus `NO_COLOR`,
//!   `CLICOLOR_FORCE`, `TERM` and tty-ness into the mode the run really uses, and
//!   [`effective_depth`] decides which palette may be emitted.
//! * **Charset** — [`Charset`] is what splits `art-table` from `dumb`: the same renderer, drawn
//!   with box-drawing characters or with ASCII.
//!
//! Reading the environment happens exactly once, in [`TermCaps::detect`], and the rules behind it
//! are pure functions of the values it read ([`TermCaps::read`], [`resolve_color`]) — edition 2024
//! makes `std::env::set_var` unsafe, so an injectable seam is the only way those rules stay under
//! test.

pub mod art;
pub mod art_table;
pub mod color;
pub mod json;
pub mod one_line;
pub mod plain;

use std::io::IsTerminal as _;

use chrono::{DateTime, FixedOffset};
use chrono_tz::Tz;
use clap::ValueEnum;

use crate::error::{Error, Result};
use crate::i18n::{I18n, LanguageId};
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

impl ColorMode {
    /// The config/flag spelling, e.g. `auto`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Always => "always",
            Self::Never => "never",
        }
    }
}

/// The kind of terminal behind stdout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TermKind {
    /// A terminal worth drawing for: box-drawing characters and escapes are fair game.
    #[default]
    Standard,
    /// `TERM` is `dumb` or unset: the layout falls back to ASCII and colour is off.
    Dumb,
}

/// How many colours the terminal understands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColorDepth {
    /// No colour at all.
    #[default]
    Mono,
    /// The ANSI palette: the 256-colour palette is folded onto it by [`color::ansi16_from_256`].
    Ansi16,
    /// The xterm 256-colour palette the palette in [`color`] is authored for.
    Ansi256,
}

/// The character set a report is drawn in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Charset {
    /// Box drawing, arrows and the unicode art blocks.
    #[default]
    Unicode,
    /// Pure 7-bit ASCII, for `--format dumb` and for terminals that cannot do better.
    Ascii,
}

/// What the environment said about colour, read once per run.
///
/// The two variables interact rather than coexist: `CLICOLOR_FORCE` set to anything but `0` is a
/// request for colour that outranks the `NO_COLOR` convention, so a single field with three states
/// is the honest representation — and keeps [`TermCaps`] a description of the terminal rather than
/// a bag of environment flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColorPreference {
    /// Neither variable is set.
    #[default]
    Neutral,
    /// `NO_COLOR` is present, with any value — an empty one included.
    NoColor,
    /// `CLICOLOR_FORCE` is set to something other than `0`.
    Force,
}

/// What the terminal itself supports, read once per run.
///
/// `is_tty` is about *this* stdout, the rest about the terminal behind it. [`ColorPreference`]
/// lives here because it is read from the same environment at the same moment and is only ever
/// consumed together with the capabilities, by [`resolve_color`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TermCaps {
    /// Whether stdout is a terminal.
    pub is_tty: bool,
    /// Whether the terminal is a dumb one.
    pub term: TermKind,
    /// Whether the ambient locale is a UTF-8 one.
    pub utf8: bool,
    /// How many colours the terminal understands.
    pub depth: ColorDepth,
    /// What `NO_COLOR` and `CLICOLOR_FORCE` said.
    pub color_pref: ColorPreference,
}

impl Default for TermCaps {
    fn default() -> Self {
        Self::read(|_| None, false)
    }
}

impl TermCaps {
    /// Reads stdout and the environment.
    #[must_use]
    pub fn detect() -> Self {
        Self::read(
            |name| std::env::var(name).ok(),
            std::io::stdout().is_terminal(),
        )
    }

    /// The detection rules, with the environment injected.
    ///
    /// * `TERM` unset or `dumb` (or `TERM=unknown`… any value that names no terminal) is
    ///   [`TermKind::Dumb`];
    /// * the locale is the first non-empty of `LC_ALL`, `LC_CTYPE`, `LANG`, and it is UTF-8 only
    ///   when it spells `utf-8` or `utf8` (case-insensitively);
    /// * the depth is [`ColorDepth::Ansi256`] when `TERM` says `256color` or `COLORTERM` says
    ///   `truecolor`/`24bit`, [`ColorDepth::Mono`] for a dumb terminal and [`ColorDepth::Ansi16`]
    ///   otherwise. 24-bit colour is deliberately *not* implemented in v1, so a truecolor terminal
    ///   is served the deepest palette there is.
    #[must_use]
    pub fn read(var: impl Fn(&str) -> Option<String>, is_tty: bool) -> Self {
        let term_value = var("TERM").unwrap_or_default();
        let term = if term_value.is_empty() || term_value == "dumb" {
            TermKind::Dumb
        } else {
            TermKind::Standard
        };

        let locale = ["LC_ALL", "LC_CTYPE", "LANG"]
            .iter()
            .find_map(|name| var(name).filter(|value| !value.is_empty()))
            .unwrap_or_default();
        let utf8 = {
            let locale = locale.to_ascii_lowercase();
            locale.contains("utf-8") || locale.contains("utf8")
        };

        let colorterm = var("COLORTERM").unwrap_or_default().to_ascii_lowercase();
        let depth = if term == TermKind::Dumb {
            ColorDepth::Mono
        } else if term_value.contains("256color")
            || matches!(colorterm.as_str(), "truecolor" | "24bit")
        {
            ColorDepth::Ansi256
        } else {
            ColorDepth::Ansi16
        };

        let color_pref = if var("CLICOLOR_FORCE").is_some_and(|value| value != "0") {
            ColorPreference::Force
        } else if var("NO_COLOR").is_some() {
            ColorPreference::NoColor
        } else {
            ColorPreference::Neutral
        };

        Self {
            is_tty,
            term,
            utf8,
            depth,
            color_pref,
        }
    }

    /// The charset this terminal can be drawn in.
    #[must_use]
    pub const fn charset(self) -> Charset {
        if matches!(self.term, TermKind::Dumb) || !self.utf8 {
            Charset::Ascii
        } else {
            Charset::Unicode
        }
    }
}

/// The colour mode a run actually uses.
///
/// An explicit `always`/`never` is never second-guessed (a user who pipes `--color always` into
/// `cat -v` asked for escapes). `auto` is the documented ladder: `CLICOLOR_FORCE` enables, then
/// `NO_COLOR` disables, then a dumb `TERM` and a stdout that is not a terminal disable, else
/// colour is on. `CLICOLOR_FORCE` wins over `NO_COLOR` because it is the more specific request.
#[must_use]
pub fn resolve_color(mode: ColorMode, caps: &TermCaps) -> ColorMode {
    match mode {
        ColorMode::Always | ColorMode::Never => mode,
        ColorMode::Auto => {
            // The ladder in one expression: `CLICOLOR_FORCE` outranks everything, `NO_COLOR`
            // disables, and with neither set colour needs a stdout that is a terminal and a
            // terminal that is not a dumb one.
            let neutral = caps.color_pref == ColorPreference::Neutral;
            let terminal_can = caps.term != TermKind::Dumb && caps.is_tty;
            if caps.color_pref == ColorPreference::Force || (neutral && terminal_can) {
                ColorMode::Always
            } else {
                ColorMode::Never
            }
        }
    }
}

/// The palette a run may emit.
///
/// A resolved [`ColorMode::Never`] is [`ColorDepth::Mono`]. A request for colour on a terminal that
/// advertises none — `--color always` in a bare pipe, `CLICOLOR_FORCE=1` under `TERM=dumb` — gets
/// the deepest palette this build implements, because the request was "emit escapes", not "tell me
/// whether I meant it". An unresolved [`ColorMode::Auto`] counts as a request for colour;
/// [`resolve_color`] is what turns `auto` into a decision, and every context carries the resolved
/// mode.
#[must_use]
pub fn effective_depth(mode: ColorMode, caps: &TermCaps) -> ColorDepth {
    if mode == ColorMode::Never {
        ColorDepth::Mono
    } else if caps.depth == ColorDepth::Mono {
        ColorDepth::Ansi256
    } else {
        caps.depth
    }
}

/// The narrowest layout the table is drawn for; narrower sources are raised to it.
pub const MIN_WIDTH: usize = 20;

/// Where the layout width came from, for the `--verbose` note.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WidthSource {
    /// `--width` or `[render] width`.
    Explicit,
    /// The `COLUMNS` environment variable.
    Columns,
    /// The terminal's own window size.
    Terminal,
    /// The built-in 80 columns.
    Fallback,
}

impl WidthSource {
    /// The description `--verbose` prints.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Explicit => "the requested width",
            Self::Columns => "COLUMNS",
            Self::Terminal => "the terminal window",
            Self::Fallback => "the 80 column default",
        }
    }
}

/// The width a run lays out for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Width {
    /// The column count to lay out for, never below [`MIN_WIDTH`].
    pub columns: usize,
    /// Where the value came from.
    pub source: WidthSource,
    /// The value the source reported when it was raised to [`MIN_WIDTH`].
    pub raised_from: Option<usize>,
}

/// Resolves the layout width: explicit > `COLUMNS` > the terminal > 80.
///
/// An unparsable or zero source is skipped rather than treated as an error — a stray `COLUMNS=abc`
/// in a shell profile must not stop a forecast — and a width below [`MIN_WIDTH`] is raised to it,
/// which the caller reports under `--verbose`.
#[must_use]
pub fn resolve_width(explicit: Option<usize>) -> Width {
    resolve_width_from(
        explicit,
        std::env::var("COLUMNS").ok().as_deref(),
        terminal_width(),
    )
}

/// The width ladder with every source injected, which is what the unit tests drive.
fn resolve_width_from(
    explicit: Option<usize>,
    columns: Option<&str>,
    terminal: Option<usize>,
) -> Width {
    let chosen = explicit
        .filter(|columns| *columns > 0)
        .map(|columns| (columns, WidthSource::Explicit))
        .or_else(|| {
            columns
                .and_then(|value| value.trim().parse::<usize>().ok())
                .filter(|columns| *columns > 0)
                .map(|columns| (columns, WidthSource::Columns))
        })
        .or_else(|| {
            terminal
                .filter(|columns| *columns > 0)
                .map(|columns| (columns, WidthSource::Terminal))
        });

    let (columns, source) = chosen.unwrap_or((80, WidthSource::Fallback));
    if columns < MIN_WIDTH {
        Width {
            columns: MIN_WIDTH,
            source,
            raised_from: Some(columns),
        }
    } else {
        Width {
            columns,
            source,
            raised_from: None,
        }
    }
}

/// The terminal's own column count, when stdout is a terminal.
///
/// The window size comes from `tcgetwinsize` through `rustix` rather than from a hand-written
/// `ioctl`: the crate forbids `unsafe`, and `libc::ioctl` cannot be called without it. Targets
/// without `termios` (and every non-Unix target) fall through to `COLUMNS`, then to 80.
#[cfg(unix)]
fn terminal_width() -> Option<usize> {
    use std::os::fd::AsFd as _;

    let size = rustix::termios::tcgetwinsize(std::io::stdout().as_fd()).ok()?;
    Some(usize::from(size.ws_col))
}

/// No `termios` on this target: `COLUMNS` and the 80 column default are the whole ladder.
#[cfg(not(unix))]
fn terminal_width() -> Option<usize> {
    None
}

/// Everything a renderer may look at beyond the report itself.
///
/// `now` and `tz` are injected rather than read from the clock inside a renderer, so a snapshot
/// test cannot drift with the wall clock; `width` and `color` have already been resolved by
/// [`resolve_width`] and [`resolve_color`], so a renderer never reads the environment.
#[derive(Debug, Clone, Copy)]
pub struct RenderContext<'a> {
    /// The units to convert into.
    pub units: ResolvedUnits,
    /// The colour mode this run uses: never `Auto`.
    pub color: ColorMode,
    /// Maximum line width in columns.
    pub width: usize,
    /// Terminal capabilities.
    pub term: TermCaps,
    /// The current instant, at the location's offset.
    pub now: DateTime<FixedOffset>,
    /// The location's time zone.
    pub tz: Tz,
    /// The language the report is rendered in.
    pub lang: LanguageId,
    /// The message catalog behind every label a renderer prints.
    pub i18n: &'a I18n,
}

impl RenderContext<'_> {
    /// The palette depth the renderers may emit with.
    #[must_use]
    pub fn depth(&self) -> ColorDepth {
        effective_depth(self.color, &self.term)
    }
}

/// A renderer turns one report into the text the CLI prints.
pub trait Renderer {
    /// Renders `report`. The returned text has no trailing newline; the caller decides how to
    /// terminate the last line.
    fn render(&self, report: &Report, ctx: &RenderContext<'_>) -> Result<String>;
}

/// The output formats.
///
/// `art-table` is the default and the reason the crate exists; `dumb` is the same renderer with
/// the ASCII charset, so the layout is implemented once. `one-line` takes a `%`-token template,
/// `plain` is the greppable line-per-record form and `json` the stable machine-readable document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Format {
    /// The wttr.in-style coloured table (default).
    ArtTable,
    /// A single line driven by `%` tokens.
    OneLine,
    /// Box-free, pipe-friendly lines.
    Plain,
    /// The stable JSON document.
    Json,
    /// `art-table` in pure ASCII, with no colour.
    Dumb,
}

impl Format {
    /// Every format, in `--help` and documentation order.
    pub const ALL: [Self; 5] = [
        Self::ArtTable,
        Self::OneLine,
        Self::Plain,
        Self::Json,
        Self::Dumb,
    ];

    /// The format's command line spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ArtTable => "art-table",
            Self::OneLine => "one-line",
            Self::Plain => "plain",
            Self::Json => "json",
            Self::Dumb => "dumb",
        }
    }

    /// Parses a format name that did **not** come from the command line — `defaults.format` and
    /// `CIRROCAST_FORMAT`.
    ///
    /// Clap already validates `--format`; this is the path a configuration file takes, and it
    /// deliberately accepts every documented spelling, including the formats whose renderers are
    /// not written yet: whether a format can be rendered is [`renderer_for`]'s judgement, not a
    /// spelling question.
    pub fn from_name(name: &str) -> Result<Self> {
        let name = name.trim().to_ascii_lowercase();
        Self::ALL
            .into_iter()
            .find(|format| format.as_str() == name)
            .ok_or_else(|| {
                Error::Usage(format!(
                    "unknown format `{name}`; known formats: {}",
                    Self::ALL.map(Self::as_str).join(", ")
                ))
            })
    }
}

/// The renderer for a format.
///
/// A terminal that cannot draw UTF-8 or box drawing gets the ASCII table automatically, so the
/// user sees a table rather than mojibake; the caller reports that switch under `--verbose`.
///
/// `template` is the resolved `--template` value ([`one_line::resolve_template`]) and belongs to
/// `one-line` alone: passing one for another format is a usage error rather than a silently
/// ignored argument.
pub fn renderer_for(
    format: Format,
    caps: &TermCaps,
    template: Option<&str>,
) -> Result<Box<dyn Renderer>> {
    if template.is_some() && format != Format::OneLine {
        return Err(Error::Usage(format!(
            "`--template` requires `--format one-line`; {} takes no template",
            format.as_str()
        )));
    }
    match format {
        Format::ArtTable => Ok(Box::new(art_table::ArtTable::new(caps.charset()))),
        Format::Dumb => Ok(Box::new(art_table::ArtTable::dumb())),
        Format::Plain => Ok(Box::new(plain::Plain)),
        Format::Json => Ok(Box::new(json::Json)),
        Format::OneLine => Ok(Box::new(one_line::OneLine::new(
            one_line::resolve_template(template)?,
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ColorDepth, ColorMode, ColorPreference, Format, MIN_WIDTH, TermCaps, TermKind, WidthSource,
        effective_depth, renderer_for, resolve_color, resolve_width_from,
    };

    /// A terminal that says: xterm-256color, UTF-8 locale, no colour preferences.
    fn caps() -> TermCaps {
        TermCaps::read(
            |name| match name {
                "TERM" => Some("xterm-256color".to_owned()),
                "LANG" => Some("en_US.UTF-8".to_owned()),
                _ => None,
            },
            true,
        )
    }

    #[test]
    fn the_environment_is_read_into_capabilities() {
        let caps = caps();
        assert!(caps.is_tty);
        assert_eq!(caps.term, TermKind::Standard);
        assert!(caps.utf8);
        assert_eq!(caps.depth, ColorDepth::Ansi256);
        assert_eq!(caps.color_pref, ColorPreference::Neutral);
        assert_eq!(caps.charset(), super::Charset::Unicode);

        let dumb = TermCaps::read(|name| (name == "TERM").then(|| "dumb".to_owned()), true);
        assert_eq!(dumb.term, TermKind::Dumb);
        assert_eq!(dumb.depth, ColorDepth::Mono);
        assert_eq!(dumb.charset(), super::Charset::Ascii);

        let bare = TermCaps::read(|_| None, false);
        assert_eq!(bare.term, TermKind::Dumb);
        assert!(!bare.utf8);
        assert_eq!(bare.depth, ColorDepth::Mono);
        assert_eq!(bare, TermCaps::default());

        let linux_console = TermCaps::read(
            |name| match name {
                "TERM" => Some("linux".to_owned()),
                "LANG" => Some("C.UTF-8".to_owned()),
                _ => None,
            },
            true,
        );
        assert_eq!(linux_console.depth, ColorDepth::Ansi16);

        let truecolor = TermCaps::read(
            |name| match name {
                "TERM" => Some("xterm".to_owned()),
                "COLORTERM" => Some("truecolor".to_owned()),
                "LC_CTYPE" => Some("tr_TR.UTF-8".to_owned()),
                "LANG" => Some("C".to_owned()),
                _ => None,
            },
            true,
        );
        assert_eq!(truecolor.depth, ColorDepth::Ansi256, "COLORTERM wins");
        assert!(truecolor.utf8, "LC_CTYPE outranks LANG");

        let empty_prefix = TermCaps::read(
            |name| match name {
                "TERM" => Some("xterm-256color".to_owned()),
                "LC_ALL" => Some(String::new()),
                "LC_CTYPE" => Some("C.utf8".to_owned()),
                _ => None,
            },
            true,
        );
        assert!(empty_prefix.utf8, "an empty LC_ALL falls through");
    }

    #[test]
    fn explicit_colour_modes_are_never_second_guessed() {
        let pipe = TermCaps::read(|_| None, false);
        assert_eq!(resolve_color(ColorMode::Always, &pipe), ColorMode::Always);
        assert_eq!(resolve_color(ColorMode::Never, &caps()), ColorMode::Never);
    }

    #[test]
    fn the_auto_ladder_is_a_truth_table() {
        let mut terminal = caps();
        assert_eq!(resolve_color(ColorMode::Auto, &terminal), ColorMode::Always);

        terminal.color_pref = ColorPreference::NoColor;
        assert_eq!(
            resolve_color(ColorMode::Auto, &terminal),
            ColorMode::Never,
            "NO_COLOR disables"
        );
        terminal.color_pref = ColorPreference::Force;
        assert_eq!(
            resolve_color(ColorMode::Auto, &terminal),
            ColorMode::Always,
            "CLICOLOR_FORCE wins over NO_COLOR"
        );

        let mut forced_pipe = TermCaps::read(
            |name| (name == "CLICOLOR_FORCE").then(|| "1".to_owned()),
            false,
        );
        assert_eq!(
            resolve_color(ColorMode::Auto, &forced_pipe),
            ColorMode::Always,
            "CLICOLOR_FORCE beats a non-tty stdout"
        );
        forced_pipe.color_pref = ColorPreference::Neutral;
        assert_eq!(
            resolve_color(ColorMode::Auto, &forced_pipe),
            ColorMode::Never,
            "a pipe disables"
        );

        let mut dumb = TermCaps::read(|name| (name == "TERM").then(|| "dumb".to_owned()), true);
        assert_eq!(resolve_color(ColorMode::Auto, &dumb), ColorMode::Never);
        dumb.term = TermKind::Standard;
        assert_eq!(resolve_color(ColorMode::Auto, &dumb), ColorMode::Always);

        let zero = TermCaps::read(
            |name| (name == "CLICOLOR_FORCE").then(|| "0".to_owned()),
            true,
        );
        assert_eq!(
            zero.color_pref,
            ColorPreference::Neutral,
            "CLICOLOR_FORCE=0 is not a force"
        );

        let both = TermCaps::read(
            |name| match name {
                "NO_COLOR" => Some(String::new()),
                "CLICOLOR_FORCE" => Some("1".to_owned()),
                _ => None,
            },
            true,
        );
        assert_eq!(
            both.color_pref,
            ColorPreference::Force,
            "the force is the more specific request"
        );
    }

    #[test]
    fn the_emitted_palette_follows_the_mode_and_the_terminal() {
        let terminal = caps();
        assert_eq!(
            effective_depth(ColorMode::Always, &terminal),
            ColorDepth::Ansi256
        );
        assert_eq!(
            effective_depth(ColorMode::Never, &terminal),
            ColorDepth::Mono
        );
        assert_eq!(
            effective_depth(ColorMode::Auto, &terminal),
            ColorDepth::Ansi256
        );

        let bare = TermCaps::default();
        assert_eq!(
            effective_depth(ColorMode::Always, &bare),
            ColorDepth::Ansi256,
            "an explicit request for colour still emits escapes"
        );
        assert_eq!(
            effective_depth(ColorMode::Auto, &bare),
            ColorDepth::Ansi256,
            "an unresolved `auto` is a request for colour; resolve_color decides first"
        );
    }

    #[test]
    fn the_width_ladder_prefers_the_explicit_value() {
        let width = resolve_width_from(Some(120), Some("100"), Some(90));
        assert_eq!((width.columns, width.source), (120, WidthSource::Explicit));
        assert_eq!(width.raised_from, None);

        let width = resolve_width_from(None, Some("100"), Some(90));
        assert_eq!((width.columns, width.source), (100, WidthSource::Columns));

        let width = resolve_width_from(None, None, Some(90));
        assert_eq!((width.columns, width.source), (90, WidthSource::Terminal));

        let width = resolve_width_from(None, None, None);
        assert_eq!((width.columns, width.source), (80, WidthSource::Fallback));
    }

    #[test]
    fn unusable_widths_are_skipped_or_raised() {
        for garbage in ["", " ", "abc", "0", "-5", "12.5"] {
            let width = resolve_width_from(None, Some(garbage), Some(90));
            assert_eq!(
                (width.columns, width.source),
                (90, WidthSource::Terminal),
                "COLUMNS={garbage:?} is not a width"
            );
        }

        let width = resolve_width_from(Some(0), Some("100"), Some(90));
        assert_eq!((width.columns, width.source), (100, WidthSource::Columns));
        assert_eq!(width.raised_from, None);

        let width = resolve_width_from(Some(7), None, None);
        assert_eq!(
            (width.columns, width.source),
            (MIN_WIDTH, WidthSource::Explicit)
        );
        assert_eq!(width.raised_from, Some(7));

        let width = resolve_width_from(Some(MIN_WIDTH), None, None);
        assert_eq!(
            (width.columns, width.source),
            (MIN_WIDTH, WidthSource::Explicit)
        );
        assert_eq!(width.raised_from, None, "the minimum itself is not raised");
    }

    #[test]
    fn format_spellings_round_trip() {
        for format in Format::ALL {
            assert_eq!(
                Format::from_name(format.as_str()).expect("its own spelling"),
                format
            );
        }
        assert_eq!(
            Format::from_name(" ART-TABLE ").expect("trimmed"),
            Format::ArtTable
        );

        let unknown = Format::from_name("yaml").expect_err("never a format");
        assert_eq!(unknown.exit_code(), 2);
        let message = unknown.to_string();
        assert!(message.contains("unknown format `yaml`"));
        assert!(message.contains("art-table, one-line, plain, json, dumb"));
    }

    #[test]
    fn every_format_has_a_renderer_and_templates_belong_to_one_line() {
        let caps = caps();
        for format in Format::ALL {
            assert!(
                renderer_for(format, &caps, None).is_ok(),
                "{} has no renderer",
                format.as_str()
            );
        }

        assert!(renderer_for(Format::OneLine, &caps, Some("@short")).is_ok());
        for format in [Format::ArtTable, Format::Dumb, Format::Plain, Format::Json] {
            let error = renderer_for(format, &caps, Some("%c %t"))
                .err()
                .expect("a template outside one-line");
            assert_eq!(error.exit_code(), 2, "{}", format.as_str());
            assert!(
                error
                    .to_string()
                    .contains("`--template` requires `--format one-line`"),
                "{error}"
            );
        }
    }
}
