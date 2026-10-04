// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The one name-folding rule shared by the offline index, the query side and the builder.
//!
//! `src/geo/data/keys.bin.gz` (step 18) is built by `build/geo-table`, which calls this same
//! function, so the index and the query can never disagree about what a name means. Folding is
//! NFKD, then combining marks dropped, then lower case, then every non-alphanumeric character
//! removed, which is what makes `São Paulo`/`Sao Paulo`, `MÜNCHEN`/`munchen`, `St. Louis`/
//! `stlouis` and the Han spellings of a city (`北京`, kept as they are) meet their index keys.
//!
//! The network path folds its candidates with the same rule, so `location search` ranks a name
//! the same way whichever source answered.

use unicode_normalization::UnicodeNormalization as _;

/// Folds a place name or a query into the form the index stores.
#[must_use]
pub fn fold(text: &str) -> String {
    text.nfkd()
        .filter(|character| !unicode_normalization::char::is_combining_mark(*character))
        .flat_map(char::to_lowercase)
        .filter(|character| character.is_alphanumeric())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::fold;

    #[test]
    fn diacritics_and_case_fold_together() {
        for (input, expected) in [
            ("São Paulo", "saopaulo"),
            ("Sao Paulo", "saopaulo"),
            ("SAO PAULO", "saopaulo"),
            ("MÜNCHEN", "munchen"),
            ("München", "munchen"),
            ("Wien", "wien"),
            ("Vienna", "vienna"),
            ("İstanbul", "istanbul"),
            ("Zürich", "zurich"),
        ] {
            assert_eq!(fold(input), expected, "{input}");
        }
    }

    #[test]
    fn punctuation_is_dropped_and_scripts_survive() {
        assert_eq!(fold("St. Louis"), "stlouis");
        assert_eq!(fold("Washington, D.C."), "washingtondc");
        assert_eq!(fold("北京"), "北京");
        assert_eq!(fold("北京市"), "北京市");
        assert_eq!(fold("Køge"), "køge");
        assert_eq!(fold("!!!"), "");
        assert_eq!(fold(""), "");
    }

    #[test]
    fn compatibility_forms_decompose() {
        // Full-width Latin letters and the ligature both have NFKD decompositions.
        assert_eq!(fold("ＢＥＩＪＩＮＧ"), "beijing");
        assert_eq!(fold("ﬁji"), "fiji");
    }
}
