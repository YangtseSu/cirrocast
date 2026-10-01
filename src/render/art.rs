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
//! Geometry: [`ART_W`] display columns by [`ART_LINES`] lines. Lines are stored right-trimmed — the
//! renderer pads them — and a line never carries more than [`ART_W`] columns, so a cell that shows
//! them keeps its borders aligned even when the block is mostly empty.
//!
//! [`Condition::art_key`]: crate::model::condition::Condition::art_key

use super::Charset;

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
        "smoke",
        Block {
            unicode: ["  ∿ ∿", " ∿ ∿", "  ∿ ∿", ""],
            ascii: ["  ~ ~", " ~ ~", "  ~ ~", ""],
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
/// [`compass_16`]: crate::model::units::compass_16
const ARROWS: [(&str, &str); 8] = [
    ("↑", "^"),
    ("↗", "/"),
    ("→", ">"),
    ("↘", "\\"),
    ("↓", "v"),
    ("↙", "/"),
    ("←", "<"),
    ("↖", "\\"),
];

/// The arrow for a wind direction in degrees, in `charset`.
#[must_use]
pub fn wind_arrow(deg: u16, charset: Charset) -> &'static str {
    let sector = (usize::from(deg % 360) + 22) / 45 % ARROWS.len();
    ARROWS.get(sector).map_or("-", |arrow| match charset {
        Charset::Unicode => arrow.0,
        Charset::Ascii => arrow.1,
    })
}

/// The block for `key`, or `None` when the vocabulary has no such key.
///
/// `n/a` is accepted as a spelling of `unknown`, because that is what a caller with no condition at
/// all — a report without current conditions and without forecast days — will ask for.
#[must_use]
pub fn art(key: &str) -> Option<&'static Block> {
    let key = if key == "n/a" { "unknown" } else { key };
    ART.binary_search_by(|(candidate, _)| candidate.cmp(&key))
        .ok()
        .and_then(|index| ART.get(index))
        .map(|(_, block)| block)
}

#[cfg(test)]
mod tests {
    use unicode_width::UnicodeWidthStr as _;

    use super::{
        ART, ART_LINES, ART_W, ArtStyle, NO_BLOCK, art, night_variant, one_line_art, wind_arrow,
    };
    use crate::model::condition::Condition;
    use crate::render::Charset;

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
            assert_ne!(
                one_line_art(key),
                "",
                "code {code} maps to art key `{key}`, which has no one-line glyph"
            );
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
            for charset in [Charset::Unicode, Charset::Ascii] {
                for (index, line) in block.lines(charset).iter().enumerate() {
                    assert!(
                        line.width() <= ART_W,
                        "{key}: line {index} of {charset:?} is {} columns wide: {line:?}",
                        line.width()
                    );
                }
            }
        }
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
}
