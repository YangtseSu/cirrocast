// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The `%`-token template engine: one implementation behind every one-line-style output.
//!
//! `one-line`, the `full`/`minimal` one-line presets, step 22's `status` probe and the future
//! wttr.in compatibility surface (B01) all render through this module. The token table is the
//! single place a `%` letter is bound to a meaning, so the three callers cannot drift apart; the
//! compat surface differs in exactly one policy — it keeps an unknown token literal instead of
//! failing — and that policy lives at its boundary, never as a second parser.
//!
//! # Tokens
//!
//! [`TOKENS`] is the exported table (letter, meaning, kind, one-line description) that the tests
//! and the compat help page enumerate. The vocabulary is wttr.in's documented one-line notation,
//! re-authored where the value comes from this crate's canonical model:
//!
//! `%c %C %x %t %f %H %L %w %h %p %P %e %u %U %m %M %v %l %d %D %T %Z %z %S %s %A %q`.
//!
//! A token whose value the provider does not report prints `n/a`; it is never invented, and it is
//! never a zero. Values come from [`crate::model::units`] like every other renderer's, so a unit
//! conversion happens exactly once in this crate. Two tokens are computed rather than read:
//! `%e` (dew point, from the reported temperature and humidity with the Magnus formula) and the
//! astro tokens (`%m`, `%M`), which are computed locally like the rest of the moon block.
//!
//! # Width and precision
//!
//! A token may carry `%[-][0][<width>][.<precision>]X`:
//!
//! * `<width>` is a minimum, measured in characters; the value is padded with spaces on the left,
//!   or on the right with `-`.
//! * The `0` flag pads a numeric token with zeros between its sign and its digits; text tokens
//!   pad with spaces, because `000Cloudy` is never what a caller meant.
//! * `<precision>` truncates a text value from the right (characters, not bytes) and rounds a
//!   numeric token to that many decimals, inside its unit suffix (`%.1t` → `+18.4°C`). The
//!   numeric tokens are the ones whose value is a number or a number with one unit suffix —
//!   `%t %f %H %L %e %u %h %p %P %v`; every other token is text, `%w` included (its value is a
//!   composite of arrow, speed and direction).
//!
//! # Escapes
//!
//! * `%%` is a literal `%`, and a trailing lone `%` is one too;
//! * `\n`, `\t` and `\\` are unescaped before the tokens are read, so a template can span lines;
//! * `%{<text>}` prints `<text>` verbatim — no token expansion — and `\}` escapes a closing brace
//!   inside it. A braced run whose content is exactly one known letter *is* that token, which is
//!   how a token is written next to text that would otherwise glue onto its letter (`%{d}usk`);
//! * an unknown `%X` prints literally as `%X` and is reported once per occurrence by [`warnings`].
//!   The CLI turns that report into a usage error; the compat surface keeps the literal and logs.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use chrono::{DateTime, FixedOffset, Timelike as _};

use crate::air::aqi::{AqiCategory, AqiIndex};
use crate::error::{Error, Result};
use crate::i18n::{DateStyle, keys};
use crate::model::units::{
    UnitStyle, fmt_decimals, fmt_int, format_precip, format_precip_prec, format_pressure,
    format_pressure_prec, format_temp_signed, format_temp_signed_prec, format_visibility,
    format_visibility_prec, format_wind,
};
use crate::model::{Condition, DayForecast, Report};
use crate::render::RenderContext;
use crate::render::art;

/// The template used when neither `--template` nor a preset-selecting format names one.
pub const DEFAULT_PRESET: &str = "default";

/// The named templates `--template @name`, `--format <name>` and the `[templates]` table accept,
/// in help order.
///
/// `minimal` is the wttr.in `format=1` shape (`%c%t`, no separator) while `short` keeps the older
/// spaced spelling; both exist so neither audience has to change.
pub const PRESETS: [(&str, &str); 6] = [
    ("default", "%l: %c %C %t (%f), %w, %h, %p, %P, %v"),
    ("short", "%c %t"),
    ("minimal", "%c%t"),
    ("full", "%l: %c %C %t (%f) %w %h %p %P %m %v %u %S %s %Z"),
    ("uv", "%l: UV %U"),
    ("sun", "%l: sunrise %S sunset %s (%z %Z)"),
];

/// The preset list as it appears in `--help` and in the unknown-preset error.
#[must_use]
pub fn preset_help() -> String {
    PRESETS
        .iter()
        .map(|(name, template)| format!("  @{name:<8} {template}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The template of a built-in preset.
#[must_use]
pub fn preset(name: &str) -> Option<&'static str> {
    PRESETS
        .iter()
        .find(|(preset, _)| *preset == name)
        .map(|(_, template)| *template)
}

/// Resolves a template spec into the literal template to render: `None` is the default preset,
/// `@name` a built-in preset or a `[templates]` entry, anything else a template of its own.
///
/// The result is *not* checked for unknown tokens: [`expand`] keeps them literal (the compat
/// rule), and [`validate`] is the CLI's separate, stricter gate. An unknown preset lists the two
/// namespaces instead of guessing, and an empty or whitespace-only template is a usage error
/// rather than an empty output line.
pub fn resolve_template(
    spec: Option<&str>,
    configured: &BTreeMap<String, String>,
) -> Result<String> {
    let Some(spec) = spec else {
        return Ok(preset(DEFAULT_PRESET).unwrap_or_default().to_owned());
    };
    if let Some(name) = spec.trim_start().strip_prefix('@') {
        let name = name.trim();
        return Ok(builtin_or_configured(name, configured)
            .ok_or_else(|| Error::Usage(unknown_preset(name, configured)))?
            .to_owned());
    }
    if spec.trim().is_empty() {
        return Err(Error::Usage(
            "the one-line template is empty; pass --template <TEMPLATE|@PRESET>".to_owned(),
        ));
    }
    Ok(spec.to_owned())
}

/// The template `name` names, from the built-in presets first and the `[templates]` table second.
#[must_use]
pub fn builtin_or_configured<'a>(
    name: &str,
    configured: &'a BTreeMap<String, String>,
) -> Option<&'a str> {
    preset(name).or_else(|| configured.get(name).map(String::as_str))
}

/// The error an unknown preset name reports: the presets, then the configured names.
fn unknown_preset(name: &str, configured: &BTreeMap<String, String>) -> String {
    let mut message = format!(
        "unknown one-line preset `@{name}`; known presets:\n{}",
        preset_help()
    );
    if !configured.is_empty() {
        let names = configured.keys().cloned().collect::<Vec<_>>().join(", ");
        let _ = write!(message, "\nconfigured [templates]: {names}");
    }
    message
}

/// The CLI's unknown-token gate: the first unknown `%X` is a usage error.
///
/// The renderer itself keeps an unknown token literal (that is the compat rule, and the only
/// behaviour [`expand`] needs to have); a user typing a template on the command line, or naming a
/// `[templates]` preset, is authoring one and wants the typo reported instead.
pub fn validate(template: &str) -> Result<()> {
    let warnings = warnings(template);
    if let Some(first) = warnings.first() {
        return Err(Error::Usage(format!(
            "{first}; known tokens: {}",
            TOKENS
                .iter()
                .map(|spec| spec.letter)
                .collect::<Vec<_>>()
                .iter()
                .collect::<String>()
        )));
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Tokens
// ---------------------------------------------------------------------------------------------

/// One `%` token of the template language.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Token {
    /// `%c` — the condition art, day/night aware, in the terminal's charset.
    ConditionArt,
    /// `%C` — the condition text.
    ConditionText,
    /// `%x` — the condition art as plain 7-bit text, whatever the terminal supports.
    ConditionPlain,
    /// `%t` — the temperature.
    Temp,
    /// `%f` — the apparent temperature.
    FeelsLike,
    /// `%H` — today's high temperature.
    High,
    /// `%L` — today's low temperature.
    Low,
    /// `%w` — wind arrow, speed and direction.
    Wind,
    /// `%h` — relative humidity.
    Humidity,
    /// `%p` — precipitation.
    Precip,
    /// `%P` — pressure.
    Pressure,
    /// `%e` — the dew point, computed from temperature and humidity.
    DewPoint,
    /// `%u` — the UV index.
    Uv,
    /// `%U` — the UV index with its band name.
    UvBand,
    /// `%m` — the moon phase's art glyph.
    Moon,
    /// `%M` — the moon phase's name (`Waxing Crescent`).
    MoonPhase,
    /// `%v` — visibility.
    Visibility,
    /// `%l` — the place name.
    Location,
    /// `%d` — the date, ISO 8601.
    Date,
    /// `%D` — the date, `Wed 30 Sep`.
    DateLong,
    /// `%T` — the local time, `14:05`.
    Time,
    /// `%Z` — the time zone name.
    TzName,
    /// `%z` — the UTC offset, `+0800`.
    TzOffset,
    /// `%S` — sunrise.
    Sunrise,
    /// `%s` — sunset.
    Sunset,
    /// `%A` — the strongest alert's event, empty when there are none.
    Alert,
    /// `%q` — the air-quality index on the selected scale.
    Quality,
}

/// What a token's value is, for the precision and zero-pad rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    /// Arbitrary text: precision truncates it, the `0` flag pads with spaces.
    Text,
    /// A number with at most one unit suffix: precision rounds it, `0` zero-pads it.
    Number,
}

/// One row of [`TOKENS`].
#[derive(Debug, Clone, Copy)]
pub struct TokenSpec {
    /// The letter after `%`.
    pub letter: char,
    /// The meaning.
    pub token: Token,
    /// What kind of value the token renders, for width and precision.
    pub kind: TokenKind,
    /// One-line description, for `--help` and the compat help page.
    pub doc: &'static str,
}

/// The token table: the one place a `%` letter is bound to a meaning.
///
/// Adding a token means adding a row here plus an arm in `value`; the tests walk this table, so a
/// row without a value is a compile error and a value without a row is unreachable.
pub const TOKENS: &[TokenSpec] = &[
    TokenSpec {
        letter: 'c',
        token: Token::ConditionArt,
        kind: TokenKind::Text,
        doc: "condition art",
    },
    TokenSpec {
        letter: 'C',
        token: Token::ConditionText,
        kind: TokenKind::Text,
        doc: "condition text",
    },
    TokenSpec {
        letter: 'x',
        token: Token::ConditionPlain,
        kind: TokenKind::Text,
        doc: "condition art, plain text",
    },
    TokenSpec {
        letter: 't',
        token: Token::Temp,
        kind: TokenKind::Number,
        doc: "temperature",
    },
    TokenSpec {
        letter: 'f',
        token: Token::FeelsLike,
        kind: TokenKind::Number,
        doc: "feels-like temperature",
    },
    TokenSpec {
        letter: 'H',
        token: Token::High,
        kind: TokenKind::Number,
        doc: "today's high",
    },
    TokenSpec {
        letter: 'L',
        token: Token::Low,
        kind: TokenKind::Number,
        doc: "today's low",
    },
    TokenSpec {
        letter: 'w',
        token: Token::Wind,
        kind: TokenKind::Text,
        doc: "wind",
    },
    TokenSpec {
        letter: 'h',
        token: Token::Humidity,
        kind: TokenKind::Number,
        doc: "humidity",
    },
    TokenSpec {
        letter: 'p',
        token: Token::Precip,
        kind: TokenKind::Number,
        doc: "precipitation",
    },
    TokenSpec {
        letter: 'P',
        token: Token::Pressure,
        kind: TokenKind::Number,
        doc: "pressure",
    },
    TokenSpec {
        letter: 'e',
        token: Token::DewPoint,
        kind: TokenKind::Number,
        doc: "dew point",
    },
    TokenSpec {
        letter: 'u',
        token: Token::Uv,
        kind: TokenKind::Number,
        doc: "UV index",
    },
    TokenSpec {
        letter: 'U',
        token: Token::UvBand,
        kind: TokenKind::Text,
        doc: "UV index with band",
    },
    TokenSpec {
        letter: 'm',
        token: Token::Moon,
        kind: TokenKind::Text,
        doc: "moon glyph",
    },
    TokenSpec {
        letter: 'M',
        token: Token::MoonPhase,
        kind: TokenKind::Text,
        doc: "moon phase",
    },
    TokenSpec {
        letter: 'v',
        token: Token::Visibility,
        kind: TokenKind::Number,
        doc: "visibility",
    },
    TokenSpec {
        letter: 'l',
        token: Token::Location,
        kind: TokenKind::Text,
        doc: "place name",
    },
    TokenSpec {
        letter: 'd',
        token: Token::Date,
        kind: TokenKind::Text,
        doc: "ISO date",
    },
    TokenSpec {
        letter: 'D',
        token: Token::DateLong,
        kind: TokenKind::Text,
        doc: "short date",
    },
    TokenSpec {
        letter: 'T',
        token: Token::Time,
        kind: TokenKind::Text,
        doc: "local time",
    },
    TokenSpec {
        letter: 'Z',
        token: Token::TzName,
        kind: TokenKind::Text,
        doc: "time zone name",
    },
    TokenSpec {
        letter: 'z',
        token: Token::TzOffset,
        kind: TokenKind::Text,
        doc: "UTC offset",
    },
    TokenSpec {
        letter: 'S',
        token: Token::Sunrise,
        kind: TokenKind::Text,
        doc: "sunrise",
    },
    TokenSpec {
        letter: 's',
        token: Token::Sunset,
        kind: TokenKind::Text,
        doc: "sunset",
    },
    TokenSpec {
        letter: 'A',
        token: Token::Alert,
        kind: TokenKind::Text,
        doc: "strongest alert event",
    },
    TokenSpec {
        letter: 'q',
        token: Token::Quality,
        kind: TokenKind::Text,
        doc: "air-quality index",
    },
];

/// The token a `%` letter stands for.
#[must_use]
pub fn token(letter: char) -> Option<Token> {
    TOKENS
        .iter()
        .find(|spec| spec.letter == letter)
        .map(|spec| spec.token)
}

/// The table row for a `%` letter.
#[must_use]
pub fn token_spec(letter: char) -> Option<&'static TokenSpec> {
    TOKENS.iter().find(|spec| spec.letter == letter)
}

// ---------------------------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------------------------

/// A width/precision specifier attached to one token.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Spec {
    /// Minimum width in characters.
    width: Option<usize>,
    /// `-`: pad on the right instead of the left.
    left: bool,
    /// `0`: zero-pad a numeric token.
    zero: bool,
    /// `.N`: truncate text to N characters, round numbers to N decimals.
    precision: Option<usize>,
}

/// One piece of a parsed template.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Piece {
    /// Literal text, with the backslash escapes already applied.
    Text(String),
    /// A known token with its kind and specifier.
    Token {
        token: Token,
        kind: TokenKind,
        spec: Spec,
    },
    /// An unknown `%X`, emitted verbatim; the fields are the letter, its 1-based character
    /// position and the raw spelling (specifier included) for the literal output.
    Unknown {
        letter: char,
        position: usize,
        raw: String,
    },
}

/// Parses `template` into literal text and tokens.
///
/// Positions are counted in characters, not bytes, so a template with CJK text reports the column
/// a reader would count.
fn parse(template: &str) -> Vec<Piece> {
    let mut pieces: Vec<Piece> = Vec::new();
    let mut text = String::new();
    let mut chars = template.char_indices().peekable();

    // The literal text collected so far is flushed as one piece, so `%%`, `\n` and the tokens
    // around them do not fragment a run of plain text.
    while let Some((offset, character)) = chars.next() {
        match character {
            '\\' => match chars.peek().map(|(_, next)| *next) {
                Some('n') => {
                    chars.next();
                    text.push('\n');
                }
                Some('t') => {
                    chars.next();
                    text.push('\t');
                }
                Some('\\') => {
                    chars.next();
                    text.push('\\');
                }
                // Any other backslash is literal, so a Windows path in a template survives.
                _ => text.push('\\'),
            },
            '%' => match chars.peek().map(|(_, next)| *next) {
                Some('%') => {
                    chars.next();
                    text.push('%');
                }
                Some('{') => {
                    chars.next();
                    let content = braced(&mut chars);
                    match single_letter(&content).and_then(token_spec) {
                        Some(row) => {
                            flush(&mut pieces, &mut text);
                            pieces.push(Piece::Token {
                                token: row.token,
                                kind: row.kind,
                                spec: Spec::default(),
                            });
                        }
                        None => text.push_str(&content),
                    }
                }
                Some(next) if spec_char(next) => {
                    let (spec, raw) = read_spec(&mut chars);
                    if let Some((_, letter)) = chars.next() {
                        flush(&mut pieces, &mut text);
                        push_letter(&mut pieces, letter, offset, spec, &raw, template);
                    } else {
                        // `%12` with nothing after it is literal text.
                        text.push('%');
                        text.push_str(&raw);
                    }
                }
                Some(letter) => {
                    chars.next();
                    flush(&mut pieces, &mut text);
                    push_letter(&mut pieces, letter, offset, Spec::default(), "", template);
                }
                // A trailing `%` is a literal one.
                None => text.push('%'),
            },
            other => text.push(other),
        }
    }
    flush(&mut pieces, &mut text);
    pieces
}

/// Whether `character` can be part of a width/precision specifier.
fn spec_char(character: char) -> bool {
    matches!(character, '-' | '0'..='9' | '.')
}

/// Reads `[-][0][width][.precision]` after a `%`, returning the specifier and its raw spelling.
///
/// The raw spelling lets an unknown token be printed back exactly as it was written. The `-` and
/// `0` flags are honoured only in front of the width (a printf convention), and a lone `.` without
/// digits is not a specifier at all.
fn read_spec(chars: &mut std::iter::Peekable<std::str::CharIndices<'_>>) -> (Spec, String) {
    let mut spec = Spec::default();
    let mut raw = String::new();
    if chars.peek().is_some_and(|(_, next)| *next == '-') {
        spec.left = true;
        raw.push('-');
        chars.next();
    }
    if chars.peek().is_some_and(|(_, next)| *next == '0') {
        spec.zero = true;
        raw.push('0');
        chars.next();
    }
    let mut width: usize = 0;
    let mut has_width = false;
    while let Some((_, digit @ '0'..='9')) = chars.peek().copied() {
        width = width
            .saturating_mul(10)
            .saturating_add(digit as usize - '0' as usize);
        raw.push(digit);
        chars.next();
        has_width = true;
    }
    if has_width {
        spec.width = Some(width.min(MAX_WIDTH));
    }
    if chars.peek().is_some_and(|(_, next)| *next == '.') {
        chars.next();
        let mut digits = String::new();
        let mut precision: usize = 0;
        while let Some((_, digit @ '0'..='9')) = chars.peek().copied() {
            precision = precision
                .saturating_mul(10)
                .saturating_add(digit as usize - '0' as usize);
            digits.push(digit);
            chars.next();
        }
        if digits.is_empty() {
            // A lone `.` is not a specifier: put it back for the literal text.
            raw.push('.');
        } else {
            spec.precision = Some(precision.min(MAX_PRECISION));
            raw.push('.');
            raw.push_str(&digits);
        }
    }
    (spec, raw)
}

/// Appends the piece for the letter after a `%` (or a specifier).
fn push_letter(
    pieces: &mut Vec<Piece>,
    letter: char,
    offset: usize,
    spec: Spec,
    raw: &str,
    template: &str,
) {
    match token_spec(letter) {
        Some(row) => pieces.push(Piece::Token {
            token: row.token,
            kind: row.kind,
            spec,
        }),
        None => pieces.push(Piece::Unknown {
            letter,
            position: template[..offset].chars().count() + 1,
            raw: format!("%{raw}{letter}"),
        }),
    }
}

/// Flushes the pending literal text as its own piece, when there is any.
fn flush(pieces: &mut Vec<Piece>, text: &mut String) {
    if !text.is_empty() {
        pieces.push(Piece::Text(std::mem::take(text)));
    }
}

/// The single character `content` consists of, when it is exactly one character.
fn single_letter(content: &str) -> Option<char> {
    let mut chars = content.chars();
    let letter = chars.next()?;
    chars.next().is_none().then_some(letter)
}

/// Reads `%{...}`, with `\}` escaping the closing brace.
///
/// An unterminated `%{` is literal: the two characters are kept and the scan continues after them,
/// so a template cannot swallow the rest of itself by accident.
fn braced(chars: &mut std::iter::Peekable<std::str::CharIndices<'_>>) -> String {
    let mut content = String::new();
    loop {
        match chars.next() {
            None => return format!("%{{{content}"),
            Some((_, '}')) => return content,
            Some((_, '\\')) => match chars.peek().map(|(_, next)| *next) {
                Some('}') => {
                    chars.next();
                    content.push('}');
                }
                _ => content.push('\\'),
            },
            Some((_, other)) => content.push(other),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Expansion
// ---------------------------------------------------------------------------------------------

/// Expands `template` for `report`.
///
/// The template is the whole output: no lines are added, and the result carries no trailing
/// newline. An empty or whitespace-only template is [`Error::Usage`] — a caller that asked for
/// nothing gets told, rather than an empty line.
pub fn expand(template: &str, report: &Report, ctx: &RenderContext<'_>) -> Result<String> {
    if template.trim().is_empty() {
        return Err(Error::Usage(
            "the one-line template is empty; pass --template <TEMPLATE|@PRESET>".to_owned(),
        ));
    }
    let snapshot = Snapshot::of(report, ctx);
    let mut out = String::new();
    for piece in parse(template) {
        match piece {
            Piece::Text(text) => out.push_str(&text),
            Piece::Token { token, kind, spec } => {
                out.push_str(&format_value(token, kind, spec, &snapshot, report, ctx));
            }
            Piece::Unknown { raw, .. } => out.push_str(&raw),
        }
    }
    Ok(out)
}

/// One warning per unknown `%X`, naming the token and its position.
///
/// Pure: it re-reads the same parse the expansion uses, so the CLI can turn the warnings into a
/// usage error (and the compat surface can log them) without the renderer ever seeing a verbosity
/// flag.
#[must_use]
pub fn warnings(template: &str) -> Vec<String> {
    parse(template)
        .into_iter()
        .filter_map(|piece| match piece {
            Piece::Unknown {
                letter, position, ..
            } => Some(format!(
                "unknown template token `%{letter}` at position {position}"
            )),
            Piece::Text(_) | Piece::Token { .. } => None,
        })
        .collect()
}

/// Renders one token and applies its width/precision specifier.
fn format_value(
    token: Token,
    kind: TokenKind,
    spec: Spec,
    snapshot: &Snapshot,
    report: &Report,
    ctx: &RenderContext<'_>,
) -> String {
    let mut value = value(token, spec.precision, snapshot, report, ctx);
    if let Some(precision) = spec.precision
        && kind == TokenKind::Text
    {
        value = truncate_chars(&value, precision);
    }
    if let Some(width) = spec.width {
        value = pad(&value, width, spec, kind);
    }
    value
}

/// Truncates `text` to `precision` characters (never mid-codepoint).
fn truncate_chars(text: &str, precision: usize) -> String {
    text.chars().take(precision).collect()
}

/// Pads `text` to `width` characters: spaces, or zeros for a numeric token with the `0` flag.
fn pad(text: &str, width: usize, spec: Spec, kind: TokenKind) -> String {
    let len = text.chars().count();
    if len >= width {
        return text.to_owned();
    }
    let fill = width - len;
    if spec.left {
        let mut out = text.to_owned();
        out.extend(std::iter::repeat_n(' ', fill));
        return out;
    }
    if spec.zero && kind == TokenKind::Number {
        return zero_pad(text, fill);
    }
    let mut out = " ".repeat(fill);
    out.push_str(text);
    out
}

/// Inserts `fill` zeros after a leading sign, so `+18°C` at width 7 reads `+0018°C`.
fn zero_pad(text: &str, fill: usize) -> String {
    let mut chars = text.chars();
    let sign = match chars.clone().next() {
        Some(first @ ('+' | '-')) => {
            chars.next();
            Some(first)
        }
        _ => None,
    };
    let mut out = String::new();
    if let Some(sign) = sign {
        out.push(sign);
    }
    out.extend(std::iter::repeat_n('0', fill));
    out.extend(chars);
    out
}

/// The most characters a `%<width>` may ask for.
const MAX_WIDTH: usize = 200;

/// The most decimals a `%.<precision>` may ask for.
const MAX_PRECISION: usize = 6;

/// The values the tokens read, resolved once per report.
///
/// Built from the current conditions when the provider reports them, else from the day part the
/// run's clock falls in — a daily-only backend still answers `%t` and `%C`. A value neither source
/// has stays `None` and prints as `n/a`.
#[derive(Debug, Default)]
struct Snapshot {
    condition: Option<Condition>,
    is_day: bool,
    temp_c: Option<f32>,
    feels_like_c: Option<f32>,
    high_c: Option<f32>,
    low_c: Option<f32>,
    wind_kmh: Option<f32>,
    wind_dir_deg: Option<u16>,
    humidity_pct: Option<u8>,
    precip_mm: Option<f32>,
    pressure_hpa: Option<f32>,
    visibility_km: Option<f32>,
    uv_index: Option<f32>,
    sunrise: Option<DateTime<FixedOffset>>,
    sunset: Option<DateTime<FixedOffset>>,
}

impl Snapshot {
    /// Reads the values for `ctx.now` at the report's location.
    fn of(report: &Report, ctx: &RenderContext<'_>) -> Self {
        let mut snapshot = Self::default();
        let today = today(report, ctx.now.date_naive());

        if let Some(day) = today {
            snapshot.sunrise = day.sunrise;
            snapshot.sunset = day.sunset;
            snapshot.high_c = Some(day.temp_max_c);
            snapshot.low_c = Some(day.temp_min_c);
        }

        if let Some(current) = &report.current {
            snapshot.condition = Some(current.weather);
            snapshot.is_day = current.is_day;
            snapshot.temp_c = Some(current.temp_c);
            snapshot.feels_like_c = current.feels_like_c;
            snapshot.wind_kmh = Some(current.wind_kmh);
            snapshot.wind_dir_deg = Some(current.wind_dir_deg);
            snapshot.humidity_pct = Some(current.humidity_pct);
            snapshot.precip_mm = Some(current.precip_mm);
            snapshot.pressure_hpa = Some(current.pressure_hpa);
            snapshot.visibility_km = current.visibility_km;
            snapshot.uv_index = current.uv_index;
            return snapshot;
        }

        if let Some(day) = today {
            let kind = hour_part(ctx.now);
            let part = &day.parts[kind.index()];
            snapshot.condition = Some(part.weather);
            snapshot.is_day = kind != crate::model::DayPartKind::Night;
            snapshot.temp_c = Some(part.temp_c);
            snapshot.feels_like_c = part.feels_like_c;
            snapshot.wind_kmh = Some(part.wind_kmh);
            snapshot.wind_dir_deg = part.wind_dir_deg;
            snapshot.humidity_pct = part.humidity_pct;
            snapshot.precip_mm = Some(part.precip_mm);
            snapshot.visibility_km = part.visibility_km;
        }
        snapshot
    }

    /// The dew point in °C from the snapshot's temperature and humidity, when both are reported.
    fn dew_point_c(&self) -> Option<f32> {
        dew_point_c(self.temp_c?, self.humidity_pct?)
    }
}

/// The forecast day the run's clock falls on, else the first day the report carries.
pub(crate) fn today(report: &Report, date: chrono::NaiveDate) -> Option<&DayForecast> {
    report
        .days
        .iter()
        .find(|day| day.date == date)
        .or_else(|| report.days.first())
}

/// The day part a local hour falls in, matching the aggregation the providers use.
pub(crate) fn hour_part(now: DateTime<FixedOffset>) -> crate::model::DayPartKind {
    use crate::model::DayPartKind;
    match now.hour() {
        6..=11 => DayPartKind::Morning,
        12..=17 => DayPartKind::Noon,
        18..=23 => DayPartKind::Evening,
        _ => DayPartKind::Night,
    }
}

/// The dew point from temperature and relative humidity (the Magnus formula, the inverse of the
/// METAR backend's humidity derivation).
///
/// A provider-reported dew point does not exist in the canonical model, so this is computed: the
/// standard approximation `Td = 243.04·α/(17.625−α)` with `α = ln(RH/100) + 17.625·T/(243.04+T)`.
/// Relative humidity is clamped into `1..=100` (a zero would take the logarithm to negative
/// infinity) and a nonsensical result for a temperature at or below the formula's pole is `None`,
/// which the `%e` token prints as `n/a`.
fn dew_point_c(temp_c: f32, humidity_pct: u8) -> Option<f32> {
    const A: f32 = 17.625;
    const B: f32 = 243.04;
    let relative = f32::from(humidity_pct).clamp(1.0, 100.0) / 100.0;
    let alpha = (A * temp_c) / (B + temp_c) + relative.ln();
    let dew = (B * alpha) / (A - alpha);
    dew.is_finite().then_some(dew)
}

/// Renders one token's value, before width and precision are applied.
fn value(
    token: Token,
    precision: Option<usize>,
    snapshot: &Snapshot,
    report: &Report,
    ctx: &RenderContext<'_>,
) -> String {
    let units = ctx.units;
    match token {
        Token::ConditionArt | Token::ConditionPlain => condition_art(snapshot, ctx),
        Token::ConditionText => snapshot.condition.map_or_else(
            || n_a(ctx),
            |condition| ctx.i18n.condition(condition).into_owned(),
        ),
        Token::Temp => temp_value(snapshot.temp_c, precision, ctx),
        Token::FeelsLike => temp_value(snapshot.feels_like_c, precision, ctx),
        Token::High => temp_value(snapshot.high_c, precision, ctx),
        Token::Low => temp_value(snapshot.low_c, precision, ctx),
        Token::DewPoint => temp_value(snapshot.dew_point_c(), precision, ctx),
        Token::Wind => wind_value(snapshot, ctx),
        Token::Humidity => snapshot.humidity_pct.map_or_else(
            || n_a(ctx),
            |humidity| match precision {
                Some(decimals) => format!("{}%", fmt_decimals(f32::from(humidity), decimals)),
                None => format!("{humidity}%"),
            },
        ),
        Token::Precip => snapshot.precip_mm.map_or_else(
            || n_a(ctx),
            |mm| match precision {
                Some(decimals) => {
                    format_precip_prec(mm, units.precip, UnitStyle::Compact, decimals)
                }
                None => format_precip(mm, units.precip, UnitStyle::Compact),
            },
        ),
        Token::Pressure => snapshot.pressure_hpa.map_or_else(
            || n_a(ctx),
            |hpa| match precision {
                Some(decimals) => {
                    format_pressure_prec(hpa, units.pressure, UnitStyle::Compact, decimals)
                }
                None => format_pressure(hpa, units.pressure, UnitStyle::Compact),
            },
        ),
        Token::Visibility => snapshot.visibility_km.map_or_else(
            || n_a(ctx),
            |km| match precision {
                Some(decimals) => {
                    format_visibility_prec(km, units.distance, UnitStyle::Compact, decimals)
                }
                None => format_visibility(km, units.distance, UnitStyle::Compact),
            },
        ),
        Token::Uv => snapshot.uv_index.map_or_else(
            || n_a(ctx),
            |uv| match precision {
                Some(decimals) => fmt_decimals(uv, decimals),
                None => fmt_int(uv),
            },
        ),
        Token::UvBand => snapshot
            .uv_index
            .map_or_else(|| n_a(ctx), |uv| uv_band_value(uv, ctx)),
        Token::Date => ctx.i18n.format_date(ctx.now.date_naive(), DateStyle::Iso),
        Token::DateLong => ctx.i18n.format_date(ctx.now.date_naive(), DateStyle::Short),
        Token::Time => ctx.now.format("%H:%M").to_string(),
        Token::TzName => ctx.tz.name().to_owned(),
        Token::TzOffset => ctx.now.format("%z").to_string(),
        Token::Sunrise => snapshot.sunrise.map_or_else(|| n_a(ctx), clock_time),
        Token::Sunset => snapshot.sunset.map_or_else(|| n_a(ctx), clock_time),
        Token::Location => report.location.name.clone(),
        Token::Moon => {
            let phase = crate::astro::phase_at(ctx.now);
            art::moon_glyph(phase, ctx.term.charset()).to_owned()
        }
        Token::MoonPhase => {
            let phase = crate::astro::phase_at(ctx.now);
            ctx.i18n.moon_phase(phase).into_owned()
        }
        // Unlike every other token, an absent alert is the empty string, not `n/a`: a template is
        // a sentence, and `%A` there reads as "the warning, if any".
        Token::Alert => report
            .alerts
            .first()
            .map_or_else(String::new, |alert| alert.event.clone()),
        Token::Quality => report
            .air
            .as_ref()
            .map_or_else(|| n_a(ctx), |air| quality_summary(air, ctx)),
    }
}

/// The `%w` value: arrow, speed and direction, or the speed alone when upstream reports none.
///
/// The direction is genuinely optional upstream — a calm `OpenWeatherMap` reading and a METAR
/// `VRB` both leave it out — and a known speed must not become `n/a` for want of an arrow: the
/// arrow-free speed is what the table and `plain` print. With a direction, the documented order is
/// arrow, speed, direction.
fn wind_value(snapshot: &Snapshot, ctx: &RenderContext<'_>) -> String {
    let Some(kmh) = snapshot.wind_kmh else {
        return n_a(ctx);
    };
    let speed = format_wind(kmh, ctx.units.wind, UnitStyle::Compact);
    match snapshot.wind_dir_deg {
        Some(deg) => format!(
            "{} {speed} {}",
            art::wind_arrow(deg, ctx.term.charset()),
            ctx.i18n.direction(deg)
        ),
        None => speed,
    }
}

/// The `%U` value: the UV index with its band name, e.g. `5 (moderate)`.
fn uv_band_value(uv: f32, ctx: &RenderContext<'_>) -> String {
    let band = ctx.i18n.uv_band(uv).into_owned();
    ctx.i18n
        .format(
            &keys::FORMAT_UV,
            &[
                ("value", fluent_bundle::FluentValue::from(fmt_int(uv))),
                ("band", fluent_bundle::FluentValue::from(band)),
            ],
        )
        .into_owned()
}

/// The condition art.
///
/// `%c` and `%x` differ by charset contract, not by table: the one-line art is deliberately 7-bit
/// in every charset, so `%x` (the plain-text symbol) is what `%c` prints when the terminal cannot
/// draw more than ASCII — and the two agree today. Keeping the tokens separate means adding a
/// Unicode condition glyph later changes only `%c`; the day/night variant is shared, because a
/// night must not draw a daytime sun in either spelling.
fn condition_art(snapshot: &Snapshot, ctx: &RenderContext<'_>) -> String {
    let Some(condition) = snapshot.condition else {
        return n_a(ctx);
    };
    let key = condition.art_key();
    let key = if snapshot.is_day {
        key
    } else {
        art::night_variant(key)
    };
    art::one_line_art(key).to_owned()
}

/// A temperature-family token: signed, in the resolved unit, with `precision` decimals.
fn temp_value(celsius: Option<f32>, precision: Option<usize>, ctx: &RenderContext<'_>) -> String {
    celsius.map_or_else(
        || n_a(ctx),
        |celsius| match precision {
            Some(decimals) => format_temp_signed_prec(celsius, ctx.units.temp, decimals),
            None => format_temp_signed(celsius, ctx.units.temp),
        },
    )
}

/// `US AQI 43 (Good)`: the index and category of the scale [`RenderContext::aqi_index`] selects.
///
/// The category word is localised like every other label; an air reading whose selected index is
/// absent (a source that reports only one of the two scales) prints `n/a` rather than the other
/// scale's number under the selected scale's name.
fn quality_summary(air: &crate::model::AirQuality, ctx: &RenderContext<'_>) -> String {
    let (value, category) = match ctx.aqi_index {
        AqiIndex::Us => (air.aqi_us, air.aqi_us.map(AqiCategory::from_us)),
        AqiIndex::European => (
            air.aqi_european,
            air.aqi_european.map(AqiCategory::from_european),
        ),
    };
    let Some(value) = value else {
        return n_a(ctx);
    };
    let label = ctx.i18n.text(&ctx.aqi_index.label_key());
    match category {
        Some(category) => format!("{label} {value} ({})", ctx.i18n.text(&category.i18n_key())),
        None => format!("{label} {value}"),
    }
}

/// The `n/a` a token prints when its value is unknown, in the report's language.
fn n_a(ctx: &RenderContext<'_>) -> String {
    ctx.i18n.text(&keys::NA).into_owned()
}

/// `HH:MM` at the instant's own offset.
fn clock_time(at: DateTime<FixedOffset>) -> String {
    at.format("%H:%M").to_string()
}

#[cfg(test)]
mod tests {
    use super::{
        MAX_PRECISION, PRESETS, Spec, TOKENS, Token, builtin_or_configured, parse, preset,
        resolve_template, token, warnings,
    };
    use std::collections::BTreeMap;

    #[test]
    fn the_token_table_binds_every_documented_letter() {
        let expected = [
            ('c', Token::ConditionArt),
            ('C', Token::ConditionText),
            ('x', Token::ConditionPlain),
            ('t', Token::Temp),
            ('f', Token::FeelsLike),
            ('H', Token::High),
            ('L', Token::Low),
            ('w', Token::Wind),
            ('h', Token::Humidity),
            ('p', Token::Precip),
            ('P', Token::Pressure),
            ('e', Token::DewPoint),
            ('u', Token::Uv),
            ('U', Token::UvBand),
            ('m', Token::Moon),
            ('M', Token::MoonPhase),
            ('v', Token::Visibility),
            ('l', Token::Location),
            ('d', Token::Date),
            ('D', Token::DateLong),
            ('T', Token::Time),
            ('Z', Token::TzName),
            ('z', Token::TzOffset),
            ('S', Token::Sunrise),
            ('s', Token::Sunset),
            ('A', Token::Alert),
            ('q', Token::Quality),
        ];
        assert_eq!(TOKENS.len(), expected.len());
        for (letter, token_value) in expected {
            assert_eq!(token(letter), Some(token_value), "%{letter}");
        }
        // The letters are unique and none is a character the specifier syntax uses.
        let mut letters: Vec<char> = TOKENS.iter().map(|spec| spec.letter).collect();
        letters.sort_unstable();
        letters.dedup();
        assert_eq!(letters.len(), TOKENS.len());
        assert!(letters.iter().all(|letter| !super::spec_char(*letter)));
    }

    #[test]
    fn a_width_specifier_is_read_off_a_token() {
        let pieces = parse("%-12C|%05.1t|%5w");
        let specs: Vec<Spec> = pieces
            .into_iter()
            .filter_map(|piece| match piece {
                super::Piece::Token { spec, .. } => Some(spec),
                _ => None,
            })
            .collect();
        assert_eq!(
            specs,
            vec![
                Spec {
                    width: Some(12),
                    left: true,
                    zero: false,
                    precision: None,
                },
                Spec {
                    width: Some(5),
                    left: false,
                    zero: true,
                    precision: Some(1),
                },
                Spec {
                    width: Some(5),
                    left: false,
                    zero: false,
                    precision: None,
                },
            ]
        );
    }

    #[test]
    fn precision_is_capped() {
        let Some(super::Piece::Token { spec, .. }) = parse("%.99t").into_iter().next() else {
            unreachable!("a token piece");
        };
        assert_eq!(spec.precision, Some(MAX_PRECISION));
    }

    #[test]
    fn a_braced_single_letter_is_a_token_and_anything_else_is_text() {
        let pieces = parse("%{c}%{%}%{Clear}%{}");
        assert!(matches!(
            pieces.first(),
            Some(super::Piece::Token {
                token: Token::ConditionArt,
                ..
            })
        ));
        let text: String = pieces
            .iter()
            .filter_map(|piece| match piece {
                super::Piece::Text(text) => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(text, "%Clear");
    }

    #[test]
    fn a_trailing_percent_or_incomplete_specifier_is_literal() {
        let pieces = parse("100%");
        let text: String = pieces
            .iter()
            .filter_map(|piece| match piece {
                super::Piece::Text(text) => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(text, "100%");

        let pieces = parse("%12");
        let text: String = pieces
            .iter()
            .filter_map(|piece| match piece {
                super::Piece::Text(text) => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(text, "%12");
    }

    #[test]
    fn an_unknown_token_keeps_its_specifier_literal() {
        let pieces = parse("%12y");
        let super::Piece::Unknown { raw, position, .. } = &pieces[0] else {
            unreachable!("an unknown token piece");
        };
        assert_eq!(raw, "%12y");
        assert_eq!(*position, 1);
        assert_eq!(warnings("%12y").len(), 1);
    }

    #[test]
    fn a_preset_is_a_template_and_the_configured_table_extends_it() {
        assert_eq!(preset("full"), Some(PRESETS[3].1));
        assert_eq!(preset("minimal"), Some("%c%t"));
        let none = BTreeMap::new();
        assert_eq!(
            resolve_template(None, &none).expect("the default preset"),
            PRESETS[0].1
        );
        assert_eq!(
            resolve_template(Some("@minimal"), &none).expect("a known preset"),
            "%c%t"
        );
        assert_eq!(
            resolve_template(Some("%c %l"), &none).expect("a literal template"),
            "%c %l"
        );
        let error = resolve_template(Some("@nope"), &none).expect_err("never a preset");
        assert!(error.to_string().contains("@default"), "{error}");

        let mut configured = BTreeMap::new();
        configured.insert("compact".to_owned(), "%c%t".to_owned());
        assert_eq!(
            resolve_template(Some("@compact"), &configured).expect("a configured preset"),
            "%c%t"
        );
        assert_eq!(builtin_or_configured("compact", &configured), Some("%c%t"));
        // A built-in name wins over a configured one of the same name.
        configured.insert("full".to_owned(), "%l".to_owned());
        assert_eq!(builtin_or_configured("full", &configured), preset("full"));
    }

    #[test]
    fn an_empty_template_is_a_usage_error() {
        let none = BTreeMap::new();
        let error = resolve_template(Some("   "), &none).expect_err("empty");
        assert_eq!(error.exit_code(), 2);
    }
}
