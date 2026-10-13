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

/// Which of the scales drives the panel's colour and the `%q` token.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AqiIndex {
    /// The US EPA scale (the default).
    #[default]
    Us,
    /// The European Environment Agency scale.
    European,
    /// The Chinese national scale (GB 3095-2012, `QWeather`'s `cn-mee`).
    Cn,
}

impl AqiIndex {
    /// Every scale, in `--help` order.
    pub const ALL: [Self; 3] = [Self::Us, Self::European, Self::Cn];

    /// The command line and configuration spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Us => "us",
            Self::European => "european",
            Self::Cn => "cn",
        }
    }

    /// The label the panel prints before the value.
    #[must_use]
    pub const fn label_key(self) -> MessageKey {
        match self {
            Self::Us => keys::AQI_US_LABEL,
            Self::European => keys::AQI_EUROPEAN_LABEL,
            Self::Cn => keys::AQI_CN_LABEL,
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
            "cn" => Ok(Self::Cn),
            other => Err(Error::Usage(format!(
                "unknown air-quality index `{other}`; known indices: us, european, cn"
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
    /// `0..=50` Chinese: 优 (Excellent).
    CnExcellent,
    /// `51..=100` Chinese: 良 (Good).
    CnGood,
    /// `101..=150` Chinese: 轻度污染 (light pollution).
    CnLight,
    /// `151..=200` Chinese: 中度污染 (moderate pollution).
    CnModerate,
    /// `201..=300` Chinese: 重度污染 (heavy pollution).
    CnHeavy,
    /// Above 300 Chinese: 严重污染 (severe pollution).
    CnSevere,
}

impl AqiCategory {
    /// Every category, in the index order [`Self::i18n_key`] relies on.
    ///
    /// The first ten are the US and European words, the last six the Chinese scale's.
    pub const ALL: [Self; 16] = [
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
        Self::CnExcellent,
        Self::CnGood,
        Self::CnLight,
        Self::CnModerate,
        Self::CnHeavy,
        Self::CnSevere,
    ];

    /// The Chinese scale's six categories, in range order.
    pub const CN: [Self; 6] = [
        Self::CnExcellent,
        Self::CnGood,
        Self::CnLight,
        Self::CnModerate,
        Self::CnHeavy,
        Self::CnSevere,
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
            Self::CnExcellent => 10,
            Self::CnGood => 11,
            Self::CnLight => 12,
            Self::CnModerate => 13,
            Self::CnHeavy => 14,
            Self::CnSevere => 15,
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

    /// The category of a raw Chinese national AQI (GB 3095-2012).
    ///
    /// The standard's six bands are 优 `0..=50`, 良 `51..=100`, 轻度污染 `101..=150`, 中度污染
    /// `151..=200`, 重度污染 `201..=300` and 严重污染 above 300; the published table stops at an
    /// index of 500, so anything above 300 (500 included) reads as 严重污染 rather than as a
    /// seventh band.
    #[must_use]
    pub const fn from_cn(index: u16) -> Self {
        match index {
            0..=50 => Self::CnExcellent,
            51..=100 => Self::CnGood,
            101..=150 => Self::CnLight,
            151..=200 => Self::CnModerate,
            201..=300 => Self::CnHeavy,
            _ => Self::CnSevere,
        }
    }

    /// The kebab-case spelling the JSON document uses.
    ///
    /// Spellings are unique within a scale, not across scales: `good` is the US scale's `0..=50`
    /// word and the Chinese scale's `51..=100` word, and the document namespaces them
    /// (`category.us` versus `category.cn`).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Good | Self::CnGood => "good",
            Self::Moderate => "moderate",
            Self::UnhealthyForSensitiveGroups => "unhealthy-sensitive",
            Self::Unhealthy => "unhealthy",
            Self::VeryUnhealthy => "very-unhealthy",
            Self::Hazardous => "hazardous",
            Self::Fair => "fair",
            Self::Poor => "poor",
            Self::VeryPoor => "very-poor",
            Self::ExtremelyPoor => "extremely-poor",
            Self::CnExcellent => "excellent",
            Self::CnLight => "light-pollution",
            Self::CnModerate => "moderate-pollution",
            Self::CnHeavy => "heavy-pollution",
            Self::CnSevere => "severe-pollution",
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
        for (position, category) in AqiCategory::ALL.into_iter().enumerate() {
            assert_eq!(category.index(), position, "{category:?}");
            assert!(keys.insert(category.i18n_key().as_str().into_owned()));
        }
        // Spellings are unique within a scale: the JSON document namespaces them by scale, and
        // `good` is deliberately the US `0..=50` word *and* the Chinese `51..=100` word.
        let mut us_european = std::collections::HashSet::new();
        let mut chinese = std::collections::HashSet::new();
        for category in AqiCategory::ALL {
            let set = if AqiCategory::CN.contains(&category) {
                &mut chinese
            } else {
                &mut us_european
            };
            assert!(set.insert(category.as_str()), "{category:?}");
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
        assert_eq!(
            AqiCategory::from_cn(20).i18n_key().as_str(),
            "aqi-category-cn-excellent"
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
        assert_eq!(AqiIndex::from_name("CN").expect("cn parses"), AqiIndex::Cn);
        assert_eq!(AqiIndex::default(), AqiIndex::Us);
        assert_eq!(AqiIndex::Us.as_str(), "us");
        assert_eq!(AqiIndex::European.to_string(), "european");
        assert_eq!(AqiIndex::Cn.to_string(), "cn");
        assert_eq!(
            AqiIndex::ALL.map(AqiIndex::as_str),
            ["us", "european", "cn"]
        );
        let error = AqiIndex::from_name("epa").expect_err("never a scale");
        assert!(
            error
                .to_string()
                .contains("known indices: us, european, cn")
        );
    }
}
