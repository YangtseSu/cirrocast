// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Severe-weather alerts: their own source registry, independent of the weather chain.
//!
//! The alert sources answer different questions than the forecast backends: they cover different
//! jurisdictions (`NWS`: the US; `HKO`: Hong Kong; `MeteoAlarm`: the EUMETNET members; `QWeather`:
//! China) or aggregate the whole world (WMO `SWIC`, `FPAS`), and which of them applies is a property of
//! the *resolved location*, not of the provider chain. A source is therefore selected by coverage
//! ([`sources_for`]), fetched through the shared HTTP client and cache, normalised to the CAP-shaped
//! [`Alert`] model, and only then filtered, de-duplicated and sorted ([`prepare`]).
//!
//! The data types live in `src/model/alert.rs` and are re-exported here: `model::Report` carries
//! the alerts, and the render layer may read the model but not this module.
//!
//! Failure policy, in one place:
//!
//! * **auto-selected sources are best-effort** — a source that fails is a `--verbose` note and the
//!   others' answers are used; a federated aggregator having a bad hour must not cost the user the
//!   forecast;
//! * **explicitly named sources are promises** — `--alerts-from` (or a config source list) that
//!   fails propagates its error, because the user asked for exactly that source;
//! * **a missing `MeteoAlarm` key is not an error at all** — the source is skipped with a note, as
//!   its token is optional BYOK;
//! * a source that does not cover the location is a usage error before any request is sent.

pub mod cap;
pub mod fpas;
pub mod geometry;
pub mod hko;
pub mod meteoalarm;
pub mod nws;
pub mod qweather;
pub mod wmoswic;

use std::collections::HashSet;
use std::time::Duration;

use crate::cache::{CacheKey, CacheMode};
use crate::config::AlertsConfig;
use crate::error::{Error, Result};
use crate::http::HttpRequest;
use crate::model::Location;
use crate::provider::{Env, ProviderId};

pub use crate::model::{Alert, AlertSource, Certainty, Severity, Urgency};

/// One run's alert policy: which sources, how strong a warning must be, and whether the user named
/// them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlertsRequest {
    /// The sources to query, already coverage-checked.
    pub sources: Vec<AlertSource>,
    /// Whether the source list came from `--alerts-from`/config rather than `auto`: explicit
    /// sources propagate their failures instead of degrading.
    pub explicit: bool,
    /// The lowest severity shown; weaker (and unknown-severity) alerts are dropped.
    pub threshold: Severity,
}

/// The sources whose coverage includes `loc`, in registry order.
///
/// `chain` is the weather provider chain of the run: `QWeather` and `VisualCrossing` alerts are
/// pulled in only when their provider is on the chain, because they reuse its credential and host.
/// The global aggregators come last (registry order), so their answers only supplement a national
/// service's.
#[must_use]
pub fn auto_sources(loc: &Location, chain: &[ProviderId]) -> Vec<AlertSource> {
    AlertSource::ALL
        .into_iter()
        .filter(|source| source.available() && source.covers(loc))
        .filter(|source| match source.provider() {
            Some(id) => chain.iter().any(|provider| provider.as_str() == id),
            None => true,
        })
        .collect()
}

/// The sources a coverage check alone admits for `loc`, for the error message of an explicit list:
/// unlike [`auto_sources`] this ignores the weather chain, so a user is told about a source their
/// current `--provider` would not have pulled in.
#[must_use]
pub fn covered_sources(loc: &Location) -> Vec<AlertSource> {
    AlertSource::ALL
        .into_iter()
        .filter(|source| source.available() && source.covers(loc))
        .collect()
}

/// Resolves the configured `[alerts] sources`: `auto` expands by coverage, an explicit list is
/// checked against the location.
pub fn sources_for(
    loc: &Location,
    chain: &[ProviderId],
    config: &AlertsConfig,
) -> Result<Vec<AlertSource>> {
    if is_auto(&config.sources) {
        return Ok(auto_sources(loc, chain));
    }
    explicit_sources(loc, &config.sources)
}

/// Parses an explicit source list into known, available sources, de-duplicated in input order.
///
/// This is the pre-flight the CLI runs *before* any location resolution: an unknown id or a source
/// this build cannot fetch is a usage error that must not cost a request, and the error the runtime
/// would raise is raised early instead of after the location lookup has opened a socket.
pub fn parse_specs(specs: &[String]) -> Result<Vec<AlertSource>> {
    let mut sources = Vec::new();
    for spec in specs {
        let source: AlertSource = spec.trim().parse()?;
        if !source.available() {
            return Err(Error::Usage(format!(
                "alert source `{source}` is not wired up yet"
            )));
        }
        if !sources.contains(&source) {
            sources.push(source);
        }
    }
    Ok(sources)
}

/// [`sources_for`] for an explicit list (`--alerts-from` or `[alerts] sources`).
pub fn explicit_sources(loc: &Location, specs: &[String]) -> Result<Vec<AlertSource>> {
    let sources = parse_specs(specs)?;
    for source in sources.iter().copied() {
        if !source.covers(loc) {
            let covered = covered_sources(loc);
            let covered = if covered.is_empty() {
                "none".to_owned()
            } else {
                covered
                    .iter()
                    .map(|source| source.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            return Err(Error::Usage(format!(
                "alert source `{source}` does not cover {}; covered here: {covered}",
                place(loc)
            )));
        }
    }
    Ok(sources)
}

/// Whether the source can be selected in this build.
///
/// This is an inherent impl rather than a method on the model because it consults the provider
/// registry, and `src/model` may not depend on `src/provider` (the render-gate rule). A
/// provider-bound source (`QWeather`, `VisualCrossing`) is available exactly when the provider it
/// borrows its credential from is implemented; a source with no provider binding is always
/// available.
impl AlertSource {
    /// Whether this source can be selected and fetched today.
    #[must_use]
    pub fn available(self) -> bool {
        match self.provider() {
            Some(id) => id
                .parse::<ProviderId>()
                .is_ok_and(|provider| provider.metadata().implemented),
            None => true,
        }
    }
}

/// Whether the configured source list is the `auto` selector.
#[must_use]
pub fn is_auto(sources: &[String]) -> bool {
    sources.len() == 1 && sources[0].trim().eq_ignore_ascii_case("auto")
}

/// `39.90,116.40`, the spelling the coverage error message uses.
#[must_use]
fn place(loc: &Location) -> String {
    format!("{:.2},{:.2}", loc.lat, loc.lon)
}

/// Fetches every source's alerts and applies the liveness, severity, order and dedup rules.
///
/// `language` is the run's resolved output language, used for CAP `info` block selection.
pub fn fetch(
    loc: &Location,
    env: &Env<'_>,
    request: &AlertsRequest,
    language: &str,
    attached: Vec<Alert>,
) -> Result<Vec<Alert>> {
    let mut alerts = Vec::new();
    for source in &request.sources {
        // `visualcrossing` is the one source whose warnings arrive inside its *provider's*
        // payload: the backend decodes them into `attached`, so there is no endpoint to call and
        // the loop skips it here (a fetch for it would be a second request for data already in
        // hand). It joins below, before the shared filter/dedup/order pass.
        if *source == AlertSource::VisualCrossing {
            continue;
        }
        match fetch_source(*source, loc, env, language) {
            Ok(mut found) => alerts.append(&mut found),
            Err(error) if request.explicit => return Err(error),
            Err(error) => {
                if env.verbose > 0 {
                    eprintln!("alerts: {source}: {error}");
                }
            }
        }
    }
    if request.sources.contains(&AlertSource::VisualCrossing) {
        alerts.extend(attached);
    }
    let now = chrono::DateTime::<chrono::Utc>::from(env.cache.clock().now()).fixed_offset();
    Ok(prepare(alerts, now, request.threshold))
}

/// Fetches one source; the dispatcher the failure policy in [`fetch`] operates on.
fn fetch_source(
    source: AlertSource,
    loc: &Location,
    env: &Env<'_>,
    language: &str,
) -> Result<Vec<Alert>> {
    match source {
        AlertSource::Nws => nws::fetch(loc, env, language),
        AlertSource::MeteoAlarm => meteoalarm::fetch(loc, env, language),
        AlertSource::QWeather => qweather::fetch(loc, env, language),
        AlertSource::Hko => hko::fetch(loc, env, language),
        AlertSource::WmoSwic => wmoswic::fetch(loc, env, language),
        AlertSource::Fpas => fpas::fetch(loc, env, language),
        // `visualcrossing` never reaches this dispatcher: its warnings arrive inside the forecast
        // payload the answering backend already decoded, and [`fetch`] merges them before
        // [`prepare`]. The arm exists because the match must be total, and answers like the
        // internal mistake it is rather than inventing a second request for data in hand.
        AlertSource::VisualCrossing => Err(Error::Other(
            "internal: the `visualcrossing` warnings travel with the forecast payload".to_owned(),
        )),
    }
}

/// The final alert list: expired ones dropped, the threshold applied, strongest first, duplicates
/// collapsed.
///
/// * **liveness** is `coalesce(ends, expires) <= now` → dropped; a missing end means live.
/// * **order** is severity descending, then earliest onset, then id, so the same input always
///   renders the same banner.
/// * **dedup** first by the source's own id (the same alert fetched twice), then by
///   `(event, onset, areas)` across sources — a warning two aggregators both carry is one banner
///   line, and the survivor is the stronger-sorted one.
#[must_use]
pub fn prepare(
    alerts: Vec<Alert>,
    now: chrono::DateTime<chrono::FixedOffset>,
    threshold: Severity,
) -> Vec<Alert> {
    let mut live: Vec<Alert> = alerts
        .into_iter()
        .filter(|alert| alert.is_live_at(now) && alert.severity >= threshold)
        .collect();
    live.sort_by(|left, right| {
        right
            .severity
            .cmp(&left.severity)
            .then_with(|| match (left.onset, right.onset) {
                (Some(left), Some(right)) => left.cmp(&right),
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, None) => std::cmp::Ordering::Equal,
            })
            .then_with(|| left.id.cmp(&right.id))
    });
    let mut seen_ids = HashSet::new();
    let mut seen_events = HashSet::new();
    live.retain(|alert| {
        let event_key = (
            alert.event.trim().to_ascii_lowercase(),
            alert.onset,
            alert.areas.join("|").to_ascii_lowercase(),
        );
        seen_ids.insert(alert.id.clone()) && seen_events.insert(event_key)
    });
    live
}

// ---------------------------------------------------------------------------------------------
// Fetch plumbing
// ---------------------------------------------------------------------------------------------

/// The cache key of one alert response: one entry per source and place, per UTC hour.
pub(crate) fn key(env: &Env<'_>, source: &str, loc: &Location) -> CacheKey {
    let hour = env.cache.clock().now().into();
    CacheKey::alert(source, loc.lat, loc.lon, hour)
}

/// The cache key of a CAP document that is identified by its own identifier rather than a place
/// (the WMO and `FPAS` documents fetched once each per index entry).
pub(crate) fn document_key(source: AlertSource, identifier: &str) -> CacheKey {
    CacheKey::hash("alerts", &format!("{}|cap|{identifier}", source.as_str()))
}

/// The cache key of an alert response whose *request* varies with the resolved output language.
///
/// Like [`key`], one entry per source, place and UTC hour, but the request's `?lang=` parameter is
/// part of the request too: serving an English run the Traditional Chinese body (or the reverse)
/// would be a wrong answer, so the language is part of the key. Hashed like [`document_key`],
/// because the request is not a file name.
pub(crate) fn language_key(
    env: &Env<'_>,
    source: &str,
    loc: &Location,
    language: &str,
) -> CacheKey {
    let hour: chrono::DateTime<chrono::Utc> = env.cache.clock().now().into();
    let stamp = hour.format("%Y%m%dT%H");
    CacheKey::hash(
        "alerts",
        &format!("{source}|{:.2}|{:.2}|{stamp}|{language}", loc.lat, loc.lon),
    )
}

/// Resolves a document link advertised in an upstream payload against the host it came from.
///
/// A relative path is joined to `base`; an absolute URL is accepted only when it is `https` and its
/// host is `base`'s host — the host the client is already talking to. Anything else is an upstream
/// error, so an aggregator (or an on-path attacker on a cleartext hop) cannot point the fetch at an
/// arbitrary host, the same way [`document_key`] refuses a traversing identifier.
pub(crate) fn document_link(base: &str, link: &str, source: AlertSource) -> Result<String> {
    let link = link.trim();
    if link.is_empty() {
        return Err(upstream(source, "a document link is empty".to_owned()));
    }
    let base_host = authority(base);
    let Some((scheme, _)) = link.split_once("://") else {
        let path = link.trim_start_matches('/');
        return Ok(format!("{}/{}", base.trim_end_matches('/'), path));
    };
    if scheme.eq_ignore_ascii_case("https") && authority(link).eq_ignore_ascii_case(base_host) {
        Ok(link.to_owned())
    } else {
        Err(upstream(
            source,
            format!("`{link}` is not an https link to {base_host}"),
        ))
    }
}

/// The authority part of a URL (`host[:port]`), the comparison key of [`document_link`].
fn authority(url: &str) -> &str {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    rest.split(['/', '?', '#']).next().unwrap_or_default()
}

/// The configured alert cache TTL.
pub(crate) fn ttl(env: &Env<'_>) -> Duration {
    Duration::from_secs(u64::from(env.config.alerts.cache_ttl_secs))
}

/// Fetches one text response through the cache, honouring the cache mode.
///
/// Fresh entry → served; `--offline` → the last entry is replayed even when expired, with a
/// `--verbose` staleness note; otherwise the shared HTTP client fetches, the body is stored and
/// returned. A `401` becomes [`Error::InvalidKey`], so a bad credential is the user's to fix and
/// keeps its exit code when the source was named explicitly.
pub(crate) fn cached_text(
    env: &Env<'_>,
    source: AlertSource,
    request: &HttpRequest,
    key: &CacheKey,
    what: &str,
) -> Result<String> {
    if env.verbose > 1 {
        eprintln!(
            "alerts: {} {} (cache {})",
            source,
            what,
            env.cache.mode().name()
        );
    }
    if let Some(entry) = env.cache.read(key)? {
        return Ok(entry.body);
    }
    if env.cache.mode() == CacheMode::Offline {
        if let Some(entry) = env.cache.read_ignoring_ttl(key)? {
            if env.verbose > 0 {
                eprintln!(
                    "note: {} {} replayed from the cache (fetched {}, offline mode)",
                    source, what, entry.fetched_at
                );
            }
            return Ok(entry.body);
        }
        return Err(Error::Network(format!(
            "offline: no cached {source} {what} at {}; rerun without `--offline` to fetch it",
            key.path().display()
        )));
    }
    let response = env
        .http
        .send(request)
        .map_err(|error| rejected_key(error, source))?;
    let status = response.status();
    let body = response.body().to_owned();
    if let Err(error) = env.cache.write(key, status, &body, ttl(env))
        && env.verbose > 1
    {
        eprintln!(
            "alerts: {}: cache write failed ({error}); serving the fetched body",
            key.path().display()
        );
    }
    Ok(body)
}

/// The credit lines the alert terms require for the sources that produced alerts.
///
/// Only the two aggregators ask for one (WMO SWIC names the issuing agencies, FPAS names the
/// instance); the national services' display names already travel in the listing. The lines are
/// resolved here, where the catalog and the configured FPAS instance are both at hand, and the
/// renderers print them verbatim.
#[must_use]
pub fn credits(alerts: &[Alert], config: &AlertsConfig, i18n: &crate::i18n::I18n) -> Vec<String> {
    let mut lines = Vec::new();
    for source in AlertSource::ALL {
        if !alerts.iter().any(|alert| alert.source == source) {
            continue;
        }
        match source {
            AlertSource::WmoSwic => {
                lines.push(
                    i18n.text(&crate::i18n::keys::ALERT_CREDIT_WMOSWIC)
                        .into_owned(),
                );
            }
            AlertSource::Fpas => {
                lines.push(
                    i18n.format(
                        &crate::i18n::keys::ALERT_CREDIT_FPAS,
                        &[(
                            "host",
                            fluent_bundle::FluentValue::from(fpas::instance_label(
                                &config.fpas_url,
                            )),
                        )],
                    )
                    .into_owned(),
                );
            }
            _ => {}
        }
    }
    lines
}

/// A `401` means the credential is wrong and only its owner can fix it; every other error keeps
/// its taxonomy.
///
/// A source bound to a provider with a JWT mode borrows that provider's credential, so its message
/// names both `key set` forms (step 27).
fn rejected_key(error: Error, source: AlertSource) -> Error {
    match error {
        Error::Upstream {
            status: Some(401), ..
        } => {
            let provider = source.to_string();
            if crate::provider::accepts_jwt(source.provider().unwrap_or(source.as_str())) {
                Error::InvalidCredential {
                    provider,
                    status: 401,
                }
            } else {
                Error::InvalidKey {
                    provider,
                    status: 401,
                }
            }
        }
        other => other,
    }
}

/// The upstream error for an alert source's own decode failure.
pub(crate) fn upstream(source: AlertSource, message: impl Into<String>) -> Error {
    Error::Upstream {
        provider: source.as_str().to_owned(),
        status: None,
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, FixedOffset};
    use chrono_tz::Tz;

    use super::{auto_sources, covered_sources, explicit_sources, prepare};
    use crate::model::{
        Alert, AlertSource, Certainty, Location, LocationSource, Severity, Urgency,
    };
    use crate::provider::ProviderId;

    fn location(name: &str, country: Option<&str>, tz: Tz) -> Location {
        Location {
            name: name.to_owned(),
            admin1: None,
            country: country.unwrap_or_default().to_owned(),
            country_code: country.map(str::to_owned),
            lat: 39.9,
            lon: 116.4,
            tz,
            elevation_m: None,
            population: None,
            source: LocationSource::Geocoder,
            station: None,
            named_by: None,
        }
    }

    fn at(text: &str) -> DateTime<FixedOffset> {
        DateTime::parse_from_rfc3339(text).expect("a valid instant")
    }

    fn alert(id: &str, source: AlertSource, severity: Severity, event: &str) -> Alert {
        Alert {
            id: id.to_owned(),
            source,
            event: event.to_owned(),
            severity,
            urgency: Urgency::Unknown,
            certainty: Certainty::Unknown,
            onset: Some(at("2026-10-03T12:00:00Z")),
            expires: Some(at("2026-10-03T18:00:00Z")),
            ends: None,
            areas: vec!["Wien".to_owned()],
            headline: event.to_owned(),
            description: None,
            instruction: None,
            sender: None,
        }
    }

    #[test]
    fn coverage_selects_the_national_source_and_always_the_aggregators_last() {
        let beijing = location("Beijing", Some("CN"), Tz::Asia__Shanghai);
        assert_eq!(
            auto_sources(&beijing, &[]),
            [AlertSource::WmoSwic, AlertSource::Fpas]
        );
        assert_eq!(
            auto_sources(&beijing, &[ProviderId::QWeather, ProviderId::OpenMeteo]),
            [
                AlertSource::QWeather,
                AlertSource::WmoSwic,
                AlertSource::Fpas
            ]
        );
        assert_eq!(
            covered_sources(&beijing),
            [
                AlertSource::QWeather,
                AlertSource::WmoSwic,
                AlertSource::Fpas,
                AlertSource::VisualCrossing
            ]
        );

        let us = location("Norman", Some("US"), Tz::America__Chicago);
        assert_eq!(
            auto_sources(&us, &[ProviderId::OpenMeteo]),
            [AlertSource::Nws, AlertSource::WmoSwic, AlertSource::Fpas]
        );

        let hong_kong = location("Hong Kong", Some("HK"), Tz::Asia__Hong_Kong);
        assert_eq!(
            auto_sources(&hong_kong, &[ProviderId::OpenMeteo]),
            [AlertSource::Hko, AlertSource::WmoSwic, AlertSource::Fpas]
        );

        let austria = location("Vienna", Some("AT"), Tz::Europe__Vienna);
        assert_eq!(
            auto_sources(&austria, &[ProviderId::OpenMeteo]),
            [
                AlertSource::MeteoAlarm,
                AlertSource::WmoSwic,
                AlertSource::Fpas
            ]
        );

        // A coordinate-only location has no jurisdiction: the aggregators, plus the two services
        // whose compact territory a bounding box can name.
        let nowhere = location("39.90, 116.40", None, Tz::Asia__Shanghai);
        assert_eq!(
            auto_sources(&nowhere, &[ProviderId::OpenMeteo]),
            [AlertSource::WmoSwic, AlertSource::Fpas]
        );
        assert_eq!(
            covered_sources(&nowhere),
            [
                AlertSource::QWeather,
                AlertSource::WmoSwic,
                AlertSource::Fpas,
                AlertSource::VisualCrossing
            ]
        );
    }

    #[test]
    fn an_explicit_source_must_cover_the_point() {
        let beijing = location("Beijing", Some("CN"), Tz::Asia__Shanghai);
        let error = explicit_sources(&beijing, &["nws".to_owned()]).unwrap_err();
        assert_eq!(
            error.to_string(),
            "alert source `nws` does not cover 39.90,116.40; covered here: qweather, wmoswic, fpas, \
             visualcrossing"
        );
        assert_eq!(
            explicit_sources(&beijing, &["fpas".to_owned(), "fpas".to_owned()]).unwrap(),
            vec![AlertSource::Fpas]
        );
        // Provider-bound but global, and its provider is implemented: named explicitly it is
        // accepted at this point too.
        assert_eq!(
            explicit_sources(&beijing, &["visualcrossing".to_owned()]).unwrap(),
            vec![AlertSource::VisualCrossing]
        );
        assert!(explicit_sources(&beijing, &["acme".to_owned()]).is_err());
    }

    #[test]
    fn prepare_filters_expired_and_weak_and_orders_strongest_first() {
        let now = at("2026-10-03T13:00:00Z");
        let strong = alert("b", AlertSource::Nws, Severity::Extreme, "Tornado Warning");
        let moderate = alert("a", AlertSource::Fpas, Severity::Moderate, "Gale");
        let weak = alert("c", AlertSource::Fpas, Severity::Minor, "Breeze");
        let mut expired = alert("d", AlertSource::Fpas, Severity::Extreme, "Old");
        expired.expires = Some(at("2026-10-03T12:00:00Z"));
        let prepared = prepare(
            vec![weak.clone(), moderate.clone(), strong.clone(), expired],
            now,
            Severity::Minor,
        );
        assert_eq!(prepared, [strong, moderate.clone(), weak.clone()]);
        // The same list under a severe threshold keeps only the strongest.
        let filtered = prepare(
            vec![
                weak,
                moderate,
                alert("b", AlertSource::Nws, Severity::Extreme, "Tornado Warning"),
            ],
            now,
            Severity::Severe,
        );
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].severity, Severity::Extreme);
    }

    #[test]
    fn prepare_collapses_the_same_id_and_the_same_event_across_sources() {
        let now = at("2026-10-03T13:00:00Z");
        let by_id = vec![
            alert("same", AlertSource::Nws, Severity::Severe, "Heat"),
            alert("same", AlertSource::MeteoAlarm, Severity::Severe, "Heat"),
        ];
        assert_eq!(prepare(by_id, now, Severity::Unknown).len(), 1);

        let mut stronger = alert(
            "x-2",
            AlertSource::MeteoAlarm,
            Severity::Extreme,
            "Heat wave",
        );
        stronger.areas = vec!["Wien".to_owned()];
        let weaker = alert("x-1", AlertSource::Nws, Severity::Moderate, "heat wave");
        let cross = prepare(vec![weaker, stronger], now, Severity::Unknown);
        assert_eq!(cross.len(), 1);
        assert_eq!(cross[0].severity, Severity::Extreme);
    }

    #[test]
    fn the_language_is_part_of_the_alert_cache_key() {
        use std::sync::Arc;

        use crate::cache::{Cache, CacheMode, SystemClock};
        use crate::config::Config;
        use crate::config::keys::KeyStore;
        use crate::http::{HttpClient, StubTransport};
        use crate::paths::Paths;
        use crate::provider::Env;

        let directory = tempfile::tempdir().expect("tempdir");
        let paths = Paths {
            config_dir: directory.path().join("config"),
            config_file: directory.path().join("config/config.toml"),
            keys_file: directory.path().join("config/keys.toml"),
            cache_dir: directory.path().join("cache"),
            data_dir: directory.path().join("data"),
        };
        let http = HttpClient::new(
            Box::new(StubTransport::new(Vec::new())),
            0,
            Arc::new(SystemClock),
            0,
        );
        let cache = Cache::with_root(
            directory.path().join("cache"),
            CacheMode::Normal,
            Arc::new(SystemClock),
            0,
        );
        let config = Config::default();
        let keys = KeyStore::new(&paths);
        let env = Env {
            http: &http,
            cache: &cache,
            config: &config,
            keys: &keys,
            quiet: true,
            verbose: 0,
        };

        let loc = location("Hong Kong", Some("HK"), Tz::Asia__Hong_Kong);
        let chinese = super::language_key(&env, "hko-warnsum", &loc, "tc");
        let english = super::language_key(&env, "hko-warnsum", &loc, "en");
        assert_ne!(
            chinese, english,
            "the resolved language must be part of the key"
        );
        assert!(chinese.normalised().contains("tc"));
        assert!(english.normalised().contains("en"));
        assert!(chinese.normalised().contains("hko-warnsum"));
        assert!(chinese.normalised().contains("39.90"));
        // The same language, place and hour still yields the same key.
        assert_eq!(
            chinese,
            super::language_key(&env, "hko-warnsum", &loc, "tc")
        );
    }

    #[test]
    fn a_missing_end_means_live() {
        let now = at("2026-10-03T13:00:00Z");
        let mut open = alert("open", AlertSource::Fpas, Severity::Severe, "Flood");
        open.expires = None;
        open.ends = None;
        assert_eq!(prepare(vec![open], now, Severity::Unknown).len(), 1);

        // `ends` wins over `expires`: a message validity that lies behind the event's end.
        let mut ends_first = alert("ends", AlertSource::Fpas, Severity::Severe, "Flood");
        ends_first.expires = Some(at("2026-10-03T12:30:00Z"));
        ends_first.ends = Some(at("2026-10-03T14:00:00Z"));
        assert_eq!(prepare(vec![ends_first], now, Severity::Unknown).len(), 1);
    }
}
