// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The `one-line` format: a single line driven by `%` tokens, wttr.in-compatible.
//!
//! This is the format a prompt, a status bar or a shell function consumes, so it is a *template*
//! language rather than a fixed sentence: `--template '%c %t'` prints the condition art and the
//! temperature, `%l` the place, and a preset (`@full`) is nothing but a longer template.
//!
//! # Tokens
//!
//! | token | output | token | output |
//! |---|---|---|---|
//! | `%c` | condition art, day/night aware | `%d` `%D` | ISO date / `Wed 30 Sep` |
//! | `%C` | condition text | `%Z` `%z` | time zone name / `+0800` |
//! | `%t` `%f` | temp / feels-like | `%u` `%U` | UV `5` / `5 (moderate)` |
//! | `%w` | wind `↗ 12km/h NE` | `%S` `%s` | sunrise / sunset `06:05` |
//! | `%h` | humidity `56%` | `%l` `%L` | name / `39.90,116.40` |
//! | `%p` | precipitation `0.0mm` | `%m` | moon phase — `n/a` until the moon step lands |
//! | `%P` | pressure `1013hPa` | `%v` | visibility `10km` |
//! | `%q` | air-quality index `US AQI 43 (Good)` | `%A` | strongest alert event |
//!
//! `%A` is the strongest alert's event name and the empty string when there are no alerts; a
//! report with alerts also gets the banner lines above the one-liner. `%q` is the air-quality
//! index of the selected scale (`--aqi-index`/`[air] index`), e.g. `US AQI 43 (Good)`; without an
//! air reading it prints `n/a`, like every other unknown value.
//!
//! A token whose value the provider does not report prints `n/a`; it is never invented, and it is
//! never a zero. Values come from [`crate::model::units`] like every other renderer's, so a unit
//! conversion happens exactly once in this crate.
//!
//! # Escapes
//!
//! * `%%` is a literal `%`, and a trailing lone `%` is one too;
//! * `\n`, `\t` and `\\` are unescaped before the tokens are read, so a template can span lines;
//! * `%{<text>}` prints `<text>` verbatim — no token expansion — and `\}` escapes a closing brace
//!   inside it;
//! * an unknown `%X` prints literally as `%X` and is reported once per occurrence by
//!   [`warnings`], which the CLI prints under `-v` (a renderer never consults the verbosity flags).

use std::fmt::Write as _;

use chrono::{DateTime, FixedOffset, Timelike as _};

use super::{RenderContext, Renderer};
use crate::air::aqi::{AqiCategory, AqiIndex};
use crate::error::{Error, Result};
use crate::i18n::{DateStyle, keys};
use crate::model::units::{
    UnitStyle, fmt_int, format_precip, format_pressure, format_temp_signed, format_visibility,
    format_wind,
};
use crate::model::{Condition, DayForecast, Location, Report};
use crate::render::art;

/// The template used when `--template` is not given.
pub const DEFAULT_PRESET: &str = "default";

/// The named templates `--template @name` accepts, in help order.
pub const PRESETS: [(&str, &str); 5] = [
    ("default", "%l: %c %C %t (%f), %w, %h, %p, %P, %v"),
    ("short", "%c %t"),
    ("full", "%l: %c %C %t (%f) %w %h %p %P %m %v %u %S %s %Z"),
    ("uv", "%l: UV %U"),
    ("sun", "%l: sunrise %S sunset %s (%z %Z)"),
];

/// The preset list as it appears in `--help` and in the unknown-preset error.
pub fn preset_help() -> String {
    PRESETS
        .iter()
        .map(|(name, template)| format!("  @{name:<8} {template}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Resolves a `--template` value: `None` is the default preset, `@name` a named one, anything else
/// a template of its own.
///
/// An unknown preset lists the presets instead of guessing, and an empty or whitespace-only
/// template is a usage error rather than an empty output line.
pub fn resolve_template(spec: Option<&str>) -> Result<String> {
    let Some(spec) = spec else {
        return Ok(preset(DEFAULT_PRESET).unwrap_or_default().to_owned());
    };
    if let Some(name) = spec.trim_start().strip_prefix('@') {
        let name = name.trim();
        return preset(name).map(str::to_owned).ok_or_else(|| {
            Error::Usage(format!(
                "unknown one-line preset `@{name}`; known presets:\n{}",
                preset_help()
            ))
        });
    }
    if spec.trim().is_empty() {
        return Err(Error::Usage(
            "the one-line template is empty; pass --template <TEMPLATE|@PRESET>".to_owned(),
        ));
    }
    Ok(spec.to_owned())
}

/// The template of a named preset.
#[must_use]
pub fn preset(name: &str) -> Option<&'static str> {
    PRESETS
        .iter()
        .find(|(preset, _)| *preset == name)
        .map(|(_, template)| *template)
}

/// The renderer for the `one-line` format.
#[derive(Debug, Clone)]
pub struct OneLine {
    template: String,
}

impl OneLine {
    /// A renderer for `template`, which is already resolved ([`resolve_template`]).
    #[must_use]
    pub fn new(template: impl Into<String>) -> Self {
        Self {
            template: template.into(),
        }
    }
}

impl Renderer for OneLine {
    fn render(&self, report: &Report, ctx: &RenderContext<'_>) -> Result<String> {
        // The banner travels above the one-liner: it is data, not a credit, and `%A` alone would
        // drop the until/instruction details. The glyph follows the terminal's charset.
        let mut out = String::new();
        for line in super::alerts::banner(&report.alerts, ctx.term.charset(), ctx) {
            out.push_str(&line.text);
            out.push('\n');
        }
        out.push_str(&expand(&self.template, report, ctx)?);
        Ok(out)
    }
}

// ---------------------------------------------------------------------------------------------
// Tokens
// ---------------------------------------------------------------------------------------------

/// One `%` token of the template language.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Token {
    /// `%c` — the condition art, day or night.
    ConditionArt,
    /// `%C` — the condition text.
    ConditionText,
    /// `%t` — the temperature.
    Temp,
    /// `%f` — the apparent temperature.
    FeelsLike,
    /// `%w` — wind arrow, speed and direction.
    Wind,
    /// `%h` — relative humidity.
    Humidity,
    /// `%p` — precipitation.
    Precip,
    /// `%P` — pressure.
    Pressure,
    /// `%v` — visibility.
    Visibility,
    /// `%u` — the UV index.
    Uv,
    /// `%U` — the UV index with its band name.
    UvBand,
    /// `%d` — the date, ISO 8601.
    Date,
    /// `%D` — the date, `Wed 30 Sep`.
    DateLong,
    /// `%Z` — the time zone name.
    TzName,
    /// `%z` — the UTC offset, `+0800`.
    TzOffset,
    /// `%S` — sunrise.
    Sunrise,
    /// `%s` — sunset.
    Sunset,
    /// `%l` — the place name.
    Location,
    /// `%L` — the coordinates.
    Coordinates,
    /// `%m` — the moon phase, `n/a` until step 17.
    Moon,
    /// `%A` — the strongest alert's event, empty when there are none.
    Alert,
    /// `%q` — the air-quality index on the selected scale.
    Quality,
}

/// The token table: the one place a `%` letter is bound to a meaning.
///
/// Adding a token means adding a row here plus an arm in [`value`]; the tests walk this table, so
/// a row without a value is a compile error and a value without a row is unreachable.
const TOKENS: &[(char, Token)] = &[
    ('c', Token::ConditionArt),
    ('C', Token::ConditionText),
    ('t', Token::Temp),
    ('f', Token::FeelsLike),
    ('w', Token::Wind),
    ('h', Token::Humidity),
    ('p', Token::Precip),
    ('P', Token::Pressure),
    ('v', Token::Visibility),
    ('u', Token::Uv),
    ('U', Token::UvBand),
    ('d', Token::Date),
    ('D', Token::DateLong),
    ('Z', Token::TzName),
    ('z', Token::TzOffset),
    ('S', Token::Sunrise),
    ('s', Token::Sunset),
    ('l', Token::Location),
    ('L', Token::Coordinates),
    ('m', Token::Moon),
    ('A', Token::Alert),
    ('q', Token::Quality),
];

/// The token a `%` letter stands for.
#[must_use]
pub fn token(letter: char) -> Option<Token> {
    TOKENS
        .iter()
        .find(|(name, _)| *name == letter)
        .map(|(_, token)| *token)
}

// ---------------------------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------------------------

/// One piece of a parsed template.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Piece {
    /// Literal text, with the backslash escapes already applied.
    Text(String),
    /// A known token.
    Token(Token),
    /// An unknown `%X`, emitted verbatim; the second field is the 1-based character position of
    /// the `%` in the template, for the warning.
    Unknown(char, usize),
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
                    text.push_str(&braced(&mut chars));
                }
                Some(letter) => {
                    chars.next();
                    flush(&mut pieces, &mut text);
                    match token(letter) {
                        Some(token) => pieces.push(Piece::Token(token)),
                        None => pieces.push(Piece::Unknown(
                            letter,
                            template[..offset].chars().count() + 1,
                        )),
                    }
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

/// Flushes the pending literal text as its own piece, when there is any.
fn flush(pieces: &mut Vec<Piece>, text: &mut String) {
    if !text.is_empty() {
        pieces.push(Piece::Text(std::mem::take(text)));
    }
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
            Piece::Token(token) => out.push_str(&value(token, &snapshot, report, ctx)),
            Piece::Unknown(letter, _) => {
                out.push('%');
                out.push(letter);
            }
        }
    }
    Ok(out)
}

/// One warning per unknown `%X`, naming the token and its position.
///
/// Pure: it re-reads the same parse the expansion uses, so the CLI can print the warnings under
/// `-v` without the renderer ever seeing a verbosity flag.
#[must_use]
pub fn warnings(template: &str) -> Vec<String> {
    parse(template)
        .into_iter()
        .filter_map(|piece| match piece {
            Piece::Unknown(letter, position) => Some(format!(
                "note: unknown one-line token `%{letter}` at position {position} is printed literally"
            )),
            Piece::Text(_) | Piece::Token(_) => None,
        })
        .collect()
}

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
}

/// The forecast day the run's clock falls on, else the first day the report carries.
fn today(report: &Report, date: chrono::NaiveDate) -> Option<&DayForecast> {
    report
        .days
        .iter()
        .find(|day| day.date == date)
        .or_else(|| report.days.first())
}

/// The day part a local hour falls in, matching the aggregation the providers use.
fn hour_part(now: DateTime<FixedOffset>) -> crate::model::DayPartKind {
    use crate::model::DayPartKind;
    match now.hour() {
        6..=11 => DayPartKind::Morning,
        12..=17 => DayPartKind::Noon,
        18..=23 => DayPartKind::Evening,
        _ => DayPartKind::Night,
    }
}

/// Renders one token.
fn value(token: Token, snapshot: &Snapshot, report: &Report, ctx: &RenderContext<'_>) -> String {
    let units = ctx.units;
    match token {
        Token::ConditionArt => snapshot.condition.map_or_else(
            || n_a(ctx),
            |condition| {
                let key = condition.art_key();
                let key = if snapshot.is_day {
                    key
                } else {
                    art::night_variant(key)
                };
                art::one_line_art(key).to_owned()
            },
        ),
        Token::ConditionText => snapshot.condition.map_or_else(
            || n_a(ctx),
            |condition| ctx.i18n.condition(condition).into_owned(),
        ),
        Token::Temp => snapshot
            .temp_c
            .map_or_else(|| n_a(ctx), |temp| format_temp_signed(temp, units.temp)),
        Token::FeelsLike => snapshot
            .feels_like_c
            .map_or_else(|| n_a(ctx), |temp| format_temp_signed(temp, units.temp)),
        Token::Wind => snapshot.wind_kmh.map_or_else(
            || n_a(ctx),
            |kmh| {
                let speed = format_wind(kmh, units.wind, UnitStyle::Compact);
                // The direction is genuinely optional upstream — a calm `OpenWeatherMap` reading
                // and a METAR `VRB` both leave it out — and a known speed must not become `n/a`
                // for want of an arrow: the arrow-free speed is what the table and `plain` print.
                // With a direction, the documented order is arrow, speed, direction.
                match snapshot.wind_dir_deg {
                    Some(deg) => format!(
                        "{} {speed} {}",
                        art::wind_arrow(deg, ctx.term.charset()),
                        ctx.i18n.direction(deg)
                    ),
                    None => speed,
                }
            },
        ),
        Token::Humidity => snapshot
            .humidity_pct
            .map_or_else(|| n_a(ctx), |humidity| format!("{humidity}%")),
        Token::Precip => snapshot.precip_mm.map_or_else(
            || n_a(ctx),
            |mm| format_precip(mm, units.precip, UnitStyle::Compact),
        ),
        Token::Pressure => snapshot.pressure_hpa.map_or_else(
            || n_a(ctx),
            |hpa| format_pressure(hpa, units.pressure, UnitStyle::Compact),
        ),
        Token::Visibility => snapshot.visibility_km.map_or_else(
            || n_a(ctx),
            |km| format_visibility(km, units.distance, UnitStyle::Compact),
        ),
        Token::Uv => snapshot.uv_index.map_or_else(|| n_a(ctx), fmt_int),
        Token::UvBand => snapshot.uv_index.map_or_else(
            || n_a(ctx),
            |uv| {
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
            },
        ),
        Token::Date => ctx.i18n.format_date(ctx.now.date_naive(), DateStyle::Iso),
        Token::DateLong => ctx.i18n.format_date(ctx.now.date_naive(), DateStyle::Short),
        Token::TzName => ctx.tz.name().to_owned(),
        Token::TzOffset => ctx.now.format("%z").to_string(),
        Token::Sunrise => snapshot.sunrise.map_or_else(|| n_a(ctx), clock_time),
        Token::Sunset => snapshot.sunset.map_or_else(|| n_a(ctx), clock_time),
        Token::Location => report.location.name.clone(),
        Token::Coordinates => coordinates(&report.location),
        Token::Moon => ctx.i18n.text(&keys::MOON_NA).into_owned(),
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

/// `39.90,116.40`, the pair the location was resolved for.
fn coordinates(location: &Location) -> String {
    let mut text = String::new();
    let _ = write!(text, "{:.2},{:.2}", location.lat, location.lon);
    text
}

#[cfg(test)]
mod tests {
    use super::{PRESETS, Token, parse, preset, resolve_template, token, warnings};

    #[test]
    fn the_token_table_binds_every_documented_letter() {
        for (letter, expected) in [
            ('c', Token::ConditionArt),
            ('C', Token::ConditionText),
            ('t', Token::Temp),
            ('f', Token::FeelsLike),
            ('w', Token::Wind),
            ('h', Token::Humidity),
            ('p', Token::Precip),
            ('P', Token::Pressure),
            ('v', Token::Visibility),
            ('u', Token::Uv),
            ('U', Token::UvBand),
            ('d', Token::Date),
            ('D', Token::DateLong),
            ('Z', Token::TzName),
            ('z', Token::TzOffset),
            ('S', Token::Sunrise),
            ('s', Token::Sunset),
            ('l', Token::Location),
            ('L', Token::Coordinates),
            ('m', Token::Moon),
            ('A', Token::Alert),
            ('q', Token::Quality),
        ] {
            assert_eq!(token(letter), Some(expected), "%{letter}");
        }
        assert_eq!(token('Q'), None);
        assert_eq!(token('%'), None, "`%%` is an escape, not a token");
        assert_eq!(token('{'), None);
    }

    #[test]
    fn the_presets_are_the_documented_ones() {
        assert_eq!(
            preset("default"),
            Some("%l: %c %C %t (%f), %w, %h, %p, %P, %v")
        );
        assert_eq!(preset("short"), Some("%c %t"));
        assert_eq!(
            preset("full"),
            Some("%l: %c %C %t (%f) %w %h %p %P %m %v %u %S %s %Z")
        );
        assert_eq!(preset("uv"), Some("%l: UV %U"));
        assert_eq!(preset("sun"), Some("%l: sunrise %S sunset %s (%z %Z)"));
        assert_eq!(preset("nope"), None);
        assert_eq!(PRESETS.len(), 5);
    }

    #[test]
    fn a_template_spec_resolves_to_its_preset_or_itself() {
        assert_eq!(
            resolve_template(None).expect("the default preset"),
            preset("default").expect("the default preset exists")
        );
        assert_eq!(resolve_template(Some("@short")).expect("a preset"), "%c %t");
        assert_eq!(
            resolve_template(Some("@ short ")).expect("a trimmed preset"),
            "%c %t"
        );
        assert_eq!(
            resolve_template(Some("%c %t")).expect("a template of its own"),
            "%c %t"
        );

        let unknown = resolve_template(Some("@nope")).expect_err("never a preset");
        assert_eq!(unknown.exit_code(), 2);
        assert!(
            unknown
                .to_string()
                .contains("unknown one-line preset `@nope`")
        );
        assert!(unknown.to_string().contains("@default"));

        for empty in ["", "   ", "\t"] {
            let error = resolve_template(Some(empty)).expect_err("never empty");
            assert_eq!(error.exit_code(), 2, "{empty:?}");
            assert!(error.to_string().contains("template is empty"));
        }
    }

    #[test]
    fn parsing_folds_escapes_into_literal_text() {
        assert_eq!(
            parse("a%%b\\nc\\t\\\\d"),
            vec![super::Piece::Text("a%b\nc\t\\d".to_owned())]
        );
        assert_eq!(
            parse("a\\qb"),
            vec![super::Piece::Text("a\\qb".to_owned())],
            "an unknown backslash escape is literal"
        );
        assert_eq!(
            parse("%{100%% %c}"),
            vec![super::Piece::Text("100%% %c".to_owned())],
            "a braced run is verbatim, tokens and escapes included"
        );
        assert_eq!(
            parse("%{a\\}b}"),
            vec![super::Piece::Text("a}b".to_owned())],
            "a backslash escapes the closing brace"
        );
        assert_eq!(
            parse("%{open"),
            vec![super::Piece::Text("%{open".to_owned())],
            "an unterminated brace run is literal"
        );
        assert_eq!(
            parse("x%ty"),
            vec![
                super::Piece::Text("x".to_owned()),
                super::Piece::Token(Token::Temp),
                super::Piece::Text("y".to_owned()),
            ]
        );
        assert_eq!(
            parse("50%"),
            vec![super::Piece::Text("50%".to_owned())],
            "a trailing lone % is literal"
        );
    }

    #[test]
    fn unknown_tokens_are_reported_with_their_position() {
        assert_eq!(parse("%y"), vec![super::Piece::Unknown('y', 1)]);
        assert_eq!(
            parse("ab%yc%Z"),
            vec![
                super::Piece::Text("ab".to_owned()),
                super::Piece::Unknown('y', 3),
                super::Piece::Text("c".to_owned()),
                super::Piece::Token(Token::TzName),
            ]
        );
        // A space after `%` is unknown too, and the position counts characters, not bytes.
        assert_eq!(parse("温度 % x")[1], super::Piece::Unknown(' ', 4));
        assert_eq!(
            warnings("a %y b %y"),
            vec![
                "note: unknown one-line token `%y` at position 3 is printed literally".to_owned(),
                "note: unknown one-line token `%y` at position 8 is printed literally".to_owned(),
            ],
            "one warning per occurrence, each with its own position"
        );
        assert_eq!(warnings("%c %t %%"), Vec::<String>::new());
    }
}
