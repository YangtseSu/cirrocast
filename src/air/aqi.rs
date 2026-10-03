// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The two AQI category scales, authored from the published breakpoints.
//!
//! Raw indices are always shown as the source reports them; this module turns an index into the
//! category word a reader recognises. The breakpoints are the documented ones:
//!
//! * **US AQI** (EPA): Good `0..=50`, Moderate `51..=100`, Unhealthy for sensitive groups
//!   `101..=150`, Unhealthy `151..=200`, Very unhealthy `201..=300`, Hazardous `301..=500`. An
//!   index above 500 is off the documented scale and clamps to Hazardous ([`us_beyond_index`] says
//!   so, and the panel notes it under `-v`).
//! * **European AQI** (EEA): Good `0..=20`, Fair `21..=40`, Moderate `41..=60`, Poor `61..=80`,
//!   Very poor `81..=100`, Extremely poor above 100.
//!
//! The ten categories are one enum because the two scales share two words (Good, Moderate) and a
//! caller should never have to know which scale it holds; `from_us`/`from_european` are the only
//! constructors, so a category cannot be built for the wrong scale.

use std::fmt;
use std::str::FromStr;

use crate::error::{Error, Result};
use crate::i18n::{MessageKey, keys};

/// Which of the two scales drives the panel's colour and the `%q` token.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AqiIndex {
    /// The US EPA scale (the default).
    #[default]
    Us,
    /// The European Environment Agency scale.
    European,
}

impl AqiIndex {
    /// Both scales, in `--help` order.
    pub const ALL: [Self; 2] = [Self::Us, Self::European];

    /// The command line and configuration spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Us => "us",
            Self::European => "european",
        }
    }

    /// The label the panel prints before the value.
    #[must_use]
    pub const fn label_key(self) -> MessageKey {
        match self {
            Self::Us => keys::AQI_US_LABEL,
            Self::European => keys::AQI_EUROPEAN_LABEL,
        }
    }

    /// Parses a scale name, case-insensitively.
    ///
    /// Both the flag parser and the configuration validator go through here, so `--aqi-index` and
    /// `[air] index` cannot disagree about a spelling.
    pub fn from_name(name: &str) -> Result<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "us" => Ok(Self::Us),
            "european" => Ok(Self::European),
            other => Err(Error::Usage(format!(
                "unknown air-quality index `{other}`; known indices: us, european"
            ))),
        }
    }
}

impl FromStr for AqiIndex {
    type Err = Error;

    fn from_str(input: &str) -> Result<Self> {
        Self::from_name(input)
    }
}

impl fmt::Display for AqiIndex {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// One AQI category, on either scale.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AqiCategory {
    /// `0..=50` US, `0..=20` European.
    Good,
    /// `51..=100` US.
    Moderate,
    /// `101..=150` US.
    UnhealthyForSensitiveGroups,
    /// `151..=200` US.
    Unhealthy,
    /// `201..=300` US.
    VeryUnhealthy,
    /// `301..=500` US (and anything above, clamped).
    Hazardous,
    /// `21..=40` European.
    Fair,
    /// `61..=80` European.
    Poor,
    /// `81..=100` European.
    VeryPoor,
    /// Above 100 European.
    ExtremelyPoor,
}

impl AqiCategory {
    /// Every category, in the index order [`Self::i18n_key`] relies on.
    pub const ALL: [Self; 10] = [
        Self::Good,
        Self::Moderate,
        Self::UnhealthyForSensitiveGroups,
        Self::Unhealthy,
        Self::VeryUnhealthy,
        Self::Hazardous,
        Self::Fair,
        Self::Poor,
        Self::VeryPoor,
        Self::ExtremelyPoor,
    ];

    /// The position in [`Self::ALL`], which is also the position in `keys::AQI_CATEGORIES`.
    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            Self::Good => 0,
            Self::Moderate => 1,
            Self::UnhealthyForSensitiveGroups => 2,
            Self::Unhealthy => 3,
            Self::VeryUnhealthy => 4,
            Self::Hazardous => 5,
            Self::Fair => 6,
            Self::Poor => 7,
            Self::VeryPoor => 8,
            Self::ExtremelyPoor => 9,
        }
    }

    /// The category of a raw US AQI. Above 500 clamps to [`Self::Hazardous`].
    #[must_use]
    pub const fn from_us(index: u16) -> Self {
        match index {
            0..=50 => Self::Good,
            51..=100 => Self::Moderate,
            101..=150 => Self::UnhealthyForSensitiveGroups,
            151..=200 => Self::Unhealthy,
            201..=300 => Self::VeryUnhealthy,
            _ => Self::Hazardous,
        }
    }

    /// The category of a raw European AQI.
    #[must_use]
    pub const fn from_european(index: u16) -> Self {
        match index {
            0..=20 => Self::Good,
            21..=40 => Self::Fair,
            41..=60 => Self::Moderate,
            61..=80 => Self::Poor,
            81..=100 => Self::VeryPoor,
            _ => Self::ExtremelyPoor,
        }
    }

    /// The kebab-case spelling the JSON document uses.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Good => "good",
            Self::Moderate => "moderate",
            Self::UnhealthyForSensitiveGroups => "unhealthy-sensitive",
            Self::Unhealthy => "unhealthy",
            Self::VeryUnhealthy => "very-unhealthy",
            Self::Hazardous => "hazardous",
            Self::Fair => "fair",
            Self::Poor => "poor",
            Self::VeryPoor => "very-poor",
            Self::ExtremelyPoor => "extremely-poor",
        }
    }

    /// The catalog key of the category word.
    #[must_use]
    pub const fn i18n_key(self) -> MessageKey {
        keys::AQI_CATEGORIES[self.index()]
    }
}

/// Whether a US AQI reading is above the scale's documented `500` top.
#[must_use]
pub const fn us_beyond_index(index: u16) -> bool {
    index > 500
}

#[cfg(test)]
mod tests {
    use super::{AqiCategory, AqiIndex};

    /// The two breakpoint tables live in `tests/air.rs`, which drives them through the public
    /// crate surface; here only the helpers the tables do not reach are pinned.

    #[test]
    fn every_category_has_its_own_index_key_and_spelling() {
        let mut keys = std::collections::HashSet::new();
        let mut spellings = std::collections::HashSet::new();
        for (position, category) in AqiCategory::ALL.into_iter().enumerate() {
            assert_eq!(category.index(), position, "{category:?}");
            assert!(keys.insert(category.i18n_key().as_str().into_owned()));
            assert!(spellings.insert(category.as_str()));
        }
        assert_eq!(
            AqiCategory::from_us(0).i18n_key().as_str(),
            "aqi-category-good"
        );
        assert_eq!(
            AqiCategory::from_us(120).i18n_key().as_str(),
            "aqi-category-unhealthy-sensitive"
        );
        assert_eq!(
            AqiCategory::from_european(120).i18n_key().as_str(),
            "aqi-category-extremely-poor"
        );
    }

    #[test]
    fn an_index_name_parses_case_insensitively() {
        assert_eq!(AqiIndex::from_name("us").expect("us parses"), AqiIndex::Us);
        assert_eq!(
            AqiIndex::from_name(" US ").expect("us parses"),
            AqiIndex::Us
        );
        assert_eq!(
            AqiIndex::from_name("European").expect("european parses"),
            AqiIndex::European
        );
        assert_eq!(AqiIndex::default(), AqiIndex::Us);
        assert_eq!(AqiIndex::Us.as_str(), "us");
        assert_eq!(AqiIndex::European.to_string(), "european");
        let error = AqiIndex::from_name("epa").expect_err("never a scale");
        assert!(error.to_string().contains("known indices: us, european"));
    }
}
