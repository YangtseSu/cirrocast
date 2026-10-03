// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The 256-colour palette and the painter.
//!
//! The palette is **re-authored** for this project: temperature ramps from deep blue through green
//! and yellow into red, precipitation deepens with the amount, wind follows a Beaufort-like ramp,
//! and the art takes the colour of the phenomenon it draws. Nothing here is copied from `wego` or
//! `wttr.in` — the stops are documented so the ramp can be reviewed and adjusted.
//!
//! Every colour is an xterm 256-colour index. [`paint`] turns one into an escape sequence, folding
//! the palette down to the ANSI colours for [`ColorDepth::Ansi16`] and returning the input
//! **borrowed** when colour is off, so a monochrome run allocates nothing and cannot leak an
//! escape into a pipe.

use std::borrow::Cow;

use super::ColorDepth;
use super::art::ArtStyle;
use crate::air::aqi::AqiCategory;
use crate::model::Severity;
use crate::model::condition::Condition;

/// The neutral grey a value with nothing to report is painted in.
pub const FG_DEFAULT: u8 = 250;

/// The temperature ramp: `(celsius, colour)`, ascending, first stop is the coldest.
///
/// Below the first stop the coldest colour is used and above the last the hottest, so the ramp is
/// clamped at both ends rather than extrapolated.
const TEMP_STOPS: [(f32, u8); 10] = [
    (-20.0, 21),
    (-10.0, 27),
    (0.0, 51),
    (5.0, 45),
    (10.0, 47),
    (15.0, 118),
    (20.0, 226),
    (25.0, 214),
    (30.0, 208),
    (35.0, 196),
];

/// The wind ramp: calm, breeze, strong breeze, gale, storm — about Beaufort 1, 4, 6 and 8.
const WIND_STOPS: [(f32, u8); 5] = [
    (0.0, FG_DEFAULT),
    (5.0, 47),
    (20.0, 226),
    (39.0, 208),
    (62.0, 196),
];

/// The colour of a temperature: the nearest stop at or below it, clamped at both ends.
#[must_use]
pub fn temp_fg(temp_c: f32) -> u8 {
    if temp_c.is_nan() {
        return FG_DEFAULT;
    }
    ramp(&TEMP_STOPS, temp_c)
}

/// The colour of a precipitation amount: dry is neutral, then light, moderate and heavy.
#[must_use]
pub fn precip_fg(mm: f32) -> u8 {
    if mm >= 7.5 {
        27
    } else if mm >= 2.5 {
        33
    } else if mm > 0.0 {
        39
    } else {
        FG_DEFAULT
    }
}

/// The colour of a wind speed, on the Beaufort-like ramp above.
#[must_use]
pub fn wind_fg(kmh: f32) -> u8 {
    if kmh.is_nan() {
        return FG_DEFAULT;
    }
    ramp(&WIND_STOPS, kmh)
}

/// The colour of a relative humidity: dry amber, neutral, humid blue.
#[must_use]
pub fn humidity_fg(pct: u8) -> u8 {
    match pct {
        0..=29 => 214,
        30..=70 => FG_DEFAULT,
        _ => 39,
    }
}

/// The colour a phenomenon's art is drawn in.
#[must_use]
pub const fn art_fg(style: ArtStyle) -> u8 {
    match style {
        ArtStyle::Sun => 220,
        ArtStyle::Cloud => 250,
        ArtStyle::Rain => 33,
        ArtStyle::Snow => 255,
        ArtStyle::Thunder => 129,
        ArtStyle::Fog => 245,
        ArtStyle::Plain => FG_DEFAULT,
    }
}

/// The colour of the art of a canonical condition.
///
/// A condition whose art key has no block (only possible for a key added to the model without
/// artwork) is painted neutrally rather than panicking.
#[must_use]
pub fn condition_fg(condition: Condition) -> u8 {
    super::art::art(condition.art_key()).map_or(FG_DEFAULT, |block| art_fg(block.style))
}

/// The nearest stop's colour; `stops` is ascending, so this is a clamped lookup, not a search.
fn ramp(stops: &[(f32, u8)], value: f32) -> u8 {
    let mut colour = stops.first().map_or(FG_DEFAULT, |stop| stop.1);
    for (stop, candidate) in stops {
        if value >= *stop {
            colour = *candidate;
        } else {
            break;
        }
    }
    colour
}

/// Paints `text` in the 256-colour palette entry `fg`.
///
/// Returns the input borrowed when `depth` is [`ColorDepth::Mono`] or `text` is empty: a
/// monochrome run must not allocate and must not emit escapes for nothing.
#[must_use]
pub fn paint(text: &str, fg: u8, depth: ColorDepth) -> Cow<'_, str> {
    if depth == ColorDepth::Mono || text.is_empty() {
        return Cow::Borrowed(text);
    }
    let colour = match depth {
        ColorDepth::Ansi16 => ansi16_from_256(fg),
        ColorDepth::Ansi256 | ColorDepth::Mono => fg,
    };
    Cow::Owned(sgr(text, colour))
}

/// One `38;5` foreground escape plus a reset.
fn sgr(text: &str, colour: u8) -> String {
    format!("\x1b[38;5;{colour}m{text}\x1b[0m")
}

/// The colour of an alert severity, authored here: grey for unknown, blue for minor, yellow for
/// moderate, red for severe; extreme is white on red and handled by [`paint_severity`].
#[must_use]
pub const fn severity_fg(severity: Severity) -> u8 {
    match severity {
        Severity::Unknown => FG_DEFAULT,
        Severity::Minor => 111,
        Severity::Moderate => 226,
        // `Extreme` never reaches the foreground table; the value is the red of the background.
        Severity::Severe | Severity::Extreme => 196,
    }
}

/// Paints `text` in an alert severity's colour.
///
/// `Extreme` is white on a red background rather than a foreground colour — the strongest level has
/// to read as a block, not as one more shade of red — and every other level is a foreground colour
/// from [`severity_fg`]. With colour off the text is returned borrowed, so a monochrome run keeps
/// the severity as a word and emits no escapes.
#[must_use]
pub fn paint_severity(text: &str, severity: Severity, depth: ColorDepth) -> Cow<'_, str> {
    if depth == ColorDepth::Mono || text.is_empty() {
        return Cow::Borrowed(text);
    }
    if severity == Severity::Extreme {
        return Cow::Owned(format!("\x1b[1;97;41m{text}\x1b[0m"));
    }
    paint(text, severity_fg(severity), depth)
}

/// The AQI category ramp, authored here: green → yellow → orange → red → purple → maroon.
///
/// The two scales share two words, so the mapping is per *category*, not per position on a scale:
/// `Good` and `Fair` read green, `Moderate` yellow, the next step orange (`Unhealthy for
/// sensitive groups` / `Poor`), then red, purple, and finally maroon for `Hazardous`. The European
/// scale has no maroon band; its `Extremely poor` is the purple top of that scale. A category is
/// never painted by `--units` or by which scale it came from — only by the air it describes.
#[must_use]
pub const fn aqi_fg(category: AqiCategory) -> u8 {
    match category {
        AqiCategory::Good | AqiCategory::Fair => 34,
        AqiCategory::Moderate => 226,
        AqiCategory::UnhealthyForSensitiveGroups | AqiCategory::Poor => 208,
        AqiCategory::Unhealthy | AqiCategory::VeryPoor => 160,
        AqiCategory::VeryUnhealthy | AqiCategory::ExtremelyPoor => 93,
        AqiCategory::Hazardous => 88,
    }
}

/// The RGB values of the sixteen ANSI colours, as xterm defines them.
const ANSI16_RGB: [(u8, u8, u8); 16] = [
    (0, 0, 0),
    (128, 0, 0),
    (0, 128, 0),
    (128, 128, 0),
    (0, 0, 128),
    (128, 0, 128),
    (0, 128, 128),
    (192, 192, 192),
    (128, 128, 128),
    (255, 0, 0),
    (0, 255, 0),
    (255, 255, 0),
    (0, 0, 255),
    (255, 0, 255),
    (0, 255, 255),
    (255, 255, 255),
];

/// The six levels the 256-colour cube steps through.
const CUBE_LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];

/// Folds a 256-colour index onto the nearest of the sixteen ANSI colours.
///
/// Indices `0..=15` are already ANSI colours, the cube is `16..=231` and `232..=255` are greys; the
/// cube and the greys are compared to the ANSI palette by squared RGB distance, which is the usual
/// cheap approximation of "which of these two looks closer".
#[must_use]
pub fn ansi16_from_256(colour: u8) -> u8 {
    if colour < 16 {
        return colour;
    }
    let (r, g, b) = rgb_of(colour);
    let mut best = 0;
    let mut best_distance = u32::MAX;
    for (index, (ar, ag, ab)) in ANSI16_RGB.iter().enumerate() {
        let distance = squared_distance((r, g, b), (*ar, *ag, *ab));
        if distance < best_distance {
            best_distance = distance;
            best = u8::try_from(index).unwrap_or(0);
        }
    }
    best
}

/// The RGB triple behind a 256-colour index at or above 16.
fn rgb_of(colour: u8) -> (u8, u8, u8) {
    if colour < 232 {
        let cube = colour - 16;
        (
            cube_level(cube / 36),
            cube_level(cube % 36 / 6),
            cube_level(cube % 6),
        )
    } else {
        let level = 8 + 10 * (colour - 232);
        (level, level, level)
    }
}

/// One of the cube's six levels; a `colour` above 231 never asks for one out of range.
fn cube_level(index: u8) -> u8 {
    CUBE_LEVELS.get(usize::from(index)).copied().unwrap_or(255)
}

/// Squared euclidean distance between two RGB triples, in `u32` so no term overflows.
fn squared_distance(left: (u8, u8, u8), right: (u8, u8, u8)) -> u32 {
    let channel = |a: u8, b: u8| {
        let difference = i32::from(a) - i32::from(b);
        u32::try_from(difference * difference).unwrap_or(u32::MAX)
    };
    channel(left.0, right.0) + channel(left.1, right.1) + channel(left.2, right.2)
}

#[cfg(test)]
mod tests {
    use super::{
        FG_DEFAULT, ansi16_from_256, art_fg, condition_fg, humidity_fg, paint, precip_fg, temp_fg,
        wind_fg,
    };
    use crate::model::condition::Condition;
    use crate::render::ColorDepth;
    use crate::render::art::ArtStyle;

    #[test]
    fn the_temperature_ramp_takes_the_nearest_lower_stop() {
        assert_eq!(temp_fg(-40.0), 21, "clamped at the cold end");
        assert_eq!(temp_fg(-20.0), 21);
        assert_eq!(temp_fg(-19.9), 21);
        assert_eq!(temp_fg(-10.0), 27);
        assert_eq!(temp_fg(-0.1), 27);
        assert_eq!(temp_fg(0.0), 51);
        assert_eq!(temp_fg(4.9), 51);
        assert_eq!(temp_fg(5.0), 45);
        assert_eq!(temp_fg(9.9), 45);
        assert_eq!(temp_fg(10.0), 47);
        assert_eq!(temp_fg(14.9), 47);
        assert_eq!(temp_fg(15.0), 118);
        assert_eq!(temp_fg(19.9), 118);
        assert_eq!(temp_fg(20.0), 226);
        assert_eq!(temp_fg(24.9), 226);
        assert_eq!(temp_fg(25.0), 214);
        assert_eq!(temp_fg(29.9), 214);
        assert_eq!(temp_fg(30.0), 208);
        assert_eq!(temp_fg(34.9), 208);
        assert_eq!(temp_fg(35.0), 196);
        assert_eq!(temp_fg(48.0), 196, "clamped at the hot end");
        assert_eq!(temp_fg(f32::NAN), FG_DEFAULT);
    }

    #[test]
    fn the_precipitation_ramp_deepens_with_the_amount() {
        assert_eq!(precip_fg(0.0), FG_DEFAULT);
        assert_eq!(precip_fg(-0.1), FG_DEFAULT);
        assert_eq!(precip_fg(0.1), 39);
        assert_eq!(precip_fg(2.4), 39);
        assert_eq!(precip_fg(2.5), 33);
        assert_eq!(precip_fg(7.4), 33);
        assert_eq!(precip_fg(7.5), 27);
        assert_eq!(precip_fg(60.0), 27);
    }

    #[test]
    fn the_wind_and_humidity_ramps_follow_their_stops() {
        assert_eq!(wind_fg(0.0), FG_DEFAULT);
        assert_eq!(wind_fg(4.9), FG_DEFAULT);
        assert_eq!(wind_fg(5.0), 47);
        assert_eq!(wind_fg(20.0), 226);
        assert_eq!(wind_fg(39.0), 208);
        assert_eq!(wind_fg(62.0), 196);
        assert_eq!(wind_fg(f32::NAN), FG_DEFAULT);

        assert_eq!(humidity_fg(0), 214);
        assert_eq!(humidity_fg(29), 214);
        assert_eq!(humidity_fg(30), FG_DEFAULT);
        assert_eq!(humidity_fg(70), FG_DEFAULT);
        assert_eq!(humidity_fg(71), 39);
        assert_eq!(humidity_fg(100), 39);
    }

    #[test]
    fn art_takes_the_colour_of_its_phenomenon() {
        assert_eq!(art_fg(ArtStyle::Sun), 220);
        assert_eq!(art_fg(ArtStyle::Cloud), 250);
        assert_eq!(art_fg(ArtStyle::Rain), 33);
        assert_eq!(art_fg(ArtStyle::Snow), 255);
        assert_eq!(art_fg(ArtStyle::Thunder), 129);
        assert_eq!(art_fg(ArtStyle::Fog), 245);

        assert_eq!(condition_fg(Condition::from_u8(0)), 220, "clear sky");
        assert_eq!(condition_fg(Condition::from_u8(3)), 250, "overcast");
        assert_eq!(condition_fg(Condition::from_u8(63)), 33, "moderate rain");
        assert_eq!(condition_fg(Condition::from_u8(73)), 255, "snow");
        assert_eq!(condition_fg(Condition::from_u8(95)), 129, "thunderstorm");
        assert_eq!(condition_fg(Condition::from_u8(45)), 245, "fog");
        assert_eq!(condition_fg(Condition::from_u8(8)), FG_DEFAULT, "unknown");
    }

    #[test]
    fn mono_paints_nothing_and_borrows() {
        let painted = paint("12\u{b0}C", 196, ColorDepth::Mono);
        assert!(matches!(painted, std::borrow::Cow::Borrowed("12\u{b0}C")));
        assert!(!painted.contains('\u{1b}'));

        let empty = paint("", 196, ColorDepth::Ansi256);
        assert!(matches!(empty, std::borrow::Cow::Borrowed("")));
    }

    #[test]
    fn the_two_colour_depths_emit_their_own_escapes() {
        assert_eq!(
            paint("x", 196, ColorDepth::Ansi256),
            "\u{1b}[38;5;196mx\u{1b}[0m"
        );
        assert_eq!(
            paint("x", 196, ColorDepth::Ansi16),
            "\u{1b}[38;5;9mx\u{1b}[0m",
            "196 is a bright red, which is ANSI colour 9"
        );
    }

    #[test]
    fn the_ansi16_fold_keeps_the_sixteen_and_approximates_the_rest() {
        for colour in 0..16 {
            assert_eq!(ansi16_from_256(colour), colour);
        }
        assert_eq!(ansi16_from_256(196), 9, "bright red");
        assert_eq!(ansi16_from_256(226), 11, "bright yellow");
        assert_eq!(ansi16_from_256(21), 12, "blue");
        assert_eq!(ansi16_from_256(51), 14, "cyan");
        assert_eq!(ansi16_from_256(255), 15, "white");
        assert_eq!(
            ansi16_from_256(250),
            7,
            "the light grey ends up on the ANSI grey"
        );
        assert_eq!(ansi16_from_256(232), 0, "the darkest grey is black");
        assert_eq!(ansi16_from_256(129), 13, "a purple thundercloud is magenta");
    }
}
