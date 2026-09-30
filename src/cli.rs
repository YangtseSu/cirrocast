// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The command line surface.
//!
//! Scope: global flags, the `config`/`key`/`provider` subcommands. Weather query flags
//! (`--provider`, `--format`, `--days`, ...) and the location argument are added by the steps that
//! implement them, so that a flag never exists before the behaviour behind it.

use std::io::{IsTerminal as _, Read as _};
use std::process::Command as StdCommand;
use std::sync::Arc;
use std::time::Duration;

use clap::{ArgAction, Args, Parser, Subcommand};

use crate::cache::{Cache, CacheMode, CacheStat, Clock, SystemClock};
use crate::config::Config;
use crate::config::keys::{KeySource, KeyStore};
use crate::error::{Error, Result};
use crate::geo::ip::{IpLocatorChain, IpService};
use crate::geo::nominatim::{DEFAULT_URL, Nominatim};
use crate::geo::open_meteo::OpenMeteoGeocoder;
use crate::geo::{
    Geocoder, LocationSpec, Resolution, ambiguity_note, attribution_line, location_line,
    osm_ambiguity_note, rank, resolve,
};
use crate::http::{HttpClient, UreqTransport};
use crate::model::Location;
use crate::paths::Paths;
use crate::provider::{ProviderId, ProviderMeta};

/// Top level command line.
#[derive(Debug, Parser)]
#[command(
    name = "cirrocast",
    version,
    about,
    long_about = None,
    propagate_version = true,
    arg_required_else_help = true
)]
pub struct Cli {
    /// Print more detail; repeat for the full error cause chain.
    #[arg(global = true, short, long, action = ArgAction::Count)]
    pub verbose: u8,

    /// Suppress non-essential output.
    #[arg(global = true, short, long)]
    pub quiet: bool,

    /// What to do.
    #[command(subcommand)]
    pub command: Command,
}

/// The subcommands `cirrocast` understands today.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Inspect and edit the configuration file.
    Config(ConfigArgs),

    /// Manage API keys for the key-requiring providers.
    Key(KeyArgs),

    /// Inspect the supported weather providers.
    Provider(ProviderArgs),

    /// Resolve a location argument without fetching weather.
    Location(LocationArgs),

    /// Inspect and maintain the on-disk cache.
    Cache(CacheArgs),
}

/// Arguments of `cirrocast location`.
#[derive(Debug, Args)]
pub struct LocationArgs {
    /// Location action.
    #[command(subcommand)]
    pub command: LocationCommand,
}

/// Actions of `cirrocast location`.
#[derive(Debug, Subcommand)]
pub enum LocationCommand {
    /// Resolve `QUERY` and print the place it means.
    #[command(after_help = crate::geo::USAGE_FORMS)]
    Search(SearchArgs),
}

/// Arguments of `cirrocast location search`.
#[derive(Debug, Args)]
pub struct SearchArgs {
    /// Location argument (`Beijing`, `:Beijing`, `~Tsinghua`, `@39.9,116.4`); omitted = the
    /// configured default location, else the public IP.
    pub query: Option<String>,

    /// Locate from the public IP address. This sends the address to ipwho.is (falling back to
    /// ipapi.co); the answer is cached for 24 hours. It never happens without `--ip` or an empty
    /// location everywhere.
    #[arg(long)]
    pub ip: bool,

    /// How many geocoder candidates to rank (1..=100).
    #[arg(long, value_name = "N", default_value_t = 10, value_parser = clap::value_parser!(u8).range(1..=100))]
    pub limit: u8,

    /// Per-request timeout in seconds; overrides `network.timeout_secs`.
    #[arg(long, value_name = "SECS", value_parser = clap::value_parser!(u32).range(1..=300))]
    pub timeout: Option<u32>,

    #[command(flatten)]
    pub cache: CacheFlags,
}

/// The mutually exclusive cache-control flags.
#[derive(Debug, Clone, Copy, Default, Args)]
pub struct CacheFlags {
    /// Ignore the cache for this run and store nothing.
    #[arg(long, conflicts_with_all = ["refresh", "offline"])]
    pub no_cache: bool,

    /// Ignore cached answers and replace them with fresh ones.
    #[arg(long, conflicts_with_all = ["no_cache", "offline"])]
    pub refresh: bool,

    /// Serve cached answers only and never touch the network.
    #[arg(long, conflicts_with_all = ["no_cache", "refresh"])]
    pub offline: bool,
}

/// Arguments of `cirrocast cache`.
#[derive(Debug, Args)]
pub struct CacheArgs {
    /// Cache action.
    #[command(subcommand)]
    pub command: CacheCommand,
}

/// Actions of `cirrocast cache`.
#[derive(Debug, Subcommand)]
pub enum CacheCommand {
    /// Show what each namespace holds.
    Stat,

    /// Remove expired entries, or every entry with `--all`.
    Clean {
        /// Remove everything, not only what has expired.
        #[arg(long)]
        all: bool,

        /// Refused on purpose: offline mode does not write (or delete) anything.
        #[arg(long)]
        offline: bool,
    },
}

/// Arguments of `cirrocast config`.
#[derive(Debug, Args)]
pub struct ConfigArgs {
    /// Configuration action.
    #[command(subcommand)]
    pub command: ConfigCommand,
}

/// Actions of `cirrocast config`.
#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    /// Print the path of the user configuration file.
    Path,

    /// Write a commented default configuration file.
    Init {
        /// Overwrite an existing configuration file.
        #[arg(long)]
        force: bool,
    },

    /// Print the effective configuration as TOML.
    Show,

    /// Print one value; `CIRROCAST_*` overrides win over the file.
    Get {
        /// Dotted key, e.g. `defaults.days`.
        key: String,
    },

    /// Change one value in the configuration file.
    ///
    /// The whole document is rewritten in canonical form, so comments added by hand are lost;
    /// `cirrocast config init --force` writes the annotated default document back.
    Set {
        /// Dotted key, e.g. `defaults.days`.
        key: String,
        /// New value.
        value: String,
    },

    /// Open the configuration file in `$VISUAL`/`$EDITOR` and validate it afterwards.
    Edit,

    /// Parse and check the configuration file.
    Validate,
}

/// Arguments of `cirrocast key`.
#[derive(Debug, Args)]
pub struct KeyArgs {
    /// Key action.
    #[command(subcommand)]
    pub command: KeyCommand,
}

/// Actions of `cirrocast key`.
#[derive(Debug, Subcommand)]
pub enum KeyCommand {
    /// Store an API key for a provider.
    ///
    /// The secret is read from stdin — never from the command line, where `ps` and the shell
    /// history would see it.
    Set(KeySetArgs),

    /// Remove a stored API key.
    Rm {
        /// Provider id, e.g. `openweathermap`.
        provider: String,
    },

    /// List the configured keys, masked.
    List,
}

/// Arguments of `cirrocast key set`.
#[derive(Debug, Args)]
pub struct KeySetArgs {
    /// Provider id, e.g. `openweathermap`.
    pub provider: String,

    /// Read the key from stdin even when stdin is a terminal.
    #[arg(long)]
    pub stdin: bool,
}

/// Arguments of `cirrocast provider`.
#[derive(Debug, Args)]
pub struct ProviderArgs {
    /// Provider action.
    #[command(subcommand)]
    pub command: ProviderCommand,
}

/// Actions of `cirrocast provider`.
#[derive(Debug, Subcommand)]
pub enum ProviderCommand {
    /// List every known provider.
    List,

    /// Show everything the registry knows about one provider.
    Info(ProviderInfoArgs),
}

/// Arguments of `cirrocast provider info`.
#[derive(Debug, Args)]
pub struct ProviderInfoArgs {
    /// Provider id, e.g. `open-meteo`.
    pub id: String,
}

impl Cli {
    /// Runs the selected subcommand and writes its output to stdout.
    pub fn run(&self) -> Result<()> {
        match &self.command {
            Command::Config(args) => run_config(&args.command),
            Command::Key(args) => run_key(&args.command),
            Command::Provider(args) => run_provider(&args.command),
            Command::Location(args) => run_location(&args.command, self),
            Command::Cache(args) => run_cache(&args.command, self),
        }
    }
}

/// Runs `cirrocast location …`.
fn run_location(command: &LocationCommand, cli: &Cli) -> Result<()> {
    match command {
        LocationCommand::Search(args) => run_location_search(args, cli),
    }
}

/// Resolves one location argument and prints the place it means.
///
/// The resolved location goes to stdout on its own line; everything that is commentary — the
/// ambiguity note, the `ODbL` attribution, the IP-lookup disclosure and the `-v` candidate list —
/// goes to stderr, so stdout stays pipeable and a script never has to filter prose.
fn run_location_search(args: &SearchArgs, cli: &Cli) -> Result<()> {
    let paths = Paths::resolve()?;
    let config = Config::load(&paths)?;
    config.validate()?;

    let clock: Arc<dyn Clock> = Arc::new(SystemClock);
    let cache = Cache::open(
        &paths,
        cache_mode(&config, args.cache)?,
        Arc::clone(&clock),
        cli.verbose,
    );
    let timeout = Duration::from_secs(u64::from(
        args.timeout.unwrap_or(config.network.timeout_secs),
    ));
    let transport = UreqTransport::new(&config.network, timeout)?;
    let http = HttpClient::new(
        Box::new(transport),
        config.network.retries,
        clock,
        cli.verbose,
    );
    let (spec, text) = location_target(args, &config)?;
    let (location, resolution, candidates) =
        resolve_location(&spec, args, &config, &http, &cache, cli)?;

    println!("{}", location_line(&location));
    if let Some(text) = &text
        && !cli.quiet
    {
        let note = if matches!(spec, LocationSpec::Osm(_)) {
            osm_ambiguity_note(text, &location, resolution)
        } else {
            ambiguity_note(text, &location, resolution)
        };
        if let Some(note) = note {
            eprintln!("{note}");
        }
    }
    if let Some(attribution) = attribution_line(&location) {
        eprintln!("{attribution}");
    }
    if cli.verbose > 0 {
        for (index, candidate) in candidates.iter().enumerate() {
            let population = candidate
                .population
                .map(|population| format!(" (population {population})"))
                .unwrap_or_default();
            eprintln!(
                "location: candidate {}/{}: {}{population}",
                index + 1,
                candidates.len(),
                location_line(candidate)
            );
        }
    }
    Ok(())
}

/// The spec to resolve and the text a note quotes: `--ip` wins, then an explicit argument, then
/// `location.default` — which is itself a spec, so `:Beijing` or `@39.9,116.4` configured there
/// behaves exactly as it does on the command line.
fn location_target(args: &SearchArgs, config: &Config) -> Result<(LocationSpec, Option<String>)> {
    let requested = LocationSpec::parse_arg(args.query.as_deref())?;
    if args.ip && requested != LocationSpec::Default {
        return Err(Error::Usage(
            "`--ip` cannot be combined with a location argument; it locates from the public IP"
                .to_owned(),
        ));
    }
    if args.ip {
        return Ok((LocationSpec::Default, None));
    }
    if requested != LocationSpec::Default {
        let text = requested.query().unwrap_or_default().to_owned();
        return Ok((requested, Some(text)));
    }
    match configured_location(config) {
        Some(text) => {
            let spec = LocationSpec::parse_arg(Some(&text))?;
            Ok((spec, Some(text)))
        }
        None => Ok((LocationSpec::Default, None)),
    }
}

/// Resolves `spec` through the geocoder it names, returning the winner, how it was chosen and the
/// ranked candidates the `-v` listing prints.
fn resolve_location(
    spec: &LocationSpec,
    args: &SearchArgs,
    config: &Config,
    http: &HttpClient,
    cache: &Cache,
    cli: &Cli,
) -> Result<(Location, Resolution, Vec<Location>)> {
    match spec {
        LocationSpec::Default => {
            let chain = IpLocatorChain::new(
                http,
                cache,
                IpService::chain(&ip_service_setting())?,
                ip_ttl(config, cli.verbose),
            );
            let (location, service) = chain.locate_with_service()?;
            if !cli.quiet {
                eprintln!("ip: located from the public IP via {}", service.label());
            }
            Ok((location, Resolution::Only, Vec::new()))
        }
        spec @ (LocationSpec::Fuzzy(_) | LocationSpec::Exact(_)) => {
            let geocoder = OpenMeteoGeocoder::new(
                http,
                cache,
                Duration::from_secs(u64::from(config.cache.geocode_ttl_secs)),
            );
            let hits = geocoder.search(spec.query().unwrap_or_default(), args.limit)?;
            let candidates = rank(hits.clone(), spec.query(), args.limit);
            let (location, resolution) = resolve(hits, spec, args.limit)?;
            Ok((location, resolution, candidates))
        }
        spec @ LocationSpec::Osm(_) => {
            let nominatim = Nominatim::new(http, cache, nominatim_url(config));
            let hits = nominatim.search(spec.query().unwrap_or_default(), args.limit)?;
            let candidates = rank(hits.clone(), spec.query(), args.limit);
            let (location, resolution) = resolve(hits, spec, args.limit)?;
            Ok((location, resolution, candidates))
        }
        spec @ LocationSpec::LatLon(..) => {
            let (location, resolution) = resolve(Vec::new(), spec, args.limit)?;
            Ok((location, resolution, Vec::new()))
        }
    }
}

/// Runs `cirrocast cache …`.
fn run_cache(command: &CacheCommand, cli: &Cli) -> Result<()> {
    let paths = Paths::resolve()?;
    match command {
        CacheCommand::Stat => {
            let cache = Cache::open(
                &paths,
                CacheMode::Normal,
                Arc::new(SystemClock),
                cli.verbose,
            );
            for line in cache_stat_lines(&cache.stat()?) {
                println!("{line}");
            }
        }
        CacheCommand::Clean { all, offline } => {
            let mode = if *offline {
                CacheMode::Offline
            } else {
                CacheMode::Normal
            };
            let cache = Cache::open(&paths, mode, Arc::new(SystemClock), cli.verbose);
            let report = cache.clean(*all)?;
            let noun = if report.removed == 1 {
                "entry"
            } else {
                "entries"
            };
            if *all {
                println!("removed {} {noun}", report.removed);
            } else {
                println!("removed {} expired {noun}", report.removed);
            }
        }
    }
    Ok(())
}

/// One `cache stat` line per namespace: name, entry count, size, then the fetch window.
///
/// Fixed column widths (13/9/10) rather than computed ones, because the layout is documented in the
/// step file and in `--help` output: the three namespaces always fit and a user comparing two runs
/// sees the same columns.
fn cache_stat_lines(stat: &CacheStat) -> Vec<String> {
    stat.namespaces
        .iter()
        .map(|namespace| {
            let entries = if namespace.entries == 1 {
                "1 entry".to_owned()
            } else {
                format!("{} entries", namespace.entries)
            };
            let window = match (namespace.oldest, namespace.newest) {
                (Some(oldest), Some(newest)) => {
                    format!("   oldest {}   newest {}", rfc3339(oldest), rfc3339(newest))
                }
                _ => String::new(),
            };
            format!(
                "{:<13}{entries:<9}{:>10}{window}",
                namespace.name,
                human_bytes(namespace.bytes)
            )
        })
        .collect()
}

/// A timestamp in the envelope's own spelling (`2026-09-30T00:46:07Z`), which is what the cache
/// files and the documentation use, rather than `chrono`'s space-separated `Display`.
fn rfc3339(time: chrono::DateTime<chrono::Utc>) -> String {
    time.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// A byte count in the largest unit that keeps it short: `0 B`, `1.6 kB`, `2.3 MB`.
///
/// Integer arithmetic with one decimal digit, rounding half up, so the function needs neither a
/// float cast nor a precision assumption about how large a cache can grow.
fn human_bytes(bytes: u64) -> String {
    const UNITS: [(&str, u64); 4] = [
        ("GB", 1_000_000_000),
        ("MB", 1_000_000),
        ("kB", 1_000),
        ("B", 1),
    ];
    let (unit, scale) = UNITS
        .iter()
        .find(|(_, scale)| bytes >= *scale)
        .copied()
        .unwrap_or(("B", 1));
    if scale == 1 {
        return format!("{bytes} B");
    }
    let tenths = (u128::from(bytes) * 10 + u128::from(scale) / 2) / u128::from(scale);
    format!("{}.{} {unit}", tenths / 10, tenths % 10)
}

/// The IP location cache lifetime: `cache.ip_ttl_secs`, capped at 24 hours.
///
/// `ipapi.co`'s terms (section 5) allow IP answers to be kept no longer than "the minimum time
/// necessary for immediate use, which shall not exceed 24 hours", and it is the fallback of the
/// default chain, so the cap applies to the chain as a whole — a longer configured TTL would be a
/// licence violation the moment the primary service is unreachable. Exceeding it is reported under
/// `-v` rather than silently ignored.
fn ip_ttl(config: &Config, verbose: u8) -> Duration {
    /// 24 hours, in seconds.
    const MAX_IP_TTL_SECS: u32 = 86_400;
    let configured = config.cache.ip_ttl_secs;
    if configured > MAX_IP_TTL_SECS {
        if verbose > 0 {
            eprintln!(
                "ip: cache.ip_ttl_secs {configured} exceeds the {MAX_IP_TTL_SECS} s that ipapi.co allows; using {MAX_IP_TTL_SECS}"
            );
        }
        return Duration::from_secs(u64::from(MAX_IP_TTL_SECS));
    }
    Duration::from_secs(u64::from(configured))
}

/// The cache mode of this run: the flags win, then `[cache] enabled`.
fn cache_mode(config: &Config, flags: CacheFlags) -> Result<CacheMode> {
    if !flags.no_cache && !flags.refresh && !flags.offline && !config.cache.enabled {
        return Ok(CacheMode::NoCache);
    }
    CacheMode::from_flags(flags.no_cache, flags.refresh, flags.offline)
}

/// The configured default location, when there is one.
fn configured_location(config: &Config) -> Option<String> {
    let text = config.location.default.trim();
    if text.is_empty() {
        None
    } else {
        Some(text.to_owned())
    }
}

/// The Nominatim base URL: `CIRROCAST_NOMINATIM_URL`, else `network.nominatim_url`, else the
/// public service.
fn nominatim_url(config: &Config) -> String {
    if let Some(value) = std::env::var("CIRROCAST_NOMINATIM_URL")
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
    {
        return value;
    }
    let configured = config.network.nominatim_url.trim();
    if configured.is_empty() {
        DEFAULT_URL.to_owned()
    } else {
        configured.to_owned()
    }
}

/// The IP-location service setting: `CIRROCAST_IP_SERVICE`, else `auto`.
fn ip_service_setting() -> String {
    std::env::var("CIRROCAST_IP_SERVICE")
        .ok()
        .map(|value| value.trim().to_lowercase())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "auto".to_owned())
}

/// Runs `cirrocast config …`.
fn run_config(command: &ConfigCommand) -> Result<()> {
    let paths = Paths::resolve()?;
    match command {
        ConfigCommand::Path => {
            println!("{}", paths.config_file.display());
            Ok(())
        }
        ConfigCommand::Init { force } => {
            let path = Config::write_default(&paths, *force)?;
            println!("wrote {}", path.display());
            Ok(())
        }
        ConfigCommand::Show => {
            let config = Config::load(&paths)?;
            let document = toml::to_string_pretty(&config).map_err(|error| {
                Error::Other(format!("cannot render the configuration: {error}"))
            })?;
            print!("{document}");
            Ok(())
        }
        ConfigCommand::Get { key } => {
            let config = Config::load(&paths)?;
            println!("{}", config.get_key(key)?);
            Ok(())
        }
        ConfigCommand::Set { key, value } => {
            let mut config = Config::load(&paths)?;
            config.set_key(key, value)?;
            config.save(&paths)?;
            Ok(())
        }
        ConfigCommand::Edit => edit_config(&paths),
        ConfigCommand::Validate => {
            let (config, source) = Config::load_with_source(&paths)?;
            config.validate()?;
            println!(
                "ok: {}",
                source
                    .unwrap_or_else(|| paths.config_file.clone())
                    .display()
            );
            Ok(())
        }
    }
}

/// Runs `cirrocast key …`.
fn run_key(command: &KeyCommand) -> Result<()> {
    let paths = Paths::resolve()?;
    let store = KeyStore::new(&paths);
    match command {
        KeyCommand::Set(args) => {
            let id: ProviderId = args.provider.parse()?;
            let secret = read_secret(id.as_str(), args.stdin)?;
            store.set(id.as_str(), &secret)?;
            println!("stored {id} API key in {}", store.path().display());
            Ok(())
        }
        KeyCommand::Rm { provider } => {
            let id: ProviderId = provider.parse()?;
            if store.remove(id.as_str())? {
                println!("removed {id} API key");
            } else {
                println!("no {id} API key stored");
            }
            Ok(())
        }
        KeyCommand::List => {
            let summaries = store.list()?;
            let width = column_width(
                "PROVIDER",
                summaries.iter().map(|row| row.provider.as_str()),
            );
            for row in &summaries {
                println!(
                    "{:<width$}  {}  ({})",
                    row.provider,
                    row.masked,
                    source_label(row.source)
                );
            }
            Ok(())
        }
    }
}

/// Runs `cirrocast provider …`.
fn run_provider(command: &ProviderCommand) -> Result<()> {
    match command {
        ProviderCommand::List => {
            for line in provider_table() {
                println!("{line}");
            }
            Ok(())
        }
        ProviderCommand::Info(info) => {
            let id: ProviderId = info.id.parse()?;
            for line in provider_details(&id.metadata()) {
                println!("{line}");
            }
            Ok(())
        }
    }
}

/// Runs the user's editor on the configuration file and validates what came back.
fn edit_config(paths: &Paths) -> Result<()> {
    if !paths.config_file.exists() {
        Config::write_default(paths, false)?;
    }
    let editor = ["VISUAL", "EDITOR"]
        .iter()
        .find_map(|name| {
            std::env::var(name)
                .ok()
                .filter(|value| !value.trim().is_empty())
        })
        .map_or_else(|| "vi".to_owned(), |editor| editor.trim().to_owned());

    let status = StdCommand::new(&editor)
        .arg(&paths.config_file)
        .status()
        .map_err(|error| Error::Other(format!("cannot run `{editor}`: {error}")))?;
    if !status.success() {
        return Err(Error::Other(format!("`{editor}` exited with {status}")));
    }

    let config = Config::load(paths)?;
    config.validate()?;
    println!("ok: {}", paths.config_file.display());
    Ok(())
}

/// Reads an API key from stdin — piped input, or the terminal with echo disabled. Never from argv.
fn read_secret(provider: &str, force_stdin: bool) -> Result<String> {
    if !force_stdin && std::io::stdin().is_terminal() {
        let secret =
            rpassword::prompt_password(format!("API key for {provider}: ")).map_err(|error| {
                Error::Other(format!(
                    "cannot read the API key from the terminal: {error}"
                ))
            })?;
        return checked_secret(&secret);
    }

    let mut input = String::new();
    std::io::stdin()
        .read_to_string(&mut input)
        .map_err(|error| Error::Other(format!("cannot read the API key from stdin: {error}")))?;
    checked_secret(input.lines().next().unwrap_or_default())
}

/// Rejects an empty secret, so a stray newline cannot store an unusable key.
fn checked_secret(value: &str) -> Result<String> {
    let value = value.trim();
    if value.is_empty() {
        Err(Error::Usage(
            "no API key given; pipe it in or answer the prompt".to_owned(),
        ))
    } else {
        Ok(value.to_owned())
    }
}

/// How `key list` spells where a key came from.
fn source_label(source: KeySource) -> &'static str {
    match source {
        KeySource::Env => "env",
        KeySource::File => "file",
    }
}

/// Header plus one row per provider, sized to the widest value in each column.
fn provider_table() -> Vec<String> {
    let metas: Vec<ProviderMeta> = ProviderId::all().iter().map(ProviderId::metadata).collect();

    let id_width = column_width("ID", metas.iter().map(|meta| meta.id.as_str()));
    let name_width = column_width("NAME", metas.iter().map(|meta| meta.display_name));
    let key_width = column_width("KEY", metas.iter().map(key_label));

    let row = |id: &str, name: &str, key: &str, obs: &str, fcst: &str, days: &str| {
        format!(
            "{id:<id_width$}  {name:<name_width$}  {key:<key_width$}  {obs:<3}  {fcst:<3}  {days:>7}"
        )
    };

    let mut lines = vec![row("ID", "NAME", "KEY", "OBS", "FCST", "MAXDAYS")];
    for meta in &metas {
        let max_days = meta.max_days.to_string();
        lines.push(row(
            meta.id.as_str(),
            meta.display_name,
            key_label(meta),
            yes_no(meta.current),
            yes_no(meta.daily || meta.hourly),
            &max_days,
        ));
    }
    lines
}

/// The detail block printed by `provider info`.
fn provider_details(meta: &ProviderMeta) -> Vec<String> {
    let locations = meta.location_kinds.summary();
    vec![
        info_line("id:", meta.id),
        info_line("name:", meta.display_name),
        info_line("key:", key_label(meta)),
        info_line("current:", yes_no(meta.current)),
        info_line("hourly:", yes_no(meta.hourly)),
        info_line("daily:", yes_no(meta.daily)),
        info_line("max days:", meta.max_days),
        info_line("locations:", locations),
        info_line("docs:", meta.docs_url),
        info_line("notes:", meta.notes),
    ]
}

/// How the `KEY` column and the `key:` line spell the credential requirement.
fn key_label(meta: &ProviderMeta) -> &'static str {
    meta.key_env.unwrap_or("none")
}

/// `yes`/`no`, for the boolean capability columns.
fn yes_no(flag: bool) -> &'static str {
    if flag { "yes" } else { "no" }
}

/// One `label: value` line, padded so the values line up.
fn info_line(label: &str, value: impl std::fmt::Display) -> String {
    format!("{label:<12}{value}")
}

/// The width of a column: the wider of its header and its longest value.
fn column_width<'a>(header: &str, values: impl Iterator<Item = &'a str>) -> usize {
    values.fold(header.len(), |width, value| width.max(value.len()))
}
