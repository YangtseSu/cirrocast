// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The condition art: one four-line block per art key, unicode and ASCII.
//!
//! The corpus is **re-authored** for this project from the WMO 4677 description of each phenomenon
//! — a sun with rays, a cloud that grows with the cloud cover, drops that get denser with the
//! precipitation, a bolt for thunder and bars for fog. No block, glyph or palette is copied from
//! `wego`, `wttr.in` or any other weather client.
//!
//! The keys are exactly the vocabulary [`Condition::art_key`] returns; this module never defines a
//! second `art_key`. The tests below make that a build-time fact in both directions: every code
//! `0..=99` must find a block, and every block must belong to a key the vocabulary can produce.
//!
//! Beside the blocks, two **icon sets** draw the same keys as one glyph each: [`EMOJI`] (Unicode
//! emoji) and [`NERD`] (the Weather Icons family inside a Nerd Font). [`draw`] resolves them
//! through an [`IconChain`] — the first set that carries a glyph for the key wins, the blocks
//! corpus is always last — and a [`Charset::Ascii`] run is always drawn with the blocks, because
//! an icon set cannot be spelled in 7-bit. Both sets carry every key and every moon phase (the
//! tests below assert it in both directions), so a glyph is never missing in a tested build; the
//! fall-through stays because a hand-built report or a new condition may ask for a key no set has.
//!
//! Geometry: [`ART_W`] display columns by [`ART_LINES`] lines. Lines are stored right-trimmed — the
//! renderer pads them — and a line never carries more than [`ART_W`] columns, so a cell that shows
//! them keeps its borders aligned even when the block is mostly empty. A glyph arrives as a single
//! sequence instead of four lines: [`Art::line`] centres it in the same cell, padded by its
//! *measured* width — never by an assumption about how wide a terminal draws it.
//!
//! [`Condition::art_key`]: crate::model::condition::Condition::art_key

use std::borrow::Cow;

use unicode_width::UnicodeWidthStr as _;

use super::{Charset, IconChain};
use crate::model::astro::MoonPhase;
use crate::model::units::compass_16;

/// The width of an art block in display columns.
pub const ART_W: usize = 7;

/// The height of an art block in lines.
pub const ART_LINES: usize = 4;

/// The lines a renderer falls back to when a key has no block. As unreachable as it is empty: the
/// vocabulary test below is what guarantees every key has one.
pub const NO_BLOCK: [&str; ART_LINES] = ["", "", "", ""];

/// What a block draws, which is also what colour it takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtStyle {
    /// A sun or a moon.
    Sun,
    /// A cloud, with or without precipitation behind it.
    Cloud,
    /// Liquid precipitation.
    Rain,
    /// Frozen precipitation.
    Snow,
    /// A thunderstorm.
    Thunder,
    /// Fog, mist, smoke, haze, dust or sand: anything that cuts visibility without falling.
    Fog,
    /// No phenomenon: the block for an undescribed code.
    Plain,
}

/// One four-line block in both character sets.
#[derive(Debug, Clone, Copy)]
pub struct Block {
    /// The unicode block, four lines, each at most [`ART_W`] columns.
    pub unicode: [&'static str; ART_LINES],
    /// The ASCII transcription of the same shape, for [`Charset::Ascii`] and `--format dumb`.
    pub ascii: [&'static str; ART_LINES],
    /// What the block draws.
    pub style: ArtStyle,
}

impl Block {
    /// The four lines in `charset`.
    #[must_use]
    pub const fn lines(&self, charset: Charset) -> [&'static str; ART_LINES] {
        match charset {
            Charset::Unicode => self.unicode,
            Charset::Ascii => self.ascii,
        }
    }
}

/// The corpus, sorted by key so [`art`] can binary search it.
///
/// Every clouded block shares one cloud — `╭───╮` / `(     )` / `╰───╯` in unicode, `.---.` /
/// `(     )` / `'---'` in ASCII — and differs in the fourth line, which carries the phenomenon:
/// drops for drizzle, strokes for rain, strokes with a `+` for freezing rain, stars for snow, a
/// bolt for thunder. That is what makes rain intensities read as one family.
const ART: &[(&str, Block)] = &[
    (
        "clear",
        Block {
            unicode: ["   \\│/", "  ─(●)─", "   /│\\", ""],
            ascii: ["   \\|/", "  -(o)-", "   /|\\", ""],
            style: ArtStyle::Sun,
        },
    ),
    (
        "clear-night",
        Block {
            unicode: ["  · * ·", "   (●)", "  * · *", ""],
            ascii: ["  . * .", "   (o)", "  * . *", ""],
            style: ArtStyle::Sun,
        },
    ),
    (
        "drizzle",
        Block {
            unicode: [" ╭───╮", "(     )", " ╰───╯", "  . ."],
            ascii: [" .---.", "(     )", " '---'", "  . ."],
            style: ArtStyle::Rain,
        },
    ),
    (
        "drizzle-dense",
        Block {
            unicode: [" ╭───╮", "(     )", " ╰───╯", " . . ."],
            ascii: [" .---.", "(     )", " '---'", " . . ."],
            style: ArtStyle::Rain,
        },
    ),
    (
        "drizzle-light",
        Block {
            unicode: [" ╭───╮", "(     )", " ╰───╯", "   ."],
            ascii: [" .---.", "(     )", " '---'", "   ."],
            style: ArtStyle::Rain,
        },
    ),
    (
        "dust",
        Block {
            unicode: [" · ─ ·", " ─ · ─", " · ─ ·", ""],
            ascii: [" . - .", " - . -", " . - .", ""],
            style: ArtStyle::Fog,
        },
    ),
    (
        "fog",
        Block {
            unicode: [" ─────", "  ─────", " ─────", ""],
            ascii: [" -----", "  -----", " -----", ""],
            style: ArtStyle::Fog,
        },
    ),
    (
        "freezing-drizzle-dense",
        Block {
            unicode: [" ╭───╮", "(     )", " ╰───╯", "  + +"],
            ascii: [" .---.", "(     )", " '---'", "  + +"],
            style: ArtStyle::Rain,
        },
    ),
    (
        "freezing-drizzle-light",
        Block {
            unicode: [" ╭───╮", "(     )", " ╰───╯", "   +"],
            ascii: [" .---.", "(     )", " '---'", "   +"],
            style: ArtStyle::Rain,
        },
    ),
    (
        "freezing-rain-heavy",
        Block {
            unicode: [" ╭───╮", "(     )", " ╰───╯", " /+/+"],
            ascii: [" .---.", "(     )", " '---'", " /+/+"],
            style: ArtStyle::Rain,
        },
    ),
    (
        "freezing-rain-light",
        Block {
            unicode: [" ╭───╮", "(     )", " ╰───╯", "  /+"],
            ascii: [" .---.", "(     )", " '---'", "  /+"],
            style: ArtStyle::Rain,
        },
    ),
    (
        "haze",
        Block {
            unicode: ["   \\│/", "  ─(●)─", " ─────", "  ─────"],
            ascii: ["   \\|/", "  -(o)-", " -----", "  -----"],
            style: ArtStyle::Fog,
        },
    ),
    (
        "ice-pellets",
        Block {
            unicode: [" ╭───╮", "(     )", " ╰───╯", "  o o"],
            ascii: [" .---.", "(     )", " '---'", "  o o"],
            style: ArtStyle::Snow,
        },
    ),
    (
        "mainly-clear",
        Block {
            unicode: ["   \\│/", "  ─(●)─", "   /│\\", "   ▁▁▁"],
            ascii: ["   \\|/", "  -(o)-", "   /|\\", "   ___"],
            style: ArtStyle::Sun,
        },
    ),
    (
        "mainly-clear-night",
        Block {
            unicode: ["  · * ·", "   (●)", "  * · *", "   ▁▁▁"],
            ascii: ["  . * .", "   (o)", "  * . *", "   ___"],
            style: ArtStyle::Sun,
        },
    ),
    (
        "mist",
        Block {
            unicode: [" ─ ─ ─", "  ─ ─ ─", " ─ ─ ─", ""],
            ascii: [" - - -", "  - - -", " - - -", ""],
            style: ArtStyle::Fog,
        },
    ),
    (
        "overcast",
        Block {
            unicode: [" ╭───╮", "╭╯   ╰╮", "(     )", "╰─────╯"],
            ascii: [" .---.", "/.   .\\", "(     )", "'-----'"],
            style: ArtStyle::Cloud,
        },
    ),
    (
        "partly-cloudy",
        Block {
            unicode: ["   \\│/", " ╭───╮", "(     )", " ╰───╯"],
            ascii: ["   \\|/", " .---.", "(     )", " '---'"],
            style: ArtStyle::Cloud,
        },
    ),
    (
        "partly-cloudy-night",
        Block {
            unicode: ["  * · *", " ╭───╮", "(     )", " ╰───╯"],
            ascii: ["  * . *", " .---.", "(     )", " '---'"],
            style: ArtStyle::Cloud,
        },
    ),
    (
        "rain",
        Block {
            unicode: [" ╭───╮", "(     )", " ╰───╯", "  / /"],
            ascii: [" .---.", "(     )", " '---'", "  / /"],
            style: ArtStyle::Rain,
        },
    ),
    (
        "rain-heavy",
        Block {
            unicode: [" ╭───╮", "(     )", " ╰───╯", "  ///"],
            ascii: [" .---.", "(     )", " '---'", "  ///"],
            style: ArtStyle::Rain,
        },
    ),
    (
        "rain-light",
        Block {
            unicode: [" ╭───╮", "(     )", " ╰───╯", "   /"],
            ascii: [" .---.", "(     )", " '---'", "   /"],
            style: ArtStyle::Rain,
        },
    ),
    (
        "rime-fog",
        Block {
            unicode: [" ──*──", "  ──*──", " ──*──", ""],
            ascii: [" --*--", "  --*--", " --*--", ""],
            style: ArtStyle::Fog,
        },
    ),
    (
        "sand",
        Block {
            unicode: [" ─ ─ ─", "  ─ ─ ─", " / /", "  / /"],
            ascii: [" - - -", "  - - -", " / /", "  / /"],
            style: ArtStyle::Fog,
        },
    ),
    (
        "showers-rain",
        Block {
            unicode: ["   \\│/", " ╭───╮", "(     )", "  / /"],
            ascii: ["   \\|/", " .---.", "(     )", "  / /"],
            style: ArtStyle::Rain,
        },
    ),
    (
        "showers-rain-light",
        Block {
            unicode: ["   \\│/", " ╭───╮", "(     )", "   /"],
            ascii: ["   \\|/", " .---.", "(     )", "   /"],
            style: ArtStyle::Rain,
        },
    ),
    (
        "showers-rain-violent",
        Block {
            unicode: ["   \\│/", " ╭───╮", "(     )", "  ///"],
            ascii: ["   \\|/", " .---.", "(     )", "  ///"],
            style: ArtStyle::Rain,
        },
    ),
    (
        "showers-snow-heavy",
        Block {
            unicode: ["   \\│/", " ╭───╮", "(     )", "  * *"],
            ascii: ["   \\|/", " .---.", "(     )", "  * *"],
            style: ArtStyle::Snow,
        },
    ),
    (
        "showers-snow-light",
        Block {
            unicode: ["   \\│/", " ╭───╮", "(     )", "   *"],
            ascii: ["   \\|/", " .---.", "(     )", "   *"],
            style: ArtStyle::Snow,
        },
    ),
    (
        "sleet-heavy",
        Block {
            unicode: [" ╭───╮", "(     )", " ╰───╯", "  ///**"],
            ascii: [" .---.", "(     )", " '---'", "  ///**"],
            style: ArtStyle::Snow,
        },
    ),
    (
        "sleet-light",
        Block {
            unicode: [" ╭───╮", "(     )", " ╰───╯", "   /*"],
            ascii: [" .---.", "(     )", " '---'", "   /*"],
            style: ArtStyle::Snow,
        },
    ),
    (
        "smoke",
        Block {
            // Three waves in every row, so the block fills all `ART_W` columns and its rows are
            // not ragged: a two-wave row measured five columns where its neighbours measured four
            // and drew the block off-centre in its own cell.
            unicode: ["  ∿ ∿ ∿", "  ∿ ∿ ∿", "  ∿ ∿ ∿", ""],
            ascii: ["  ~ ~ ~", "  ~ ~ ~", "  ~ ~ ~", ""],
            style: ArtStyle::Fog,
        },
    ),
    (
        "snow",
        Block {
            unicode: [" ╭───╮", "(     )", " ╰───╯", "  * *"],
            ascii: [" .---.", "(     )", " '---'", "  * *"],
            style: ArtStyle::Snow,
        },
    ),
    (
        "snow-grains",
        Block {
            unicode: [" ╭───╮", "(     )", " ╰───╯", "  · ·"],
            ascii: [" .---.", "(     )", " '---'", "  . ."],
            style: ArtStyle::Snow,
        },
    ),
    (
        "snow-heavy",
        Block {
            unicode: [" ╭───╮", "(     )", " ╰───╯", " * * *"],
            ascii: [" .---.", "(     )", " '---'", " * * *"],
            style: ArtStyle::Snow,
        },
    ),
    (
        "snow-light",
        Block {
            unicode: [" ╭───╮", "(     )", " ╰───╯", "   *"],
            ascii: [" .---.", "(     )", " '---'", "   *"],
            style: ArtStyle::Snow,
        },
    ),
    (
        "thunderstorm",
        Block {
            unicode: [" ╭───╮", "(     )", " ╰──╮", "  ╲╱"],
            ascii: [" .---.", "(     )", " '--.", "  \\/"],
            style: ArtStyle::Thunder,
        },
    ),
    (
        "thunderstorm-hail-heavy",
        Block {
            unicode: [" ╭───╮", "(     )", " ╰──╮", " ╲╱ oo"],
            ascii: [" .---.", "(     )", " '--.", " \\/ oo"],
            style: ArtStyle::Thunder,
        },
    ),
    (
        "thunderstorm-hail-light",
        Block {
            unicode: [" ╭───╮", "(     )", " ╰──╮", "  ╲╱ o"],
            ascii: [" .---.", "(     )", " '--.", "  \\/ o"],
            style: ArtStyle::Thunder,
        },
    ),
    (
        "thunderstorm-heavy",
        Block {
            unicode: [" ╭───╮", "(     )", " ╰──╮", " ╲╱╲"],
            ascii: [" .---.", "(     )", " '--.", " \\/\\"],
            style: ArtStyle::Thunder,
        },
    ),
    (
        "unknown",
        Block {
            unicode: [" · · ·", "·  ?  ·", " · · ·", ""],
            ascii: [" . . .", ".  ?  .", " . . .", ""],
            style: ArtStyle::Plain,
        },
    ),
];

// ---------------------------------------------------------------------------------------------
// The moon
// ---------------------------------------------------------------------------------------------

/// One moon block: the four-line disc for each charset and the one-line `%m` glyphs.
///
/// The disc is drawn with `█` for the lit limb and `░`/`▒` for the dark one and its terminator —
/// re-authored here from the phase geometry, like every other block in this module, never copied.
/// The ASCII fallback is a four-column transcription of the same silhouette, and the glyphs are
/// the single-column form the `one-line` `%m` token prints.
#[derive(Debug, Clone, Copy)]
pub struct MoonBlock {
    /// The unicode block, four lines, each at most [`ART_W`] columns.
    pub unicode: [&'static str; ART_LINES],
    /// The ASCII transcription of the same shape.
    pub ascii: [&'static str; ART_LINES],
    /// The single-column unicode glyph (`%m` on a UTF-8 terminal).
    pub glyph: &'static str,
    /// The single-column 7-bit glyph (`%m` on a dumb terminal).
    pub ascii_glyph: &'static str,
}

/// The eight moon blocks, in [`MoonPhase::ALL`] order — [`moon`] indexes them by
/// [`MoonPhase::index`].
///
/// The glyph set distinguishes six of the eight phases: `☽`/`☾` split the crescents and `◐`/`◑`
/// the quarters, while the two gibbous phases share `◕` (Unicode has no mirrored "all but one
/// quadrant" circle, and the four-line block and the phase name carry the direction). ASCII has
/// no glyph family at all, so it names the sides with punctuation: `(`/`)` for the crescents,
/// `[`/`]` for the quarters, `O` for a gibbous and `@` for the full disc.
const MOON_ART: [(&str, MoonBlock); 8] = [
    (
        "moon/new",
        MoonBlock {
            unicode: [" ░░░░░ ", "░░░░░░░", "░░░░░░░", " ░░░░░ "],
            ascii: [" .. ", "....", "....", " .. "],
            glyph: "○",
            ascii_glyph: ".",
        },
    ),
    (
        "moon/waxing-crescent",
        MoonBlock {
            unicode: [" ░░░▒█ ", "░░░░░▒█", "░░░░░▒█", " ░░░▒█ "],
            ascii: [" +# ", "..+#", "..+#", " +# "],
            glyph: "☽",
            ascii_glyph: "(",
        },
    ),
    (
        "moon/first-quarter",
        MoonBlock {
            unicode: [" ░░▒██ ", "░░▒████", "░░▒████", " ░░▒██ "],
            ascii: [" +# ", ".+##", ".+##", " +# "],
            glyph: "◑",
            ascii_glyph: "]",
        },
    ),
    (
        "moon/waxing-gibbous",
        MoonBlock {
            unicode: [" ▒████ ", "▒██████", "▒██████", " ▒████ "],
            ascii: [" +# ", "+###", "+###", " +# "],
            glyph: "◕",
            ascii_glyph: "O",
        },
    ),
    (
        "moon/full",
        MoonBlock {
            unicode: [" █████ ", "███████", "███████", " █████ "],
            ascii: [" ## ", "####", "####", " ## "],
            glyph: "●",
            ascii_glyph: "@",
        },
    ),
    (
        "moon/waning-gibbous",
        MoonBlock {
            unicode: [" ████▒ ", "██████▒", "██████▒", " ████▒ "],
            ascii: [" #+ ", "###+", "###+", " #+ "],
            glyph: "◕",
            ascii_glyph: "O",
        },
    ),
    (
        "moon/last-quarter",
        MoonBlock {
            unicode: [" ██▒░░ ", "████▒░░", "████▒░░", " ██▒░░ "],
            ascii: [" #+ ", "##+.", "##+.", " #+ "],
            glyph: "◐",
            ascii_glyph: "[",
        },
    ),
    (
        "moon/waning-crescent",
        MoonBlock {
            unicode: [" █▒░░░ ", "█▒░░░░░", "█▒░░░░░", " █▒░░░ "],
            ascii: [" #+ ", "#+..", "#+..", " #+ "],
            glyph: "☾",
            ascii_glyph: ")",
        },
    ),
];

/// The moon block of a phase.
///
/// Indexed by [`MoonPhase::index`], which the test below pins against [`MoonPhase::art_key`], so
/// the table order and the phase order cannot drift apart.
#[must_use]
pub fn moon(phase: MoonPhase) -> &'static MoonBlock {
    &MOON_ART[phase.index()].1
}

/// The four lines of a phase's block in `charset`.
#[must_use]
pub fn moon_lines(phase: MoonPhase, charset: Charset) -> [&'static str; ART_LINES] {
    let block = moon(phase);
    match charset {
        Charset::Unicode => block.unicode,
        Charset::Ascii => block.ascii,
    }
}

/// The one-column glyph of a phase's block in `charset`, for the `%m` token.
#[must_use]
fn moon_block_glyph(phase: MoonPhase, charset: Charset) -> &'static str {
    let block = moon(phase);
    match charset {
        Charset::Unicode => block.glyph,
        Charset::Ascii => block.ascii_glyph,
    }
}

// ---------------------------------------------------------------------------------------------
// Icon sets
// ---------------------------------------------------------------------------------------------

/// One glyph of an icon set: the sequence to print and the display columns it measures.
///
/// The width is *recorded* beside the sequence rather than measured on every draw so the corpus
/// reads as data; the test below re-measures every row with `unicode-width`, so the record cannot
/// drift from what the layout pads for. Emoji presentation is spelled out — a sequence Unicode did
/// not default to emoji presentation carries U+FE0F — because a terminal is free to draw the bare
/// codepoint as a one-column text glyph.
#[derive(Debug, Clone, Copy)]
pub struct Glyph {
    /// The sequence, ready to print.
    pub text: &'static str,
    /// The display columns it measures in `unicode-width`.
    pub width: usize,
}

/// One corpus row: the sequence and its measured width.
const fn icon(text: &'static str, width: usize) -> Glyph {
    Glyph { text, width }
}

/// The emoji corpus: one glyph per art key, sorted by key like [`ART`].
///
/// Two rules shape it. **An emoji that draws the sun is only used for a key that has a `-night`
/// sibling** — rain, snow, fog and wind look the same around the clock in the blocks corpus, and a
/// glyph with a sun on it would lie in the night column. And **a key family shares one glyph where
/// Unicode has no second one**: there is no drizzle-versus-rain, no snow-grain and no moon-behind-
/// cloud emoji, so the rain family draws `🌧️`, the freezing family `🧊`, the snow family `🌨️`;
/// the condition's own text and the blocks corpus carry the nuance. Emoji presentation is
/// explicit: `☀️`, `☁️`, `⛈️`, `🌤️`, `🌫️`, `🌬️`, `🌧️`, `🌨️`, `🌩️`, `🏜️` and `❄️` carry
/// U+FE0F, the rest are single codepoints that Unicode already defaults to emoji presentation.
const EMOJI: &[(&str, Glyph)] = &[
    ("clear", icon("☀️", 2)),
    ("clear-night", icon("🌙", 2)),
    ("drizzle", icon("💧", 2)),
    ("drizzle-dense", icon("🌧️", 2)),
    ("drizzle-light", icon("💧", 2)),
    ("dust", icon("🌬️", 2)),
    ("fog", icon("🌫️", 2)),
    ("freezing-drizzle-dense", icon("🧊", 2)),
    ("freezing-drizzle-light", icon("🧊", 2)),
    ("freezing-rain-heavy", icon("🧊", 2)),
    ("freezing-rain-light", icon("🧊", 2)),
    ("haze", icon("🌫️", 2)),
    ("ice-pellets", icon("🧊", 2)),
    ("mainly-clear", icon("🌤️", 2)),
    ("mainly-clear-night", icon("🌙", 2)),
    ("mist", icon("🌫️", 2)),
    ("overcast", icon("☁️", 2)),
    ("partly-cloudy", icon("⛅", 2)),
    ("partly-cloudy-night", icon("☁️", 2)),
    ("rain", icon("🌧️", 2)),
    ("rain-heavy", icon("🌧️", 2)),
    ("rain-light", icon("🌧️", 2)),
    ("rime-fog", icon("❄️", 2)),
    ("sand", icon("🏜️", 2)),
    ("showers-rain", icon("🌧️", 2)),
    ("showers-rain-light", icon("🌧️", 2)),
    ("showers-rain-violent", icon("⛈️", 2)),
    ("showers-snow-heavy", icon("🌨️", 2)),
    ("showers-snow-light", icon("🌨️", 2)),
    ("sleet-heavy", icon("🌨️", 2)),
    ("sleet-light", icon("🌨️", 2)),
    ("smoke", icon("💨", 2)),
    ("snow", icon("🌨️", 2)),
    ("snow-grains", icon("❄️", 2)),
    ("snow-heavy", icon("🌨️", 2)),
    ("snow-light", icon("❄️", 2)),
    ("thunderstorm", icon("⛈️", 2)),
    ("thunderstorm-hail-heavy", icon("🌩️", 2)),
    ("thunderstorm-hail-light", icon("🌩️", 2)),
    ("thunderstorm-heavy", icon("⛈️", 2)),
    ("unknown", icon("❓", 2)),
];

/// The Nerd Font corpus: the Weather Icons family (U+E300–U+E3E3) inside a patched font, sorted by
/// art key like [`ART`].
///
/// Every row names the `glyphnames.json` glyph it is, so the escape can be checked against the
/// upstream table; the test below pins the codepoints by name and proves none leaves the block.
/// The same two rules as the emoji set apply: a glyph that draws the sun is only used where the key
/// has a `-night` sibling (`weather-day_*` for those three, `weather-night_*` for their siblings),
/// and everything else draws a neutral glyph. Where Weather Icons has no gradation the family
/// shares one (`weather-rain` for both light and moderate rain, `weather-sleet` for the freezing
/// family) rather than inventing a glyph.
const NERD: &[(&str, Glyph)] = &[
    // weather-day_sunny
    ("clear", icon("\u{e30d}", 1)),
    // weather-night_clear
    ("clear-night", icon("\u{e32b}", 1)),
    // weather-raindrop
    ("drizzle", icon("\u{e371}", 1)),
    // weather-raindrops
    ("drizzle-dense", icon("\u{e34a}", 1)),
    // weather-sprinkle
    ("drizzle-light", icon("\u{e31b}", 1)),
    // weather-dust
    ("dust", icon("\u{e35d}", 1)),
    // weather-fog
    ("fog", icon("\u{e313}", 1)),
    // weather-rain_mix
    ("freezing-drizzle-dense", icon("\u{e316}", 1)),
    // weather-rain_mix
    ("freezing-drizzle-light", icon("\u{e316}", 1)),
    // weather-sleet
    ("freezing-rain-heavy", icon("\u{e3ad}", 1)),
    // weather-sleet
    ("freezing-rain-light", icon("\u{e3ad}", 1)),
    // weather-smog
    ("haze", icon("\u{e36d}", 1)),
    // weather-hail
    ("ice-pellets", icon("\u{e314}", 1)),
    // weather-day_sunny_overcast
    ("mainly-clear", icon("\u{e30c}", 1)),
    // weather-night_alt_partly_cloudy
    ("mainly-clear-night", icon("\u{e379}", 1)),
    // weather-fog
    ("mist", icon("\u{e313}", 1)),
    // weather-cloudy
    ("overcast", icon("\u{e312}", 1)),
    // weather-day_cloudy
    ("partly-cloudy", icon("\u{e302}", 1)),
    // weather-night_alt_cloudy
    ("partly-cloudy-night", icon("\u{e37e}", 1)),
    // weather-rain
    ("rain", icon("\u{e318}", 1)),
    // weather-rain_wind
    ("rain-heavy", icon("\u{e317}", 1)),
    // weather-rain
    ("rain-light", icon("\u{e318}", 1)),
    // weather-snowflake_cold
    ("rime-fog", icon("\u{e36f}", 1)),
    // weather-sandstorm
    ("sand", icon("\u{e37a}", 1)),
    // weather-showers
    ("showers-rain", icon("\u{e319}", 1)),
    // weather-showers
    ("showers-rain-light", icon("\u{e319}", 1)),
    // weather-storm_showers
    ("showers-rain-violent", icon("\u{e31c}", 1)),
    // weather-snow_wind
    ("showers-snow-heavy", icon("\u{e35e}", 1)),
    // weather-snow
    ("showers-snow-light", icon("\u{e31a}", 1)),
    // weather-sleet
    ("sleet-heavy", icon("\u{e3ad}", 1)),
    // weather-sleet
    ("sleet-light", icon("\u{e3ad}", 1)),
    // weather-smoke
    ("smoke", icon("\u{e35c}", 1)),
    // weather-snow
    ("snow", icon("\u{e31a}", 1)),
    // weather-snowflake_cold
    ("snow-grains", icon("\u{e36f}", 1)),
    // weather-snow_wind
    ("snow-heavy", icon("\u{e35e}", 1)),
    // weather-snow
    ("snow-light", icon("\u{e31a}", 1)),
    // weather-thunderstorm
    ("thunderstorm", icon("\u{e31d}", 1)),
    // weather-lightning
    ("thunderstorm-hail-heavy", icon("\u{e315}", 1)),
    // weather-lightning
    ("thunderstorm-hail-light", icon("\u{e315}", 1)),
    // weather-storm_showers
    ("thunderstorm-heavy", icon("\u{e31c}", 1)),
    // weather-na
    ("unknown", icon("\u{e374}", 1)),
];

/// The emoji moon glyphs, one per [`MoonPhase::ALL`] entry in its own order.
const EMOJI_MOON: &[(&str, Glyph)] = &[
    ("moon/first-quarter", icon("🌓", 2)),
    ("moon/full", icon("🌕", 2)),
    ("moon/last-quarter", icon("🌗", 2)),
    ("moon/new", icon("🌑", 2)),
    ("moon/waning-crescent", icon("🌘", 2)),
    ("moon/waning-gibbous", icon("🌖", 2)),
    ("moon/waxing-crescent", icon("🌒", 2)),
    ("moon/waxing-gibbous", icon("🌔", 2)),
];

/// The Nerd Font moon glyphs: `weather-moon_*`, the phase steps that sit in the middle of each
/// six-step run (`weather-moon_waxing_crescent_3` and friends) so the glyph reads as the phase it
/// names rather than as its first or last step.
const NERD_MOON: &[(&str, Glyph)] = &[
    // weather-moon_first_quarter
    ("moon/first-quarter", icon("\u{e394}", 1)),
    // weather-moon_full
    ("moon/full", icon("\u{e39b}", 1)),
    // weather-moon_third_quarter
    ("moon/last-quarter", icon("\u{e3a2}", 1)),
    // weather-moon_new
    ("moon/new", icon("\u{e38d}", 1)),
    // weather-moon_waning_crescent_3
    ("moon/waning-crescent", icon("\u{e3a5}", 1)),
    // weather-moon_waning_gibbous_3
    ("moon/waning-gibbous", icon("\u{e39e}", 1)),
    // weather-moon_waxing_crescent_3
    ("moon/waxing-crescent", icon("\u{e390}", 1)),
    // weather-moon_waxing_gibbous_3
    ("moon/waxing-gibbous", icon("\u{e397}", 1)),
];

/// One drawing of a key: the hand-drawn four lines, or one glyph from an icon set.
#[derive(Debug, Clone, Copy)]
pub enum Art {
    /// The four lines of the blocks corpus (or of a moon block), in `charset`.
    Lines([&'static str; ART_LINES]),
    /// One glyph of an icon set, drawn centred in the [`ART_W`] cell.
    Glyph(&'static Glyph),
    /// No set carried the key — not even the blocks: the empty drawing.
    Missing,
}

/// The line of the four-line cell an icon glyph is drawn on: the second one, so the glyph sits
/// beside the temperature in a day cell (and beside the illumination in the moon panel) instead of
/// over the label row.
pub const GLYPH_LINE: usize = 1;

impl Art {
    /// The drawing's line `index`, padded to the [`ART_W`] columns of the cell: a corpus line as it
    /// is stored (the renderer pads the rest), the glyph centred by its measured width, or `""`.
    #[must_use]
    pub fn line(&self, index: usize) -> Cow<'static, str> {
        match self {
            Self::Lines(lines) => lines
                .get(index)
                .map_or(Cow::Borrowed(""), |line| Cow::Borrowed(*line)),
            Self::Glyph(glyph) if index == GLYPH_LINE => Cow::Owned(centred(glyph)),
            Self::Glyph(_) | Self::Missing => Cow::Borrowed(""),
        }
    }
}

/// A glyph centred in the [`ART_W`] columns of a cell, so the table's geometry cannot move: the
/// padding comes from the glyph's *measured* width, never from an assumption about it.
fn centred(glyph: &Glyph) -> String {
    let left = ART_W.saturating_sub(glyph.width) / 2;
    let right = ART_W.saturating_sub(left + glyph.width);
    let mut line = String::with_capacity(glyph.text.len() + left + right);
    line.extend(std::iter::repeat_n(' ', left));
    line.push_str(glyph.text);
    line.extend(std::iter::repeat_n(' ', right));
    line
}

/// The two tables of one icon set: the condition corpus and the moon corpus, both sorted by key.
#[derive(Debug, Clone, Copy)]
struct Corpus {
    /// The condition table.
    art: &'static [(&'static str, Glyph)],
    /// The moon table, keyed by [`MoonPhase::art_key`].
    moon: &'static [(&'static str, Glyph)],
}

/// The corpus of `set`, or `None` for [`IconSet::Blocks`], whose drawing is the [`ART`] corpus.
///
/// [`IconSet::Blocks`]: super::IconSet::Blocks
const fn corpus(set: super::IconSet) -> Option<Corpus> {
    match set {
        super::IconSet::Blocks => None,
        super::IconSet::Emoji => Some(Corpus {
            art: EMOJI,
            moon: EMOJI_MOON,
        }),
        super::IconSet::Nerd => Some(Corpus {
            art: NERD,
            moon: NERD_MOON,
        }),
    }
}

/// Which corpus of an icon set a lookup asks for.
#[derive(Debug, Clone, Copy)]
enum Family {
    /// The condition corpus.
    Art,
    /// The moon corpus.
    Moon,
}

/// The first glyph, in chain order, that any icon set carries for `key` in `family`.
///
/// The chain always ends with `blocks`, which has no table here — it is the drawing underneath,
/// not a glyph source — so `None` means "the blocks corpus draws this one".
fn first_glyph(chain: IconChain, key: &str, family: Family) -> Option<&'static Glyph> {
    let mut tables: [&'static [(&'static str, Glyph)]; 2] = [&[], &[]];
    let mut len = 0;
    for set in chain.iter() {
        if let Some(corpus) = corpus(set) {
            tables[len] = match family {
                Family::Art => corpus.art,
                Family::Moon => corpus.moon,
            };
            len += 1;
        }
    }
    first_with(&tables[..len], key)
}

/// The glyph the first table that carries `key` holds.
///
/// Split out from [`first_glyph`] because this is the rule the test drives with *holed* tables:
/// the fall-through is proven by construction, not by hoping a set lacks a key.
fn first_with(tables: &[&'static [(&'static str, Glyph)]], key: &str) -> Option<&'static Glyph> {
    tables.iter().find_map(|table| find(table, key))
}

/// The glyph `table` carries for `key`, by binary search over the sorted keys.
///
/// `n/a` is a spelling of `unknown`, exactly as in [`art`].
fn find(table: &'static [(&'static str, Glyph)], key: &str) -> Option<&'static Glyph> {
    let key = if key == "n/a" { "unknown" } else { key };
    table
        .binary_search_by(|(candidate, _)| (*candidate).cmp(key))
        .ok()
        .and_then(|index| table.get(index))
        .map(|(_, glyph)| glyph)
}

/// The drawing of a condition `key` in the resolved chain: the first icon set that carries a glyph
/// wins, and the blocks corpus draws whatever is left (an unknown key included).
///
/// A [`Charset::Ascii`] run draws the blocks whatever the chain says: an icon set is UTF-8 art, and
/// a terminal that cannot draw UTF-8 — or a run that asked for `--format dumb` — must not see a
/// byte outside 7-bit.
#[must_use]
pub fn draw(key: &str, chain: IconChain, charset: Charset) -> Art {
    if let Some(glyph) = first_glyph(chain.for_charset(charset), key, Family::Art) {
        return Art::Glyph(glyph);
    }
    art(key).map_or(Art::Missing, |block| Art::Lines(block.lines(charset)))
}

/// The moon drawing of a phase: the set's moon glyph when the chain carries one, else the phase's
/// four-line block in `charset`.
#[must_use]
pub fn moon_art(phase: MoonPhase, chain: IconChain, charset: Charset) -> Art {
    if let Some(glyph) = first_glyph(chain.for_charset(charset), phase.art_key(), Family::Moon) {
        return Art::Glyph(glyph);
    }
    Art::Lines(moon_lines(phase, charset))
}

/// The compact glyph of a condition key — the set's glyph, or the 7-bit one-line art of the blocks.
///
/// This is what `%c` and the stacked layout print, where a four-line block or a seven-column cell
/// would not fit.
#[must_use]
pub fn glyph(key: &str, chain: IconChain, charset: Charset) -> &'static str {
    match draw(key, chain, charset) {
        Art::Glyph(glyph) => glyph.text,
        Art::Lines(_) | Art::Missing => one_line_art(key),
    }
}

/// The one-column moon glyph of a phase in the resolved chain, for the `%m` token and the `plain`
/// record: the set's glyph, or the block's own glyph in `charset`.
#[must_use]
pub fn moon_glyph(phase: MoonPhase, chain: IconChain, charset: Charset) -> &'static str {
    match moon_art(phase, chain, charset) {
        Art::Glyph(glyph) => glyph.text,
        Art::Lines(_) | Art::Missing => moon_block_glyph(phase, charset),
    }
}

/// The one-line glyph of a key, used where a whole block would not fit (the stacked narrow layout,
/// and `%c` in the `one-line` format).
///
/// Deliberately 7-bit: the glyph is shared by both charsets, so it can never be the reason a `dumb`
/// terminal sees a byte outside `0x20..=0x7e`.
#[must_use]
pub fn one_line_art(key: &str) -> &'static str {
    match if key == "n/a" { "unknown" } else { key } {
        "clear" => "\\o/",
        "clear-night" => "*o*",
        "mainly-clear" => "\\o_",
        "mainly-clear-night" => "*o_",
        "partly-cloudy" => "~o~",
        "partly-cloudy-night" | "snow" => "~*~",
        "overcast" => "~~~",
        "fog" => "===",
        "rime-fog" => "=*=",
        "mist" => "-.-",
        "haze" => "-o-",
        "smoke" => ",~,",
        "dust" => ".:.",
        "sand" => "/_/",
        "ice-pellets" => "o*o",
        "drizzle-light" => "~.~",
        "drizzle" => "~,~",
        "drizzle-dense" => ".,.",
        "freezing-drizzle-light" => "~.=",
        "freezing-drizzle-dense" => ".,=",
        "rain-light" => "~/~",
        "rain" => "~|~",
        "rain-heavy" => "/|/",
        "freezing-rain-light" | "snow-light" => "~+~",
        "freezing-rain-heavy" => "+|+",
        "snow-heavy" => "*|*",
        "snow-grains" => "...",
        "showers-rain-light" => "o,~",
        "showers-rain" => "o|~",
        "showers-rain-violent" => "o||",
        "showers-snow-light" => "o+~",
        "showers-snow-heavy" => "o**",
        "sleet-light" => "~/+",
        "sleet-heavy" => "/+*",
        "thunderstorm" => "~!~",
        "thunderstorm-hail-light" => "~!o",
        "thunderstorm-heavy" => "!|!",
        "thunderstorm-hail-heavy" => "!oo",
        _ => "???",
    }
}

/// The night sibling of a day key, or the key itself when night looks the same.
///
/// Only the three sky keys change at night: a clear, mainly clear or partly cloudy night must not
/// draw a daytime sun. Rain, snow, thunder and fog look the same around the clock.
#[must_use]
pub fn night_variant(key: &str) -> &str {
    match key {
        "clear" => "clear-night",
        "mainly-clear" => "mainly-clear-night",
        "partly-cloudy" => "partly-cloudy-night",
        other => other,
    }
}

/// The eight compass arrows, clockwise from north, with their 7-bit fallbacks.
///
/// The arrow points the way the label beside it reads: the direction the wind blows *from*, which
/// is what [`compass_16`] names. Rain and snow do not care, but a reader comparing `↗` with `NE`
/// does.
///
/// ASCII has only the two slashes for four diagonals, so the two westerly ones take punctuation
/// whose tail or head points the way the arrow does: `,` (south-west) and `` ` `` (north-west).
/// In the narrow ASCII cell the label is dropped, and an arrow that two opposite winds print
/// identically is no direction cue at all. No two marks below are equal, and none is a digit —
/// `1 6.0km/h` would read as a number beside the speed.
///
/// [`compass_16`]: crate::model::units::compass_16
const ARROWS: [(&str, &str); 8] = [
    ("↑", "^"),
    ("↗", "/"),
    ("→", ">"),
    ("↘", "\\"),
    ("↓", "v"),
    ("↙", ","),
    ("←", "<"),
    ("↖", "`"),
];

/// The arrow for a wind direction in degrees, in `charset`.
///
/// The sector is [`compass_16`]'s own: an arrow owns the point it is named for and that point's
/// counter-clockwise neighbour, so the arrow turns exactly where the cardinal label does and the
/// two can never name different half-sectors.
#[must_use]
pub fn wind_arrow(deg: u16, charset: Charset) -> &'static str {
    let sector = match compass_16(deg) {
        "N" | "NNW" => 0,
        "NNE" | "NE" => 1,
        "ENE" | "E" => 2,
        "ESE" | "SE" => 3,
        "SSE" | "S" => 4,
        "SSW" | "SW" => 5,
        "WSW" | "W" => 6,
        _ => 7,
    };
    ARROWS.get(sector).map_or("-", |arrow| match charset {
        Charset::Unicode => arrow.0,
        Charset::Ascii => arrow.1,
    })
}

/// The block for `key`, or `None` when the vocabulary has no such key.
///
/// `n/a` is accepted as a spelling of `unknown`, because that is what a caller with no condition at
/// all — a report without current conditions and without forecast days — will ask for.
///
/// The width invariant [`ART_W`] documents is enforced here as well as by the test below: the
/// array's arity pins the number of lines, never their width, so a mis-measured block would
/// otherwise only show up as a metric column that shifts left in a release build.
#[must_use]
pub fn art(key: &str) -> Option<&'static Block> {
    let key = if key == "n/a" { "unknown" } else { key };
    let block = ART
        .binary_search_by(|(candidate, _)| candidate.cmp(&key))
        .ok()
        .and_then(|index| ART.get(index))
        .map(|(_, block)| block);
    debug_assert!(
        block.is_none_or(|block| [Charset::Unicode, Charset::Ascii]
            .into_iter()
            .flat_map(|charset| block.lines(charset))
            .all(|line| line.width() <= ART_W)),
        "the block for `{key}` has a line wider than ART_W = {ART_W}"
    );
    block
}

#[cfg(test)]
mod tests {
    use unicode_width::UnicodeWidthStr as _;

    use super::{
        ARROWS, ART, ART_LINES, ART_W, Art, ArtStyle, EMOJI, EMOJI_MOON, GLYPH_LINE, Glyph,
        MOON_ART, NERD, NERD_MOON, NO_BLOCK, art, draw, find, first_with, glyph, icon, moon,
        moon_art, moon_glyph, moon_lines, night_variant, one_line_art, wind_arrow,
    };
    use crate::model::astro::MoonPhase;
    use crate::model::condition::Condition;
    use crate::model::units::compass_16;
    use crate::render::{Charset, IconChain, IconSet};

    /// Every key the vocabulary can produce.
    fn vocabulary() -> Vec<&'static str> {
        let mut keys: Vec<&'static str> = (0..=99_u8)
            .map(|code| Condition::from_u8(code).art_key())
            .collect();
        keys.sort_unstable();
        keys.dedup();
        keys
    }

    #[test]
    fn every_condition_code_has_a_block_and_a_glyph() {
        for code in 0..=99_u8 {
            let key = Condition::from_u8(code).art_key();
            assert!(
                art(key).is_some(),
                "code {code} maps to art key `{key}`, which has no block"
            );
            // `one_line_art` ends in a `???` catch-all, so comparing against `""` could never
            // fail. A code with a described art key must get a real glyph; the undescribed codes
            // share the `unknown` key and keep the placeholder.
            let glyph = one_line_art(key);
            if key == "unknown" {
                assert_eq!(glyph, "???", "the undescribed code keeps the placeholder");
            } else {
                assert_ne!(
                    glyph, "???",
                    "code {code} maps to art key `{key}`, which has no one-line glyph"
                );
            }
        }
    }

    #[test]
    fn the_table_is_sorted_and_total_in_both_directions() {
        for window in ART.windows(2) {
            let (left, right) = (window[0], window[1]);
            assert!(left.0 < right.0, "keys must ascend: {left:?} {right:?}");
        }

        let vocabulary = vocabulary();
        for (key, _) in ART {
            let known = vocabulary.contains(key)
                || *key == "unknown"
                || key
                    .strip_suffix("-night")
                    .is_some_and(|day| vocabulary.contains(&day));
            assert!(known, "`{key}` is not in the condition vocabulary");
        }
        for key in &vocabulary {
            assert!(art(key).is_some(), "`{key}` has no block");
        }
    }

    #[test]
    fn blocks_are_four_lines_of_at_most_art_width() {
        assert_eq!(NO_BLOCK.len(), ART_LINES);
        for (key, block) in ART {
            for (index, (unicode, ascii)) in block.unicode.iter().zip(block.ascii).enumerate() {
                let (unicode_w, ascii_w) = (unicode.width(), ascii.width());
                assert!(
                    unicode_w <= ART_W,
                    "{key}: line {index} of unicode is {unicode_w} columns wide: {unicode:?}"
                );
                assert!(
                    ascii_w <= ART_W,
                    "{key}: line {index} of ASCII is {ascii_w} columns wide: {ascii:?}"
                );
                assert_eq!(
                    unicode_w, ascii_w,
                    "{key}: line {index} measures {unicode_w} columns in unicode but {ascii_w} in \
                     ASCII, so the fallback draws another shape"
                );
            }
        }
    }

    /// Every row of a block is either empty or as wide as the block; a row of its own width draws
    /// the block off-centre in its cell, which is what made `smoke`'s metrics look staggered.
    #[test]
    fn the_smoke_block_fills_the_art_width() {
        let smoke = art("smoke").expect("the smoke block exists");
        for charset in [Charset::Unicode, Charset::Ascii] {
            for (index, line) in smoke.lines(charset).iter().enumerate() {
                assert!(
                    line.is_empty() || line.width() == ART_W,
                    "smoke: line {index} of {charset:?} is {} columns wide: {line:?}",
                    line.width()
                );
            }
        }
    }

    /// The eight ASCII arrows must be pairwise distinct: the narrow ASCII cell drops the cardinal
    /// label, so two opposite winds printing the same mark are indistinguishable.
    #[test]
    fn the_eight_ascii_arrows_are_pairwise_distinct() {
        let mut marks: Vec<&str> = ARROWS.iter().map(|(_, ascii)| *ascii).collect();
        assert_eq!(marks.len(), 8);
        marks.sort_unstable();
        marks.dedup();
        assert_eq!(
            marks.len(),
            ARROWS.len(),
            "two ASCII arrows share a mark: {marks:?}"
        );
        for (unicode, ascii) in ARROWS {
            assert!(
                ascii.is_ascii() && ascii.len() == 1,
                "the fallback of {unicode} is {ascii:?}, not one 7-bit column"
            );
        }
    }

    /// The arrow may only turn where [`compass_16`] does: the two read one table, so a wind can
    /// never print `↗` beside a label the arrow's own sector does not name.
    #[test]
    fn the_wind_arrow_only_turns_where_the_compass_does() {
        let mut arrow = wind_arrow(0, Charset::Unicode);
        let mut label = compass_16(0);
        for deg in 1..=360_u16 {
            let (next_arrow, next_label) = (wind_arrow(deg, Charset::Unicode), compass_16(deg));
            if next_arrow != arrow {
                assert_ne!(
                    next_label, label,
                    "the arrow turned at {deg}° while the label stayed {label}"
                );
            }
            arrow = next_arrow;
            label = next_label;
        }
        // The boundary is a `compass_16` edge: N keeps the north arrow, NNE takes the NE one.
        assert_eq!(wind_arrow(11, Charset::Unicode), "↑");
        assert_eq!(wind_arrow(12, Charset::Unicode), "↗");
        assert_eq!(compass_16(11), "N");
        assert_eq!(compass_16(12), "NNE");
    }

    #[test]
    fn the_ascii_blocks_are_seven_bit_and_never_empty() {
        for (key, block) in ART {
            for line in block.ascii {
                assert!(
                    line.is_ascii(),
                    "{key}: the ASCII block line {line:?} needs a unicode terminal"
                );
            }
            assert!(
                block
                    .lines(Charset::Ascii)
                    .iter()
                    .any(|line| !line.is_empty()),
                "{key}: an all-empty ASCII block is not a fallback"
            );
            assert!(
                block
                    .lines(Charset::Unicode)
                    .iter()
                    .any(|line| !line.is_empty()),
                "{key}: an all-empty unicode block is not a fallback"
            );
        }
    }

    #[test]
    fn the_wind_arrow_follows_the_compass() {
        use crate::model::units::compass_16;
        assert_eq!(wind_arrow(0, Charset::Unicode), "↑");
        assert_eq!(wind_arrow(45, Charset::Unicode), "↗");
        assert_eq!(wind_arrow(90, Charset::Unicode), "→");
        assert_eq!(wind_arrow(180, Charset::Unicode), "↓");
        assert_eq!(wind_arrow(270, Charset::Unicode), "←");
        assert_eq!(wind_arrow(315, Charset::Unicode), "↖");
        assert_eq!(wind_arrow(359, Charset::Unicode), "↑");
        assert_eq!(wind_arrow(360, Charset::Unicode), "↑");
        assert!(wind_arrow(135, Charset::Unicode).starts_with('↘'));

        for deg in (0..=360).step_by(15) {
            let arrow = wind_arrow(deg, Charset::Ascii);
            assert!(arrow.is_ascii() && arrow.len() == 1, "{deg}: {arrow:?}");
        }
        // The arrow and the cardinal label name the same sector.
        for deg in [0_u16, 45, 90, 135, 180, 225, 270, 315] {
            let label = compass_16(deg);
            let arrow = wind_arrow(deg, Charset::Unicode);
            assert_ne!(label, "", "{deg}");
            assert_ne!(arrow, "", "{deg}");
        }
    }

    #[test]
    fn the_corpus_covers_every_style() {
        let mut styles: Vec<ArtStyle> = ART.iter().map(|(_, block)| block.style).collect();
        styles.sort_by_key(|style| format!("{style:?}"));
        styles.dedup();
        assert_eq!(styles.len(), 7, "every style must be exercised: {styles:?}");
    }

    #[test]
    fn night_variants_exist_and_leave_the_rest_alone() {
        for key in vocabulary() {
            let night = night_variant(key);
            assert!(art(night).is_some(), "`{key}` has no night block `{night}`");
            assert!(
                night == key || night == format!("{key}-night"),
                "`{night}` is not a sibling of `{key}`"
            );
        }
        assert_eq!(night_variant("clear"), "clear-night");
        assert_eq!(night_variant("mainly-clear"), "mainly-clear-night");
        assert_eq!(night_variant("partly-cloudy"), "partly-cloudy-night");
        assert_eq!(night_variant("rain"), "rain");
        assert_eq!(night_variant("unknown"), "unknown");
    }

    #[test]
    fn n_a_is_a_spelling_of_unknown() {
        let unknown = art("unknown").expect("the unknown block exists");
        let n_a = art("n/a").expect("n/a is the unknown block");
        assert_eq!(n_a.unicode, unknown.unicode);
        assert_eq!(one_line_art("n/a"), "???");
        assert!(art("no-such-key").is_none());
    }

    #[test]
    fn the_glyphs_are_short_ascii() {
        for key in vocabulary() {
            let glyph = one_line_art(key);
            assert!(glyph.is_ascii(), "`{key}`: {glyph:?} is not ASCII");
            assert!(
                glyph.width() <= 3,
                "`{key}`: {glyph:?} is {} columns wide",
                glyph.width()
            );
        }
    }

    #[test]
    fn every_phase_has_a_moon_block_with_the_documented_shape() {
        for phase in MoonPhase::ALL {
            let block = moon(phase);
            assert_eq!(
                MOON_ART[phase.index()].0,
                phase.art_key(),
                "{phase:?} is not in its own table row"
            );
            for (index, (unicode, ascii)) in block.unicode.iter().zip(block.ascii).enumerate() {
                let (unicode_w, ascii_w) = (unicode.width(), ascii.width());
                assert!(
                    unicode_w <= ART_W && ascii_w <= ART_W,
                    "{phase:?}: line {index} is wider than ART_W"
                );
                assert!(
                    unicode_w > 0 && ascii_w > 0,
                    "{phase:?}: line {index} is empty in one charset"
                );
                assert!(
                    ascii.is_ascii(),
                    "{phase:?}: {ascii:?} needs a unicode terminal"
                );
            }
            for glyph in [block.glyph, block.ascii_glyph] {
                assert_eq!(
                    glyph.width(),
                    1,
                    "{phase:?}: the glyph {glyph:?} is not one column"
                );
                assert_ne!(glyph, "");
            }
            assert!(block.ascii_glyph.is_ascii(), "{phase:?}");
        }
        assert_eq!(MOON_ART.len(), MoonPhase::ALL.len());
    }

    /// A full disc is all lit, a new one all dark, and the two gibbous rows are the mirrors of
    /// each other: the geometry a reader relies on cannot silently invert.
    #[test]
    fn the_moon_blocks_draw_the_phase_they_name() {
        let lit = |phase: MoonPhase| {
            let lines = moon_lines(phase, Charset::Unicode);
            lines
                .iter()
                .map(|line| line.chars().filter(|c| *c == '█').count())
                .sum::<usize>()
        };
        assert_eq!(lit(MoonPhase::New), 0);
        assert_eq!(lit(MoonPhase::Full), 24);
        assert_eq!(lit(MoonPhase::FirstQuarter), lit(MoonPhase::LastQuarter));
        assert_eq!(
            lit(MoonPhase::WaxingCrescent),
            lit(MoonPhase::WaningCrescent)
        );
        assert_eq!(lit(MoonPhase::WaxingGibbous), lit(MoonPhase::WaningGibbous));
        for (earlier, later) in [
            (MoonPhase::New, MoonPhase::WaxingCrescent),
            (MoonPhase::WaxingCrescent, MoonPhase::FirstQuarter),
            (MoonPhase::FirstQuarter, MoonPhase::WaxingGibbous),
            (MoonPhase::WaxingGibbous, MoonPhase::Full),
        ] {
            assert!(
                lit(earlier) < lit(later),
                "{earlier:?} is not dimmer than {later:?}"
            );
        }
        // The lit side follows the direction: waxing on the right, waning on the left.
        let edge_lit = |phase: MoonPhase, right: bool| {
            moon_lines(phase, Charset::Unicode).iter().any(|line| {
                let characters: Vec<char> = line.chars().collect();
                if right {
                    characters.iter().rev().take(2).any(|c| *c == '█')
                } else {
                    characters.iter().take(2).any(|c| *c == '█')
                }
            })
        };
        assert!(edge_lit(MoonPhase::WaxingCrescent, true));
        assert!(!edge_lit(MoonPhase::WaxingCrescent, false));
        assert!(edge_lit(MoonPhase::WaningCrescent, false));
        assert!(!edge_lit(MoonPhase::WaningCrescent, true));
    }

    #[test]
    fn the_moon_glyphs_are_charset_specific() {
        for phase in crate::model::astro::MoonPhase::ALL {
            let unicode = moon_glyph(phase, IconChain::blocks(), Charset::Unicode);
            let ascii = moon_glyph(phase, IconChain::blocks(), Charset::Ascii);
            assert!(ascii.is_ascii(), "{phase:?}: {ascii:?}");
            assert!(!unicode.is_ascii(), "{phase:?}: {unicode:?} is not unicode");
            assert_eq!(ascii.len(), 1);
        }
    }

    // -----------------------------------------------------------------------------------------
    // Icon sets
    // -----------------------------------------------------------------------------------------

    /// One icon set with its condition and moon tables.
    type SetTables = (
        IconSet,
        &'static [(&'static str, Glyph)],
        &'static [(&'static str, Glyph)],
    );

    /// The icon sets with their tables, for the tests that walk both.
    fn sets() -> [SetTables; 2] {
        [
            (IconSet::Emoji, EMOJI, EMOJI_MOON),
            (IconSet::Nerd, NERD, NERD_MOON),
        ]
    }

    /// A chain that ends in `blocks`, spelled the way the CLI spells it.
    fn chain(value: &str) -> IconChain {
        IconChain::parse(value).expect("the test chain parses")
    }

    /// The keys an icon table must cover: the vocabulary plus the three night siblings, exactly the
    /// set the blocks corpus draws.
    fn icon_vocabulary() -> Vec<&'static str> {
        let mut keys: Vec<&'static str> = vocabulary()
            .into_iter()
            .flat_map(|key| [key, night_variant(key)])
            .collect();
        keys.sort_unstable();
        keys.dedup();
        keys
    }

    /// Every key has a glyph in every set, in both directions like the blocks corpus: a new
    /// condition without a glyph, or a table row no condition can produce, fails here.
    #[test]
    fn every_art_key_has_a_glyph_in_every_icon_set() {
        let vocabulary = icon_vocabulary();
        for (set, table, _) in sets() {
            for key in &vocabulary {
                assert!(
                    find(table, key).is_some(),
                    "`{key}` has no {} glyph",
                    set.as_str()
                );
            }
            for (key, _) in table {
                assert!(
                    vocabulary.contains(key),
                    "`{key}` is not in the condition vocabulary, so the {} table carries a row \
                     nothing can ask for",
                    set.as_str()
                );
            }
            for window in table.windows(2) {
                let (left, right) = (window[0], window[1]);
                assert!(
                    left.0 < right.0,
                    "{} keys must ascend: {left:?} {right:?}",
                    set.as_str()
                );
            }
        }
    }

    #[test]
    fn every_moon_phase_has_a_glyph_in_every_icon_set() {
        for (set, _, table) in sets() {
            assert_eq!(table.len(), MoonPhase::ALL.len());
            for phase in MoonPhase::ALL {
                assert!(
                    find(table, phase.art_key()).is_some(),
                    "{phase:?} has no {} glyph",
                    set.as_str()
                );
            }
            for (key, _) in table {
                assert!(
                    MoonPhase::ALL.iter().any(|phase| phase.art_key() == *key),
                    "`{key}` is not a moon phase"
                );
            }
        }
    }

    /// The recorded width is what `unicode-width` measures, so the padding uses the measurement:
    /// emoji-presentation sequences are two columns, Nerd Font glyphs one.
    #[test]
    fn the_icon_widths_are_recorded_as_measured() {
        for (set, table, moon) in sets() {
            for (key, glyph) in table.iter().chain(moon) {
                let measured = glyph.text.width();
                assert_eq!(
                    glyph.width,
                    measured,
                    "{}: `{key}` records {} columns but measures {measured}",
                    set.as_str(),
                    glyph.width
                );
                let expected = match set {
                    IconSet::Emoji => 2,
                    IconSet::Nerd => 1,
                    IconSet::Blocks => unreachable!("the blocks have no table"),
                };
                assert_eq!(
                    measured,
                    expected,
                    "{}: `{key}` measures {measured} columns",
                    set.as_str()
                );
            }
        }
    }

    /// The Nerd Font corpus is the Weather Icons block and nothing else: every glyph is one
    /// private-use codepoint in U+E300–U+E3E3, and the distinct codepoints are exactly the ones
    /// `glyphnames.json` publishes for the names the table's comments spell. A typo in an escape
    /// (`\u{e30e}` for `\u{e30d}`) cannot pass as "some private-use glyph".
    #[test]
    fn the_nerd_glyphs_are_the_weather_icons_codepoints() {
        const USED: &[(char, &str)] = &[
            ('\u{e302}', "weather-day_cloudy"),
            ('\u{e30c}', "weather-day_sunny_overcast"),
            ('\u{e30d}', "weather-day_sunny"),
            ('\u{e312}', "weather-cloudy"),
            ('\u{e313}', "weather-fog"),
            ('\u{e314}', "weather-hail"),
            ('\u{e315}', "weather-lightning"),
            ('\u{e316}', "weather-rain_mix"),
            ('\u{e317}', "weather-rain_wind"),
            ('\u{e318}', "weather-rain"),
            ('\u{e319}', "weather-showers"),
            ('\u{e31a}', "weather-snow"),
            ('\u{e31b}', "weather-sprinkle"),
            ('\u{e31c}', "weather-storm_showers"),
            ('\u{e31d}', "weather-thunderstorm"),
            ('\u{e32b}', "weather-night_clear"),
            ('\u{e34a}', "weather-raindrops"),
            ('\u{e35c}', "weather-smoke"),
            ('\u{e35d}', "weather-dust"),
            ('\u{e35e}', "weather-snow_wind"),
            ('\u{e36d}', "weather-smog"),
            ('\u{e36f}', "weather-snowflake_cold"),
            ('\u{e371}', "weather-raindrop"),
            ('\u{e374}', "weather-na"),
            ('\u{e379}', "weather-night_alt_partly_cloudy"),
            ('\u{e37a}', "weather-sandstorm"),
            ('\u{e37e}', "weather-night_alt_cloudy"),
            ('\u{e38d}', "weather-moon_new"),
            ('\u{e390}', "weather-moon_waxing_crescent_3"),
            ('\u{e394}', "weather-moon_first_quarter"),
            ('\u{e397}', "weather-moon_waxing_gibbous_3"),
            ('\u{e39b}', "weather-moon_full"),
            ('\u{e39e}', "weather-moon_waning_gibbous_3"),
            ('\u{e3a2}', "weather-moon_third_quarter"),
            ('\u{e3a5}', "weather-moon_waning_crescent_3"),
            ('\u{e3ad}', "weather-sleet"),
        ];

        let mut used: Vec<char> = Vec::new();
        for (key, glyph) in NERD.iter().chain(NERD_MOON) {
            let mut characters = glyph.text.chars();
            let character = characters.next().expect("a glyph is not empty");
            assert!(
                characters.next().is_none(),
                "`{key}`: {glyph:?} is not one codepoint"
            );
            assert!(
                ('\u{e300}'..='\u{e3e3}').contains(&character),
                "`{key}`: {character:?} leaves the Weather Icons block U+E300–U+E3E3"
            );
            assert!(
                USED.iter().any(|(used, _)| *used == character),
                "`{key}`: {character:?} is not one of the documented glyphs"
            );
            used.push(character);
        }
        used.sort_unstable();
        used.dedup();
        let expected: Vec<char> = USED.iter().map(|(character, _)| *character).collect();
        assert_eq!(used, expected, "the corpus uses {used:?}");

        // The names are the ones the table rows comment, so a renamed row cannot leave a stale
        // comment behind: the spot checks below name the mapping for the keys the plan lists.
        assert_eq!(find(NERD, "clear").expect("a glyph").text, "\u{e30d}");
        assert_eq!(find(NERD, "clear-night").expect("a glyph").text, "\u{e32b}");
        assert_eq!(find(NERD, "overcast").expect("a glyph").text, "\u{e312}");
        assert_eq!(find(NERD, "unknown").expect("a glyph").text, "\u{e374}");
        assert_eq!(
            find(NERD_MOON, MoonPhase::Full.art_key())
                .expect("a glyph")
                .text,
            "\u{e39b}"
        );
    }

    /// The emoji corpus spells emoji presentation: a sequence Unicode does not default to it
    /// carries U+FE0F, which is also what makes the two-column measurement hold.
    #[test]
    fn the_emoji_corpus_spells_emoji_presentation() {
        // The sequences that need U+FE0F, by codepoint, because a bare U+2600 and so on draw as
        // one-column text glyphs in a terminal that has the text font first.
        const EXPLICIT: &[(char, u32)] = &[
            ('\u{2600}', 0xfe0f),  // ☀️
            ('\u{2601}', 0xfe0f),  // ☁️
            ('\u{26c8}', 0xfe0f),  // ⛈️
            ('\u{2744}', 0xfe0f),  // ❄️
            ('\u{1f324}', 0xfe0f), // 🌤️
            ('\u{1f327}', 0xfe0f), // 🌧️
            ('\u{1f328}', 0xfe0f), // 🌨️
            ('\u{1f329}', 0xfe0f), // 🌩️
            ('\u{1f32b}', 0xfe0f), // 🌫️
            ('\u{1f32c}', 0xfe0f), // 🌬️
            ('\u{1f3dc}', 0xfe0f), // 🏜️
        ];
        let mut explicit: Vec<(char, u32)> = Vec::new();
        for (key, glyph) in EMOJI.iter().chain(EMOJI_MOON) {
            let characters: Vec<char> = glyph.text.chars().collect();
            assert!(
                (1..=2).contains(&characters.len()),
                "`{key}`: {glyph:?} is not one or two codepoints"
            );
            if let [base, variation] = characters[..] {
                assert_eq!(variation, '\u{fe0f}', "`{key}`: {glyph:?}");
                explicit.push((base, 0xfe0f));
            }
        }
        explicit.sort_unstable_by_key(|(character, _)| *character);
        explicit.dedup();
        assert_eq!(
            explicit, EXPLICIT,
            "the corpus stopped spelling emoji presentation for {explicit:?}"
        );
    }

    /// The rule that shapes both corpora: a glyph that draws the sun is only used where the key has
    /// a night sibling, so a night column never shows daylight — and the three sky keys that do
    /// change at night draw different glyphs from their day halves.
    #[test]
    fn a_sun_glyph_is_only_used_where_the_key_has_a_night_sibling() {
        // The sun-bearing glyphs of each set: `☀️`, `🌤️`, `⛅` and the three `weather-day_*` glyphs.
        let sunny: [&[&str]; 2] = [
            &["\u{2600}\u{fe0f}", "\u{1f324}\u{fe0f}", "\u{26c5}"],
            &["\u{e302}", "\u{e30c}", "\u{e30d}"],
        ];
        for (set, table, _) in sets() {
            let sun = sunny[match set {
                IconSet::Emoji => 0,
                IconSet::Nerd => 1,
                IconSet::Blocks => unreachable!("the blocks have no table"),
            }];
            for (key, glyph) in table {
                if sun.contains(&glyph.text) {
                    assert_ne!(
                        night_variant(key),
                        *key,
                        "{}: `{key}` draws the sun and has no night sibling",
                        set.as_str()
                    );
                }
            }
            for day in ["clear", "mainly-clear", "partly-cloudy"] {
                let night = night_variant(day);
                let day_glyph = find(table, day).expect("the day glyph");
                let night_glyph = find(table, night).expect("the night glyph");
                assert_ne!(
                    day_glyph.text,
                    night_glyph.text,
                    "{}: `{day}` and `{night}` draw the same glyph",
                    set.as_str()
                );
                assert!(
                    !sun.contains(&night_glyph.text),
                    "{}: `{night}` draws the sun",
                    set.as_str()
                );
            }
        }
        assert_eq!(
            find(EMOJI, "clear").expect("a glyph").text,
            "\u{2600}\u{fe0f}"
        );
        assert_eq!(
            find(EMOJI, "clear-night").expect("a glyph").text,
            "\u{1f319}"
        );
    }

    /// The chain resolves per glyph: the first set that carries the key wins, a hole falls through
    /// to the next table, and a key no table has resolves to nothing at all — driven by test-only
    /// tables, so the fall-through is proven rather than hoped for.
    #[test]
    fn the_chain_falls_through_a_table_that_lacks_the_key() {
        const FIRST: &[(&str, Glyph)] = &[("clear", icon("A", 1)), ("rain", icon("B", 1))];
        const SECOND: &[(&str, Glyph)] = &[("rain", icon("C", 1)), ("snow", icon("D", 1))];
        let tables: [&'static [(&str, Glyph)]; 2] = [FIRST, SECOND];
        assert_eq!(
            first_with(&tables, "clear").expect("the first table").text,
            "A"
        );
        assert_eq!(
            first_with(&tables, "rain")
                .expect("the first table that carries the key")
                .text,
            "B",
            "the first table that carries the key wins"
        );
        assert_eq!(
            first_with(&tables, "snow").expect("the second table").text,
            "D"
        );
        assert!(
            first_with(&tables, "fog").is_none(),
            "a key no table carries falls through to the blocks"
        );
    }

    #[test]
    fn the_chain_resolves_keys_and_moon_phases_per_glyph() {
        let emoji = chain("emoji");
        let nerd = chain("nerd");
        let blocks = IconChain::blocks();

        let drawn = draw("clear", emoji, Charset::Unicode);
        let Art::Glyph(emoji_glyph) = drawn else {
            panic!("the emoji set carries `clear`");
        };
        assert_eq!(emoji_glyph.text, "\u{2600}\u{fe0f}");
        assert!(matches!(
            draw("clear", blocks, Charset::Unicode),
            Art::Lines(_)
        ));
        assert_eq!(glyph("clear", emoji, Charset::Unicode), "☀️");
        assert_eq!(glyph("clear", blocks, Charset::Unicode), "\\o/");
        assert_eq!(glyph("n/a", blocks, Charset::Unicode), "???");

        // The chain's order decides: a Nerd Font first still renders Nerd glyphs, and the emoji
        // set answers only where the chain has it.
        assert_eq!(
            glyph("clear", chain("nerd,emoji"), Charset::Unicode),
            "\u{e30d}"
        );
        assert_eq!(glyph("clear", chain("emoji,nerd"), Charset::Unicode), "☀️");

        // A key no set has renders as nothing at all, never a panic and never tofu.
        assert!(matches!(
            draw("no-such-key", chain("nerd,emoji"), Charset::Unicode),
            Art::Missing
        ));
        assert_eq!(
            glyph("no-such-key", chain("nerd,emoji"), Charset::Unicode),
            "???"
        );

        // The moon family resolves through the same chain.
        assert_eq!(
            moon_glyph(MoonPhase::Full, emoji, Charset::Unicode),
            "\u{1f315}"
        );
        assert_eq!(
            moon_glyph(MoonPhase::Full, nerd, Charset::Unicode),
            "\u{e39b}"
        );
        assert_eq!(moon_glyph(MoonPhase::Full, blocks, Charset::Unicode), "●");
        assert!(matches!(
            moon_art(MoonPhase::Full, emoji, Charset::Unicode),
            Art::Glyph(_)
        ));
    }

    /// An ASCII run draws the blocks whatever the chain says: an icon set cannot be spelled in
    /// 7-bit, so `--format dumb` and a non-UTF-8 terminal never see one.
    #[test]
    fn an_ascii_run_draws_the_blocks_whatever_the_chain_says() {
        let chain = chain("nerd,emoji");
        for key in icon_vocabulary() {
            let drawing = draw(key, chain, Charset::Ascii);
            let Art::Lines(lines) = drawing else {
                panic!("`{key}` is not drawn with the blocks in ASCII");
            };
            assert!(
                lines.iter().all(|line| line.is_ascii()),
                "`{key}`: the ASCII block needs a unicode terminal"
            );
            assert!(glyph(key, chain, Charset::Ascii).is_ascii(), "`{key}`");
        }
        for phase in MoonPhase::ALL {
            assert!(
                moon_glyph(phase, chain, Charset::Ascii).is_ascii(),
                "{phase:?}"
            );
            assert!(matches!(
                moon_art(phase, chain, Charset::Ascii),
                Art::Lines(_)
            ));
        }
    }

    /// A glyph is centred in the same 7×4 cell the blocks draw in, padded by its *measured* width,
    /// so the table's borders cannot move when a set is selected.
    #[test]
    fn a_glyph_is_centred_in_the_cell_by_its_measured_width() {
        let drawing = draw("clear", chain("emoji"), Charset::Unicode);
        for index in 0..ART_LINES {
            let line = drawing.line(index);
            if index == GLYPH_LINE {
                assert_eq!(line.width(), ART_W, "{line:?} is not the cell's width");
                assert!(line.starts_with("  ") && line.ends_with("   "), "{line:?}");
            } else {
                assert_eq!(line, "", "the glyph sits on line {GLYPH_LINE} alone");
            }
        }
        // The blocks keep their own lines, and an unknown key draws nothing at all.
        let Art::Lines(lines) = draw("clear", IconChain::blocks(), Charset::Unicode) else {
            panic!("the blocks draw four lines");
        };
        assert_eq!(
            draw("clear", IconChain::blocks(), Charset::Unicode).line(0),
            lines[0]
        );
        assert_eq!(
            draw("no-such-key", chain("emoji"), Charset::Unicode).line(0),
            ""
        );
    }

    /// `n/a` is a spelling of `unknown` in every set, exactly as in the blocks corpus.
    #[test]
    fn the_icon_tables_answer_to_n_a() {
        for (set, table, _) in sets() {
            let unknown = find(table, "unknown").expect("the unknown row");
            let n_a = find(table, "n/a").expect("n/a is the unknown row");
            assert_eq!(n_a.text, unknown.text, "{}", set.as_str());
        }
    }
}
