// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! `cirrocast status`: one line for a status bar, and a contract that keeps it there.
//!
//! The probe is the query pipeline with a different failure policy and a smaller surface: it
//! resolves one location through the same sources, fetches through the same provider chain and the
//! same cache, and renders with the same [`crate::template`] engine as `--format one-line`. What
//! it adds is that
//!
//! * stdout is **exactly one line plus `\n`** (a template newline becomes a space, the line is
//!   trimmed), so a bar module never has to guess how many lines arrived;
//! * colour is **off** unless `--color always` was given, because bars strip ANSI inconsistently;
//! * a transient or data failure is **not** an exit code: the placeholder (`n/a` by default) goes
//!   to stdout, one `error: …` line to stderr, and the process still exits 0 — only a usage
//!   mistake (2) or a configuration problem (4) fails, and those are permanent, not weather;
//! * the **public-IP lookup never runs**: with no `--location`, no `CIRROCAST_LOCATION` and no
//!   `[location] default` the probe exits 4 telling the user to configure one, because a status bar
//!   has no way to consent to a lookup;
//! * it **never prompts** ([`crate::cli::Prompt::Never`]): an ambiguous name takes the ranked
//!   winner, since a bar reads no stdin;
//! * it **never adds a request to name a coordinate** (step 25): the bundled tables may name the
//!   point, and the online half of `[geo] reverse` is pinned offline, because a bar must not wait
//!   on a donated service for a display name;
//! * `--max-age` widens the window in which a cached answer is served without revalidating
//!   ([`crate::cache::Cache::with_max_age`]), and `--offline` never opens a socket and accepts a
//!   cached answer however old it is.
//!
//! A token whose value needs a second upstream request — `%A` (the alert set) and `%q` (the air
//! quality) — is populated only when the template actually shows it, so the default `%c %t` probe
//! costs exactly one fetch. Both are best-effort: a failure is a warning, never a placeholder.

use std::str::FromStr as _;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};

use crate::air::aqi::AqiIndex;
use crate::cache::{Clock, OfflineMode, SystemClock};
use crate::cli::{
    CacheFlags, Cli, GeoRequest, Prompt, QUERY_CANDIDATES, StatusArgs, StatusColor,
    configured_alert_request, configured_station, location_for_run, location_target,
    open_query_caches, print_line, request_days,
};
use crate::config::keys::KeyStore;
use crate::config::{CliOverrides, Config, Settings};
use crate::error::{Error, Result};
use crate::geo::attribution_line;
use crate::http::{HttpClient, UreqTransport};
use crate::i18n::{I18n, LanguageRequest};
use crate::model::LocalTimes;
use crate::model::units::{ResolvedUnits, UnitSystem};
use crate::model::{Location, Report};
use crate::paths::Paths;
use crate::provider::{Env, FetchRequest, HourlyResolution, fetch_chain, licence_line, select};
use crate::render::{ColorMode, RenderContext, TermCaps, resolve_color, resolve_width};
use crate::template;

/// The line the probe prints when `--format`/`--template` names none.
pub const DEFAULT_TEMPLATE: &str = "%c %t";

/// Runs `cirrocast status` and returns the process exit code.
///
/// The ordering is the same as a query's, and it is what makes the exit codes meaningful: the
/// template, the colour and the location are resolved before any traffic, so a typo is exit 2 and
/// a missing location exit 4 without a single request; everything a retry could fix is caught below
/// and rendered as the placeholder.
pub fn run(args: &StatusArgs, cli: &Cli) -> Result<u8> {
    let paths = Paths::resolve()?;
    let config = Config::load(&paths)?;
    let settings = Settings::resolve(
        &config,
        &CliOverrides {
            // The precedence ladder is the repository's: the flag, then `CIRROCAST_LOCATION` (the
            // query's positional reads the same variable, and the probe has no positional of its
            // own), then `[location] default` inside `Settings::resolve`, then nobody — which the
            // probe refuses below rather than falling back to the public-IP lookup.
            location: args.location.clone().or_else(env_location),
            offline: args.offline.then_some(OfflineMode::All),
            ..CliOverrides::default()
        },
    )?;
    let line = Line::resolve(args, &settings, &config, cli)?;
    let placeholder = args
        .placeholder
        .clone()
        .unwrap_or_else(|| config.status.placeholder.clone());

    let text = match probe(args, &settings, &config, &paths, &line, cli) {
        Ok(text) => text,
        Err(error) if degradable(&error) => {
            // The full error goes to stderr — a bar hides it, but a user running the probe by hand
            // needs to know why the placeholder appeared.
            eprintln!("error: {error}");
            one_line(&placeholder)
        }
        Err(error) => return Err(error),
    };
    print_line(text)?;
    Ok(0)
}

/// `CIRROCAST_LOCATION`, unless it names nothing.
///
/// The query's positional is bound to this variable, and the probe has no positional of its own, so
/// it reads the environment tier here: the flag still wins over it and `[location] default` still
/// fills in below it, which is the one precedence ladder the `--help` epilogue documents. An empty
/// or all-whitespace value is "absent", exactly as an empty positional is for the query.
fn env_location() -> Option<String> {
    std::env::var("CIRROCAST_LOCATION")
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

/// The one line's rendering inputs, resolved before any traffic.
///
/// The same principle as the query's render setup: a bad template or an unusable language costs no
/// request, and the units, the catalog and the palette are decided in one place.
struct Line {
    /// The resolved template ([`crate::template::resolve_template`] and `validate` have run).
    template: String,
    /// The units the report is converted into.
    units: ResolvedUnits,
    /// The language and the catalog behind every label.
    i18n: I18n,
    /// What the terminal supports.
    term: TermCaps,
    /// The colour mode (`never` unless `--color always`).
    color: ColorMode,
    /// The AQI scale behind `%q`.
    aqi_index: AqiIndex,
}

impl Line {
    /// Resolves the template, the units, the language and the palette for this probe.
    fn resolve(args: &StatusArgs, settings: &Settings, config: &Config, cli: &Cli) -> Result<Self> {
        // `--format` and `--template` are one flag under two names, so naming both is a usage
        // error rather than a silent precedence rule (clap refuses them together as well; this
        // keeps the rule beside the surface that documents it).
        let spec = match (&args.format, &args.template) {
            (Some(_), Some(_)) => {
                return Err(Error::Usage(
                    "--format and --template are synonyms on `status`; pass one".to_owned(),
                ));
            }
            (Some(spec), None) | (None, Some(spec)) => spec.as_str(),
            (None, None) => DEFAULT_TEMPLATE,
        };
        let template = template::resolve_template(Some(spec), &config.templates)?;
        template::validate(&template)?;
        let units = UnitSystem::from_str(&settings.units)?.resolve(&config.units)?;
        let i18n = I18n::load(&LanguageRequest::parse(&settings.lang), |name| {
            std::env::var(name).ok()
        });
        if !cli.quiet {
            for warning in i18n.warnings() {
                eprintln!("{warning}");
            }
        }
        let aqi_index = settings
            .aqi_index
            .parse::<AqiIndex>()
            .map_err(|error| Error::Config(format!("air.index: {error}")))?;
        let term = TermCaps::detect();
        let color = resolve_color(probe_color(args.color), &term);
        Ok(Self {
            template,
            units,
            i18n,
            term,
            color,
            aqi_index,
        })
    }

    /// Expands the template against the report and collapses the result to one line.
    ///
    /// The credits the data licences require are printed to stderr, exactly as `--format one-line`
    /// does for the same reason: a one-line document cannot carry them.
    fn render(
        &self,
        report: &crate::model::Report,
        now: DateTime<Utc>,
        credits: &[String],
        cli: &Cli,
    ) -> Result<String> {
        if !cli.quiet {
            for credit in self.credits(report, credits) {
                eprintln!("{credit}");
            }
        }
        let ctx = RenderContext {
            units: self.units,
            color: self.color,
            width: resolve_width(None).columns,
            term: self.term,
            times: LocalTimes::new(now, report.location.tz),
            lang: self.i18n.lang(),
            i18n: &self.i18n,
            alert_credits: credits,
            aqi_index: self.aqi_index,
        };
        let text = template::expand(&self.template, report, &ctx)?;
        if cli.verbose > 0 {
            for note in self.i18n.notes() {
                if matches!(note, crate::i18n::Note::MissingKey(_)) {
                    eprintln!("{}", note.text());
                }
            }
        }
        Ok(one_line(&text))
    }

    /// The credit lines this probe owes upstream, in the order the formats print them.
    fn credits(&self, report: &crate::model::Report, alert_credits: &[String]) -> Vec<String> {
        let mut credits = Vec::new();
        if let Some(credit) = attribution_line(&report.location) {
            credits.push(credit.to_owned());
        }
        if let Some(licence) = licence_line(&report.attribution.provider) {
            credits.push(format!(
                "{} {licence}",
                self.i18n.text(&crate::i18n::keys::LABEL_DATA)
            ));
        }
        credits.extend(alert_credits.iter().cloned());
        credits
    }
}

/// Resolves the location, fetches the report and renders the one line.
fn probe(
    args: &StatusArgs,
    settings: &Settings,
    config: &Config,
    paths: &Paths,
    line: &Line,
    cli: &Cli,
) -> Result<String> {
    let ids = select(&settings.provider)?;
    // A station-head chain may fall back to `[providers.metar] station`, exactly as a query does
    // when nothing else names a location.
    let station = settings
        .location
        .is_none()
        .then(|| configured_station(config, &ids))
        .flatten();
    if station.is_none() && settings.location.is_none() {
        // Privacy: the probe never asks for the public IP, and a `LocationSpec::Default` would do
        // exactly that. The missing location is refused before resolution starts.
        return Err(Error::Config(
            "no location for `status`: pass --location, set CIRROCAST_LOCATION, or set \
             [location] default in config.toml; the status probe never performs the public-IP lookup"
                .to_owned(),
        ));
    }

    let clock: Arc<dyn Clock> = Arc::new(SystemClock);
    let offline = settings.offline;
    config.check_offline(offline)?;
    let (geo_cache, weather_cache) = open_query_caches(
        paths,
        config,
        CacheFlags::default(),
        offline,
        &clock,
        cli.verbose,
    )?;
    // `--offline` promises a reading rather than a socket: any entry on disk counts. Otherwise the
    // window is the wider of the entry's TTL and `--max-age` (defaulting to the weather TTL, which
    // is what the flag documents as "no widening").
    let max_age = if args.offline {
        Duration::MAX
    } else {
        Duration::from_secs(
            args.max_age
                .unwrap_or(u64::from(config.cache.weather_ttl_secs)),
        )
    };
    let weather_cache = weather_cache.with_max_age(max_age);

    let transport = UreqTransport::new(
        &config.network,
        Duration::from_secs(u64::from(settings.timeout_secs)),
    )?;
    let http = HttpClient::new(
        Box::new(transport),
        config.network.retries,
        Arc::clone(&clock),
        cli.verbose,
    );
    let keys = KeyStore::new(paths);
    let geo = GeoRequest {
        config,
        paths,
        http: &http,
        cache: &geo_cache,
        offline,
        prompt: Prompt::Never,
        // The probe names a coordinate from the bundled tables at most: adding a reverse request
        // would make a status bar wait on a donated service for a display name.
        online_naming: false,
        limit: QUERY_CANDIDATES,
    };
    let env = Env {
        http: &http,
        cache: &weather_cache,
        config,
        keys: &keys,
        quiet: cli.quiet,
        verbose: cli.verbose,
    };

    let now: DateTime<Utc> = weather_cache.clock().now().into();
    let location = if let Some(icao) = station {
        crate::provider::metar::placeholder_location(&icao)
    } else {
        let target = location_target(settings.location.as_deref().unwrap_or_default(), config)?;
        location_for_run(None, &target, &geo, cli)?
    };

    // `auto` ranks by coverage, and the probe resolves its location only after the station
    // fallback (which needs a chain), so the expansion happens here, once the place is known.
    let ids = if settings.provider.trim().eq_ignore_ascii_case("auto") {
        crate::provider::select_for(&settings.provider, Some(&location))?
    } else {
        ids
    };
    let (days, warning) = request_days(settings.days, &ids, false, false)?;
    if let Some(warning) = warning
        && !cli.quiet
    {
        eprintln!("{warning}");
    }
    let request = FetchRequest::new(days, HourlyResolution::Hourly);
    let mut report = fetch_chain(&ids, &location, &request, &env)?;

    // The two tokens backed by a second upstream request are fetched only when the template shows
    // them, and neither may turn a readable line into a placeholder.
    if template::uses(&line.template, 'A')
        && let Some(request) = configured_alert_request(&location, &ids, config)?
    {
        attach_alerts(&mut report, &location, &request, &env, line, cli);
    }
    if template::uses(&line.template, 'q') {
        match crate::air::fetch(&location, &env) {
            Ok(air) => report.air = Some(air),
            Err(error) => panel_note("air quality", &error, cli),
        }
    }
    // Astro is computed locally (no request), so it is always attached: `%m`, `%M`, `%S` and `%s`
    // then work in any template, exactly as they do in a query that passed `--moon`.
    report.astro = Some(crate::astro::Astro::compute(
        &report,
        now.with_timezone(&report.location.tz).fixed_offset(),
    ));

    let credits = crate::alerts::credits(&report.alerts, &config.alerts, &line.i18n);
    line.render(&report, now, &credits, cli)
}

/// The colour mode a probe asks for: `never` unless `--color always` was given.
///
/// `Auto` is never returned — the probe does not detect, so a bar's environment (`NO_COLOR`,
/// `CLICOLOR_FORCE`, a `TERM` that claims support) cannot change the bytes of the line. Note that
/// the `%`-token vocabulary renders no ANSI escapes today, so the mode is currently inert in the
/// output; it is resolved here anyway because the policy is part of the contract and a future
/// token must not silently inherit the environment's preference.
const fn probe_color(requested: StatusColor) -> ColorMode {
    match requested {
        StatusColor::Never => ColorMode::Never,
        StatusColor::Always => ColorMode::Always,
    }
}

/// Collapses a rendered template to the single line the probe prints.
///
/// Every line terminator becomes one space — `\r\n` is a single break, a lone `\n` or `\r` the
/// same — and the result is trimmed. A template that came from a CRLF file therefore cannot
/// smuggle a carriage return into a bar.
fn one_line(text: &str) -> String {
    text.replace("\r\n", "\n")
        .replace(['\r', '\n'], " ")
        .trim()
        .to_owned()
}

/// Whether a failure is one the probe renders as a placeholder rather than as its own exit code.
///
/// The classification is the contract: everything a retry, a different source or a corrected
/// credential could fix degrades to one `error: …` line on stderr plus the placeholder on stdout,
/// and the process still exits 0. A usage mistake and a broken configuration must not be turned
/// into a silent `n/a` — the user asked for something impossible — and an unexpected failure
/// ([`Error::Other`]) keeps its own code. The match is exhaustive on purpose: a new variant cannot
/// join the contract without a decision here.
fn degradable(error: &Error) -> bool {
    match error {
        Error::Network(_)
        | Error::Upstream { .. }
        | Error::LocationNotFound(_)
        | Error::MissingKey { .. }
        | Error::MissingCredential { .. }
        | Error::InvalidKey { .. }
        | Error::InvalidCredential { .. }
        | Error::InvalidToken { .. }
        | Error::Chain { .. } => true,
        Error::Usage(_) | Error::Config(_) | Error::Other(_) => false,
    }
}

/// Fetches the alert set the `%A` token needs and attaches it to the report.
///
/// The alerts the answering backend carried in its own payload (`visualcrossing`) enter the same
/// layer as the fetched ones, so the probe filters and orders them exactly like a query run; the
/// panel is best-effort, and a failure is a note rather than a placeholder line.
fn attach_alerts(
    report: &mut Report,
    location: &Location,
    request: &crate::alerts::AlertsRequest,
    env: &Env<'_>,
    line: &Line,
    cli: &Cli,
) {
    match crate::alerts::fetch(
        location,
        env,
        request,
        line.i18n.lang().tag(),
        std::mem::take(&mut report.alerts),
    ) {
        Ok(alerts) => report.alerts = alerts,
        Err(error) => panel_note("alerts", &error, cli),
    }
}

/// A best-effort panel that could not be fetched: a warning, never a failure of the probe.
fn panel_note(what: &str, error: &Error, cli: &Cli) {
    if !cli.quiet {
        eprintln!("warning: {what} unavailable: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::{degradable, one_line, probe_color};
    use crate::cli::StatusColor;
    use crate::error::Error;
    use crate::render::{ColorMode, TermCaps, resolve_color};

    #[test]
    fn one_line_collapses_every_terminator() {
        assert_eq!(one_line("a\nb"), "a b");
        assert_eq!(one_line("a\r\nb"), "a b");
        assert_eq!(one_line("a\rb"), "a b");
        assert_eq!(one_line("a\r\n\r\nb"), "a  b", "one space per line break");
        assert_eq!(one_line("  +18°C  "), "+18°C");
        assert_eq!(one_line("\n\n"), "");
        assert_eq!(one_line("+18°C"), "+18°C");
    }

    #[test]
    fn colour_starts_from_never_whatever_the_terminal_claims() {
        // A terminal that advertises full colour: `never` must win anyway, because the probe's
        // colour is an explicit choice and not a detection — a bar's `TERM` says nothing about
        // whether the bytes reach a terminal.
        let term = TermCaps::read(
            |name| matches!(name, "TERM" | "COLORTERM").then(|| "xterm-256color".to_owned()),
            true,
        );
        assert_eq!(
            resolve_color(probe_color(StatusColor::Never), &term),
            ColorMode::Never
        );
        assert_eq!(
            resolve_color(probe_color(StatusColor::Always), &term),
            ColorMode::Always
        );
    }

    #[test]
    fn the_failure_classification_follows_the_contract() {
        assert!(degradable(&Error::Network("no route".to_owned())));
        assert!(degradable(&Error::Upstream {
            provider: "open-meteo".to_owned(),
            status: Some(503),
            message: "unusable body".to_owned(),
        }));
        assert!(degradable(&Error::LocationNotFound("nowhere".to_owned())));
        assert!(degradable(&Error::MissingKey {
            provider: "qweather".to_owned(),
            env: "CIRROCAST_QWEATHER_KEY".to_owned(),
        }));
        assert!(degradable(&Error::Chain {
            subject: "providers",
            attempts: vec!["open-meteo (network: timeout)".to_owned()],
        }));
        assert!(!degradable(&Error::Usage("unknown token".to_owned())));
        assert!(!degradable(&Error::Config("no location".to_owned())));
        assert!(!degradable(&Error::Other("bug".to_owned())));
    }
}
