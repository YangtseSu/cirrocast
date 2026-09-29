// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The canonical weather condition: a WMO 4677 code.
//!
//! Every provider translates whatever its own API calls a condition into a code from this table,
//! and everything downstream — day-part aggregation, art blocks, translated text, severity
//! ordering — works on the code alone. Provider specific codes never leave a provider module.
//!
//! The type is deliberately **total**. Upstream APIs return codes this table does not describe
//! (and occasionally codes outside the WMO range), and clamping those to "clear" or "overcast"
//! would invent weather that was never reported. An unknown code keeps its number, reports
//! [`Condition::is_known`] as `false`, sorts below every described code and renders as "Unknown".

use serde::{Deserialize, Serialize};

/// One row of [`CODES`]: `(code, i18n key, English description, severity rank, art key)`.
type Row = (u8, &'static str, &'static str, u8, &'static str);

/// Every WMO 4677 code `cirrocast` describes, in code order.
///
/// The descriptions are re-authored from the WMO code names (the same names the Open-Meteo
/// documentation publishes); nothing here is copied from `wego` or `wttr.in`.
///
/// The severity rank is a **project local** total order, not a WMO one. Its only promise is
/// "higher means more significant weather", which is what the day-part aggregation needs when it
/// picks a single code per part. Clear and mainly clear share rank 1, and unknown codes rank below
/// both, so a described code always wins a tie.
const CODES: [Row; 29] = [
    (0, "cond.0", "Clear sky", 1, "clear"),
    (1, "cond.1", "Mainly clear", 1, "mainly-clear"),
    (2, "cond.2", "Partly cloudy", 2, "partly-cloudy"),
    (3, "cond.3", "Overcast", 3, "overcast"),
    (45, "cond.45", "Fog", 4, "fog"),
    (48, "cond.48", "Depositing rime fog", 5, "rime-fog"),
    (51, "cond.51", "Light drizzle", 6, "drizzle-light"),
    (53, "cond.53", "Moderate drizzle", 7, "drizzle"),
    (55, "cond.55", "Dense drizzle", 8, "drizzle-dense"),
    (
        56,
        "cond.56",
        "Light freezing drizzle",
        9,
        "freezing-drizzle-light",
    ),
    (
        57,
        "cond.57",
        "Dense freezing drizzle",
        10,
        "freezing-drizzle-dense",
    ),
    (61, "cond.61", "Slight rain", 11, "rain-light"),
    (63, "cond.63", "Moderate rain", 12, "rain"),
    (65, "cond.65", "Heavy rain", 13, "rain-heavy"),
    (
        66,
        "cond.66",
        "Light freezing rain",
        14,
        "freezing-rain-light",
    ),
    (
        67,
        "cond.67",
        "Heavy freezing rain",
        15,
        "freezing-rain-heavy",
    ),
    (71, "cond.71", "Slight snow fall", 16, "snow-light"),
    (73, "cond.73", "Moderate snow fall", 17, "snow"),
    (75, "cond.75", "Heavy snow fall", 18, "snow-heavy"),
    (77, "cond.77", "Snow grains", 19, "snow-grains"),
    (
        80,
        "cond.80",
        "Slight rain showers",
        20,
        "showers-rain-light",
    ),
    (81, "cond.81", "Moderate rain showers", 21, "showers-rain"),
    (
        82,
        "cond.82",
        "Violent rain showers",
        22,
        "showers-rain-violent",
    ),
    (
        85,
        "cond.85",
        "Slight snow showers",
        23,
        "showers-snow-light",
    ),
    (
        86,
        "cond.86",
        "Heavy snow showers",
        24,
        "showers-snow-heavy",
    ),
    (95, "cond.95", "Thunderstorm", 25, "thunderstorm"),
    (
        96,
        "cond.96",
        "Thunderstorm with slight hail",
        26,
        "thunderstorm-hail-light",
    ),
    (
        97,
        "cond.97",
        "Heavy thunderstorm",
        27,
        "thunderstorm-heavy",
    ),
    (
        99,
        "cond.99",
        "Thunderstorm with heavy hail",
        28,
        "thunderstorm-hail-heavy",
    ),
];

/// A weather condition as a WMO 4677 code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Condition(u8);

impl Condition {
    /// Wraps a raw upstream code. Total by design: this call never fails.
    #[must_use]
    pub const fn from_u8(code: u8) -> Self {
        Self(code)
    }

    /// The raw code, exactly as a provider or a cache entry carried it.
    #[must_use]
    pub const fn code(self) -> u8 {
        self.0
    }

    /// Whether this code has a description in [`CODES`].
    #[must_use]
    pub fn is_known(self) -> bool {
        self.row().is_some()
    }

    /// Lookup key for the Fluent catalog, e.g. `cond.95`; `cond.unknown` when undescribed.
    #[must_use]
    pub fn i18n_key(self) -> &'static str {
        self.row().map_or("cond.unknown", |row| row.1)
    }

    /// The English description, or `Unknown` for unnamed codes.
    ///
    /// This is the only condition text that exists in the model; localised text is the Fluent
    /// catalogs' job (step 09), which key off [`Condition::i18n_key`].
    #[must_use]
    pub fn description_en(self) -> &'static str {
        self.row().map_or("Unknown", |row| row.2)
    }

    /// The art key from the fixed vocabulary, e.g. `snow-heavy`; `unknown` when undescribed.
    ///
    /// The render layer maps these keys to artwork, never raw codes.
    #[must_use]
    pub fn art_key(self) -> &'static str {
        self.row().map_or("unknown", |row| row.4)
    }

    /// The project local severity rank; higher means more significant weather, `0` is unknown.
    #[must_use]
    pub fn severity_rank(self) -> u8 {
        self.row().map_or(0, |row| row.3)
    }

    /// Whether precipitation reaches the ground: drizzle, rain, freezing rain, snow, snow grains,
    /// rain and snow showers, and every thunderstorm code.
    #[must_use]
    pub fn is_precipitation(self) -> bool {
        self.is_known() && matches!(self.0, 51..=57 | 61..=67 | 71..=77 | 80..=86 | 95..=99)
    }

    /// Whether visibility is reduced by fog (including depositing rime fog).
    #[must_use]
    pub fn is_fog(self) -> bool {
        self.is_known() && matches!(self.0, 45 | 48)
    }

    /// Whether the code is a thunderstorm, with or without hail.
    #[must_use]
    pub fn is_thunder(self) -> bool {
        self.is_known() && matches!(self.0, 95..=99)
    }

    /// Whether the sky is clear or mainly clear.
    #[must_use]
    pub fn is_clear(self) -> bool {
        self.is_known() && matches!(self.0, 0 | 1)
    }

    /// The table row for this code, if the table describes it.
    fn row(self) -> Option<Row> {
        CODES.iter().copied().find(|row| row.0 == self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::{CODES, Condition};

    /// The table itself has invariants the lookup methods silently rely on; a typo in a row would
    /// otherwise only surface as a wrong description number.
    #[test]
    fn table_is_ordered_unique_and_self_consistent() {
        for window in CODES.windows(2) {
            let (previous, next) = (window[0], window[1]);
            assert!(
                previous.0 < next.0,
                "codes must ascend: {previous:?} {next:?}"
            );
            assert!(
                previous.3 <= next.3,
                "ranks must not go backwards: {previous:?} {next:?}"
            );
        }

        let mut art_keys: Vec<&str> = CODES.iter().map(|row| row.4).collect();
        art_keys.sort_unstable();
        let unique = art_keys.len();
        art_keys.dedup();
        assert_eq!(unique, art_keys.len(), "art keys must be unique");

        for row in CODES {
            assert_eq!(row.1, format!("cond.{}", row.0), "i18n key for {}", row.0);
            assert!(!row.2.is_empty(), "empty description for {}", row.0);
            assert!(row.3 >= 1, "described codes rank at least 1: {}", row.0);
            assert!(!row.4.is_empty(), "empty art key for {}", row.0);
        }
        assert_eq!(CODES.first().map(|row| row.3), Some(1));
    }

    #[test]
    fn unknown_codes_are_inert() {
        for code in [4_u8, 20, 46, 87, 100, 255] {
            let condition = Condition::from_u8(code);
            assert!(!condition.is_known(), "{code} must be unknown");
            assert_eq!(condition.code(), code);
            assert_eq!(condition.i18n_key(), "cond.unknown");
            assert_eq!(condition.description_en(), "Unknown");
            assert_eq!(condition.art_key(), "unknown");
            assert_eq!(condition.severity_rank(), 0);
            assert!(!condition.is_precipitation());
            assert!(!condition.is_fog());
            assert!(!condition.is_thunder());
            assert!(!condition.is_clear());
        }
    }

    #[test]
    fn a_described_code_outranks_an_unknown_one() {
        assert!(Condition::from_u8(0).severity_rank() > Condition::from_u8(200).severity_rank());
    }
}
