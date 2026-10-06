// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The name-search chain: which geocoding sources a plain name query asks, and in what order
//! (step 25).
//!
//! `[geo] search` (or `CIRROCAST_GEO_SEARCH`) selects them exactly as `--provider` selects
//! forecast backends: `auto` — the default — is Open-Meteo first, then `GeoNames` when an account
//! name is configured, then Nominatim `/search` as the last resort; the three single-source
//! spellings exist so a user can pin one. Only the failures a later source can plausibly answer
//! differently move the chain on ([`Error::Network`] and [`Error::Upstream`], which includes the
//! service's own refusals); anything else — a missing `GeoNames` account under an explicit
//! selection, a usage mistake — stops the run.
//!
//! Three rules are worth restating because they are easy to undo by accident:
//!
//! * **Nominatim is the last resort.** It is a donated service with a one-request-per-second
//!   policy, so it is asked only when nothing before it found a candidate. Under an explicit
//!   `search = "nominatim"` it is the only source and therefore always asked — the rule is about
//!   not *adding* a request to a query that already has an answer;
//! * **a missing `GeoNames` account is not an error under `auto`** — the source is simply skipped,
//!   and the note names the two commands that would add it. An explicitly selected `geonames`
//!   without an account is [`Error::MissingKey`] (exit 6), because the user asked for exactly that
//!   source;
//! * **offline is a miss, not a failure.** With the geo scope silenced the cache never writes, so
//!   a [`Error::Network`] from a source means "no cached answer" — a legitimate no-hit that lets
//!   the caller report `(no offline match)` instead of a network error.
//!
//! The chain returns the per-source hit lists, not one merged list: the caller merges them
//! ([`crate::geo::merge`]) and ranks the result for the query it asked, which is what keeps "which
//! sources answered" and "which place won" separate decisions.

use std::time::Duration;

use crate::cache::Cache;
use crate::error::{Error, Result};
use crate::geo::Geocoder;
use crate::geo::geonames::{self, GeoNamesGeocoder};
use crate::geo::nominatim::Nominatim;
use crate::geo::open_meteo::OpenMeteoGeocoder;
use crate::http::HttpClient;
use crate::model::Location;

/// The values `[geo] search` accepts, in one place so the configuration, the CLI help and the
/// chain cannot disagree.
pub const SEARCH_SETTINGS: &[&str] = &["auto", "open-meteo", "geonames", "nominatim"];

/// A geocoding source behind a plain name query.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeoSource {
    /// The Open-Meteo geocoding API: keyless, the default.
    OpenMeteo,
    /// The `GeoNames` `searchJSON` endpoint: BYOK, the fuzzy index.
    GeoNames,
    /// OpenStreetMap Nominatim `/search`: donated, throttled, the last resort.
    Nominatim,
}

impl GeoSource {
    /// The id `[geo] search`, `CIRROCAST_GEO_SEARCH` and the `-v` lines use.
    #[must_use]
    pub const fn slug(self) -> &'static str {
        match self {
            Self::OpenMeteo => "open-meteo",
            Self::GeoNames => "geonames",
            Self::Nominatim => "nominatim",
        }
    }

    /// The sources `setting` selects, in the order they are asked.
    ///
    /// `auto` is the documented default order; anything else is [`Error::Config`], because the
    /// value comes from the configuration (or its environment override) and a typo there must stop
    /// the run rather than silently pick a source.
    pub fn chain(setting: &str) -> Result<Vec<Self>> {
        match setting.trim() {
            "auto" => Ok(vec![Self::OpenMeteo, Self::GeoNames, Self::Nominatim]),
            "open-meteo" => Ok(vec![Self::OpenMeteo]),
            "geonames" => Ok(vec![Self::GeoNames]),
            "nominatim" => Ok(vec![Self::Nominatim]),
            other => Err(Error::Config(format!(
                "geo.search: `{other}` is not one of {}",
                SEARCH_SETTINGS.join(", ")
            ))),
        }
    }

    /// Whether the source is asked only when the ones before it found nothing.
    const fn last_resort(self) -> bool {
        matches!(self, Self::Nominatim)
    }
}

/// The inputs a chain run needs beyond the query itself.
pub struct SearchInputs<'a> {
    /// The shared HTTP client.
    pub http: &'a HttpClient,
    /// The cache view of the geo scope.
    pub cache: &'a Cache,
    /// How long a source's answer is reused (`cache.geocode_ttl_secs`).
    pub ttl: Duration,
    /// How many candidates each source may return.
    pub limit: u8,
    /// The `GeoNames` account name, when one is configured (`CIRROCAST_GEONAMES_USER` or
    /// `keys.toml`).
    pub geonames_user: Option<String>,
    /// The Nominatim base URL (`network.nominatim_url`, empty = the public service).
    pub nominatim_url: String,
    /// Whether the geo scope is silenced, which turns a network failure into a cache miss.
    pub offline: bool,
}

/// What a chain run found.
#[derive(Debug, Default)]
pub struct SearchReport {
    /// The sources that answered, in chain order, each with its hits (possibly empty).
    pub answered: Vec<(GeoSource, Vec<Location>)>,
    /// One line per source, in chain order, for a `-v` run: the hit count, why it was skipped, or
    /// why it failed.
    pub notes: Vec<String>,
}

impl SearchReport {
    /// Whether no source contributed a single candidate.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.answered.iter().all(|(_, hits)| hits.is_empty())
    }
}

/// One source's answer.
enum Answer {
    /// The hits, in the source's own order.
    Hits(Vec<Location>),
    /// The source has nothing to ask with: no `GeoNames` account under `auto`.
    NoCredential,
}

/// The selected chain, ready to be asked.
pub struct SearchChain<'a> {
    /// The sources, in the order they are asked.
    sources: Vec<GeoSource>,
    /// Whether the selection was `auto`, which decides how a missing account is treated.
    automatic: bool,
    /// The run's shared inputs.
    inputs: SearchInputs<'a>,
}

impl<'a> SearchChain<'a> {
    /// Builds the chain `setting` selects.
    pub fn new(setting: &str, inputs: SearchInputs<'a>) -> Result<Self> {
        Ok(Self {
            sources: GeoSource::chain(setting)?,
            automatic: setting.trim() == "auto",
            inputs,
        })
    }

    /// The sources, in the order they will be asked, for the caller's `-v` line.
    #[must_use]
    pub fn sources(&self) -> &[GeoSource] {
        &self.sources
    }

    /// Asks every source the selection covers, in order.
    ///
    /// Returns the per-source hit lists and the narration lines; fails only when *no* source
    /// answered and at least one failed, with every attempt named (so the reader can tell which
    /// services were tried, not only which one happened to fail last).
    pub fn search(&self, query: &str) -> Result<SearchReport> {
        let mut report = SearchReport::default();
        let mut attempts: Vec<String> = Vec::new();
        for &source in &self.sources {
            let found = report.answered.iter().any(|(_, hits)| !hits.is_empty());
            if source.last_resort() && found {
                report.notes.push(format!(
                    "{}: not asked (an earlier source answered)",
                    source.slug()
                ));
                continue;
            }
            match self.ask(source, query) {
                Ok(Answer::Hits(hits)) => {
                    report
                        .notes
                        .push(format!("{}: {} candidates", source.slug(), hits.len()));
                    report.answered.push((source, hits));
                }
                Ok(Answer::NoCredential) => {
                    report.notes.push(format!(
                        "{}: skipped (no account name; `cirrocast key set geonames` or {} adds it)",
                        source.slug(),
                        geonames::ENV_VAR
                    ));
                }
                // Offline, the only failure a source can meet is a cache miss: the transport is
                // never reached, so "nothing on disk" is a legitimate no-hit, exactly as it is for
                // a single-source run.
                Err(error) if self.inputs.offline && matches!(error, Error::Network(_)) => {
                    report
                        .notes
                        .push(format!("{}: offline, no cached answer", source.slug()));
                    report.answered.push((source, Vec::new()));
                }
                Err(error) if is_skippable(&error) => {
                    attempts.push(format!("{} ({})", source.slug(), error.chain_reason()));
                    report
                        .notes
                        .push(format!("{}: {}", source.slug(), error.chain_reason()));
                }
                Err(error) => return Err(error),
            }
        }
        if report.answered.is_empty() && !attempts.is_empty() {
            return Err(Error::Chain {
                subject: "geocoding sources",
                attempts,
            });
        }
        Ok(report)
    }

    /// One source's answer.
    fn ask(&self, source: GeoSource, query: &str) -> Result<Answer> {
        let inputs = &self.inputs;
        let hits = match source {
            GeoSource::OpenMeteo => OpenMeteoGeocoder::new(inputs.http, inputs.cache, inputs.ttl)
                .search(query, inputs.limit)?,
            GeoSource::GeoNames => {
                let Some(user) = inputs.geonames_user.as_deref() else {
                    if self.automatic {
                        return Ok(Answer::NoCredential);
                    }
                    return Err(Error::MissingKey {
                        provider: geonames::CREDENTIAL.to_owned(),
                        env: geonames::ENV_VAR.to_owned(),
                    });
                };
                GeoNamesGeocoder::new(inputs.http, inputs.cache, inputs.ttl, user)
                    .search(query, inputs.limit)?
            }
            GeoSource::Nominatim => {
                Nominatim::new(inputs.http, inputs.cache, inputs.nominatim_url.clone())
                    .search(query, inputs.limit)?
            }
        };
        Ok(Answer::Hits(hits))
    }
}

/// Whether a failure is one the next source can plausibly answer differently.
fn is_skippable(error: &Error) -> bool {
    matches!(error, Error::Network(_) | Error::Upstream { .. })
}

#[cfg(test)]
mod tests {
    use super::{GeoSource, SEARCH_SETTINGS};

    /// The setting to source list mapping, including the rejections.
    #[test]
    fn the_setting_selects_the_sources() {
        assert_eq!(
            GeoSource::chain("auto").expect("`auto` is a known setting"),
            vec![
                GeoSource::OpenMeteo,
                GeoSource::GeoNames,
                GeoSource::Nominatim
            ]
        );
        assert_eq!(
            GeoSource::chain("open-meteo").expect("a known setting"),
            vec![GeoSource::OpenMeteo]
        );
        assert_eq!(
            GeoSource::chain("geonames").expect("a known setting"),
            vec![GeoSource::GeoNames]
        );
        assert_eq!(
            GeoSource::chain("nominatim").expect("a known setting"),
            vec![GeoSource::Nominatim]
        );
        assert_eq!(
            GeoSource::chain(" open-meteo ").expect("blanks are trimmed"),
            vec![GeoSource::OpenMeteo]
        );

        for junk in ["", " ", "Open-Meteo", "osm", "all", "open-meteo,geonames"] {
            let error = GeoSource::chain(junk).expect_err("junk must be rejected");
            assert_eq!(error.exit_code(), 4, "`{junk}` should be a config error");
            assert!(
                error.to_string().contains("geo.search"),
                "`{junk}`: {error}"
            );
        }
        assert_eq!(SEARCH_SETTINGS.len(), 4);
    }

    /// The last-resort rule is about the request, not about the selection.
    #[test]
    fn nominatim_is_the_only_last_resort_source() {
        assert!(GeoSource::Nominatim.last_resort());
        assert!(!GeoSource::OpenMeteo.last_resort());
        assert!(!GeoSource::GeoNames.last_resort());
    }

    #[test]
    fn the_slugs_are_the_documented_ids() {
        assert_eq!(GeoSource::OpenMeteo.slug(), "open-meteo");
        assert_eq!(GeoSource::GeoNames.slug(), "geonames");
        assert_eq!(GeoSource::Nominatim.slug(), "nominatim");
    }
}
