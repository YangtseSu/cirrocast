// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The command line surface.
//!
//! Scope: global flags, the `config`/`key`/`provider` subcommands. Weather query flags
//! (`--provider`, `--format`, `--days`, ...) and the location argument are added by the steps that
//! implement them, so that a flag never exists before the behaviour behind it.

use std::io::{IsTerminal as _, Read as _};
use std::process::Command as StdCommand;
use std::str::FromStr as _;
use std::sync::Arc;
use std::time::Duration;

use clap::{ArgAction, Args, Parser, Subcommand, ValueEnum as _};

use crate::cache::{Cache, CacheMode, CacheStat, Clock, SystemClock};
use crate::config::keys::{KeySource, KeyStore};
use crate::config::{Config, Settings};
use crate::error::{Error, Result};
use crate::geo::ip::{IpLocatorChain, IpService};
use crate::geo::nominatim::{DEFAULT_URL, Nominatim};
use crate::geo::open_meteo::OpenMeteoGeocoder;
use crate::geo::{
    Geocoder, LocationSpec, Resolution, ambiguity_note, attribution_line, location_line,
    osm_ambiguity_note, rank, resolve,
};
use crate::http::{HttpClient, UreqTransport};
use crate::i18n::{I18n, LanguageRequest};
use crate::model::Location;
use crate::model::units::{ResolvedUnits, UnitSystem};
use crate::paths::Paths;
use crate::provider::{Env, fetch_chain, licence_line, select};
use crate::provider::{FetchRequest, HourlyResolution, ProviderId, ProviderMeta};
use crate::render::{
    Charset, ColorMode, Format, RenderContext, TermCaps, effective_depth, renderer_for,
    resolve_color, resolve_width,
};

/// Writes a line to stdout, treating a closed pipe as success.
///
/// `cirrocast ... | head` is a normal thing to do and the reader going away is not a failure the
/// user asked to hear about; `println!` would panic on the write error, this returns quietly.
fn print_line(line: impl std::fmt::Display) -> Result<()> {
    write_stdout(line, true)
}

/// Writes text to stdout without a trailing newline.
fn print_text(text: impl std::fmt::Display) -> Result<()> {
    write_stdout(text, false)
}

/// The shared stdout write: one lock, and a broken pipe swallowed.
fn write_stdout(value: impl std::fmt::Display, newline: bool) -> Result<()> {
    use std::io::Write as _;

    let stdout = std::io::stdout();
    let mut handle = stdout.lock();
    let written = if newline {
        writeln!(handle, "{value}")
    } else {
        write!(handle, "{value}")
    };
    match written {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
        Err(error) => Err(Error::Other(format!("cannot write to stdout: {error}"))),
    }
}

/// A stdout sink for generated documents (completion scripts, man pages), for the same reason as
/// [`print_line`]: `clap_complete` and `clap_mangen` write into a [`std::io::Write`] and would
/// report the broken pipe of `cirrocast man | head` as a failure.
struct StdoutSink;

impl std::io::Write for StdoutSink {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        match std::io::stdout().write(buffer) {
            Err(error) if error.kind() == std::io::ErrorKind::BrokenPipe => Ok(buffer.len()),
            other => other,
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        std::io::stdout().flush()
    }
}

/// Top level command line.
#[derive(Debug, Parser)]
#[command(
    name = "cirrocast",
    version,
    about,
    long_about = None,
    after_long_help = HELP_EPILOG,
    propagate_version = true
)]
pub struct Cli {
    /// Print more detail; repeat for the full error cause chain.
    #[arg(global = true, short, long, action = ArgAction::Count)]
    pub verbose: u8,

    /// Suppress non-essential output.
    #[arg(global = true, short, long, conflicts_with = "verbose")]
    pub quiet: bool,

    /// What to do; omitted = fetch the weather for the location argument.
    #[command(subcommand)]
    pub command: Option<Command>,

    #[command(flatten)]
    pub query: QueryArgs,
}

/// The tables `--help` ends with.
///
/// They are part of the CLI's contract, not decoration: the precedence ladder, the exit codes and
/// the token vocabulary are what a script or a user reads before writing anything against the
/// tool. The token and preset rows are duplicated from [`crate::render::one_line`] because clap
/// takes a `&'static str` here; a unit test compares the two so they cannot drift.
const HELP_EPILOG: &str = "\
PRECEDENCE (highest first)
  command line flag > CIRROCAST_* environment variable > config.toml > built-in default
  --provider  CIRROCAST_PROVIDER   --days    CIRROCAST_DAYS     --timeout CIRROCAST_TIMEOUT
  --format    CIRROCAST_FORMAT     --units   CIRROCAST_UNITS    --location CIRROCAST_LOCATION
  --lang      CIRROCAST_LANG
  The configuration file is consulted only when neither the flag nor the variable is set, so an
  environment value is never overridden by config.toml. `config get <key>` prints the variable's
  value when one is set.

EXIT CODES
  0  success                 4  configuration or state on disk
  1  generic failure         5  location not found
  2  usage                   6  missing or invalid API key
  3  network or upstream failure

ONE-LINE TOKENS (--format one-line)
  %c condition art    %C condition text   %t temp        %f feels-like
  %w wind             %h humidity         %p precip      %P pressure
  %v visibility       %u UV index         %U UV + band   %m moon (n/a)
  %d ISO date         %D Wed 30 Sep       %Z zone name   %z +0800
  %S sunrise          %s sunset           %l name        %L 39.90,116.40
  %% is a literal %, %{...} is verbatim, \\n \\t \\\\ are escapes; an unknown %X stays literal.
  Presets (@NAME), listed with their templates:
    @default  %l: %c %C %t (%f), %w, %h, %p, %P, %v
    @short    %c %t
    @full     %l: %c %C %t (%f) %w %h %p %P %m %v %u %S %s %Z
    @uv       %l: UV %U
    @sun      %l: sunrise %S sunset %s (%z %Z)";

/// The weather query: the whole flag matrix of `cirrocast <LOCATION>`.
///
/// The flags are grouped the way `--help` shows them: what to fetch, where from, what units and
/// language the answer is rendered in, how wide and how colourful the layout is, and how the cache
/// and the transport behave. Everything that also exists as a configuration key carries the
/// `CIRROCAST_*` override, whose precedence the epilog spells out.
#[derive(Debug, Args)]
pub struct QueryArgs {
    /// Location argument (`Beijing`, `:Beijing`, `~Tsinghua`, `@39.9,116.4`); omitted = the
    /// configured default location, else the public IP.
    #[arg(value_name = "LOCATION", env = "CIRROCAST_LOCATION")]
    pub location: Option<String>,

    /// Provider chain, comma separated; `auto` expands to the implemented keyless backends (plus
    /// `metar` with `--station`).
    #[arg(short = 'p', long, value_name = "LIST", env = "CIRROCAST_PROVIDER")]
    pub provider: Option<String>,

    /// Output format.
    #[arg(short = 'f', long, value_name = "FORMAT", env = "CIRROCAST_FORMAT")]
    pub format: Option<Format>,

    /// Forecast days, `0` = current conditions only (clamped to what the provider serves).
    #[arg(
        short = 'd',
        long,
        value_name = "N",
        env = "CIRROCAST_DAYS",
        value_parser = clap::value_parser!(u8).range(0..=14)
    )]
    pub days: Option<u8>,

    /// Unit system for the rendered values: `metric`, `us` or `uk`. The JSON format always emits
    /// canonical metric, whatever this says.
    #[arg(short = 'u', long, value_name = "SYSTEM", env = "CIRROCAST_UNITS", value_parser = parse_units)]
    pub units: Option<UnitSystem>,

    /// Output language as a BCP-47 tag, or `auto`.
    #[arg(long, value_name = "TAG", env = "CIRROCAST_LANG")]
    pub lang: Option<String>,

    /// Latitude in degrees, north positive; needs `--lon`, and replaces the location argument.
    #[arg(long, value_name = "DEG", value_parser = parse_latitude, requires = "lon", conflicts_with_all = ["ip", "station"])]
    pub lat: Option<f64>,

    /// Longitude in degrees, east positive; needs `--lat`.
    #[arg(long, value_name = "DEG", value_parser = parse_longitude, requires = "lat", conflicts_with_all = ["ip", "station"])]
    pub lon: Option<f64>,

    /// Locate from the public IP address. This sends the address to ipwho.is (falling back to
    /// ipapi.co); the answer is cached for 24 hours. It never happens without `--ip` or an empty
    /// location everywhere.
    #[arg(long, conflicts_with = "station")]
    pub ip: bool,

    /// METAR station identifier (ICAO); needs `--provider metar` or `auto`.
    #[arg(long, value_name = "ICAO")]
    pub station: Option<String>,

    /// Template for `--format one-line`: a literal `%`-token string, or `@PRESET`. The presets
    /// are `@default`, `@short`, `@full`, `@uv` and `@sun`; `--help` lists their templates and
    /// every token.
    #[arg(long, value_name = "TEMPLATE")]
    pub template: Option<String>,

    /// When to colour the output.
    #[arg(long, value_name = "WHEN", value_enum)]
    pub color: Option<ColorMode>,

    /// Layout width in columns for the table formats (1..=500; below 20 is raised to it).
    #[arg(long, value_name = "COLS", value_parser = clap::value_parser!(u16).range(1..=500))]
    pub width: Option<u16>,

    /// Per-request timeout in seconds; overrides `network.timeout_secs`.
    #[arg(
        long,
        value_name = "SECS",
        env = "CIRROCAST_TIMEOUT",
        value_parser = clap::value_parser!(u32).range(1..=300)
    )]
    pub timeout: Option<u32>,

    #[command(flatten)]
    pub cache: CacheFlags,
}

/// `--units`, parsed by the unit module: the flag, `CIRROCAST_UNITS` and `defaults.units` accept
/// the same three spellings and share one error message.
fn parse_units(value: &str) -> Result<UnitSystem, Error> {
    value.parse()
}

/// `--lat`: degrees in `-90..=90`, the same range and wording the `@lat,lon` argument uses.
fn parse_latitude(value: &str) -> Result<f64, Error> {
    let degrees = parse_degrees(value, "latitude")?;
    if (-90.0..=90.0).contains(&degrees) {
        Ok(degrees)
    } else {
        Err(Error::Usage(format!(
            "latitude {value} is out of range -90..=90"
        )))
    }
}

/// `--lon`: degrees in `-180..=180`.
fn parse_longitude(value: &str) -> Result<f64, Error> {
    let degrees = parse_degrees(value, "longitude")?;
    if (-180.0..=180.0).contains(&degrees) {
        Ok(degrees)
    } else {
        Err(Error::Usage(format!(
            "longitude {value} is out of range -180..=180"
        )))
    }
}

/// A finite number of degrees; `NaN` and infinities are rejected before the range check, which
/// they would pass.
fn parse_degrees(value: &str, name: &str) -> Result<f64, Error> {
    match value.parse::<f64>() {
        Ok(degrees) if degrees.is_finite() => Ok(degrees),
        _ => Err(Error::Usage(format!("{name} `{value}` is not a number"))),
    }
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

    /// Print a shell completion script.
    Completion(CompletionArgs),

    /// Print the manual page as roff.
    Man(ManArgs),
}

/// Arguments of `cirrocast completion`.
#[derive(Debug, Args)]
pub struct CompletionArgs {
    /// Shell to generate the completion script for.
    pub shell: clap_complete::Shell,

    /// Program name the completions are generated for.
    #[arg(long, value_name = "NAME", default_value = "cirrocast")]
    pub bin_name: String,
}

/// Arguments of `cirrocast man`.
#[derive(Debug, Args)]
pub struct ManArgs {
    /// Program name the manual page documents.
    #[arg(long, value_name = "NAME", default_value = "cirrocast")]
    pub bin_name: String,
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
    #[arg(value_name = "LOCATION", conflicts_with = "ip")]
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
    /// Runs the selected subcommand, or the weather query when none was given, and writes its
    /// output to stdout.
    ///
    /// `sources` is the provenance clap recorded at parse time ([`Sources::read`]); it is what
    /// tells a setting that came from the environment apart from one that came from the command
    /// line, which the value alone cannot say.
    pub fn run(&self, sources: &Sources) -> Result<()> {
        match &self.command {
            Some(Command::Config(args)) => run_config(&args.command),
            Some(Command::Key(args)) => run_key(&args.command),
            Some(Command::Provider(args)) => run_provider(&args.command),
            Some(Command::Location(args)) => run_location(&args.command, self),
            Some(Command::Cache(args)) => run_cache(&args.command, self),
            Some(Command::Completion(args)) => {
                run_completion(args);
                Ok(())
            }
            Some(Command::Man(args)) => run_man(args),
            None => run_query(&self.query, self, *sources),
        }
    }
}

/// Where a setting's value came from, as clap recorded it.
///
/// The tier matters because the merge is `flag > environment > config > built-in`: a value that
/// arrived from `CIRROCAST_*` must never be replaced by the configuration file, and a rule that
/// only applies to what the user typed on the command line (`--ip` next to a location argument)
/// must not fire for an environment value either.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Source {
    /// Given on the command line.
    CommandLine,
    /// Taken from the `CIRROCAST_*` variable the flag is bound to.
    Environment,
    /// Neither: the configuration file supplies it, else the built-in default.
    #[default]
    Default,
}

impl Source {
    /// The tier, for the `--verbose` report.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CommandLine => "the command line",
            Self::Environment => "the environment",
            Self::Default => "the config or the built-in default",
        }
    }
}

/// The provenance of every setting that can come from more than one tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Sources {
    /// `--provider` / `CIRROCAST_PROVIDER`.
    pub provider: Source,
    /// `--format` / `CIRROCAST_FORMAT`.
    pub format: Source,
    /// `--days` / `CIRROCAST_DAYS`.
    pub days: Source,
    /// `--units` / `CIRROCAST_UNITS`.
    pub units: Source,
    /// `--lang` / `CIRROCAST_LANG`.
    pub lang: Source,
    /// The location argument / `CIRROCAST_LOCATION`.
    pub location: Source,
    /// `--timeout` / `CIRROCAST_TIMEOUT`.
    pub timeout: Source,
}

impl Sources {
    /// Reads the provenance out of clap's own parse result.
    ///
    /// [`clap::ArgMatches::value_source`] is the only reliable answer to "was this given?": the
    /// value alone cannot say whether it came from the command line, from the environment or from
    /// a default.
    #[must_use]
    pub fn read(matches: &clap::ArgMatches) -> Self {
        let source = |id: &str| match matches.value_source(id) {
            Some(clap::parser::ValueSource::CommandLine) => Source::CommandLine,
            Some(clap::parser::ValueSource::EnvVariable) => Source::Environment,
            _ => Source::Default,
        };
        Self {
            provider: source("provider"),
            format: source("format"),
            days: source("days"),
            units: source("units"),
            lang: source("lang"),
            location: source("location"),
            timeout: source("timeout"),
        }
    }

    /// Prints the resolved settings and their tiers, one line each, under `--verbose`.
    fn note(self, settings: &Settings) {
        eprintln!(
            "provider: {} (from {})",
            settings.provider,
            self.provider.as_str()
        );
        eprintln!(
            "format: {} (from {})",
            settings.format,
            self.format.as_str()
        );
        eprintln!("days: {} (from {})", settings.days, self.days.as_str());
        eprintln!("units: {} (from {})", settings.units, self.units.as_str());
        eprintln!("language: {} (from {})", settings.lang, self.lang.as_str());
        eprintln!(
            "timeout: {}s (from {})",
            settings.timeout_secs,
            self.timeout.as_str()
        );
        if let Some(location) = &settings.location {
            eprintln!("location: {location} (from {})", self.location.as_str());
        }
    }
}

/// The rules that need the resolved settings rather than the raw flags.
///
/// Static conflicts live in clap, which then owns the message and the exit code. Two things cannot
/// be expressed there: a location argument that came from the environment must *not* conflict with
/// a flag that overrides it (the flag wins by precedence), and `--station` depends on the provider
/// chain, which the configuration file may be the source of.
fn validate_query(query: &QueryArgs, sources: Sources, settings: &Settings) -> Result<()> {
    if sources.location == Source::CommandLine {
        for (given, name) in [
            (query.lat.is_some() || query.lon.is_some(), "--lat/--lon"),
            (query.ip, "--ip"),
            (query.station.is_some(), "--station"),
        ] {
            if given {
                return Err(Error::Usage(format!(
                    "a location argument cannot be combined with {name}; drop one of the two"
                )));
            }
        }
    }
    if query.station.is_some() && !station_chain(&settings.provider) {
        return Err(Error::Usage(
            "--station requires --provider metar (or auto)".to_owned(),
        ));
    }
    Ok(())
}

/// Whether a provider chain can answer a station identifier: `auto` (which gains `metar` for the
/// run) or a chain whose first entry is `metar`.
fn station_chain(spec: &str) -> bool {
    let spec = spec.trim();
    if spec.eq_ignore_ascii_case("auto") {
        return true;
    }
    spec.split(',')
        .next()
        .unwrap_or_default()
        .trim()
        .parse::<ProviderId>()
        .is_ok_and(|id| id == ProviderId::Metar)
}

/// The chain this run fetches from.
///
/// `--station` is what makes `auto` pick up `metar`: without one, the station-only backend can
/// never answer a resolved place, and [`select`] keeps it out of the keyless chain.
fn provider_chain(settings: &Settings, station: Option<&str>) -> Result<Vec<ProviderId>> {
    if station.is_some() && settings.provider.trim().eq_ignore_ascii_case("auto") {
        return select(&format!("{},metar", settings.provider.trim()));
    }
    select(&settings.provider)
}

/// The forecast days to request, clamped to what the first provider of the chain serves, plus the
/// warning that explains the clamp.
///
/// Clamping here rather than inside the provider means the warning is printed exactly once, in the
/// CLI's own vocabulary, and the cache key is built for the horizon that is really fetched. The
/// caller silences the warning with `-q`; an observations-only backend (`max_days == 0`) clamps
/// everything to zero, which is what a station forecast is.
fn request_days(requested: u8, ids: &[ProviderId]) -> (u8, Option<String>) {
    let Some(primary) = ids.first() else {
        return (requested, None);
    };
    let max_days = primary.metadata().max_days;
    if requested > max_days {
        return (
            max_days,
            Some(format!(
                "warning: {primary} supports at most {max_days} days; --days {requested} clamped to {max_days}"
            )),
        );
    }
    (requested, None)
}

/// Prints `cirrocast completion <shell>`.
fn run_completion(args: &CompletionArgs) {
    use clap::CommandFactory as _;

    let mut command = Cli::command();
    clap_complete::generate(args.shell, &mut command, &args.bin_name, &mut StdoutSink);
}

/// Prints `cirrocast man`.
fn run_man(args: &ManArgs) -> Result<()> {
    use clap::CommandFactory as _;

    let command = Cli::command().bin_name(args.bin_name.clone());
    clap_mangen::Man::new(command)
        .title(args.bin_name.clone())
        .render(&mut StdoutSink)
        .map_err(|error| Error::Other(format!("cannot render the man page: {error}")))
}

/// Runs the weather query: resolve a location, fetch a report, render it.
///
/// The order matters for what a user sees when something fails: the provider list and the format
/// are checked before any network request, so a typo in `--provider` costs no traffic, and the
/// location is resolved before the forecast because every backend needs it.
fn run_query(query: &QueryArgs, cli: &Cli, sources: Sources) -> Result<()> {
    let paths = Paths::resolve()?;
    let config = Config::load(&paths)?;
    let settings = Settings::resolve(
        &config,
        &crate::config::CliOverrides {
            provider: query.provider.clone(),
            format: query.format.map(|format| format.as_str().to_owned()),
            units: query.units.map(|units| units.to_string()),
            days: query.days,
            lang: query.lang.clone(),
            location: location_arg(query),
            timeout_secs: query.timeout,
            no_cache: query.cache.no_cache,
            refresh: query.cache.refresh,
            offline: query.cache.offline,
        },
    )?;
    validate_query(query, sources, &settings)?;

    let ids = provider_chain(&settings, query.station.as_deref())?;
    let (days, warning) = request_days(settings.days, &ids);
    if let Some(warning) = warning
        && !cli.quiet
    {
        eprintln!("{warning}");
    }
    let setup = RenderSetup::resolve(query, &config, &settings, cli.verbose, cli.quiet)?;
    if cli.verbose > 0 {
        sources.note(&settings);
        render_notes(&setup);
    }

    let clock: Arc<dyn Clock> = Arc::new(SystemClock);
    let cache = Cache::open(
        &paths,
        cache_mode(&config, query.cache)?,
        Arc::clone(&clock),
        cli.verbose,
    );
    let transport = UreqTransport::new(
        &config.network,
        Duration::from_secs(u64::from(settings.timeout_secs)),
    )?;
    let http = HttpClient::new(
        Box::new(transport),
        config.network.retries,
        clock,
        cli.verbose,
    );
    let keys = KeyStore::new(&paths);

    let location = query_location(query, &settings, &config, &http, &cache, cli)?;

    let env = Env {
        http: &http,
        cache: &cache,
        config: &config,
        keys: &keys,
        quiet: cli.quiet,
        verbose: cli.verbose,
    };
    let request = FetchRequest::new(days, HourlyResolution::Hourly);
    let report = fetch_chain(&ids, &location, &request, &env)?;

    if cli.verbose > 0 {
        // The credit the terms require, plus the exact request that produced the answer, so a
        // bug report can name the upstream call without a packet capture.
        let credit = licence_line(&report.attribution.provider).unwrap_or("no credit line");
        eprintln!("attribution: {credit} ({})", report.attribution.url);
    }

    let now: chrono::DateTime<chrono::Utc> = cache.clock().now().into();
    let ctx = RenderContext {
        units: setup.units,
        color: setup.color,
        width: setup.width.columns,
        term: setup.term,
        now: now.with_timezone(&report.location.tz).fixed_offset(),
        tz: report.location.tz,
        lang: setup.i18n.lang(),
        i18n: &setup.i18n,
    };
    // `one-line` is one line by contract, so the credits the licences require cannot travel in the
    // output: they go to stderr, where `plain` (a document) and `json` (an envelope) keep theirs.
    if setup.format == Format::OneLine {
        if let Some(credit) = attribution_line(&report.location) {
            eprintln!("{credit}");
        }
        if let Some(licence) = licence_line(&report.attribution.provider) {
            eprintln!(
                "{} {licence}",
                setup.i18n.text(&crate::i18n::keys::LABEL_DATA)
            );
        }
    }
    print_line(format_args!("{}", setup.renderer.render(&report, &ctx)?))?;
    Ok(())
}

/// The location a weather query forecasts for, with the commentary a user needs to trust it.
///
/// The ambiguity note (silenced by `-q`) and the `-v` candidate list go to stderr; stdout carries
/// only the report, so a script piping the query never has to filter prose out of the answer.
fn query_location(
    query: &QueryArgs,
    settings: &Settings,
    config: &Config,
    http: &HttpClient,
    cache: &Cache,
    cli: &Cli,
) -> Result<Location> {
    let (spec, text) = location_target(settings.location.as_deref(), query.ip, config)?;
    let (location, resolution, candidates) =
        resolve_location(&spec, QUERY_CANDIDATES, config, http, cache, cli)?;
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
    if cli.verbose > 0 {
        for (index, candidate) in candidates.iter().enumerate() {
            eprintln!(
                "location: candidate {}/{}: {}",
                index + 1,
                candidates.len(),
                location_line(candidate)
            );
        }
    }
    Ok(location)
}

/// The location argument this run resolves.
///
/// `--lat/--lon` is folded into the `@lat,lon` spelling the resolver already understands, and it
/// outranks an environment location by the usual precedence. `--station` is deliberately absent:
/// it survives [`validate_query`] only for a `metar`-first (or `auto`) chain, and [`provider_chain`]
/// refuses `metar` until step 11 implements the backend — the station becomes the location then.
fn location_arg(query: &QueryArgs) -> Option<String> {
    match (query.lat, query.lon) {
        (Some(lat), Some(lon)) => Some(format!("@{lat},{lon}")),
        _ => query.location.clone(),
    }
}

/// How many ranked geocoder candidates a weather query asks for; the ambiguity note and the `-v`
/// listing use them, and a later step may expose the number as a flag.
const QUERY_CANDIDATES: u8 = 10;

/// The configured colour mode.
///
/// `[render] color` is validated when the configuration loads, so a value that does not parse here
/// means the configuration was built by hand.
fn color_mode(name: &str) -> Result<ColorMode> {
    ColorMode::from_str(name, true)
        .map_err(|_| Error::Config(format!("unknown colour mode `{name}`")))
}

/// Everything the renderer needs, resolved before a single request is sent: the format, the
/// units, the language and its catalog, the layout width and the palette.
///
/// Resolving this first is what makes a typo in `--format` or an unusable language cost no traffic
/// — and what keeps the query function about fetching.
struct RenderSetup {
    /// The renderer for the selected format.
    renderer: Box<dyn crate::render::Renderer>,
    /// The format it renders.
    format: Format,
    /// The units the report is converted into.
    units: ResolvedUnits,
    /// The language the report is rendered in, and the catalog behind every label.
    i18n: I18n,
    /// The layout width and where it came from.
    width: crate::render::Width,
    /// The colour mode this run uses.
    color: ColorMode,
    /// What the terminal supports.
    term: TermCaps,
}

impl RenderSetup {
    /// Resolves the settings for one run.
    ///
    /// `verbose` is what decides whether the template's unknown tokens are reported, and `quiet`
    /// whether a language fallback is announced: the renderer itself never sees either flag, so
    /// [`crate::render::one_line::warnings`] and [`I18n::warnings`] are read here.
    fn resolve(
        query: &QueryArgs,
        config: &Config,
        settings: &Settings,
        verbose: u8,
        quiet: bool,
    ) -> Result<Self> {
        let format = match query.format {
            Some(format) => format,
            None => Format::from_name(&settings.format)?,
        };
        let term = TermCaps::detect();
        // A template belongs to `one-line` alone; `renderer_for` refuses it everywhere else, and
        // the warnings for unknown tokens are reported here, where `-v` is known.
        if verbose > 0 && format == Format::OneLine {
            let template = crate::render::one_line::resolve_template(query.template.as_deref())?;
            for warning in crate::render::one_line::warnings(&template) {
                eprintln!("{warning}");
            }
        }
        let renderer = renderer_for(format, &term, query.template.as_deref())?;
        if verbose > 0 && format == Format::Json {
            eprintln!(
                "note: JSON output stays canonical metric; --units, --width and --color do not apply"
            );
        }
        let units = UnitSystem::from_str(&settings.units)?.resolve(&config.units)?;
        // An unsupported language is a warning, not a usage error: the forecast is still what the
        // user asked for, so the run falls back to English and says so (unless `-q`).
        let i18n = I18n::load(&LanguageRequest::parse(&settings.lang), |name| {
            std::env::var(name).ok()
        });
        if !quiet {
            for warning in i18n.warnings() {
                eprintln!("{warning}");
            }
        }
        let width = resolve_width(
            query
                .width
                .map(usize::from)
                .or((config.render.width > 0).then_some(config.render.width)),
        );
        let color = if format == Format::Dumb {
            ColorMode::Never
        } else {
            let requested = match query.color {
                Some(color) => color,
                None => color_mode(&config.render.color)?,
            };
            resolve_color(requested, &term)
        };
        Ok(Self {
            renderer,
            format,
            units,
            i18n,
            width,
            color,
            term,
        })
    }
}

/// What the run resolved to, under `--verbose`: which width, which palette, and why the table
/// fell back to ASCII.
fn render_notes(setup: &RenderSetup) {
    let (format, caps, width, color) = (setup.format, &setup.term, setup.width, setup.color);
    eprintln!(
        "width: {} columns (from {})",
        width.columns,
        width.source.as_str()
    );
    if let Some(raised) = width.raised_from {
        eprintln!(
            "note: {raised} columns is below the {} column minimum; laying out for {}",
            crate::render::MIN_WIDTH,
            width.columns
        );
    }
    eprintln!(
        "color: {} ({} palette, tty: {})",
        color.as_str(),
        match effective_depth(color, caps) {
            crate::render::ColorDepth::Mono => "none",
            crate::render::ColorDepth::Ansi16 => "16 colour",
            crate::render::ColorDepth::Ansi256 => "256 colour",
        },
        caps.is_tty
    );
    eprintln!("{}", setup.i18n.report());
    // A message a catalog lacks is a bug in this crate, not a user error: it renders its key and is
    // reported here, where `-v` asked for exactly this kind of detail.
    for note in setup.i18n.notes() {
        if matches!(note, crate::i18n::Note::MissingKey(_)) {
            eprintln!("{}", note.text());
        }
    }
    if format == Format::Dumb {
        eprintln!("note: `--format dumb` draws the ASCII table without colour");
    } else if caps.charset() == Charset::Ascii {
        eprintln!("note: drawing the ASCII table (dumb TERM or a non-UTF-8 locale)");
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
    let (spec, text) = location_target(args.query.as_deref(), args.ip, &config)?;
    let (location, resolution, candidates) =
        resolve_location(&spec, args.limit, &config, &http, &cache, cli)?;

    print_line(format_args!("{}", location_line(&location)))?;
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
///
/// `--ip` is not checked against the argument here: whether that is a conflict or the flag simply
/// winning depends on *where* the argument came from, and only [`validate_query`] can see that (a
/// command line argument conflicts, an environment or configured location is overridden).
fn location_target(
    requested: Option<&str>,
    ip: bool,
    config: &Config,
) -> Result<(LocationSpec, Option<String>)> {
    if ip {
        return Ok((LocationSpec::Default, None));
    }
    let requested = LocationSpec::parse_arg(requested)?;
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
    limit: u8,
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
            let hits = geocoder.search(spec.query().unwrap_or_default(), limit)?;
            let candidates = rank(hits.clone(), spec.query(), limit);
            let (location, resolution) = resolve(hits, spec, limit)?;
            Ok((location, resolution, candidates))
        }
        spec @ LocationSpec::Osm(_) => {
            let nominatim = Nominatim::new(http, cache, nominatim_url(config));
            let hits = nominatim.search(spec.query().unwrap_or_default(), limit)?;
            let candidates = rank(hits.clone(), spec.query(), limit);
            let (location, resolution) = resolve(hits, spec, limit)?;
            Ok((location, resolution, candidates))
        }
        spec @ LocationSpec::LatLon(..) => {
            let (location, resolution) = resolve(Vec::new(), spec, limit)?;
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
                print_line(format_args!("{line}"))?;
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
                print_line(format_args!("removed {} {noun}", report.removed))?;
            } else {
                print_line(format_args!("removed {} expired {noun}", report.removed))?;
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
            print_line(format_args!("{}", paths.config_file.display()))?;
            Ok(())
        }
        ConfigCommand::Init { force } => {
            let path = Config::write_default(&paths, *force)?;
            print_line(format_args!("wrote {}", path.display()))?;
            Ok(())
        }
        ConfigCommand::Show => {
            let config = Config::load(&paths)?;
            let document = toml::to_string_pretty(&config).map_err(|error| {
                Error::Other(format!("cannot render the configuration: {error}"))
            })?;
            print_text(format_args!("{document}"))?;
            Ok(())
        }
        ConfigCommand::Get { key } => {
            let config = Config::load(&paths)?;
            print_line(format_args!("{}", config.get_key(key)?))?;
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
            print_line(format_args!(
                "ok: {}",
                source
                    .unwrap_or_else(|| paths.config_file.clone())
                    .display()
            ))?;
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
            print_line(format_args!(
                "stored {id} API key in {}",
                store.path().display()
            ))?;
            Ok(())
        }
        KeyCommand::Rm { provider } => {
            let id: ProviderId = provider.parse()?;
            if store.remove(id.as_str())? {
                print_line(format_args!("removed {id} API key"))?;
            } else {
                print_line(format_args!("no {id} API key stored"))?;
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
                print_line(format_args!(
                    "{:<width$}  {}  ({})",
                    row.provider,
                    row.masked,
                    source_label(row.source)
                ))?;
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
                print_line(format_args!("{line}"))?;
            }
            Ok(())
        }
        ProviderCommand::Info(info) => {
            let id: ProviderId = info.id.parse()?;
            for line in provider_details(&id.metadata()) {
                print_line(format_args!("{line}"))?;
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
    print_line(format_args!("ok: {}", paths.config_file.display()))?;
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
    let mut lines = vec![
        info_line("id:", meta.id),
        info_line("name:", meta.display_name),
        info_line(
            "status:",
            if meta.implemented {
                "implemented"
            } else {
                "planned"
            },
        ),
        info_line("auth:", meta.auth),
        info_line("key:", key_label(meta)),
    ];
    if meta.requires_key {
        lines.push(info_line(
            "store key:",
            format!("cirrocast key set {}", meta.id),
        ));
    }
    lines.extend([
        info_line("current:", yes_no(meta.current)),
        info_line("hourly:", yes_no(meta.hourly)),
        info_line("daily:", yes_no(meta.daily)),
        info_line("alerts:", yes_no(meta.alerts)),
        info_line("max days:", meta.max_days),
        info_line("locations:", locations),
        info_line("coverage:", meta.coverage),
        info_line("granularity:", meta.granularity),
        info_line("limits:", meta.limits),
        info_line(
            "credit:",
            meta.licence.unwrap_or(
                "not printed yet; the backend is not implemented (obligation in docs/providers.md)",
            ),
        ),
        info_line("docs:", meta.docs_url),
        info_line("verified:", meta.verified),
        info_line("notes:", meta.notes),
    ]);
    lines
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
    format!("{label:<13}{value}")
}

/// The width of a column: the wider of its header and its longest value.
fn column_width<'a>(header: &str, values: impl Iterator<Item = &'a str>) -> usize {
    values.fold(header.len(), |width, value| width.max(value.len()))
}

#[cfg(test)]
mod tests {
    use clap::{CommandFactory as _, FromArgMatches as _, Parser as _};

    use super::{
        Cli, Source, Sources, location_arg, provider_chain, request_days, station_chain,
        validate_query,
    };
    use crate::config::{CliOverrides, Config, Settings};
    use crate::provider::ProviderId;

    /// Parses a command line the way `main` does, provenance included.
    fn parse(args: &[&str]) -> (Cli, Sources) {
        let matches = Cli::command()
            .try_get_matches_from(args)
            .expect("the command line parses");
        let sources = Sources::read(&matches);
        let cli = Cli::from_arg_matches(&matches).expect("the command line parses");
        (cli, sources)
    }

    /// The settings of a run with `provider` selected and nothing else set.
    fn settings(provider: &str) -> Settings {
        Settings::resolve(
            &Config::default(),
            &CliOverrides {
                provider: Some(provider.to_owned()),
                ..CliOverrides::default()
            },
        )
        .expect("the default configuration resolves")
    }

    #[test]
    fn clap_records_where_each_setting_came_from() {
        let (_, sources) = parse(&["cirrocast", "Beijing", "-p", "open-meteo", "-d", "3"]);
        assert_eq!(sources.provider, Source::CommandLine);
        assert_eq!(sources.days, Source::CommandLine);
        assert_eq!(sources.location, Source::CommandLine);
        assert_eq!(sources.units, Source::Default);
        assert_eq!(sources.timeout, Source::Default);
        assert_eq!(Source::CommandLine.as_str(), "the command line");
        assert_eq!(
            Source::Default.as_str(),
            "the config or the built-in default"
        );
    }

    #[test]
    fn the_days_clamp_is_reported_once_and_silenceable() {
        // Open-Meteo's 16 days are beyond the flag's own 0..=14, so the clamp cannot bite here.
        let (days, warning) = request_days(14, &[ProviderId::OpenMeteo]);
        assert_eq!(days, 14);
        assert_eq!(warning, None);

        // A station-only backend has no forecast at all: everything clamps to zero.
        let max = ProviderId::Metar.metadata().max_days;
        let (days, warning) = request_days(14, &[ProviderId::Metar]);
        assert_eq!(days, max);
        assert_eq!(
            warning.expect("the clamp is reported"),
            format!("warning: metar supports at most {max} days; --days 14 clamped to {max}")
        );

        // The clamp follows the *first* entry: a fallback cannot widen the request.
        let (days, warning) = request_days(14, &[ProviderId::Metar, ProviderId::OpenMeteo]);
        assert_eq!(days, max);
        assert!(warning.is_some());

        // No chain at all is not a clamp case.
        assert_eq!(request_days(3, &[]), (3, None));
    }

    #[test]
    fn a_station_chain_is_metar_first_or_auto() {
        for spec in [
            "auto",
            "AUTO",
            "metar",
            "METAR",
            "metar,open-meteo",
            " metar , smhi ",
        ] {
            assert!(station_chain(spec), "`{spec}` answers a station");
        }
        for spec in ["open-meteo", "", "open-meteo,metar", "smhi"] {
            assert!(!station_chain(spec), "`{spec}` does not answer a station");
        }
    }

    #[test]
    fn the_auto_chain_gains_metar_only_for_a_station() {
        assert_eq!(
            provider_chain(&settings("auto"), None).expect("auto expands"),
            vec![ProviderId::OpenMeteo, ProviderId::Smhi]
        );
        let error = provider_chain(&settings("auto"), Some("ZBAA"))
            .expect_err("metar is not implemented yet");
        assert!(
            error
                .to_string()
                .contains("provider `metar` is not implemented yet")
        );

        assert_eq!(
            provider_chain(&settings("open-meteo"), None).expect("an explicit chain"),
            vec![ProviderId::OpenMeteo]
        );
    }

    #[test]
    fn latitude_and_longitude_become_the_coordinate_spelling() {
        let (cli, _) = parse(&["cirrocast", "--lat", "39.9", "--lon", "116.4"]);
        assert_eq!(location_arg(&cli.query).as_deref(), Some("@39.9,116.4"));

        let (cli, _) = parse(&["cirrocast", "Beijing"]);
        assert_eq!(location_arg(&cli.query).as_deref(), Some("Beijing"));

        let (cli, _) = parse(&["cirrocast"]);
        assert_eq!(location_arg(&cli.query), None);
    }

    #[test]
    fn a_command_line_location_conflicts_with_the_other_location_forms() {
        let defaults = settings("open-meteo");
        for args in [
            vec!["cirrocast", "Beijing", "--ip"],
            vec!["cirrocast", "Beijing", "--station", "ZBAA"],
        ] {
            let (cli, sources) = parse(&args);
            let error = validate_query(&cli.query, sources, &defaults)
                .expect_err("a location argument is exclusive");
            assert_eq!(error.exit_code(), 2, "{args:?}");
            assert!(
                error
                    .to_string()
                    .contains("a location argument cannot be combined"),
                "{error}"
            );
        }

        // The same argument from the environment is overridden by the flag, not a conflict: the
        // environment tier loses to the command line by precedence.
        let (cli, mut sources) = parse(&["cirrocast", "Beijing", "--ip"]);
        sources.location = Source::Environment;
        validate_query(&cli.query, sources, &defaults).expect("the flag overrides the environment");

        // `--station` needs a chain that can answer it.
        let (cli, sources) = parse(&["cirrocast", "--station", "ZBAA"]);
        let error = validate_query(&cli.query, sources, &defaults)
            .expect_err("open-meteo cannot answer a station");
        assert_eq!(
            error.to_string(),
            "--station requires --provider metar (or auto)"
        );
        let (cli, sources) = parse(&["cirrocast", "--station", "ZBAA", "-p", "metar"]);
        validate_query(&cli.query, sources, &settings("metar")).expect("metar answers a station");
    }

    #[test]
    fn the_flag_matrix_parses_into_typed_values() {
        let (cli, _) = parse(&[
            "cirrocast",
            "--units",
            "uk",
            "--color",
            "always",
            "--width",
            "120",
            "--days",
            "0",
            "--lang",
            "auto",
            "-f",
            "one-line",
            "--template",
            "@full",
        ]);
        let query = &cli.query;
        assert_eq!(
            query.units.map(|units| units.to_string()).as_deref(),
            Some("uk")
        );
        assert_eq!(query.color, Some(crate::render::ColorMode::Always));
        assert_eq!(query.width, Some(120));
        assert_eq!(query.days, Some(0));
        assert_eq!(query.lang.as_deref(), Some("auto"));
        assert_eq!(query.format, Some(crate::render::Format::OneLine));
        assert_eq!(query.template.as_deref(), Some("@full"));
    }

    #[test]
    fn out_of_range_coordinates_and_widths_are_usage_errors() {
        for args in [
            vec!["cirrocast", "--lat", "91", "--lon", "0"],
            vec!["cirrocast", "--lat", "0", "--lon", "181"],
            vec!["cirrocast", "--width", "0"],
            vec!["cirrocast", "--width", "501"],
            vec!["cirrocast", "--units", "kelvin"],
            vec!["cirrocast", "--days", "15"],
        ] {
            let error = Cli::try_parse_from(&args).expect_err("rejected by clap");
            let rendered = error.to_string();
            assert!(
                rendered.contains("invalid value") || rendered.contains("error:"),
                "{args:?}: {rendered}"
            );
            assert_eq!(error.exit_code(), 2, "{args:?}");
        }
    }
}
