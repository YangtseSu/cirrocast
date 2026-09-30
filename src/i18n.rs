// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Language selection and the message lookup every renderer goes through.
//!
//! A renderer never writes a user visible word itself: it asks [`I18n`] for a key, and the key is
//! what a catalog is written against. Step 07 ships the indirection and the built-in `en-US`
//! messages; step 09 adds the Fluent catalogs and the environment negotiation behind
//! [`LanguageId`], without any renderer changing.
//!
//! Keys follow the naming the plan fixed: `part-morning` … `part-night` for the day-part labels,
//! `label-*` for the fixed labels of a line, and conditions through [`Condition`] so that the WMO
//! table stays the single source of the English text.

use chrono::NaiveDate;

use crate::error::{Error, Result};
use crate::model::DayPartKind;
use crate::model::condition::Condition;

/// A language a report can be rendered in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LanguageId {
    tag: &'static str,
}

impl LanguageId {
    /// The built-in language: every message exists in it, and it is what `auto` resolves to until
    /// step 09 negotiates from the environment.
    pub const EN_US: Self = Self { tag: "en-US" };

    /// The BCP-47 tag, e.g. `en-US`.
    #[must_use]
    pub const fn tag(self) -> &'static str {
        self.tag
    }

    /// Resolves a `language` setting (`defaults.language`, and the `--lang` flag).
    ///
    /// A tag this build has no messages for is refused instead of silently rendering English: a
    /// user who asked for `zh-CN` must be told that the build cannot do it, not shown a language
    /// they did not ask for.
    pub fn from_setting(value: &str) -> Result<Self> {
        let value = value.trim();
        if value.eq_ignore_ascii_case("auto")
            || value.eq_ignore_ascii_case("en")
            || value.eq_ignore_ascii_case("en-US")
        {
            return Ok(Self::EN_US);
        }
        Err(Error::Usage(format!(
            "language `{value}` has no messages in this build; only en-US is implemented so far"
        )))
    }
}

impl Default for LanguageId {
    fn default() -> Self {
        Self::EN_US
    }
}

/// The message table of one language.
///
/// Cheap to copy and to build: it names a language, nothing else, so a caller can put it in a
/// context without thinking about lifetimes.
#[derive(Debug, Clone, Copy, Default)]
pub struct I18n {
    lang: LanguageId,
}

impl I18n {
    /// The catalog for `lang`.
    #[must_use]
    pub const fn new(lang: LanguageId) -> Self {
        Self { lang }
    }

    /// The language this catalog is written in.
    #[must_use]
    pub const fn lang(&self) -> LanguageId {
        self.lang
    }

    /// The message for `key`, or `key` itself when the catalog has no entry.
    ///
    /// Returning the key keeps a missing message visible in the output rather than panicking or
    /// inventing text; the catalog completeness test of step 09 is what makes it unreachable.
    #[must_use]
    pub fn text(&self, key: &'static str) -> &'static str {
        match key {
            "label-report" => "Weather report:",
            "label-data" => "Data:",
            "uv-band-low" => "low",
            "uv-band-moderate" => "moderate",
            "uv-band-high" => "high",
            "uv-band-very-high" => "very high",
            "uv-band-extreme" => "extreme",
            "part-morning" => DayPartKind::Morning.label(),
            "part-noon" => DayPartKind::Noon.label(),
            "part-evening" => DayPartKind::Evening.label(),
            "part-night" => DayPartKind::Night.label(),
            _ => key,
        }
    }

    /// The label of one day part.
    #[must_use]
    pub fn part(&self, kind: DayPartKind) -> &'static str {
        self.text(match kind {
            DayPartKind::Morning => "part-morning",
            DayPartKind::Noon => "part-noon",
            DayPartKind::Evening => "part-evening",
            DayPartKind::Night => "part-night",
        })
    }

    /// The text of a canonical condition.
    ///
    /// Today this is the WMO table's English description; step 09 looks the condition's
    /// `i18n_key` up in the catalog. Renderers call this instead of the model so that the switch
    /// happens in one place.
    #[must_use]
    pub fn condition(&self, condition: Condition) -> &'static str {
        condition.description_en()
    }

    /// The band name of a UV index, as the WHO scale defines it.
    ///
    /// `2.9` is low and `3.0` moderate: the bands are cut on the value the token prints, so the
    /// number and its name can never disagree (`%u` prints `2` for `2.9` and `3` for `3.0`).
    #[must_use]
    pub fn uv_band(&self, uv: f32) -> &'static str {
        let key = if uv < 3.0 {
            "uv-band-low"
        } else if uv < 6.0 {
            "uv-band-moderate"
        } else if uv < 8.0 {
            "uv-band-high"
        } else if uv < 11.0 {
            "uv-band-very-high"
        } else {
            "uv-band-extreme"
        };
        self.text(key)
    }

    /// The day heading: `Today, Sep 30` for `today`, else `Tue 30 Sep`.
    ///
    /// The weekday and month names come from `chrono`'s own English tables, which no ambient
    /// locale can change — the locale-independence test in `tests/render_snapshots.rs` pins that
    /// nothing from the environment reaches a rendered report.
    #[must_use]
    pub fn date_short(&self, date: NaiveDate, today: NaiveDate) -> String {
        if date == today {
            format!("Today, {}", date.format("%b %d"))
        } else {
            date.format("%a %d %b").to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;

    use super::{I18n, LanguageId};
    use crate::model::DayPartKind;
    use crate::model::condition::Condition;

    fn date(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).expect("a valid date")
    }

    #[test]
    fn the_setting_accepts_the_built_in_language_and_auto() {
        for setting in ["auto", "en", "en-US", "en-us", " AUTO ", "EN-us"] {
            assert_eq!(
                LanguageId::from_setting(setting).expect("a supported setting"),
                LanguageId::EN_US,
                "{setting}"
            );
        }
        assert_eq!(LanguageId::EN_US.tag(), "en-US");
        assert_eq!(LanguageId::default(), LanguageId::EN_US);
    }

    #[test]
    fn a_language_without_messages_is_refused() {
        for setting in ["zh-CN", "de", "fr-CA", "en-GB", ""] {
            let error = LanguageId::from_setting(setting).expect_err("no messages");
            assert_eq!(error.exit_code(), 2, "{setting}");
            assert!(error.to_string().contains("only en-US"), "{setting}");
        }
    }

    #[test]
    fn labels_and_conditions_come_from_the_catalog() {
        let i18n = I18n::new(LanguageId::EN_US);
        assert_eq!(i18n.lang(), LanguageId::EN_US);
        assert_eq!(i18n.text("label-report"), "Weather report:");
        assert_eq!(i18n.text("label-data"), "Data:");
        assert_eq!(i18n.part(DayPartKind::Morning), "Morning");
        assert_eq!(i18n.part(DayPartKind::Night), "Night");
        assert_eq!(i18n.condition(Condition::from_u8(95)), "Thunderstorm");
        assert_eq!(i18n.condition(Condition::from_u8(4)), "Unknown");

        // The bands are cut on the printed value, so a number and its label never disagree.
        for (uv, band) in [
            (0.0, "low"),
            (2.9, "low"),
            (3.0, "moderate"),
            (5.9, "moderate"),
            (6.0, "high"),
            (7.9, "high"),
            (8.0, "very high"),
            (10.9, "very high"),
            (11.0, "extreme"),
            (14.5, "extreme"),
        ] {
            assert_eq!(i18n.uv_band(uv), band, "UV {uv}");
        }
        assert_eq!(
            i18n.text("no-such-key"),
            "no-such-key",
            "a missing message stays visible"
        );
    }

    #[test]
    fn the_day_heading_names_today_and_dates_it_otherwise() {
        let i18n = I18n::new(LanguageId::EN_US);
        assert_eq!(
            i18n.date_short(date(2026, 9, 30), date(2026, 9, 30)),
            "Today, Sep 30"
        );
        assert_eq!(
            i18n.date_short(date(2026, 9, 30), date(2026, 10, 1)),
            "Wed 30 Sep"
        );
        assert_eq!(
            i18n.date_short(date(2026, 10, 1), date(2026, 9, 30)),
            "Thu 01 Oct"
        );
    }
}
