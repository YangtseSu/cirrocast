// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The astronomy block: moon phase, sun times and the next phase instants.
//!
//! The types live in the model, next to everything else a [`Report`](super::Report) carries, for
//! the same reason the air types do: the renderers may read the model but never the module that
//! produces the data (here `src/astro`, which is pure arithmetic — no request, no cache, no key).
//! A hand-built report or a fixture therefore leaves the field `None`, and the renderers shape
//! their output from the presence of the value alone.
//!
//! The rules that shape the types:
//!
//! * **`None` means "does not happen", never a clamped midnight.** Inside the polar circles the
//!   Sun may not rise or set on a given day; the Moon's day is 24 h 50 m, so a calendar day can
//!   have a rise without a set, a set without a rise, or neither. `polar` names the state instead
//!   of inventing a time.
//! * **`source` is part of the data.** A sunrise from the weather backend and one computed here
//!   are the same number on screen but not the same claim; the field records which one a reader is
//!   looking at, and the `--verbose` note speaks it out loud.
//! * **The phase is one of eight**, matching the 45°-wide windows of the synodic elongation; the
//!   illuminated fraction is the continuous value beside it, so a consumer can print `98%` while
//!   the name still says `Waxing Gibbous`.

use chrono::{DateTime, FixedOffset, Utc};
use serde::{Deserialize, Serialize};

/// Which of the eight named windows of the synodic cycle the Moon is in.
///
/// The names come from the elongation `E` in `[0, 360)`: `New` wraps `[337.5, 22.5)`, then each
/// following window is 45° wide, so the boundaries fall on the quarters and the midpoints of the
/// crescents and gibbous phases.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MoonPhase {
    /// New Moon: `E ∈ [337.5, 22.5)`.
    New,
    /// Waxing crescent: `E ∈ [22.5, 67.5)`.
    WaxingCrescent,
    /// First quarter: `E ∈ [67.5, 112.5)`.
    FirstQuarter,
    /// Waxing gibbous: `E ∈ [112.5, 157.5)`.
    WaxingGibbous,
    /// Full Moon: `E ∈ [157.5, 202.5)`.
    Full,
    /// Waning gibbous: `E ∈ [202.5, 247.5)`.
    WaningGibbous,
    /// Last quarter: `E ∈ [247.5, 292.5)`.
    LastQuarter,
    /// Waning crescent: `E ∈ [292.5, 337.5)`.
    WaningCrescent,
}

impl MoonPhase {
    /// The eight phases in cycle order, starting at New Moon.
    pub const ALL: [Self; 8] = [
        Self::New,
        Self::WaxingCrescent,
        Self::FirstQuarter,
        Self::WaxingGibbous,
        Self::Full,
        Self::WaningGibbous,
        Self::LastQuarter,
        Self::WaningCrescent,
    ];

    /// The stable slug: `new`, `waxing-crescent`, …, the JSON `phase_key` and the tail of the
    /// catalog key.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::New => "new",
            Self::WaxingCrescent => "waxing-crescent",
            Self::FirstQuarter => "first-quarter",
            Self::WaxingGibbous => "waxing-gibbous",
            Self::Full => "full",
            Self::WaningGibbous => "waning-gibbous",
            Self::LastQuarter => "last-quarter",
            Self::WaningCrescent => "waning-crescent",
        }
    }

    /// The position of the phase in [`Self::ALL`], which is the index the catalog array uses.
    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            Self::New => 0,
            Self::WaxingCrescent => 1,
            Self::FirstQuarter => 2,
            Self::WaxingGibbous => 3,
            Self::Full => 4,
            Self::WaningGibbous => 5,
            Self::LastQuarter => 6,
            Self::WaningCrescent => 7,
        }
    }

    /// The key of the phase's block in the renderer's moon art table, e.g. `moon/waxing-crescent`.
    #[must_use]
    pub const fn art_key(self) -> &'static str {
        match self {
            Self::New => "moon/new",
            Self::WaxingCrescent => "moon/waxing-crescent",
            Self::FirstQuarter => "moon/first-quarter",
            Self::WaxingGibbous => "moon/waxing-gibbous",
            Self::Full => "moon/full",
            Self::WaningGibbous => "moon/waning-gibbous",
            Self::LastQuarter => "moon/last-quarter",
            Self::WaningCrescent => "moon/waning-crescent",
        }
    }
}

impl std::fmt::Display for MoonPhase {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Whether the Sun stays above or below the horizon for the whole local day.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Polar {
    /// The midnight sun: no set, no rise, the Sun up all day.
    Day,
    /// The polar night: no rise, no set, the Sun down all day.
    Night,
}

impl Polar {
    /// The JSON spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Day => "day",
            Self::Night => "night",
        }
    }
}

/// Where a [`Sun`] block's times came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SunSource {
    /// The weather backend's own sunrise/sunset for the day.
    Provider,
    /// Computed locally, because the backend sends none (`met-no`, an observation-only backend, a
    /// report without days).
    Local,
}

impl SunSource {
    /// The JSON spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Provider => "provider",
            Self::Local => "local",
        }
    }
}

/// The Sun's day: rise, set, daylight length and the polar state.
///
/// `sunrise` and `sunset` are `None` when the event does not happen on the local day — a polar
/// day or night, or a provider that reports only one of the pair — and `daylight_secs` is `Some`
/// whenever the question has an answer: the difference of the two instants, `86400` for a polar
/// day and `0` for a polar night.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sun {
    /// Sunrise on the location-local day, when it happens.
    pub sunrise: Option<DateTime<FixedOffset>>,
    /// Sunset on the location-local day, when it happens.
    pub sunset: Option<DateTime<FixedOffset>>,
    /// Seconds of daylight; `None` when neither a full pair of times nor a polar state is known.
    pub daylight_secs: Option<u32>,
    /// The polar state of the day, `None` outside the circles and on transition days.
    pub polar: Option<Polar>,
    /// Whether the times came from the provider or from this module.
    pub source: SunSource,
}

/// The Moon: phase, illumination, age, the day's rise/set and the next phase instants.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Moon {
    /// The named phase — one of [`MoonPhase::ALL`].
    pub phase: MoonPhase,
    /// Illuminated fraction of the disc, `0.0..=1.0` (geocentric; the panel says so).
    pub illuminated_fraction: f64,
    /// Days since the preceding New Moon, `0.0..≈29.53`.
    pub age_days: f64,
    /// Moonrise on the location-local day, when it happens.
    pub moonrise: Option<DateTime<FixedOffset>>,
    /// Moonset on the location-local day, when it happens.
    pub moonset: Option<DateTime<FixedOffset>>,
    /// The next four phase instants after the run's clock, in chronological order, at the
    /// location's offset.
    pub next: Vec<(MoonPhase, DateTime<FixedOffset>)>,
}

/// Everything the `--moon` view and the `json` `astro` object read.
///
/// The whole block is attached to the [`Report`](super::Report) only when the run asked for it
/// (`--moon` or `--format moon`), exactly like the air reading: the renderer has no display
/// settings of its own, so the presence of the value *is* the request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Astro {
    /// The Moon block; always computed locally.
    pub moon: Moon,
    /// The Sun block: the provider's values when it sends any, the local computation otherwise.
    pub sun: Sun,
    /// The instant the block was computed at, UTC.
    pub computed_at: DateTime<Utc>,
}
