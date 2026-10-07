// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The command line surface.
//!
//! Scope: global flags, the `config`/`key`/`provider` subcommands. Weather query flags
//! (`--provider`, `--format`, `--days`, ...) and the location argument are added by the steps that
//! implement them, so that a flag never exists before the behaviour behind it.

use std::collections::BTreeMap;
use std::fs;
use std::io::{IsTerminal as _, Read as _, Write as _};
use std::process::Command as StdCommand;
use std::str::FromStr as _;
use std::sync::Arc;
use std::time::Duration;

use clap::{ArgAction, Args, Parser, Subcommand, ValueEnum};

use crate::air::aqi::AqiIndex;
use crate::alerts::{self, AlertsRequest};
use crate::cache::{Cache, CacheMode, CacheStat, Clock, OfflineMode, Scope, SystemClock};
use crate::config::keys::{JwtCredential, KeyForm, KeySource, KeyStore, KeySummary};
use crate::config::{Config, Settings};
use crate::error::{Error, Result};
use crate::geo::chain::{SearchChain, SearchInputs};
use crate::geo::ip::{IpLocatorChain, IpService};
use crate::geo::nominatim::{DEFAULT_URL, Nominatim};
use crate::geo::{
    Geocoder, LocationSpec, Resolution, Resolved, ambiguity_note, attribution_line, location_line,
    offline_not_found, osm_ambiguity_note, place, resolve_candidates,
};
use crate::http::{HttpClient, UreqTransport};
use crate::i18n::{I18n, LanguageRequest};
use crate::model::LocalTimes;
use crate::model::Location;
use crate::model::Severity;
use crate::model::alert::AlertSource;
use crate::model::units::{ResolvedUnits, UnitSystem};
use crate::paths::Paths;
use crate::provider::{Env, fetch_chain, licence_line, select, select_for};
use crate::provider::{FetchRequest, HourlyResolution, ProviderId, ProviderMeta};
use crate::render::{
    Charset, ColorMode, Format, RenderContext, TermCaps, effective_depth, renderer_for,
    resolve_color, resolve_width,
};

/// Writes a line to stdout, treating a closed pipe as success.
///
/// `cirrocast ... | head` is a normal thing to do and the reader going away is not a failure the
/// user asked to hear about; `println!` would panic on the write error, this returns quietly.
pub(crate) fn print_line(line: impl std::fmt::Display) -> Result<()> {
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
    after_long_help = HELP_EPILOG.as_str(),
    propagate_version = true,
    allow_negative_numbers = true
)]
pub struct Cli {
    /// Print more detail: settings, resolution notes, the upstream request behind the answer
    /// (secrets redacted) and the error cause chain. Repeat (`-vv`) for every HTTP attempt, its
    /// status, and the cache decisions as well.
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

/// The precedence ladder: the first block of the `--help` epilog and of the man page's EXTRA
/// section.
const HELP_PRECEDENCE: &str = "\
CONFIG PRECEDENCE (highest first)
  command line flag > CIRROCAST_* environment variable > config.toml > built-in default
  --provider  CIRROCAST_PROVIDER   --days    CIRROCAST_DAYS     --timeout CIRROCAST_TIMEOUT
  --format    CIRROCAST_FORMAT     --units   CIRROCAST_UNITS    LOCATION  CIRROCAST_LOCATION
  --lang      CIRROCAST_LANG
  env only    CIRROCAST_LOCATION_PICK  CIRROCAST_NOMINATIM_URL  CIRROCAST_IP_SERVICE
              CIRROCAST_GEO_SEARCH  CIRROCAST_GEO_REVERSE  CIRROCAST_GEONAMES_USER
              CIRROCAST_NORMALS_PERIOD  CIRROCAST_NORMALS_MAX_DISTANCE_KM
  The configuration file is consulted only when neither the flag nor the variable is set, so an
  environment value is never overridden by config.toml. `config get <key>` prints the variable's
  value when one is set, and `config validate` checks the file without touching the network.";

/// The exit-code table: `--help` prints it under its EXIT CODES heading and the man page under EXIT
/// STATUS, from this one literal.
const EXIT_CODES: &str = "  0  success
  1  generic failure: an unexpected error outside the classes below
  2  usage: a flag or value the command line rejects, or mutually exclusive flags
  3  network or upstream failure: no connection, a retryable status, an unusable body
  4  configuration or state on disk: config.toml, keys.toml, the cache
  5  location not found: an unresolvable name or station
  6  missing or invalid API key: `cirrocast key set <id>` (or CIRROCAST_<ID>_KEY) fixes it";

/// The token vocabulary and the multi-location note: the last block of the `--help` epilog and of
/// the man page's EXTRA section.
const HELP_TOKENS: &str = "\
ONE-LINE TOKENS (--format one-line, full, minimal, or a [templates] key)
  %c condition art    %C condition text   %x condition, plain text
  %t temp             %f feels-like       %H today's high    %L today's low
  %w wind             %h humidity         %p precip          %P pressure
  %e dew point        %u UV index         %U UV + band       %m moon glyph
  %M moon phase       %v visibility       %l name            %d ISO date
  %D Wed 30 Sep       %T 15:04            %Z zone name       %z +0800
  %S sunrise          %s sunset           %q air-quality index
  %A strongest alert event, empty when no alerts are in force
  %[-][0][width][.prec]X pads (right with -, zero for numbers); .prec truncates text and rounds
  numbers; %% prints one %; %{...} verbatim unless one token letter; \\n \\t \\\\ escapes; bad %X is exit 2.
  Presets (@NAME, and --format NAME), listed with their templates:
    @default  %l: %c %C %t (%f), %w, %h, %p, %P, %v   @short  %c %t
    @minimal  %c%t   @uv  %l: UV %U   @sun  %l: sunrise %S sunset %s (%z %Z)
    @full     %l: %c %C %t (%f) %w %h %p %P %m %v %u %S %s %Z

MULTI-LOCATION RUNS
  Several LOCATION arguments are fetched at most four at a time and printed in argument order.
  A failed location keeps its slot on stdout (`error: <query>: <message>`) and the run exits with
  the largest mapped code among the failures; `json` becomes an array, and `art-table` draws a
  combined summary for up to four locations. CIRROCAST_LOCATION names one location — the environment
  tier has no second argument — so a multi-location run passes its locations on the command line.";

/// The tables the long `--help` ends with.
///
/// They are part of the CLI's contract, not decoration: the precedence ladder, the exit codes and
/// the token vocabulary are what a script or a user reads before writing anything against the
/// tool. The token and preset rows are duplicated from [`crate::render::one_line`] because clap
/// takes a compile-time value here; a unit test compares the two so they cannot drift. The man
/// page prints the same three blocks — its EXTRA section is the epilog without the exit-code table,
/// which it carries under EXIT STATUS of its own — so the blocks are three constants assembled
/// here rather than one literal.
static HELP_EPILOG: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
    format!("{HELP_PRECEDENCE}\n\nEXIT CODES\n{EXIT_CODES}\n\n{HELP_TOKENS}")
});

/// The weather query: the whole flag matrix of `cirrocast <LOCATION>`.
///
/// The flags are grouped the way `--help` shows them: what to fetch, where from, what units and
/// language the answer is rendered in, how wide and how colourful the layout is, and how the cache
/// and the transport behave. Everything that also exists as a configuration key carries the
/// `CIRROCAST_*` override, whose precedence the epilog spells out.
// The query flags really are independent switches (`--ip`, `--alerts`, `--no-alerts`, `--aqi`);
// folding them into a state machine would obscure the clap surface rather than simplify it.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Args)]
pub struct QueryArgs {
    /// Location arguments (`Beijing`, `:Beijing`, `~Tsinghua`, `@39.9,116.4`, `@home`); several
    /// arguments are fetched at most four at a time and printed in argument order. Omitted = the
    /// configured default location, else the public IP.
    #[arg(value_name = "LOCATION", env = "CIRROCAST_LOCATION")]
    pub location: Vec<String>,

    /// Provider chain, comma separated; `auto` ranks the keyless backends by coverage for the
    /// resolved place (plus `metar` with `--station`).
    #[arg(short = 'p', long, value_name = "LIST", env = "CIRROCAST_PROVIDER")]
    pub provider: Option<String>,

    /// Output format: a built-in name (`art-table`, `one-line`, `plain`, `json`, `dumb`, `alerts`,
    /// `aqi`, `moon`, `normals`), a one-line preset (`default`, `short`, `minimal`, `full`, `uv`,
    /// `sun`) or a `[templates]` key.
    #[arg(short = 'f', long, value_name = "NAME", env = "CIRROCAST_FORMAT")]
    pub format: Option<String>,

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
    /// ipapi.co and IP.SB); the answer is cached for 24 hours. It never happens without `--ip` or
    /// an empty location everywhere.
    #[arg(long, conflicts_with = "station")]
    pub ip: bool,

    /// METAR station identifier (ICAO, four characters); `metar` heads the provider chain.
    #[arg(long, value_name = "ICAO", value_parser = parse_station)]
    pub station: Option<String>,

    /// Fetch severe-weather warnings: the sources covering the location are selected automatically
    /// (national services first, the global aggregators after). Also on by default unless
    /// `[alerts] enabled = false`; `--no-alerts` turns it off for one run.
    #[arg(long, conflicts_with = "no_alerts")]
    pub alerts: bool,

    /// Do not fetch severe-weather warnings in this run.
    #[arg(long)]
    pub no_alerts: bool,

    /// Alert sources to query instead of the coverage-selected set, comma separated: `nws`,
    /// `meteoalarm`, `qweather`, `hko`, `wmoswic`, `fpas`, `visualcrossing`. A source that does
    /// not cover the location is refused, and `visualcrossing` needs `--provider visualcrossing`
    /// because its warnings travel in that payload.
    #[arg(long, value_name = "LIST")]
    pub alerts_from: Option<String>,

    /// Lowest alert severity to show (`unknown`, `minor`, `moderate`, `severe`, `extreme`);
    /// overrides `[alerts] severity_threshold`.
    #[arg(long, value_name = "LEVEL", value_parser = parse_severity)]
    pub severity: Option<Severity>,

    /// Append the air-quality panel (US and European AQI, the six pollutants, pollen where the
    /// source covers it and the report's UV reading) to the table and plain output, and carry it
    /// as a typed object in `json`. `--format aqi` prints the panel standalone.
    #[arg(long)]
    pub aqi: bool,

    /// Which AQI scale drives the panel's colour and the one-line `%q` token: `us` or `european`;
    /// overrides `[air] index`. Needs `--aqi` or `--format aqi`.
    #[arg(long, value_name = "SCALE", value_parser = parse_aqi_index)]
    pub aqi_index: Option<AqiIndex>,

    /// Compute the moon and sun block locally (no request) and append it to the table and `plain`
    /// output; `json` then carries it as the `astro` object. `--format moon` prints the standalone
    /// view, and `one-line` shows the moon through the `%m`/`%M` tokens.
    #[arg(long)]
    pub moon: bool,

    /// Render the archive for one date (`YYYY-MM-DD`) instead of a forecast; needs a
    /// history-capable backend.
    #[arg(long, value_name = "YYYY-MM-DD", value_parser = parse_date, conflicts_with_all = ["history", "days"])]
    pub date: Option<chrono::NaiveDate>,

    /// Render the archive for the last `<N>d` days, ending yesterday; needs a history-capable
    /// backend.
    #[arg(long, value_name = "Nd", value_parser = parse_history, conflicts_with = "days")]
    pub history: Option<u16>,

    /// Append the marine block (waves, swell, sea-surface temperature).
    #[arg(long)]
    pub marine: bool,

    /// Compare the forecast with the climate: fetch the month's normal for the station nearest the
    /// location from NOAA NCEI's Global Summary of the Month (two extra requests, cached for 30
    /// days) and append the comparison to the table and `plain` output, or carry it as the typed
    /// `normals` object in `json`. `--format normals` prints it standalone. Off unless asked for,
    /// or unless `[defaults] normals = true`.
    #[arg(long)]
    pub normals: bool,

    /// Template for a one-line output: a literal `%`-token string, or `@PRESET`. The presets are
    /// `@default`, `@short`, `@minimal`, `@full`, `@uv` and `@sun`, plus any `[templates]` key;
    /// `--help` lists every token and the width/precision syntax. An unknown token is a usage
    /// error. Needs `--format one-line` (or a format name that selects no template itself).
    #[arg(long, value_name = "TEMPLATE", conflicts_with = "template_file")]
    pub template: Option<String>,

    /// Read the one-line template from a file; `-` reads standard input. Same restrictions as
    /// `--template`.
    #[arg(long, value_name = "PATH", conflicts_with = "template")]
    pub template_file: Option<String>,

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

    /// Ask which candidate to use when a location name matches several places, instead of taking
    /// the ranked winner. The ranked list goes to stderr and one line is read from stdin.
    #[arg(long, conflicts_with = "yes")]
    pub pick: bool,

    /// Take the ranked winner when a location name matches several places, without asking. This
    /// is the default on a non-terminal run; the flag makes that explicit and keeps a prompt from
    /// ever appearing.
    #[arg(long, conflicts_with = "pick")]
    pub yes: bool,
}

/// `--units`, parsed by the unit module: the flag, `CIRROCAST_UNITS` and `defaults.units` accept
/// the same three spellings and share one error message.
fn parse_units(value: &str) -> Result<UnitSystem, Error> {
    value.parse()
}

/// `--station`: an ICAO identifier, four characters, first a letter, then letters or digits.
///
/// The value is upper-cased before use, so `kjfk` and `KJFK` are the same station. Anything else is
/// a usage error that names the offending value and the accepted form — a three-letter IATA code or
/// a five-digit WMO number is a different vocabulary this backend does not speak.
fn parse_station(value: &str) -> Result<String, Error> {
    let station = value.trim();
    if crate::provider::metar::is_icao_station(station) {
        Ok(station.to_ascii_uppercase())
    } else {
        Err(Error::Usage(format!(
            "`{station}` is not an ICAO station identifier; expected four characters starting with a letter, e.g. `--station EGLL`"
        )))
    }
}

/// `--severity`: one of the CAP levels, parsed by the model so flag and config share one message.
fn parse_severity(value: &str) -> Result<Severity, Error> {
    value.parse()
}

/// `--date`: an absolute calendar date, in the one spelling the request and the archive header
/// both use.
fn parse_date(value: &str) -> Result<chrono::NaiveDate, Error> {
    chrono::NaiveDate::parse_from_str(value.trim(), "%Y-%m-%d").map_err(|_| {
        Error::Usage(format!(
            "--date takes a calendar date like `2026-09-14`, not `{value}`"
        ))
    })
}

/// `--history <N>d`: a whole number of days back, with the documented `d` suffix.
///
/// The upper bound is the archive backend's own span (`history_days = 30000`); the per-date limit
/// is checked against the answering backend before the request is built.
fn parse_history(value: &str) -> Result<u16, Error> {
    let digits = value
        .trim()
        .strip_suffix(['d', 'D'])
        .unwrap_or(value.trim());
    let days: u16 = digits.parse().map_err(|_| {
        Error::Usage(format!(
            "--history takes a number of days like `7d`, not `{value}`"
        ))
    })?;
    if days == 0 || days > 30000 {
        return Err(Error::Usage(format!(
            "--history must be between `1d` and `30000d`, not `{value}`"
        )));
    }
    Ok(days)
}

/// `--aqi-index`: one of the two AQI scales, parsed by the air module so flag and config share one
/// message.
fn parse_aqi_index(value: &str) -> Result<AqiIndex, Error> {
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

    /// Print one line for a status bar: never aborts on a network failure.
    #[command(long_about = STATUS_LONG_ABOUT)]
    Status(StatusArgs),

    /// Print a shell completion script.
    Completion(CompletionArgs),

    /// Print the manual page as roff.
    Man(ManArgs),
}

/// Arguments of `cirrocast status`.
///
/// The whole output is one template, so `-f/--format` here names a `%`-template rather than the
/// global format enum — `--template` is the same flag under its literal name. Everything the probe
/// renders comes from the provider chain, the cache and the template engine the query uses.
#[derive(Debug, Args)]
pub struct StatusArgs {
    /// The `%`-template to render, or `@NAME` for a `[templates]` key. Default: `%c %t`.
    #[arg(
        short = 'f',
        long,
        value_name = "TEMPLATE",
        conflicts_with = "template"
    )]
    pub format: Option<String>,

    /// A synonym of `--format` on this subcommand. The global `-f/--format <NAME>` enum (with
    /// `art-table`, `json`, …) does not apply to `status`, whose output is always one line.
    #[arg(long, value_name = "TEMPLATE", conflicts_with = "format")]
    pub template: Option<String>,

    /// Location to report on: any location argument (`Beijing`, `@39.9,116.4`, `@home`). Omitted,
    /// `CIRROCAST_LOCATION` then `[location] default` supplies it. The public-IP lookup never runs
    /// here.
    #[arg(long, value_name = "SPEC")]
    pub location: Option<String>,

    /// Serve a cached answer younger than this many seconds without revalidating; `0` follows
    /// `[cache] weather_ttl_secs`, which is also the default of the flag. At most a week.
    #[arg(long, value_name = "SECS", value_parser = clap::value_parser!(u64).range(0..=604_800))]
    pub max_age: Option<u64>,

    /// Serve from the cache only: never open a socket, and accept any cached answer however old.
    #[arg(long)]
    pub offline: bool,

    /// What to print on stdout instead of a reading when the probe cannot produce one (the
    /// network is unavailable and nothing is cached). Default: `[status] placeholder`.
    #[arg(long, value_name = "TEXT")]
    pub placeholder: Option<String>,

    /// Colour: `never` (default, so a bar sees no ANSI escapes) or `always`.
    #[arg(long, value_name = "WHEN", value_enum, default_value_t = StatusColor::Never)]
    pub color: StatusColor,
}

/// `cirrocast status --color`: the two modes that make sense for a one-line probe.
///
/// `auto` is deliberately absent — the probe is read by tools that strip ANSI inconsistently, so
/// colour is opt-in rather than detected. `never` is the default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum StatusColor {
    /// Never emit ANSI escapes.
    Never,
    /// Emit ANSI escapes even into a pipe.
    Always,
}

/// The long help of `cirrocast status`: the contract a status bar is written against.
const STATUS_LONG_ABOUT: &str = "\
Print exactly one line (plus a newline) for a status bar or a prompt, and exit 0 whatever the
weather, the network or the cache is doing.

  cirrocast status --location Beijing               # -> `+18°C`
  cirrocast status -f '%c %t' --offline             # cache only, never a socket
  cirrocast status --format '%l %t' --max-age 900   # at most one fetch per 15 minutes

The output is the `%`-template named by --format/--template (default `%c %t`) expanded by the
same engine as `--format one-line`; a template newline becomes a space and the line is trimmed.
Colour is off unless --color always is given.

Exit codes: 0 for a reading *and* for every transient or data failure (the placeholder goes to
stdout and one `error: …` line to stderr), 2 for a usage mistake (an unknown token, an unknown
flag) and 4 for a configuration problem (an unreadable config, no location configured). A status
bar can therefore run this on a timer without ever showing a crashed module.

With no --location, `CIRROCAST_LOCATION` then `[location] default` must name one: the probe never
performs the public-IP lookup, so an empty location is exit 4 rather than a lookup. Alerts are
fetched only when the template shows `%A`, and the air reading only when it shows `%q`.";

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

    /// Build a city table from a `GeoNames` `cities15000` dump and install it under
    /// `$XDG_DATA_HOME/cirrocast/geo/`, where name resolution prefers it over the bundled table.
    UpdateData(UpdateDataArgs),
}

/// Arguments of `cirrocast location update-data`.
#[derive(Debug, Args)]
pub struct UpdateDataArgs {
    /// Read the dump from this path or URL (`.txt` or `.zip`) instead of `[geo] update_url` and
    /// the official `https://download.geonames.org/export/dump/cities15000.zip`.
    #[arg(long, value_name = "PATH|URL")]
    pub from: Option<String>,

    /// Only report whether the source differs from the table this run would use; write nothing.
    #[arg(long)]
    pub check: bool,

    /// Per-request timeout in seconds; overrides `network.timeout_secs`.
    #[arg(
        long,
        value_name = "SECS",
        env = "CIRROCAST_TIMEOUT",
        value_parser = clap::value_parser!(u32).range(1..=300)
    )]
    pub timeout: Option<u32>,

    /// Refused on purpose: the update command fetches by definition.
    #[arg(long)]
    pub offline: bool,
}

/// Arguments of `cirrocast location search`.
#[derive(Debug, Args)]
pub struct SearchArgs {
    /// Location argument (`Beijing`, `:Beijing`, `~Tsinghua`, `@39.9,116.4`); omitted = the
    /// configured default location, else the public IP.
    #[arg(value_name = "LOCATION", conflicts_with = "ip")]
    pub query: Option<String>,

    /// Locate from the public IP address. This sends the address to ipwho.is (falling back to
    /// ipapi.co and IP.SB); the answer is cached for 24 hours. It never happens without `--ip` or
    /// an empty location everywhere.
    #[arg(long)]
    pub ip: bool,

    /// Only count candidates whose name is exactly the query (folded), the same narrowing as the
    /// `:query` spelling.
    #[arg(long)]
    pub exact: bool,

    /// Print the ranked candidate table instead of the single winning line.
    #[arg(long)]
    pub all: bool,

    /// How many geocoder candidates to rank (1..=100).
    #[arg(long, value_name = "N", default_value_t = 10, value_parser = clap::value_parser!(u8).range(1..=100))]
    pub limit: u8,

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

/// The mutually exclusive cache-control flags.
#[derive(Debug, Clone, Copy, Default, Args)]
pub struct CacheFlags {
    /// Ignore the cache for this run and store nothing.
    #[arg(long, conflicts_with_all = ["refresh", "offline"])]
    pub no_cache: bool,

    /// Ignore cached answers and replace them with fresh ones.
    #[arg(long, conflicts_with_all = ["no_cache", "offline"])]
    pub refresh: bool,

    /// Run offline: `=weather` serves the weather from the cache only, `=geo` resolves names from
    /// the bundled city table only, and bare `--offline` (same as `=all`) never opens a socket.
    /// Overrides `[network] offline`.
    #[arg(
        long,
        value_name = "MODE",
        num_args = 0..=1,
        require_equals = true,
        default_missing_value = "all",
        value_parser = parse_offline_mode
    )]
    pub offline: Option<OfflineMode>,
}

/// `--offline[=<weather|geo|all>]`, parsed by the cache module so flag and configuration share one
/// vocabulary.
fn parse_offline_mode(value: &str) -> Result<OfflineMode, Error> {
    OfflineMode::from_str(value)
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
    Validate {
        /// Also check the run combination `cache.enabled = false` + `--offline`, which could never
        /// be served.
        #[arg(long)]
        offline: bool,
    },
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
    /// Store an API key for a provider, or a JWT credential with `--jwt`.
    ///
    /// The secret is read from stdin — never from the command line, where `ps` and the shell
    /// history would see it; `--key-file -` does the same for a JWT private key.
    Set(KeySetArgs),

    /// Remove every stored credential of a provider.
    Rm {
        /// Provider id, e.g. `openweathermap`.
        provider: String,
    },

    /// List the configured credentials, masked (a JWT credential shows its identifiers only).
    List,
}

/// Arguments of `cirrocast key set`.
#[derive(Debug, Args)]
pub struct KeySetArgs {
    /// Provider id (e.g. `openweathermap`) or credential name (`geonames`).
    pub provider: String,

    /// Read the key from stdin even when stdin is a terminal.
    #[arg(long)]
    pub stdin: bool,

    /// Store a JWT credential (Ed25519) instead of an API key; needs the four flags below.
    #[arg(
        long,
        conflicts_with = "stdin",
        requires_all = ["key_file", "credential_id", "developer_id", "project_id"]
    )]
    pub jwt: bool,

    /// File holding the PKCS#8 Ed25519 private key; `-` reads it from stdin.
    #[arg(long, value_name = "PATH", requires = "jwt")]
    pub key_file: Option<String>,

    /// Credential id the console issued for the uploaded public key (`kid`).
    #[arg(long, value_name = "ID", requires = "jwt")]
    pub credential_id: Option<String>,

    /// Developer id shown in the console (`iss`), ten characters starting with `Q`.
    #[arg(long, value_name = "ID", requires = "jwt")]
    pub developer_id: Option<String>,

    /// Project id shown in the console (`sub`).
    #[arg(long, value_name = "ID", requires = "jwt")]
    pub project_id: Option<String>,
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
    ///
    /// On success the returned `u8` is the process exit code: `0`, or the largest mapped code among
    /// a multi-location run's per-location failures ([`crate::worst_exit_code`]). Everything that
    /// failed as a whole run is still an `Err`.
    pub fn run(&self, sources: &Sources) -> Result<u8> {
        match &self.command {
            Some(Command::Config(args)) => run_config(&args.command).map(|()| 0),
            Some(Command::Key(args)) => run_key(&args.command).map(|()| 0),
            Some(Command::Provider(args)) => run_provider(&args.command).map(|()| 0),
            Some(Command::Location(args)) => run_location(&args.command, self).map(|()| 0),
            Some(Command::Cache(args)) => run_cache(&args.command, self).map(|()| 0),
            Some(Command::Status(args)) => crate::status::run(args, self),
            Some(Command::Completion(args)) => {
                run_completion(args);
                Ok(0)
            }
            Some(Command::Man(args)) => run_man(args).map(|()| 0),
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
    /// `--aqi-index`.
    pub aqi_index: Source,
    /// The location argument / `CIRROCAST_LOCATION`.
    pub location: Source,
    /// Whether `--lat/--lon` supplied the location. Kept apart from [`Self::location`] because the
    /// coordinates outrank the argument by precedence, while [`validate_query`] must still see the
    /// argument's own tier to decide what conflicts with what.
    pub coordinates: bool,
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
        let given =
            |id: &str| matches.value_source(id) == Some(clap::parser::ValueSource::CommandLine);
        // An all-whitespace positional is "absent": it does not outrank the configured default, so
        // `-v` must not report the resolved value as coming from the command line either.
        let location = match matches.get_many::<String>("location") {
            Some(mut values) => {
                if values.all(|value| value.trim().is_empty()) {
                    Source::Default
                } else {
                    source("location")
                }
            }
            None => source("location"),
        };
        Self {
            provider: source("provider"),
            format: source("format"),
            days: source("days"),
            units: source("units"),
            lang: source("lang"),
            aqi_index: source("aqi_index"),
            location,
            coordinates: given("lat") || given("lon"),
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
            "aqi index: {} (from {})",
            settings.aqi_index,
            self.aqi_index.as_str()
        );
        eprintln!(
            "timeout: {}s (from {})",
            settings.timeout_secs,
            self.timeout.as_str()
        );
        if let Some(location) = settings
            .location
            .as_deref()
            .filter(|location| !location.trim().is_empty())
        {
            let source = if self.coordinates {
                Source::CommandLine
            } else {
                self.location
            };
            eprintln!("location: {location} (from {})", source.as_str());
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
    if location_args(query).count() > 1 {
        for (given, name) in [
            (query.lat.is_some() || query.lon.is_some(), "--lat/--lon"),
            (query.ip, "--ip"),
            (query.station.is_some(), "--station"),
        ] {
            if given {
                return Err(Error::Usage(format!(
                    "{name} cannot be combined with more than one location argument; pass exactly one or drop the flag"
                )));
            }
        }
    }
    if sources.location == Source::CommandLine && location_args(query).count() == 1 {
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
    if let Some(station) = &query.station
        && sources.provider != Source::Default
        && !station_chain(&settings.provider)
    {
        return Err(Error::Usage(format!(
            "--station {station} needs a station-capable provider: use `--provider metar` (or a chain containing it), or drop `--provider`"
        )));
    }
    Ok(())
}

/// Whether a provider chain can answer a station identifier: `auto` (which gains `metar` for the
/// run) or any chain that names `metar`, wherever it sits in the chain.
fn station_chain(spec: &str) -> bool {
    let spec = spec.trim();
    if spec.eq_ignore_ascii_case("auto") {
        return true;
    }
    spec.split(',').any(|entry| {
        entry
            .trim()
            .parse::<ProviderId>()
            .is_ok_and(|id| id == ProviderId::Metar)
    })
}

/// The chain this run fetches from.
///
/// A station identifier needs a station-capable backend, so `metar` is put at the head:
///
/// * `--station` with no `--provider`/`CIRROCAST_PROVIDER` value (the provider comes from the
///   configuration or the built-in default) or with `auto` prepends `metar` to the chain, so the
///   configured fallbacks still apply when the observation does not come back, exactly as `auto`
///   gains it for a station run;
/// * an explicit chain is used as written; [`validate_query`] has already refused one without a
///   station-capable entry.
fn provider_chain(
    settings: &Settings,
    station: Option<&str>,
    source: Source,
) -> Result<Vec<ProviderId>> {
    let auto = settings.provider.trim().eq_ignore_ascii_case("auto");
    let prepend = source == Source::Default || auto;
    match station {
        Some(_) if prepend => select(&format!("metar,{}", settings.provider.trim())),
        _ => select(&settings.provider),
    }
}

/// The station `[providers.metar] station` configures, when this run is answered by `metar`.
///
/// A station configured while another backend is the default has no effect on that backend (the
/// registry's location forms say a station is not a city), and a configured `location.default`
/// outranks it: a station is the aviation backend's *fallback* location, not a global one.
pub(crate) fn configured_station(config: &Config, ids: &[ProviderId]) -> Option<String> {
    if ids.first() != Some(&ProviderId::Metar) {
        return None;
    }
    let station = config.providers.metar.station.trim();
    if station.is_empty() {
        None
    } else {
        Some(station.to_ascii_uppercase())
    }
}

/// The forecast days to request, clamped to what the first provider of the chain serves, plus the
/// warning that explains the clamp.
///
/// Clamping here rather than inside the provider means the warning is printed exactly once, in the
/// CLI's own vocabulary, and the cache key is built for the horizon that is really fetched. The
/// caller silences the warning with `-q`; an observations-only backend (`max_days == 0`) clamps
/// everything to zero, which is what a station forecast is.
///
/// Two rows do not clamp but refuse: an **archive-only** backend (`max_days == 0` with
/// `history_days > 0`) has no forecast to shorten, so asking it for days is a usage error naming
/// the two flags that do work; and `window` says the run asked for an absolute date window
/// instead, in which case `days` is the window's length and no forecast clamp applies.
pub(crate) fn request_days(
    requested: u8,
    ids: &[ProviderId],
    days_explicit: bool,
    window: bool,
) -> Result<(u8, Option<String>)> {
    let Some(primary) = ids.first() else {
        return Ok((requested, None));
    };
    let meta = primary.metadata();
    if window {
        // `--date`/`--history`: the window decides the span, per location (it needs the location's
        // own calendar), and `validate_query` already checked that some chain entry has a history.
        return Ok((requested, None));
    }
    if meta.max_days == 0 && meta.history_days > 0 {
        return Err(Error::Usage(format!(
            "provider {primary} is archive only; pass `--date <YYYY-MM-DD>` or `--history <N>d` instead of a forecast"
        )));
    }
    let max_days = meta.max_days;
    if max_days == 0 && requested > 0 {
        // An observation-only backend has nothing to clamp *to*: the days value is dropped, and
        // only a `--days` the user actually typed is worth a warning about — the configured or
        // built-in default is not a request for a forecast.
        let warning = days_explicit
            .then(|| format!("warning: {primary} reports observations only; --days is ignored"));
        return Ok((0, warning));
    }
    if requested > max_days {
        return Ok((
            max_days,
            Some(format!(
                "warning: {primary} supports at most {max_days} days; --days {requested} clamped to {max_days}"
            )),
        ));
    }
    Ok((requested, None))
}

/// The absolute window a `--date`/`--history` run asks for, in the location's own calendar.
///
/// `--date` is exactly that date; `--history <N>d` is the `N` days ending yesterday, because a
/// "last 7 days" reading that included today's half-finished one would be neither a forecast nor a
/// complete archive. The window is computed after the location is resolved (its time zone decides
/// what "yesterday" means), so a multi-location run gets the right window per slot.
fn request_window(
    query: &QueryArgs,
    location: &Location,
    now: chrono::DateTime<chrono::Utc>,
) -> Option<crate::provider::DateWindow> {
    if let Some(date) = query.date {
        return Some(crate::provider::DateWindow::day(date));
    }
    let history = query.history?;
    let today = now.with_timezone(&location.tz).date_naive();
    let end = today - chrono::Days::new(u64::from(history));
    Some(crate::provider::DateWindow {
        start: end,
        end: end + chrono::Days::new(u64::from(history - 1)),
    })
}

/// Refuses a window that reaches further back than the answering backend can serve.
///
/// `open-meteo` carries 92 days and the archive carries everything since 1940-01-01, so the bound
/// is per chain head and the error names the oldest date it can answer for rather than a bare
/// refusal.
fn check_window(
    window: crate::provider::DateWindow,
    ids: &[ProviderId],
    now: chrono::DateTime<chrono::Utc>,
) -> Result<()> {
    let Some(primary) = ids.first() else {
        return Ok(());
    };
    let history = primary.metadata().history_days;
    if history == 0 {
        return Err(Error::Usage(format!(
            "provider {primary} has no archive; drop `--date`/`--history` or use a backend with a history span"
        )));
    }
    let earliest = now.date_naive() - chrono::Days::new(u64::from(history));
    if window.start < earliest {
        return Err(Error::Usage(format!(
            "provider {primary} can answer for {history} days back at most; the requested window starts {} (earliest {earliest})",
            window.start
        )));
    }
    Ok(())
}

/// Prints `cirrocast completion <shell>`.
fn run_completion(args: &CompletionArgs) {
    use clap::CommandFactory as _;

    let mut command = Cli::command();
    clap_complete::generate(args.shell, &mut command, &args.bin_name, &mut StdoutSink);
}

/// Prints `cirrocast man`.
///
/// The page is assembled one section at a time rather than through a single
/// [`clap_mangen::Man::render`] call, so the man page's own sections (EXIT STATUS, ENVIRONMENT,
/// SEE ALSO) can sit between EXTRA and VERSION where a `man(1)` reader looks for them. Every
/// section call repeats `clap_mangen`'s two-line roff preamble, and [`man_section`] drops all but
/// the first, so the page carries it exactly once.
fn run_man(args: &ManArgs) -> Result<()> {
    use clap::CommandFactory as _;

    let command = Cli::command()
        .bin_name(args.bin_name.clone())
        .after_long_help(man_extra_help());
    let man = clap_mangen::Man::new(command).title(args.bin_name.clone());
    let mut sink = StdoutSink;
    sink.write_all(MAN_PREAMBLE)
        .map_err(|error| man_error(&error))?;
    man_section(&mut sink, |buffer| man.render_title(buffer))?;
    man_section(&mut sink, |buffer| man.render_name_section(buffer))?;
    man_section(&mut sink, |buffer| man.render_synopsis_section(buffer))?;
    man_section(&mut sink, |buffer| man.render_description_section(buffer))?;
    man_section(&mut sink, |buffer| man.render_options_section(buffer))?;
    man_section(&mut sink, |buffer| man.render_subcommands_section(buffer))?;
    man_section(&mut sink, |buffer| man.render_extra_section(buffer))?;
    man_section(&mut sink, man_extras)?;
    man_section(&mut sink, |buffer| man.render_version_section(buffer))?;
    Ok(())
}

/// The roff preamble `clap_mangen` writes before every section it renders; the page carries the
/// first copy and [`man_section`] drops the rest.
const MAN_PREAMBLE: &[u8] = b".ie \\n(.g .ds Aq \\(aq\n.el .ds Aq '\n";

/// Renders one man page section, stripping the repeated roff preamble.
fn man_section(
    sink: &mut StdoutSink,
    render: impl FnOnce(&mut Vec<u8>) -> std::io::Result<()>,
) -> Result<()> {
    let mut buffer = Vec::new();
    render(&mut buffer).map_err(|error| man_error(&error))?;
    let section = buffer.strip_prefix(MAN_PREAMBLE).unwrap_or(&buffer);
    sink.write_all(section).map_err(|error| man_error(&error))
}

/// The one error rendering the man page can produce.
fn man_error(error: &std::io::Error) -> Error {
    Error::Other(format!("cannot render the man page: {error}"))
}

/// The man page's EXTRA section: the `--help` epilog without its exit-code table, which the page
/// prints as its own EXIT STATUS section.
fn man_extra_help() -> String {
    format!("{HELP_PRECEDENCE}\n\n{HELP_TOKENS}")
}

/// The man page's own sections: EXIT STATUS, ENVIRONMENT and SEE ALSO.
fn man_extras(buffer: &mut Vec<u8>) -> std::io::Result<()> {
    use clap_mangen::roff::Roff;

    let mut roff = Roff::default();
    roff.control("SH", ["EXIT STATUS"]);
    for line in EXIT_CODES.lines() {
        let line = line.trim_start();
        let (code, description) = match line.split_once(' ') {
            Some((code, rest)) => (code, rest.trim_start()),
            None => (line, ""),
        };
        man_entry(&mut roff, code, description);
    }
    roff.control("SH", ["ENVIRONMENT"]);
    for (name, description) in MAN_ENVIRONMENT {
        man_entry(&mut roff, name, description);
    }
    roff.control("SH", ["SEE ALSO"]);
    for (name, description) in MAN_SEE_ALSO {
        man_entry(&mut roff, name, description);
    }
    roff.to_writer(buffer)
}

/// One `.TP` entry: a bold term and the filled description under it.
fn man_entry(roff: &mut clap_mangen::roff::Roff, term: &str, description: &str) {
    use clap_mangen::roff::{bold, roman};

    roff.control("TP", []);
    roff.text([bold(term)]);
    roff.text([roman(description)]);
}

/// The man page's ENVIRONMENT entries, `(term, description)`.
const MAN_ENVIRONMENT: [(&str, &str); 7] = [
    (
        "Flag overrides",
        "CIRROCAST_PROVIDER, CIRROCAST_FORMAT, CIRROCAST_UNITS, CIRROCAST_DAYS, CIRROCAST_LANG, \
         CIRROCAST_LOCATION and CIRROCAST_TIMEOUT override the matching flag; a flag wins over the \
         variable, and the variable over the configuration file.",
    ),
    (
        "Variable-only settings",
        "CIRROCAST_LOCATION_PICK, CIRROCAST_NOMINATIM_URL, CIRROCAST_IP_SERVICE, \
         CIRROCAST_GEO_SEARCH, CIRROCAST_GEO_REVERSE, CIRROCAST_GEONAMES_USER, \
         CIRROCAST_NORMALS_PERIOD and CIRROCAST_NORMALS_MAX_DISTANCE_KM have no flag; the \
         configuration reference names the key each one overrides.",
    ),
    (
        "Provider credentials",
        "CIRROCAST_<ID>_KEY holds a provider's API key, as an alternative to `key set <id>` and \
         keys.toml.",
    ),
    (
        "Colour and terminal",
        "NO_COLOR, CLICOLOR_FORCE, TERM and COLORTERM drive colour and terminal-capability \
         detection; NO_COLOR is honoured unless CLICOLOR_FORCE asks for colour.",
    ),
    (
        "Directories",
        "XDG_CONFIG_HOME, XDG_CONFIG_DIRS, XDG_CACHE_HOME and XDG_DATA_HOME name the directory \
         roots; HOME is the base when an XDG variable is unset.",
    ),
    (
        "Proxy",
        "HTTPS_PROXY, HTTP_PROXY, ALL_PROXY and NO_PROXY (and the lower-case spellings) name the \
         proxy, unless [network] proxy is set; a SOCKS URL is refused.",
    ),
    (
        "Editor",
        "VISUAL, then EDITOR, is the editor `config edit` starts.",
    ),
];

/// The man page's SEE ALSO entries, `(term, description)`.
const MAN_SEE_ALSO: [(&str, &str); 4] = [
    (
        "cirrocast(1)",
        "This manual page, generated from the same definitions as `cirrocast --help`, which stays \
         the canonical flag reference.",
    ),
    (
        "https://github.com/YangtseSu/cirrocast",
        "Home page, release archives and the issue tracker.",
    ),
    (
        "docs/ in the source tree",
        "getting-started, configuration, providers, formats, location, i18n, troubleshooting, \
         architecture, performance, ecosystem and schema: one document per audience.",
    ),
    ("wttr.in", "The output-layout reference, not a data source."),
];

/// Runs the weather query: resolve a location, fetch a report, render it.
///
/// The order matters for what a user sees when something fails: the provider list and the format
/// are checked before any network request, so a typo in `--provider` costs no traffic, and the
/// location is resolved before the forecast because every backend needs it.
fn run_query(query: &QueryArgs, cli: &Cli, sources: Sources) -> Result<u8> {
    let paths = Paths::resolve()?;
    let config = Config::load(&paths)?;
    let settings = Settings::resolve(
        &config,
        &crate::config::CliOverrides {
            provider: query.provider.clone(),
            format: query.format.clone(),
            units: query.units.map(|units| units.to_string()),
            days: query.days,
            lang: query.lang.clone(),
            aqi_index: query.aqi_index.map(|index| index.to_string()),
            location: location_arg(query),
            timeout_secs: query.timeout,
            no_cache: query.cache.no_cache,
            refresh: query.cache.refresh,
            offline: query.cache.offline,
        },
    )?;
    validate_query(query, sources, &settings)?;
    let offline = offline_policy(query.cache, &config)?;

    let ids = provider_chain(&settings, query.station.as_deref(), sources.provider)?;
    // A station is the location: `--station` when it is given, else `[providers.metar] station`
    // when the run is answered by `metar` and nothing else named a location.
    let station = query.station.clone().or_else(|| {
        (settings.location.is_none() && !query.ip)
            .then(|| configured_station(&config, &ids))
            .flatten()
    });
    let days_explicit = matches!(sources.days, Source::CommandLine | Source::Environment);
    let window_requested = query.date.is_some() || query.history.is_some();
    // The chain built here answers "does *any* reachable backend support this flag"; the fetch
    // itself re-ranks `auto` per location (step 24), so the days clamp and the chain warning moved
    // into the slot.
    validate_window(query, &ids)?;
    if cli.verbose > 0 {
        sources.note(&settings);
    }
    let setup = RenderSetup::resolve(query, &config, &settings, cli.verbose, cli.quiet)?;
    validate_surfaces(query, setup.format)?;
    validate_alert_sources(query)?;
    if cli.verbose > 0 {
        sources.note(&settings);
        render_notes(&setup);
        if !offline.is_off() {
            eprintln!("offline: {offline} mode");
        }
    }

    let clock: Arc<dyn Clock> = Arc::new(SystemClock);
    let (geo_cache, cache) =
        open_query_caches(&paths, &config, query.cache, offline, &clock, cli.verbose)?;
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
    let keys = KeyStore::new(&paths);

    let geo_request = GeoRequest {
        config: &config,
        paths: &paths,
        http: &http,
        cache: &geo_cache,
        offline,
        prompt: Prompt::Policy,
        online_naming: true,
        limit: QUERY_CANDIDATES,
    };
    let targets = location_targets(query, &settings, &config)?;

    let env = Env {
        http: &http,
        cache: &cache,
        config: &config,
        keys: &keys,
        quiet: cli.quiet,
        verbose: cli.verbose,
    };
    // One clock read for the whole run, so every slot's astro block and every render context agree
    // on "now" — including a multi-location run, where the locations are in different time zones.
    let now: chrono::DateTime<chrono::Utc> = cache.clock().now().into();
    let context = SlotContext {
        query,
        config: &config,
        provider_spec: &settings.provider,
        provider_source: sources.provider,
        station: station.as_deref(),
        days: settings.days,
        days_explicit,
        window: window_requested,
        format: setup.format,
        lang: setup.i18n.lang().tag(),
        env: &env,
        cli,
        now,
    };

    // One location keeps the pre-step-19 shape: a whole-run failure is an `Err` (and the error
    // path `main` has always had), never a placeholder slot.
    if let [target] = targets.as_slice() {
        let location = location_for_run(station.as_deref(), target, &geo_request, cli)?;
        let report = fetch_for_location(&context, &location)?;
        let alert_credits = alerts::credits(&report.alerts, &config.alerts, &setup.i18n);
        output_report(&setup, &report, &alert_credits, now, cli)?;
        return Ok(0);
    }

    let results = location_reports(&context, station.as_deref(), &targets, &geo_request, cli);
    output_slots(&setup, &targets, &results, &config, now, cli)
}

/// Checks `--alerts-from` before any location is resolved.
///
/// A source id is a promise: an unknown or unwired id, or an empty list, is a usage error that
/// must not cost a request. Coverage needs the resolved location and is checked per slot.
fn validate_alert_sources(query: &QueryArgs) -> Result<()> {
    if let Some(ids) = alert_source_ids(query) {
        if ids.is_empty() {
            return Err(Error::Usage(
                "--alerts-from needs at least one source id".to_owned(),
            ));
        }
        alerts::parse_specs(&ids)?;
    }
    Ok(())
}

/// `--date`/`--history` need a chain entry that has an archive, checked before any request.
///
/// The concrete span (whether the *answering* backend covers the window) is checked per location,
/// because "yesterday" is the location's own calendar; this pre-flight only refuses a chain that
/// could never answer an archive request at all.
fn validate_window(query: &QueryArgs, ids: &[ProviderId]) -> Result<()> {
    if query.date.is_none() && query.history.is_none() {
        return Ok(());
    }
    if ids
        .iter()
        .any(|id| id.metadata().history_days > 0 && id.metadata().implemented)
    {
        return Ok(());
    }
    Err(Error::Usage(
        "`--date`/`--history` need a backend with a history span; add `open-meteo` or `open-meteo-archive` to `--provider`"
            .to_owned(),
    ))
}

/// Everything one location's fetch needs beyond the location itself, shared by every slot.
///
/// Deliberately `Sync`: a multi-location run hands it to worker threads, so it holds no renderer
/// and no message catalog (neither is `Sync`) — only the fetch policy, the shared HTTP client and
/// caches, and the run's flags.
struct SlotContext<'a> {
    /// The flags that shape the fetch (`--aqi`, `--moon`, `--severity`, …).
    query: &'a QueryArgs,
    /// The validated configuration.
    config: &'a Config,
    /// The `--provider`/config spec, so each slot ranks its own `auto` for its own location.
    provider_spec: &'a str,
    /// Which tier named that spec, for the `--station` rule.
    provider_source: Source,
    /// The station the run answers for, when one was named or configured.
    station: Option<&'a str>,
    /// Forecast days as the run resolved them, before the per-provider clamp.
    days: u8,
    /// Whether `--days` (or its environment variable) was given, for the clamp warning.
    days_explicit: bool,
    /// Whether the run asked for an absolute window (`--date`/`--history`).
    window: bool,
    /// The selected format, for the format-driven fetches (`--format aqi`/`moon`).
    format: Format,
    /// The language tag the alert sources are fetched for.
    lang: &'a str,
    /// The shared HTTP client, caches and key store.
    env: &'a Env<'a>,
    /// The run's flags.
    cli: &'a Cli,
    /// The run's clock instant.
    now: chrono::DateTime<chrono::Utc>,
}

impl SlotContext<'_> {
    /// The chain one location is fetched with.
    ///
    /// `auto` ranks by coverage for *this* location (step 24), so a US slot can start at `nws`
    /// while a neighbouring country's starts at `open-meteo`; an explicit list is the same
    /// everywhere. A named station prepends `metar` under the same rule as before: a station run
    /// answers from the station's own observation, with the rest of the chain as fallback.
    fn chain(&self, location: &Location) -> Result<Vec<ProviderId>> {
        let spec = self.provider_spec.trim();
        let prepend = self.provider_source == Source::Default || spec.eq_ignore_ascii_case("auto");
        let ids = select_for(spec, Some(location))?;
        let station_head =
            self.station.is_some() && prepend && ids.first() != Some(&ProviderId::Metar);
        let chain: Vec<ProviderId> = if station_head {
            let mut chain = vec![ProviderId::Metar];
            chain.extend(ids.into_iter().filter(|id| *id != ProviderId::Metar));
            chain
        } else {
            ids
        };
        if self.cli.verbose > 0 && spec.eq_ignore_ascii_case("auto") {
            eprintln!(
                "provider: auto for {} ({:.2}, {:.2}{}): {}",
                location.name,
                location.lat,
                location.lon,
                location
                    .country_code
                    .as_deref()
                    .map_or_else(String::new, |code| format!(", {code}")),
                chain
                    .iter()
                    .map(ProviderId::as_str)
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        Ok(chain)
    }
}

/// Resolves every location of a multi-location run serially, then fetches them in parallel.
///
/// The resolution is a pre-pass in argument order, before any request: `--pick` then reads stdin in
/// the order the user typed the locations, whatever order the workers would have reached the
/// prompt in, so an identical command line produces identical stdout. A slot whose resolution
/// failed carries its error and is never fetched.
fn location_reports(
    context: &SlotContext<'_>,
    station: Option<&str>,
    targets: &[LocationTarget],
    geo: &GeoRequest<'_>,
    cli: &Cli,
) -> Vec<Result<crate::model::Report>> {
    let resolved = resolve_all(targets, |target| {
        location_for_run(station, target, geo, cli)
    });
    let slots: Vec<&Location> = resolved
        .iter()
        .filter_map(|slot| slot.as_ref().ok())
        .collect();
    let fetched = crate::fetch_reports(&slots, |_, location| fetch_for_location(context, location));
    drop(slots);
    let mut fetched = fetched.into_iter();
    resolved
        .into_iter()
        .map(|slot| match slot {
            Ok(_) => match fetched.next() {
                Some(report) => report,
                // Exactly one fetched report per resolvable slot, in the same order, so this arm
                // is unreachable; it keeps the merge panic-free by construction.
                None => Err(Error::Other(INTERNAL_SLOT.to_owned())),
            },
            Err(error) => Err(error),
        })
        .collect()
}

/// The internal error [`location_reports`]'s merge cannot produce, for a slot that was resolved
/// but somehow has no fetched report.
const INTERNAL_SLOT: &str = "internal: a location slot was resolved but not fetched";

/// The serial resolution pre-pass of [`location_reports`]: one result per target, in argument
/// order.
///
/// It is separate so the ordering contract is unit-testable without a network — the resolver is the
/// only part that reaches outside the process.
fn resolve_all(
    targets: &[LocationTarget],
    resolve: impl Fn(&LocationTarget) -> Result<Location>,
) -> Vec<Result<Location>> {
    targets.iter().map(resolve).collect()
}

/// Fetches one already-resolved location end to end: provider chain, then the alert, air and astro
/// panels the run asked for.
fn fetch_for_location(
    context: &SlotContext<'_>,
    location: &Location,
) -> Result<crate::model::Report> {
    // The alert policy is resolved before the forecast is fetched: a coverage or source-list
    // mistake is a usage error that must not cost a request, and `--alerts-from nws` at a Beijing
    // point fails here, not after the weather round trip.
    let ids = context.chain(location)?;
    let (days, warning) = request_days(context.days, &ids, context.days_explicit, context.window)?;
    if let Some(warning) = warning
        && !context.cli.quiet
    {
        eprintln!("{warning}");
    }
    let alert_request = alert_request(
        context.query,
        context.config,
        location,
        &ids,
        context.format,
        context.cli.verbose,
    )?;
    let request = match request_window(context.query, location, context.now) {
        Some(window) => {
            check_window(window, &ids, context.now)?;
            FetchRequest::for_window(window, HourlyResolution::Hourly)
        }
        None => FetchRequest::new(days, HourlyResolution::Hourly),
    };
    let mut report = fetch_chain(&ids, location, &request, context.env)?;

    if context.cli.verbose > 0 {
        verbose_report(&report);
    }

    // Alerts are a separate source registry, so they are fetched after the weather answer: a
    // forecast failure is then reported without any alert traffic. The alerts the *answering
    // backend* carried in its own payload (`visualcrossing`) enter the same layer here, which is
    // the only way they can be filtered, de-duplicated and ordered like every other source's.
    if let Some(alert_request) = alert_request {
        report.alerts = alerts::fetch(
            location,
            context.env,
            &alert_request,
            context.lang,
            std::mem::take(&mut report.alerts),
        )?;
    }

    // Air quality is best-effort by contract: `--aqi` (or `--format aqi`) asks for it, and a
    // failure is a warning — the weather output the user asked for is already in hand and the exit
    // code stays 0. The reading travels on the report, where the renderers find it.
    if context.query.aqi || context.format == Format::Aqi {
        attach_air(&mut report, context.env, context.cli.quiet);
    }

    // The marine block is best-effort in the same way, and the flag is the only way to ask for it:
    // the marine API is a supplementary source, never a chain entry.
    if context.query.marine {
        attach_marine(&mut report, context.env, context.cli.quiet);
    }

    // The climate comparison is best-effort too, and the one extra surface three things can ask
    // for: `--normals`, `[defaults] normals`, or `--format normals` (whose renderer would
    // otherwise always print `unavailable`).
    if context.query.normals || context.config.defaults.normals || context.format == Format::Normals
    {
        attach_normals(&mut report, context.env, context.now, context.cli.quiet);
    }

    // The astro block is attached only when the run asks for it (see `attach_astro`).
    if context.query.moon || context.format == Format::Moon {
        attach_astro(&mut report, context.now, context.cli.verbose);
    }
    Ok(report)
}

/// Renders a multi-location run and returns the process exit code: `0`, or the largest mapped code
/// among the failed slots.
///
/// A failed slot keeps its place: its full error goes to stderr, a one-line `error: …` placeholder
/// takes its slot on stdout (the renderer decides the exact shape; `json` uses an error document),
/// and every other location is printed as usual.
fn output_slots(
    setup: &RenderSetup,
    targets: &[LocationTarget],
    results: &[Result<crate::model::Report>],
    config: &Config,
    now: chrono::DateTime<chrono::Utc>,
    cli: &Cli,
) -> Result<u8> {
    // Errors first, in argument order, so a user reading stderr sees them before the document.
    for result in results {
        if let Err(error) = result {
            eprintln!("error: {error}");
        }
    }

    // Every slot's credits are resolved before any context borrows them: `RenderContext` keeps a
    // slice of credit lines for the whole render.
    let mut credits: Vec<Vec<String>> = Vec::with_capacity(results.len());
    for result in results {
        match result {
            Ok(report) => {
                credits.push(alerts::credits(&report.alerts, &config.alerts, &setup.i18n));
            }
            Err(_) => credits.push(Vec::new()),
        }
    }

    let mut contexts: Vec<Option<crate::render::RenderContext<'_>>> =
        Vec::with_capacity(results.len());
    for (index, result) in results.iter().enumerate() {
        match result {
            Ok(report) => contexts.push(Some(RenderContext {
                units: setup.units,
                color: setup.color,
                width: setup.width.columns,
                term: setup.term,
                times: LocalTimes::new(now, report.location.tz),
                lang: setup.i18n.lang(),
                i18n: &setup.i18n,
                alert_credits: &credits[index],
                aqi_index: setup.aqi_index,
            })),
            Err(_) => contexts.push(None),
        }
    }

    // `one-line` is one line by contract, so the credits the licences require cannot travel in the
    // output: they go to stderr, per location, exactly as the single-location path does.
    for (index, result) in results.iter().enumerate() {
        if let Ok(report) = result {
            credit_to_stderr(setup, report, &credits[index]);
        }
    }

    let slots: Vec<crate::render::Slot<'_>> = targets
        .iter()
        .enumerate()
        .map(|(index, target)| crate::render::Slot {
            query: &target.text,
            report: results.get(index).and_then(|result| result.as_ref().ok()),
            error: results.get(index).and_then(|result| result.as_ref().err()),
            ctx: contexts.get(index).cloned().flatten(),
        })
        .collect();

    if let Some(note) = setup.renderer.slot_note(slots.len())
        && !cli.quiet
    {
        eprintln!("{note}");
    }
    print_line(format_args!("{}", setup.renderer.render_slots(&slots)?))?;
    if cli.verbose > 0 {
        report_missing_keys(setup);
    }
    Ok(crate::worst_exit_code(results))
}

/// The flags whose effect depends on the format, checked before any traffic.
///
/// `--aqi-index` shapes the air panel, so it needs a run that fetches one; `--moon` appends the
/// block to the formats that have one, while `one-line` has the `%m`/`%M` tokens and the
/// alert/AQI listings show their own panel. A flag that would draw nothing is a usage error
/// rather than a silent no-op.
fn validate_surfaces(query: &QueryArgs, format: Format) -> Result<()> {
    if query.aqi_index.is_some() && !query.aqi && format != Format::Aqi {
        return Err(Error::Usage(
            "--aqi-index needs --aqi or `--format aqi`; it shapes the air-quality panel".to_owned(),
        ));
    }
    if query.moon && matches!(format, Format::OneLine | Format::Alerts | Format::Aqi) {
        return Err(Error::Usage(match format {
            Format::OneLine => {
                "`--format one-line` shows the moon through the `%m`/`%M` tokens; `--moon` appends \
                 the block to `art-table`, `dumb`, `plain` and `json`"
                    .to_owned()
            }
            format => format!(
                "`--moon` appends the block to `art-table`, `dumb`, `plain`, `json` and `moon`; \
                 `--format {}` has no astro surface",
                format.as_str()
            ),
        }));
    }
    Ok(())
}

/// Attaches the locally computed astro block when the run asked for it.
///
/// It costs no request, but it is still attached only when asked for, so the renderers stay dumb
/// about display settings: `--moon`/`--format moon` here, `Report::astro` there. `now` is the
/// run's clock, read once, so `ctx.now` and `astro.computed_at` are the same instant.
fn attach_astro(
    report: &mut crate::model::Report,
    now: chrono::DateTime<chrono::Utc>,
    verbose: u8,
) {
    let astro = crate::astro::Astro::compute(
        report,
        now.with_timezone(&report.location.tz).fixed_offset(),
    );
    if verbose > 0 && astro.sun.source == crate::model::SunSource::Local {
        eprintln!("sun: computed locally (provider sends none)");
    }
    report.astro = Some(astro);
}

/// Renders and prints the report: the context `main` builds once, the credits `one-line` cannot
/// carry, and the `-v` report of catalog misses.
fn output_report(
    setup: &RenderSetup,
    report: &crate::model::Report,
    alert_credits: &[String],
    now: chrono::DateTime<chrono::Utc>,
    cli: &Cli,
) -> Result<()> {
    let ctx = RenderContext {
        units: setup.units,
        color: setup.color,
        width: setup.width.columns,
        term: setup.term,
        times: LocalTimes::new(now, report.location.tz),
        lang: setup.i18n.lang(),
        i18n: &setup.i18n,
        alert_credits,
        aqi_index: setup.aqi_index,
    };
    // `one-line` is one line by contract, so the credits the licences require cannot travel in the
    // output: they go to stderr, where `plain` (a document) and `json` (an envelope) keep theirs.
    credit_to_stderr(setup, report, alert_credits);
    print_line(format_args!("{}", setup.renderer.render(report, &ctx)?))?;
    // A message a catalog lacks is a bug in this crate, not a user error: it renders its key and is
    // reported here, after the render — the only point at which every key a renderer will ask for
    // has been asked for — where `-v` asked for exactly this kind of detail.
    if cli.verbose > 0 {
        report_missing_keys(setup);
    }
    Ok(())
}

/// Prints the `one-line` credits to stderr; other formats carry them in the document itself.
fn credit_to_stderr(setup: &RenderSetup, report: &crate::model::Report, alert_credits: &[String]) {
    if setup.format != Format::OneLine {
        return;
    }
    if let Some(credit) = attribution_line(&report.location) {
        eprintln!("{credit}");
    }
    if let Some(licence) = licence_line(&report.attribution.provider) {
        eprintln!(
            "{} {licence}",
            setup.i18n.text(&crate::i18n::keys::LABEL_DATA)
        );
    }
    for credit in alert_credits {
        eprintln!("{credit}");
    }
}

/// Fetches the air-quality reading and attaches it to the report; a failure is a warning.
///
/// Split out of [`run_query`] so the failure policy reads as one sentence: `--aqi` promises a
/// panel, not a successful second API call, and a broken air source must never change the exit
/// code of a weather run that already succeeded.
fn attach_air(report: &mut crate::model::Report, env: &Env<'_>, quiet: bool) {
    match crate::air::fetch(&report.location, env) {
        Ok(air) => report.air = Some(air),
        Err(error) => {
            if !quiet {
                eprintln!("warning: air quality unavailable: {error}");
            }
        }
    }
}

/// Fetches the marine reading and attaches it to the report; a failure is a warning.
///
/// The marine API is a supplementary source (never a chain entry), and `--marine` promises a
/// panel, not a successful second call: an inland point whose sea cell cannot be resolved still
/// shows the forecast the user asked for, with exit code 0.
fn attach_marine(report: &mut crate::model::Report, env: &Env<'_>, quiet: bool) {
    match crate::provider::open_meteo_marine::fetch(&report.location, env) {
        Ok(marine) => report.marine = Some(marine),
        Err(error) => {
            if !quiet {
                eprintln!("warning: marine data unavailable: {error}");
            }
        }
    }
}

/// Fetches the climate-normal comparison and attaches it to the report; a failure is a warning.
///
/// The decoder answers `Ok(None)` for the conditions that make a normal impossible — no station
/// inside the configured radius, a record thinner than twenty years, a month the rows do not cover
/// — and prints the reason on the `-v` stream, so the common outcome never even reaches this
/// warning. A real transport or decode failure is one, like the marine block's, because
/// `--normals` promises a comparison, not a successful second call.
fn attach_normals(
    report: &mut crate::model::Report,
    env: &Env<'_>,
    now: chrono::DateTime<chrono::Utc>,
    quiet: bool,
) {
    let month = normals_month(report, now);
    match crate::normals::fetch(&report.location, month, env) {
        Ok(normals) => report.normals = normals,
        Err(error) => {
            if !quiet {
                eprintln!("warning: climate normals unavailable: {error}");
            }
        }
    }
}

/// The calendar month a report's comparison is for.
///
/// The first forecast day's own month, so a `--history` run normalises the month it actually
/// renders rather than the month it runs in; a report without days (an observation-only backend)
/// falls back to the run clock at the location. The month is `1..=12` by construction, and a
/// failed conversion keeps the January default rather than panicking.
fn normals_month(report: &crate::model::Report, now: chrono::DateTime<chrono::Utc>) -> u8 {
    use chrono::Datelike as _;
    let month = match report.days.first() {
        Some(day) => day.date.month(),
        None => now.with_timezone(&report.location.tz).date_naive().month(),
    };
    u8::try_from(month).unwrap_or(1)
}

/// The `--alerts-from` ids, split on commas and trimmed; `None` when the flag was not given.
///
/// `Some(vec![])` means the flag was given with nothing usable in it, which the callers refuse: an
/// empty source list is a request with nothing to honour.
fn alert_source_ids(query: &QueryArgs) -> Option<Vec<String>> {
    query.alerts_from.as_deref().map(|spec| {
        spec.split(',')
            .map(str::trim)
            .filter(|entry| !entry.is_empty())
            .map(str::to_owned)
            .collect()
    })
}

/// The alert policy of this run, or `None` when it fetches no alerts.
///
/// Precedence, from `--help`: `--no-alerts` wins over everything; `--alerts`, `--alerts-from` and
/// `--format alerts` force a fetch; without any of those, `[alerts] enabled` decides. A forced run
/// whose source set is empty is a usage error — the user asked for warnings and there is nothing
/// to query — while the automatic path skips with a `--verbose` line (which cannot happen while a
/// global aggregator is available, but is the honest answer for a build that disables one).
fn alert_request(
    query: &QueryArgs,
    config: &Config,
    location: &Location,
    ids: &[ProviderId],
    format: Format,
    verbose: u8,
) -> Result<Option<AlertsRequest>> {
    if query.no_alerts {
        if query.alerts_from.is_some() {
            return Err(Error::Usage(
                "--no-alerts cannot be combined with --alerts-from".to_owned(),
            ));
        }
        if query.severity.is_some() {
            return Err(Error::Usage(
                "--no-alerts cannot be combined with --severity".to_owned(),
            ));
        }
        if format == Format::Alerts {
            return Err(Error::Usage(
                "--no-alerts cannot be combined with `--format alerts`".to_owned(),
            ));
        }
        return Ok(None);
    }

    let named = alert_source_ids(query);
    let forced = query.alerts || format == Format::Alerts || named.is_some();
    if !forced && !config.alerts.enabled {
        return Ok(None);
    }
    // A source list from `--alerts-from` *or* the configuration is an explicit request: its
    // failure propagates instead of degrading to a `-v` note, so the promise the module documents
    // holds whichever tier named the sources.
    let explicit = named.is_some() || !alerts::is_auto(&config.alerts.sources);
    let sources = match &named {
        Some(specs) if !specs.is_empty() => alerts::explicit_sources(location, specs)?,
        Some(_) => {
            return Err(Error::Usage(
                "--alerts-from needs at least one source id".to_owned(),
            ));
        }
        None => alerts::sources_for(location, ids, &config.alerts)?,
    };
    if sources.is_empty() {
        if forced {
            return Err(Error::Usage(format!(
                "no alert source covers {:.2},{:.2}; name one with --alerts-from",
                location.lat, location.lon
            )));
        }
        if verbose > 0 {
            eprintln!(
                "alerts: no source covers {:.2},{:.2}; skipping",
                location.lat, location.lon
            );
        }
        return Ok(None);
    }
    // `visualcrossing` is the one source whose warnings travel in its *provider's* payload, so an
    // explicit selection needs that provider on the chain — otherwise the promise "these sources
    // answer" cannot be kept and saying nothing would look like "no warnings".
    if sources.contains(&AlertSource::VisualCrossing)
        && !ids.iter().any(|id| id.as_str() == "visualcrossing")
    {
        return Err(Error::Usage(
            "alert source `visualcrossing` travels with its provider's payload; add `--provider visualcrossing`"
                .to_owned(),
        ));
    }
    let threshold = match query.severity {
        Some(severity) => severity,
        None => config
            .alerts
            .severity_threshold
            .parse::<Severity>()
            .map_err(|error| Error::Config(format!("alerts.severity_threshold: {error}")))?,
    };
    Ok(Some(AlertsRequest {
        sources,
        explicit,
        threshold,
    }))
}

/// The alert request of a run with no alert flags: the coverage-selected (or configured explicit)
/// sources under the configured threshold, or `None` when `[alerts] enabled = false`.
///
/// The `status` probe uses this: its template decides whether an alert fetch is worth a request at
/// all (`%A`), and it has no `--alerts`/`--no-alerts`/`--severity` of its own. The query's
/// [`alert_request`] layers those flags on top of the same rules.
pub(crate) fn configured_alert_request(
    location: &Location,
    ids: &[ProviderId],
    config: &Config,
) -> Result<Option<AlertsRequest>> {
    if !config.alerts.enabled {
        return Ok(None);
    }
    let sources = alerts::sources_for(location, ids, &config.alerts)?;
    if sources.is_empty() {
        return Ok(None);
    }
    let threshold = config
        .alerts
        .severity_threshold
        .parse::<Severity>()
        .map_err(|error| Error::Config(format!("alerts.severity_threshold: {error}")))?;
    Ok(Some(AlertsRequest {
        sources,
        explicit: !alerts::is_auto(&config.alerts.sources),
        threshold,
    }))
}

/// Prints the catalog misses the render recorded, one line each, under `-v`.
fn report_missing_keys(setup: &RenderSetup) {
    for note in setup.i18n.notes() {
        if matches!(note, crate::i18n::Note::MissingKey(_)) {
            eprintln!("{}", note.text());
        }
    }
}

/// What `-v` says about the report that came back: its credit and request, the raw upstream text
/// when the provider kept it, and why a day section is missing when there is none.
///
/// A backend with `daily == false` (or one that answered with observations only) still renders:
/// `art-table` falls back to the current-conditions block and the record formats keep their keys,
/// but the user should know why no day section is there.
fn verbose_report(report: &crate::model::Report) {
    // The credit the terms require, plus the exact request that produced the answer, so a bug
    // report can name the upstream call without a packet capture.
    let credit = licence_line(&report.attribution.provider).unwrap_or("no credit line");
    eprintln!("attribution: {credit} ({})", report.attribution.url);
    if let Some(raw) = &report.attribution.raw {
        // The provider puts the raw upstream text here when `-v` asked for it (the METAR report
        // and its TAF); indentation inside a multi-line product is preserved.
        for line in raw.lines() {
            eprintln!("{}: {line}", report.attribution.provider);
        }
    }
    if report.days.is_empty() {
        let capabilities = crate::provider::capabilities_of(&report.attribution.provider);
        if capabilities.is_some_and(|capabilities| !capabilities.daily) {
            eprintln!(
                "note: {} reports observations only; for a forecast use a forecast backend such as `--provider open-meteo`",
                report.attribution.provider
            );
        } else {
            eprintln!(
                "note: {} reports no forecast days; rendering current conditions only",
                report.attribution.provider
            );
        }
    }
}

/// The location this run forecasts for: a station placeholder the provider fills in, or the
/// resolved location argument.
pub(crate) fn location_for_run(
    station: Option<&str>,
    target: &LocationTarget,
    geo: &GeoRequest<'_>,
    cli: &Cli,
) -> Result<Location> {
    match station {
        // The provider resolves the station (table, then cached stationinfo) and replaces this
        // provisional location with the real one before anything is rendered.
        Some(icao) => {
            if cli.verbose > 0 {
                eprintln!("location: station {icao}");
            }
            Ok(crate::provider::metar::placeholder_location(icao))
        }
        None => query_location(target, geo, cli),
    }
}

/// Whether a resolution run may ask which candidate to use.
///
/// The picker reads stdin, so only the query path (where the user typed the command) may reach it:
/// a run that must never block — the `status` probe, which a status bar executes on a timer —
/// takes the ranked winner instead, exactly as a non-terminal query does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Prompt {
    /// Ask per `[location] pick` when stdin and stderr are terminals (`--pick` forces it).
    Policy,
    /// Never ask; take the ranked winner.
    Never,
}

impl Prompt {
    /// Whether the pick policy applies at all.
    const fn allowed(self) -> bool {
        matches!(self, Self::Policy)
    }
}

/// One location argument as this run treats it: the text as typed (for the ambiguity note and the
/// JSON `query` field) and the alias-expanded spec to resolve.
pub(crate) struct LocationTarget {
    /// The argument as typed; empty when nothing was given and the configured default or the IP
    /// lookup applies.
    text: String,
    /// The spec to resolve, with `@name` aliases already expanded.
    spec: LocationSpec,
}

/// The locations this run resolves, in argument order.
///
/// `--lat/--lon` folds into the `@lat,lon` spelling the resolver understands and outranks an
/// environment location; otherwise the positional arguments are taken as typed, and with none the
/// configured default (`settings.location`) or the default/IP spec applies. Alias expansion happens
/// here — the one place that has both the typed text and the `[locations]` table.
fn location_targets(
    query: &QueryArgs,
    settings: &Settings,
    config: &Config,
) -> Result<Vec<LocationTarget>> {
    let default = || LocationTarget {
        text: String::new(),
        spec: LocationSpec::Default,
    };
    if query.ip {
        return Ok(vec![default()]);
    }
    let raw: Vec<String> = if let (Some(lat), Some(lon)) = (query.lat, query.lon) {
        vec![format!("@{lat},{lon}")]
    } else {
        let given: Vec<String> = location_args(query).map(str::to_owned).collect();
        if given.is_empty() {
            settings.location.iter().cloned().collect()
        } else {
            given
        }
    };
    if raw.is_empty() {
        return Ok(vec![default()]);
    }
    raw.into_iter()
        .map(|text| location_target(&text, config))
        .collect()
}

/// One location argument as a target: parsed and with its `@name` aliases expanded.
///
/// The single step [`location_targets`] repeats per argument and the `status` probe runs once for
/// its `--location`, so both agree on what an alias means and on which parse errors are usage
/// errors.
pub(crate) fn location_target(text: &str, config: &Config) -> Result<LocationTarget> {
    let parsed = LocationSpec::parse_arg(Some(text))?;
    let spec = crate::geo::expand_aliases(parsed, &config.locations)?;
    Ok(LocationTarget {
        text: text.to_owned(),
        spec,
    })
}

/// The location a weather query forecasts for, with the commentary a user needs to trust it.
///
/// The ambiguity note (silenced by `-q`, and not printed at all when `geo.prompt` forbids the
/// picker, since its `--pick`/`--yes` advice would then describe flags that run does not have) and
/// the `-v` candidate list go to stderr; stdout carries only the report, so a script piping the
/// query never has to filter prose out of the answer.
fn query_location(target: &LocationTarget, geo: &GeoRequest<'_>, cli: &Cli) -> Result<Location> {
    let Resolved {
        location,
        candidates,
        resolution,
    } = resolve_location(&target.spec, geo, cli)?;
    let query = target.spec.query().unwrap_or_default();
    // The picker replaces the location outright, so it runs only where the candidates *are* the
    // fetch key — a name the user typed. A coordinate's nearby names and an IP answer's are
    // display-only (step 25): those paths pick for themselves when `--pick` asks.
    let picked = target.spec.query().is_some()
        && geo.prompt.allowed()
        && should_pick(&cli.query, geo.config, candidates.len())?;
    let location = if picked {
        let chosen = prompt_location(query, &candidates)?;
        // The echo is a coordinate spec, not the name: a name would re-run the ranking that just
        // produced the ambiguity (step 04's risk note), while `@lat,lon` pins the choice. The
        // contract states it unconditionally — it is the reproducibility affordance for a prompted
        // choice — so `-q` does not suppress it.
        eprintln!(
            "selected: {} — use @{},{} to skip the prompt",
            place(&chosen),
            chosen.lat,
            chosen.lon
        );
        chosen
    } else {
        location
    };
    if !picked
        && geo.prompt.allowed()
        && let Some(text) = target.spec.query()
        && !cli.quiet
    {
        let note = if matches!(target.spec, LocationSpec::Osm(_)) {
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

/// Whether this run asks which candidate to use, resolved in one place.
///
/// A prompt happens only when there is a choice (`candidates >= 2`) and one of: `--pick` was
/// given, or stdin *and* stderr are terminals and `[location] pick = "auto"`. `--yes` and
/// `pick = "never"` take the ranked winner silently; `-q` silences the surrounding notes but never
/// a prompt this policy asked for.
fn should_pick(query: &QueryArgs, config: &Config, candidates: usize) -> Result<bool> {
    if candidates < 2 || query.yes {
        return Ok(false);
    }
    if query.pick {
        return Ok(true);
    }
    Ok(matches!(pick_policy(config)?, PickPolicy::Auto)
        && std::io::stdin().is_terminal()
        && std::io::stderr().is_terminal())
}

/// `[location] pick` with the `CIRROCAST_LOCATION_PICK` override resolved like every other key.
///
/// [`Config::validate`] rejects an invalid value in the document; this parse exists for the
/// environment tier and for a hand-built document, and shares the message with the configuration.
fn pick_policy(config: &Config) -> Result<PickPolicy> {
    let value = std::env::var("CIRROCAST_LOCATION_PICK")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| config.location.pick.clone());
    match value.trim() {
        "auto" => Ok(PickPolicy::Auto),
        "never" => Ok(PickPolicy::Never),
        other => Err(Error::Config(format!(
            "location.pick: `{other}` is not one of {}",
            crate::config::PICK_POLICIES.join(", ")
        ))),
    }
}

/// What `[location] pick` says about ambiguous names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PickPolicy {
    /// Ask on a terminal; take the ranked winner anywhere else.
    Auto,
    /// Always take the ranked winner.
    Never,
}

/// Asks which candidate to use: the list and the prompt go to stderr, the answer comes from stdin.
///
/// A multi-location run asks every question in one serial pre-pass, in argument order, before any
/// fetch ([`location_reports`]), so two questions never share the one input stream and the mapping
/// from answers to slots is deterministic.
fn prompt_location(query: &str, candidates: &[Location]) -> Result<Location> {
    let stdin = std::io::stdin();
    let stderr = std::io::stderr();
    let mut input = stdin.lock();
    let mut output = stderr.lock();
    crate::geo::pick::Picker::new(&mut input, &mut output).choose(query, candidates)
}

/// The positional location arguments that count for this run.
///
/// An all-whitespace argument is "absent": clap accepts it, but it must not override a configured
/// `location.default`, conflict with `--ip`/`--station`/`--lat`, or make `--verbose` print an empty
/// `location:` line. Filtering in one place keeps [`location_arg`], [`location_targets`] and
/// [`validate_query`] agreeing on what "given" means.
fn location_args(query: &QueryArgs) -> impl Iterator<Item = &str> {
    query
        .location
        .iter()
        .map(String::as_str)
        .filter(|arg| !arg.trim().is_empty())
}

/// The location argument this run resolves, for the settings merge.
///
/// `--lat/--lon` is folded into the `@lat,lon` spelling the resolver already understands, and it
/// outranks an environment location by the usual precedence; a single positional is passed through
/// so `--verbose` can report its tier. Several positionals are not a `settings.location` value —
/// each is its own target — and the merge leaves the key unset for them. An all-whitespace argument
/// is treated as absent, so `location.default` from the configuration wins instead.
fn location_arg(query: &QueryArgs) -> Option<String> {
    if let (Some(lat), Some(lon)) = (query.lat, query.lon) {
        return Some(format!("@{lat},{lon}"));
    }
    let mut args = location_args(query);
    match (args.next(), args.next()) {
        (Some(only), None) => Some(only.to_owned()),
        _ => None,
    }
}

/// How many ranked geocoder candidates a weather query asks for; the ambiguity note and the `-v`
/// listing use them, and a later step may expose the number as a flag.
pub(crate) const QUERY_CANDIDATES: u8 = 10;

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
    /// The template a one-line output expands, already resolved and validated.
    template: Option<String>,
    /// The units the report is converted into.
    units: ResolvedUnits,
    /// The language the report is rendered in, and the catalog behind every label.
    i18n: I18n,
    /// The layout width and where it came from.
    width: crate::render::Width,
    /// The colour mode this run uses.
    color: ColorMode,
    /// The AQI scale that drives the air panel's colour and `%q`.
    aqi_index: AqiIndex,
    /// What the terminal supports.
    term: TermCaps,
}

impl RenderSetup {
    /// Resolves the settings for one run.
    ///
    /// `verbose` is what decides whether the resolved template is reported, and `quiet` whether a
    /// language fallback is announced: the renderer itself never sees either flag. The template
    /// gate is here — a `--template`/`--template-file` value or a `[templates]` preset with an
    /// unknown token is an [`Error::Usage`] before any traffic, while the renderer keeps unknown
    /// tokens literal for the compat surface that serves the same table.
    fn resolve(
        query: &QueryArgs,
        config: &Config,
        settings: &Settings,
        verbose: u8,
        quiet: bool,
    ) -> Result<Self> {
        let choice = crate::render::resolve_format(&settings.format, &config.templates)?;
        let format = choice.format;
        let term = TermCaps::detect();

        // At most one source names a template: the format name itself (`full`, `minimal`, a
        // `[templates]` key) or the two flags. Two sources is a usage error rather than a silent
        // precedence rule, because both spellings are explicit.
        let flag_template = match (&query.template, &query.template_file) {
            (Some(spec), _) => Some(spec.clone()),
            (None, Some(path)) => Some(read_template_file(path)?),
            (None, None) => None,
        };
        let template = match (flag_template, choice.template) {
            (Some(_), Some(_)) => {
                return Err(Error::Usage(format!(
                    "`--format {}` already selects a template; drop --template/--template-file or \
                     pick a format without one",
                    settings.format.trim()
                )));
            }
            (Some(spec), None) => Some(crate::template::resolve_template(
                Some(&spec),
                &config.templates,
            )?),
            (None, Some(template)) => Some(template),
            (None, None) if format == Format::OneLine => {
                Some(crate::template::resolve_template(None, &config.templates)?)
            }
            (None, None) => None,
        };
        // A `[templates]` body may itself be `@other`, and `renderer_for` re-resolves the value
        // against an *empty* table; resolving the `@` chain here, against the real one, is what
        // makes `-f <configured-template>` with an `@`-prefixed body work at all.
        let template = match template {
            Some(template) => Some(resolve_template_chain(&template, &config.templates)?),
            None => None,
        };
        if let Some(template) = &template {
            crate::template::validate(template)?;
        }
        let renderer = renderer_for(format, &term, template.as_deref())?;
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
        // The configuration was validated on load, so a value that does not parse here means the
        // document was built by hand; the CLI flag already went through clap's parser.
        let aqi_index = settings
            .aqi_index
            .parse::<AqiIndex>()
            .map_err(|error| Error::Config(format!("air.index: {error}")))?;
        let color = if format == Format::Dumb {
            // `dumb` is uncoloured by design; `render_notes` reports that under `--verbose`, so an
            // explicit `--color always` is explained rather than silently swallowed.
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
            template,
            units,
            i18n,
            width,
            color,
            aqi_index,
            term,
        })
    }
}

/// How many `@name` hops a template may take before the chain is refused as cyclic.
const TEMPLATE_CHAIN_CAP: usize = 8;

/// Resolves the `@name` chain a `[templates]` body may form to a fixed point.
///
/// `resolve_format` hands `renderer_for` the body of a configured template verbatim, and
/// `renderer_for` re-resolves an `@`-prefixed value against an *empty* table; a body that is itself
/// `@other` would fail there with `configured [templates]: none`. Resolving the hops here, against
/// the real table and bounded like the `[locations]` alias chain, keeps the configured map the only
/// lookup table and turns a cycle into a usage error instead of a hang.
fn resolve_template_chain(body: &str, templates: &BTreeMap<String, String>) -> Result<String> {
    let mut current = body.to_owned();
    for _ in 0..=TEMPLATE_CHAIN_CAP {
        // The owned name keeps the borrow of `current` from outliving the lookup below.
        let name = current
            .trim_start()
            .strip_prefix('@')
            .map(|name| name.trim().to_owned());
        match name {
            None => return Ok(current),
            Some(name) => match crate::template::builtin_or_configured(&name, templates) {
                Some(next) => next.clone_into(&mut current),
                // Let the shared resolver produce the "unknown preset" message, namespaces
                // included.
                None => return crate::template::resolve_template(Some(&current), templates),
            },
        }
    }
    Err(Error::Usage(format!(
        "the `[templates]` entry `{body}` follows an `@` chain that does not end within \
         {TEMPLATE_CHAIN_CAP} hops; check for a cycle"
    )))
}

/// Reads a `--template-file` value: a path, or `-` for standard input.
///
/// A read failure is [`Error::Config`] (state on disk, exit 4) and names the path; an empty file is
/// the same usage error an empty `--template` is, because a template that renders nothing is a
/// mistake either way. Exactly one trailing line terminator is dropped: a text file ends with a
/// newline that is not part of the template, so `printf '%l\n'` stays a one-line template while a
/// deliberate blank last line survives.
fn read_template_file(path: &str) -> Result<String> {
    let text = if path == "-" {
        std::io::read_to_string(std::io::stdin()).map_err(|error| {
            Error::Config(format!("--template-file -: cannot read stdin: {error}"))
        })?
    } else {
        std::fs::read_to_string(path)
            .map_err(|error| Error::Config(format!("--template-file {path}: {error}")))?
    };
    let text = trim_one_newline(text);
    if text.trim().is_empty() {
        return Err(Error::Usage(format!(
            "--template-file {}: the template is empty",
            if path == "-" { "stdin" } else { path }
        )));
    }
    Ok(text)
}

/// Drops one trailing line terminator (`\n` or `\r\n`) from a template read from a file or stdin.
fn trim_one_newline(mut text: String) -> String {
    if text.ends_with('\n') {
        text.pop();
        if text.ends_with('\r') {
            text.pop();
        }
    }
    text
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
    if format == Format::OneLine
        && let Some(template) = &setup.template
    {
        eprintln!("template: {template}");
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
        LocationCommand::UpdateData(args) => run_location_update_data(args, cli),
    }
}

/// Resolves one location argument and prints the place it means.
///
/// The winner line goes to stdout by default; `--all` replaces it with the ranked candidate table.
/// Everything that is commentary — the ambiguity note, the source attribution, the IP-lookup
/// disclosure and the `-v` candidate list — goes to stderr, so stdout stays pipeable and a script
/// never has to filter prose.
fn run_location_search(args: &SearchArgs, cli: &Cli) -> Result<()> {
    let paths = Paths::resolve()?;
    let config = Config::load(&paths)?;
    config.validate()?;
    let offline = offline_policy(args.cache, &config)?;

    let clock: Arc<dyn Clock> = Arc::new(SystemClock);
    let cache = Cache::open(
        &paths,
        cache_mode(&config, args.cache, offline, Scope::Geo)?,
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
    let target = search_target(args.query.as_deref(), args.ip, &config)?;
    let spec = exact_spec(target.spec, args.exact)?;
    let geo_request = GeoRequest {
        config: &config,
        paths: &paths,
        http: &http,
        cache: &cache,
        offline,
        prompt: Prompt::Policy,
        online_naming: true,
        limit: args.limit,
    };
    let resolved = resolve_location(&spec, &geo_request, cli)?;
    let Resolved {
        location,
        candidates,
        resolution,
    } = &resolved;

    if args.all {
        for (index, candidate) in candidates.iter().enumerate() {
            print_line(format_args!("{}", candidate_line(index + 1, candidate)))?;
        }
    } else {
        print_line(format_args!("{}", location_line(location)))?;
    }
    if let Some(text) = spec.query()
        && !cli.quiet
        && !args.all
    {
        let note = if matches!(spec, LocationSpec::Osm(_)) {
            osm_ambiguity_note(text, location, *resolution)
        } else {
            ambiguity_note(text, location, *resolution)
        };
        if let Some(note) = note {
            eprintln!("{note}");
        }
    }
    if let Some(attribution) = attribution_line(location) {
        eprintln!("{attribution}");
    }
    if cli.verbose > 0 {
        for (index, candidate) in candidates.iter().enumerate() {
            eprintln!(
                "location: candidate {}/{}: {}",
                index + 1,
                candidates.len(),
                candidate_text(candidate)
            );
        }
    }
    Ok(())
}

/// Runs `cirrocast location update-data`: build a table from a dump and install it (step 18b).
#[cfg(feature = "offline-geo")]
fn run_location_update_data(args: &UpdateDataArgs, cli: &Cli) -> Result<()> {
    if args.offline {
        return Err(Error::Usage(
            "`location update-data` fetches by definition; drop `--offline`".to_owned(),
        ));
    }
    let paths = Paths::resolve()?;
    let config = Config::load(&paths)?;
    config.validate()?;

    let clock: Arc<dyn Clock> = Arc::new(SystemClock);
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
    let source = args
        .from
        .as_deref()
        .map(str::trim)
        .filter(|source| !source.is_empty())
        .map_or_else(
            || {
                let configured = config.geo.update_url.trim();
                if configured.is_empty() {
                    crate::geo::update::OFFICIAL_URL.to_owned()
                } else {
                    configured.to_owned()
                }
            },
            str::to_owned,
        );

    let candidate = crate::geo::update::build_candidate(&source, &http, cli.verbose)?;
    if args.check {
        let comparison = crate::geo::update::compare(&candidate, &paths)?;
        if comparison.same {
            print_line(format_args!(
                "up to date: {} matches {source} (dump {}, {} rows, {} keys)",
                comparison.against, candidate.dump_date, candidate.rows, candidate.keys
            ))?;
            return Ok(());
        }
        return Err(Error::Other(format!(
            "{source} differs from {} in {}; run without `--check` to install it",
            comparison.against,
            comparison.differences.join(", ")
        )));
    }

    let dir = crate::geo::update::install(&candidate, &paths)?;
    print_line(format_args!(
        "installed the city table: dump {}, {} rows, {} keys",
        candidate.dump_date, candidate.rows, candidate.keys
    ))?;
    print_line(format_args!(
        "  from {source} (input sha256 {})",
        candidate.input_sha256
    ))?;
    print_line(format_args!("  into {}", dir.display()))?;
    Ok(())
}

/// A build without the `offline-geo` feature has no table to install.
#[cfg(not(feature = "offline-geo"))]
fn run_location_update_data(_args: &UpdateDataArgs, _cli: &Cli) -> Result<()> {
    Err(Error::Config(
        "this build has no offline city table (the `offline-geo` feature is off); nothing to install"
            .to_owned(),
    ))
}

/// One `--all` row: the rank number and the shared place line.
///
/// The shape is the winner line plus a number and a population, so the two search outputs — and
/// the offline and network sources behind them — cannot drift apart.
fn candidate_line(index: usize, location: &Location) -> String {
    format!("{index:>2}. {}", candidate_text(location))
}

/// The place line `--all` and the `-v` listing share: the location header plus the population
/// that broke ties, when the source reports one.
fn candidate_text(location: &Location) -> String {
    let population = location
        .population
        .map(|population| format!(" (population {population})"))
        .unwrap_or_default();
    format!("{}{population}", location_line(location))
}

/// The spec `cirrocast location search` resolves: `--ip` wins, then an explicit argument, then
/// `location.default` — which is itself a spec, so `:Beijing` or `@39.9,116.4` configured there
/// behaves exactly as it does on the command line.
///
/// `--ip` is not checked against the argument here: whether that is a conflict or the flag simply
/// winning depends on *where* the argument came from, and only [`validate_query`] can see that (a
/// command line argument conflicts, an environment or configured location is overridden). `@name`
/// is expanded against `[locations]` like every other CLI path.
fn search_target(requested: Option<&str>, ip: bool, config: &Config) -> Result<LocationTarget> {
    if ip {
        return Ok(LocationTarget {
            text: String::new(),
            spec: LocationSpec::Default,
        });
    }
    let (text, parsed) = match LocationSpec::parse_arg(requested)? {
        LocationSpec::Default => match configured_location(config) {
            Some(text) => {
                let spec = LocationSpec::parse_arg(Some(&text))?;
                (text, spec)
            }
            None => {
                return Ok(LocationTarget {
                    text: String::new(),
                    spec: LocationSpec::Default,
                });
            }
        },
        spec => (spec.query().unwrap_or_default().to_owned(), spec),
    };
    let spec = crate::geo::expand_aliases(parsed, &config.locations)?;
    Ok(LocationTarget { text, spec })
}

/// Resolves `spec` through the sources this run may use: the winner, how it was chosen and the
/// ranked candidates the picker, the `--all` output and the `-v` listing share.
///
/// Name queries go to the bundled city table first under the default `geo.strategy = "auto"` and
/// fall back to the network geocoder only when it has no hit; `--offline=geo|all` removes the
/// fallback entirely (step 18).
fn resolve_location(spec: &LocationSpec, geo: &GeoRequest<'_>, cli: &Cli) -> Result<Resolved> {
    match spec {
        LocationSpec::Default => {
            let chain = IpLocatorChain::new(
                geo.http,
                geo.cache,
                IpService::chain(&ip_service_setting())?,
                ip_ttl(geo.config, cli.verbose),
            );
            let (location, service) = chain.locate_with_service()?;
            if !cli.quiet {
                eprintln!("ip: located from the public IP via {}", service.label());
            }
            let (location, candidates) = name_ip_answer(location, geo, cli)?;
            Ok(Resolved {
                location,
                candidates,
                resolution: Resolution::Only,
            })
        }
        spec @ (LocationSpec::Fuzzy(_) | LocationSpec::Exact(_)) => name_location(spec, geo, cli),
        spec @ LocationSpec::Osm(_) => {
            if geo.offline.silences(Scope::Geo) {
                return Err(Error::Network(format!(
                    "offline: `~{}` searches ask OpenStreetMap over the network; \
                     use a plain name (the bundled table) or `@lat,lon` instead",
                    spec.query().unwrap_or_default()
                )));
            }
            let nominatim = Nominatim::new(geo.http, geo.cache, nominatim_url(geo.config));
            let hits = nominatim.search(spec.query().unwrap_or_default(), geo.limit)?;
            resolve_candidates(hits, spec, geo.limit)
        }
        spec @ LocationSpec::LatLon(..) => {
            let resolved = resolve_candidates(Vec::new(), spec, geo.limit)?;
            let (location, candidates) = name_coordinate(resolved.location, geo, cli)?;
            Ok(Resolved {
                location,
                candidates,
                resolution: Resolution::Coordinates,
            })
        }
        // `@name` is expanded against `[locations]` before this function is reached; a spec that
        // slips through is a wiring bug, not a user error, and must not be resolved as a name.
        spec @ LocationSpec::Alias(_) => Err(Error::Config(format!(
            "location alias {spec} was not expanded before resolution"
        ))),
    }
}

/// A name query through the bundled table and/or the network geocoder.
fn name_location(spec: &LocationSpec, geo: &GeoRequest<'_>, cli: &Cli) -> Result<Resolved> {
    let query = spec.query().unwrap_or_default();
    let strategy = GeoStrategy::from_config(geo.config)?;
    // The bundled table is skipped by `strategy = "network"` and by a build without the feature.
    let bundled = cfg!(feature = "offline-geo") && !matches!(strategy, GeoStrategy::Network);
    // The geocoder is skipped by `strategy = "bundled"`; whether it may open a socket is the cache
    // mode's decision (`Scope::Geo` is pinned to `CacheMode::Offline` by an offline policy), which
    // is also what keeps a previously cached answer servable offline (step 04).
    let geocoder = !matches!(strategy, GeoStrategy::Bundled);

    if bundled {
        // `local_lookup` returns the rows in the shared ranking order (the `City` rows carry the
        // ascii spellings the ranking uses), so the list is used as it comes: re-ranking the
        // converted `Location`s would drop that spelling and reorder an exonym match.
        if let Some(answer) = local_lookup(geo, query, matches!(spec, LocationSpec::Exact(_)), cli)?
        {
            if cli.verbose > 0 {
                eprintln!("location: {query} resolved from {}", answer.table);
            }
            let hits = answer.hits;
            let resolution = match spec {
                LocationSpec::Exact(_) => Resolution::Exact,
                _ if hits.len() == 1 => Resolution::Only,
                _ => Resolution::Fuzzy {
                    candidates: hits.len(),
                },
            };
            let location = hits
                .first()
                .cloned()
                .ok_or_else(|| offline_not_found(query))?;
            freshness_note(geo, cli, &answer.table, answer.dump_date);
            return Ok(Resolved {
                location,
                candidates: hits,
                resolution,
            });
        }
    }

    if geocoder {
        let setting = geo_search_setting(geo.config);
        let inputs = SearchInputs {
            http: geo.http,
            cache: geo.cache,
            ttl: Duration::from_secs(u64::from(geo.config.cache.geocode_ttl_secs)),
            limit: geo.limit,
            geonames_user: geonames_user(geo)?,
            nominatim_url: nominatim_url(geo.config),
            offline: geo.offline.silences(Scope::Geo),
        };
        let chain = SearchChain::new(&setting, inputs)?;
        if cli.verbose > 0 && bundled {
            let sources: Vec<&str> = chain.sources().iter().map(|source| source.slug()).collect();
            eprintln!(
                "location: {query} not in the bundled city database; asking {}",
                sources.join(", ")
            );
        }
        let report = chain.search(query)?;
        if cli.verbose > 0 {
            for note in &report.notes {
                eprintln!("location: {note}");
            }
        }
        if !report.is_empty() {
            let merged = crate::geo::merge::merge(&report.answered);
            return resolve_candidates(merged, spec, geo.limit);
        }
    }

    Err(offline_not_found(query))
}

/// `[geo] search` with the `CIRROCAST_GEO_SEARCH` override resolved like every other key.
///
/// [`Config::validate`] rejects an invalid value in the document; the chain parses the result, so
/// the environment tier and a hand-built document share the one message.
fn geo_search_setting(config: &Config) -> String {
    std::env::var("CIRROCAST_GEO_SEARCH")
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| config.geo.search.trim().to_owned())
}

/// `[geo] reverse` with the `CIRROCAST_GEO_REVERSE` override resolved like every other key.
fn reverse_policy(geo: &GeoRequest<'_>) -> Result<crate::geo::reverse::Policy> {
    let value = std::env::var("CIRROCAST_GEO_REVERSE")
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| geo.config.geo.reverse.trim().to_owned());
    crate::geo::reverse::Policy::parse(&value)
}

/// Names a coordinate from the bundled tables, else Nominatim (step 25).
///
/// The name is a display attribute: the location keeps its own coordinates, provenance and
/// provisional zone, and only `name`/`admin1`/`country`/`country_code` plus
/// [`Location::named_by`] are filled in. Returns the ranked candidates (nearest first) so `--all`
/// and `-v` can show the alternatives.
///
/// Only `--pick` asks which candidate to use. The automatic picker policy stays out of this one:
/// the choice cannot change what is fetched — the coordinate is the request key — so an automatic
/// prompt would ask the user to choose something that changes nothing but the label, on every
/// terminal run whose coordinate happens to have several neighbours.
fn name_coordinate(
    location: Location,
    geo: &GeoRequest<'_>,
    cli: &Cli,
) -> Result<(Location, Vec<Location>)> {
    let mut policy = reverse_policy(geo)?;
    if !geo.online_naming && policy == crate::geo::reverse::Policy::Auto {
        policy = crate::geo::reverse::Policy::Offline;
    }
    let inputs = crate::geo::reverse::Inputs {
        policy,
        paths: geo.paths,
        data: &geo.config.geo.data,
        http: geo.http,
        cache: geo.cache,
        nominatim_url: &nominatim_url(geo.config),
        ttl: Duration::from_secs(u64::from(geo.config.cache.geocode_ttl_secs)),
        limit: geo.limit,
        offline: geo.offline.silences(Scope::Geo),
        quiet: cli.quiet,
    };
    let named = crate::geo::reverse::name(location.lat, location.lon, &inputs)?;
    if named.nearby.is_empty() {
        if cli.verbose > 0
            && let Some(note) = &named.note
        {
            eprintln!("location: {note}");
        }
        return Ok((location, Vec::new()));
    }

    let candidates: Vec<Location> = named
        .nearby
        .iter()
        .map(|near| near.location.clone())
        .collect();
    let chosen = if cli.query.pick && candidates.len() >= 2 {
        prompt_location(&location.name, &candidates)?
    } else {
        candidates[0].clone()
    };
    let distance = named
        .nearby
        .iter()
        .find(|near| near.location == chosen)
        .map(|near| near.distance_km);
    let mut named_location = location;
    named_location.name = chosen.name;
    named_location.admin1 = chosen.admin1.or(named_location.admin1);
    if !chosen.country.is_empty() {
        named_location.country = chosen.country;
    }
    named_location.country_code = chosen.country_code.or(named_location.country_code);
    named_location.named_by = Some(chosen.source);
    if cli.verbose > 0 {
        let distance = distance.map_or_else(String::new, |km| format!(", {km:.1} km away"));
        eprintln!(
            "location: named by {}{distance}: {}",
            naming_source(chosen.source),
            place(&named_location)
        );
    }
    Ok((named_location, candidates))
}

/// Names an IP answer whose service reported no city (step 25).
///
/// An answer that carries a city is already named; one that does not gets exactly the treatment a
/// typed coordinate gets — the bundled tables, else Nominatim — and when nothing names it the pair
/// itself, so the header is never blank. The location keeps its coordinates, its `Ip` provenance
/// and the zone the service reported; only the display fields and `named_by` change.
fn name_ip_answer(
    location: Location,
    geo: &GeoRequest<'_>,
    cli: &Cli,
) -> Result<(Location, Vec<Location>)> {
    if !location.name.trim().is_empty() {
        return Ok((location, Vec::new()));
    }
    let (mut named, candidates) = name_coordinate(location, geo, cli)?;
    if named.name.trim().is_empty() {
        named.name = crate::geo::coordinate_name(named.lat, named.lon);
    }
    Ok((named, candidates))
}

/// How a naming source is described in the `-v` line.
fn naming_source(source: crate::model::LocationSource) -> &'static str {
    match source {
        crate::model::LocationSource::Osm => "nominatim",
        _ => "the bundled tables",
    }
}

/// The `GeoNames` account name for this run: the environment variable first, then `keys.toml`
/// (step 25's named credential), or `None` when neither has one.
///
/// Reading the store can fail on its own terms — a group-readable `keys.toml` is refused, not
/// silently skipped — and that failure stops the run, exactly as it does for a provider key.
fn geonames_user(geo: &GeoRequest<'_>) -> Result<Option<String>> {
    let store = crate::config::keys::KeyStore::new(geo.paths);
    store.get(crate::geo::geonames::CREDENTIAL)
}

/// What the local city table answered, when it did.
struct LocalAnswer {
    /// The ranked rows, already in the shared order.
    hits: Vec<Location>,
    /// A phrase naming the table, for the `-v` line and the freshness note.
    table: String,
    /// The table's dump date, for the freshness note.
    dump_date: Option<chrono::NaiveDate>,
}

/// The rows the local city table (`[geo] data`) has for `query`, in the shared ranking order, or
/// `None` when it has no match.
///
/// The rows are ranked as `geo::table::City` values — with their ascii spellings — and only then
/// converted, so the order (and the `--all` table and the ambiguity note built from it) is the one
/// the table decided. The table itself is opened here, so a run that never resolves a name never
/// touches it (and `--version` never decodes anything).
///
/// A build without the `offline-geo` feature has no table at all; the fallback below keeps the
/// call sites free of `#[cfg]`.
#[cfg(feature = "offline-geo")]
fn local_lookup(
    geo: &GeoRequest<'_>,
    query: &str,
    exact: bool,
    cli: &Cli,
) -> Result<Option<LocalAnswer>> {
    let table =
        crate::geo::offline::OfflineTable::open(geo.paths, &geo.config.geo.data, cli.quiet)?;
    let mode = if exact {
        crate::geo::table::MatchMode::Exact
    } else {
        crate::geo::table::MatchMode::Prefix
    };
    let hits: Vec<Location> = table
        .search(query, mode, geo.limit)?
        .into_iter()
        .map(|city| city.location())
        .collect();
    if hits.is_empty() {
        return Ok(None);
    }
    Ok(Some(LocalAnswer {
        hits,
        table: table.describe(),
        dump_date: table.dump_date(),
    }))
}

#[cfg(not(feature = "offline-geo"))]
fn local_lookup(
    _geo: &GeoRequest<'_>,
    _query: &str,
    _exact: bool,
    _cli: &Cli,
) -> Result<Option<LocalAnswer>> {
    Ok(None)
}

/// The once-a-day nudge that a newer dump exists (`[geo] update = "check"`).
///
/// It never fetches: it compares the answering table's dump date with the configured interval and
/// points at `location update-data`, at most once per 24 hours (a state file in the cache dir).
fn freshness_note(
    geo: &GeoRequest<'_>,
    cli: &Cli,
    table: &str,
    dump_date: Option<chrono::NaiveDate>,
) {
    if cli.quiet || geo.config.geo.update != "check" {
        return;
    }
    let Some(dump_date) = dump_date else {
        return;
    };
    let now: chrono::DateTime<chrono::Utc> = geo.cache.clock().now().into();
    let last = crate::geo::update::last_notice(geo.cache);
    let Some(age_days) =
        crate::geo::update::note_due(now, dump_date, geo.config.geo.update_interval_days, last)
    else {
        return;
    };
    crate::geo::update::record_notice(geo.cache, now);
    eprintln!(
        "note: {table} is {age_days} days old; run `cirrocast location update-data` to install a fresh one"
    );
}

/// `[geo] strategy`: how a name query picks its source (step 18).
///
/// The configuration validates the spelling on load, so a parse failure here means the document
/// was built by hand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GeoStrategy {
    /// The bundled table first, the network geocoder on a miss (the default).
    Auto,
    /// The bundled table only.
    Bundled,
    /// The network geocoder only.
    Network,
}

impl GeoStrategy {
    fn from_config(config: &Config) -> Result<Self> {
        match config.geo.strategy.as_str() {
            "auto" => Ok(Self::Auto),
            "bundled" => Ok(Self::Bundled),
            "network" => Ok(Self::Network),
            other => Err(Error::Config(format!(
                "geo.strategy: `{other}` is not auto, bundled or network"
            ))),
        }
    }
}

/// `--exact` on `location search`: the same narrowing as the `:query` spelling.
///
/// A `~` search or an IP lookup has no name to match exactly, so the flag is a usage error there
/// rather than a silent no-op.
fn exact_spec(spec: LocationSpec, exact: bool) -> Result<LocationSpec> {
    if !exact {
        return Ok(spec);
    }
    match spec {
        LocationSpec::Fuzzy(query) | LocationSpec::Exact(query) => Ok(LocationSpec::Exact(query)),
        other => Err(Error::Usage(format!(
            "--exact needs a name query; {other} is not one"
        ))),
    }
}

/// The location-resolution inputs a run shares between the weather query and `location search`.
///
/// Bundled so the resolution helpers keep one parameter instead of six, and so the offline policy
/// and the geo-scoped cache cannot be passed inconsistently.
pub(crate) struct GeoRequest<'a> {
    /// The effective configuration (the strategy, the table source and the geocode TTL).
    pub(crate) config: &'a Config,
    /// The XDG directories, for the user-installed table (step 18b). Unread in a build without
    /// the `offline-geo` feature, where there is no local table to open.
    #[cfg_attr(not(feature = "offline-geo"), allow(dead_code))]
    pub(crate) paths: &'a Paths,
    /// The shared HTTP client.
    pub(crate) http: &'a HttpClient,
    /// The cache view for the geo scope: pinned to `CacheMode::Offline` when the policy silences
    /// it, so a socket can never be opened on this path.
    pub(crate) cache: &'a Cache,
    /// The run's offline policy.
    pub(crate) offline: OfflineMode,
    /// Whether the picker may run (see [`Prompt`]): a run that must not block reads stdin never.
    pub(crate) prompt: Prompt,
    /// Whether naming a coordinate may ask Nominatim when the bundled tables find nothing
    /// (step 25). The `status` probe sets it `false`: its whole contract is one cheap line, so it
    /// names from the bundled tables and never *adds* a request to the run.
    pub(crate) online_naming: bool,
    /// How many candidates to rank.
    pub(crate) limit: u8,
}

/// Opens the two cache views of a weather query: the geo scope and the weather scope, each under
/// its own offline policy.
///
/// An offline policy silences only its own scope (`--offline=geo` still fetches live weather,
/// `--offline=weather` still geocodes), and two `Cache` handles over the same root are what keep
/// that split out of every provider and geocoder call site.
pub(crate) fn open_query_caches(
    paths: &Paths,
    config: &Config,
    flags: CacheFlags,
    offline: OfflineMode,
    clock: &Arc<dyn Clock>,
    verbose: u8,
) -> Result<(Cache, Cache)> {
    let geo = Cache::open(
        paths,
        cache_mode(config, flags, offline, Scope::Geo)?,
        Arc::clone(clock),
        verbose,
    );
    let weather = Cache::open(
        paths,
        cache_mode(config, flags, offline, Scope::Weather)?,
        Arc::clone(clock),
        verbose,
    );
    Ok((geo, weather))
}

/// The offline policy of this run: `--offline` wins over `[network] offline`, and the combination
/// the configuration cannot serve (a silenced weather scope with caching disabled) is refused
/// before any request.
fn offline_policy(flags: CacheFlags, config: &Config) -> Result<OfflineMode> {
    let mode = config.offline_mode(flags.offline)?;
    config.check_offline(mode)?;
    Ok(mode)
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

/// One `cache stat` line per namespace: name, entry count, size, how many of the entries are past
/// their TTL, then the fetch window.
///
/// Fixed column widths (13/9/10) rather than computed ones, because the layout is documented in the
/// step file and in `--help` output: the seven namespaces always fit and a user comparing two runs
/// sees the same columns. The expired count is printed only when it is non-zero, so a healthy cache
/// reads exactly as it always did and `0 expired` never implies "delete me".
fn cache_stat_lines(stat: &CacheStat) -> Vec<String> {
    stat.namespaces
        .iter()
        .map(|namespace| {
            let entries = if namespace.entries == 1 {
                "1 entry".to_owned()
            } else {
                format!("{} entries", namespace.entries)
            };
            let expired = match namespace.expired {
                0 => String::new(),
                1 => "   1 expired".to_owned(),
                expired => format!("   {expired} expired"),
            };
            let window = match (namespace.oldest, namespace.newest) {
                (Some(oldest), Some(newest)) => {
                    format!("   oldest {}   newest {}", rfc3339(oldest), rfc3339(newest))
                }
                _ => String::new(),
            };
            format!(
                "{:<13}{entries:<9}{:>10}{expired}{window}",
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

/// The cache mode one scope of this run uses.
///
/// An offline policy silences its scope by pinning it to [`CacheMode::Offline`] (reads only, and a
/// miss is a hard error naming the missing entry); every other scope follows the
/// `--no-cache`/`--refresh` flags and `[cache] enabled` exactly as before step 18.
fn cache_mode(
    config: &Config,
    flags: CacheFlags,
    offline: OfflineMode,
    scope: Scope,
) -> Result<CacheMode> {
    if offline.silences(scope) {
        return Ok(CacheMode::Offline);
    }
    if !flags.no_cache && !flags.refresh && !config.cache.enabled {
        return Ok(CacheMode::NoCache);
    }
    CacheMode::from_flags(flags.no_cache, flags.refresh, false)
}

/// The configured default location, when there is one.
pub(crate) fn configured_location(config: &Config) -> Option<String> {
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
            // The effective configuration, exactly like `config get`: a `CIRROCAST_*` variable
            // overrides the file, and the two subcommands must not disagree about the same key.
            let config = Config::load(&paths)?.with_env_overrides()?;
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
        ConfigCommand::Validate { offline } => {
            let (config, document) = Config::load_document(&paths)?;
            // Strict key check first: a typo is the most common problem and its message names the
            // key path, not whatever type error the unknown key happens to cause downstream.
            let source = match document {
                Some((path, document)) => {
                    crate::config::check_known_keys(&document)?;
                    Some(path)
                }
                None => None,
            };
            config.validate()?;
            if *offline {
                // The flag asks about the harshest policy: with `--offline` (all) the weather and
                // the names both come from disk, so a disabled cache can never serve it.
                config.check_offline(OfflineMode::All)?;
            }
            for note in config.unit_override_notes()? {
                eprintln!("{note}");
            }
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
            let name = KeyStore::canonical(&args.provider)?;
            if args.jwt {
                let credential = jwt_credential(args)?;
                store.set_jwt(&name, &credential)?;
                print_line(format_args!(
                    "stored {name} JWT credential in {}",
                    store.path().display()
                ))?;
                return Ok(());
            }
            let noun = credential_noun(&name);
            let secret = read_secret(&prompt_for(&name), args.stdin)?;
            store.set(&name, &secret)?;
            print_line(format_args!(
                "stored {name} {noun} in {}",
                store.path().display()
            ))?;
            Ok(())
        }
        KeyCommand::Rm { provider } => {
            let name = KeyStore::canonical(provider)?;
            let removed = store.remove(&name)?;
            if removed.any() {
                let what = if removed.jwt && removed.api_key {
                    "JWT credential and API key"
                } else if removed.jwt {
                    "JWT credential"
                } else {
                    credential_noun(&name)
                };
                print_line(format_args!("removed {name} {what}"))?;
            } else {
                print_line(format_args!("no {name} {} stored", stored_noun(&name)))?;
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
                    credential_column(row),
                    source_column(row)
                ))?;
            }
            Ok(())
        }
    }
}

/// The `key set --jwt` credential: the PEM read from the file or stdin, validated before storage.
fn jwt_credential(args: &KeySetArgs) -> Result<JwtCredential> {
    let path = args
        .key_file
        .as_deref()
        .ok_or_else(|| Error::Usage("--key-file is required with --jwt".to_owned()))?;
    let private_key = read_key_file(path)?;
    let credential = JwtCredential {
        credential_id: args
            .credential_id
            .clone()
            .unwrap_or_default()
            .trim()
            .to_owned(),
        developer_id: args
            .developer_id
            .clone()
            .unwrap_or_default()
            .trim()
            .to_owned(),
        project_id: args
            .project_id
            .clone()
            .unwrap_or_default()
            .trim()
            .to_owned(),
        private_key,
    };
    // Refuse a mistyped identifier or the wrong key file here, where the console step that
    // produced it is still in front of the user, rather than at the first fetch.
    crate::auth::jwt::validate(&credential)?;
    Ok(credential)
}

/// The PEM text `key set --jwt --key-file` reads: a path, or stdin for `-`.
fn read_key_file(path: &str) -> Result<String> {
    if path == "-" {
        let mut pem = String::new();
        std::io::stdin().read_to_string(&mut pem).map_err(|error| {
            Error::Config(format!("cannot read the private key from stdin: {error}"))
        })?;
        return Ok(pem);
    }
    fs::read_to_string(path).map_err(|error| Error::Config(format!("{path}: {error}")))
}

/// The credential column of one `key list` row.
///
/// A single form prints bare, exactly as it always has; when both are stored the API key gains its
/// label so the two cannot be confused.
fn credential_column(row: &KeySummary) -> String {
    let labelled = row.forms.len() > 1;
    let forms: Vec<String> = row
        .forms
        .iter()
        .map(|form| match form {
            KeyForm::ApiKey { masked, .. } if labelled => format!("api key {masked}"),
            KeyForm::ApiKey { masked, .. } => masked.clone(),
            KeyForm::QWeatherJwt { ids, .. } => format!(
                "jwt (kid {}, iss {}, sub {})",
                ids.credential_id, ids.developer_id, ids.project_id
            ),
        })
        .collect();
    forms.join(", ")
}

/// The source column of one `key list` row: one label per distinct place a form came from.
fn source_column(row: &KeySummary) -> String {
    let mut labels: Vec<&str> = Vec::new();
    for form in &row.forms {
        let label = source_label(form.source());
        if !labels.contains(&label) {
            labels.push(label);
        }
    }
    labels.join(", ")
}

/// The `key set` prompt: a provider gets the API-key wording, a named credential its own.
fn prompt_for(name: &str) -> String {
    match KeyStore::named(name) {
        Some(credential) => format!("{} for {name}: ", credential.what),
        None => format!("API key for {name}: "),
    }
}

/// What the `key` subcommands call the thing they store: a provider's API key, or a service's
/// named credential.
fn credential_noun(name: &str) -> &'static str {
    if KeyStore::named(name).is_some() {
        "credential"
    } else {
        "API key"
    }
}

/// What `key rm` calls the thing that is *not* stored.
///
/// A provider with a JWT mode has two forms, so the plural is the only accurate word for it; a
/// single-mode provider keeps the wording it always had.
fn stored_noun(name: &str) -> &'static str {
    if KeyStore::named(name).is_some() {
        "credential"
    } else if name
        .parse::<ProviderId>()
        .is_ok_and(|id| id.jwt_env().is_some())
    {
        "credentials"
    } else {
        "API key"
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

/// The alert row of `provider info`: the alert sources this provider brings with it.
///
/// The registry's alert sources are independent of the weather chain — the global aggregators
/// apply everywhere — so this row names only what follows *this* provider: a backend whose own
/// payload carries warnings, or an alert source bound to its credential and host (`qweather`,
/// `visualcrossing`). A provider with neither prints `none`, and the coverage-selected sources
/// still apply at run time.
fn provider_alerts(meta: &ProviderMeta) -> String {
    let mut names: Vec<&str> = Vec::new();
    if meta.alerts {
        names.push("its own payload");
    }
    for source in AlertSource::ALL {
        if source.provider() == Some(meta.id.as_str()) && source.available() {
            names.push(source.as_str());
        }
    }
    if names.is_empty() {
        "none".to_owned()
    } else {
        names.join(", ")
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

    let (config, document) = Config::load_document(paths)?;
    // A typo introduced in the editor is the most likely problem, and `validate` names the key
    // path; check it here so `edit` and `validate` agree about the same file.
    if let Some((_, document)) = document {
        crate::config::check_known_keys(&document)?;
    }
    config.validate()?;
    print_line(format_args!("ok: {}", paths.config_file.display()))?;
    Ok(())
}

/// Reads a secret from the terminal (with echo off) or from stdin.
///
/// `prompt` is the full question, e.g. `API key for qweather: ` or `GeoNames user name for
/// geonames: `; the secret itself is never echoed, never taken from argv and never logged.
fn read_secret(prompt: &str, force_stdin: bool) -> Result<String> {
    if !force_stdin && std::io::stdin().is_terminal() {
        let secret = rpassword::prompt_password(prompt).map_err(|error| {
            Error::Other(format!("cannot read the secret from the terminal: {error}"))
        })?;
        return checked_secret(&secret);
    }

    let mut input = String::new();
    std::io::stdin()
        .read_to_string(&mut input)
        .map_err(|error| Error::Other(format!("cannot read the secret from stdin: {error}")))?;
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

    let row = |id: &str, name: &str, key: &str, net: &str, obs: &str, fcst: &str, days: &str| {
        format!(
            "{id:<id_width$}  {name:<name_width$}  {key:<key_width$}  {net:<7}  {obs:<3}  {fcst:<3}  {days:>7}"
        )
    };

    let mut lines = vec![row("ID", "NAME", "KEY", "NET", "OBS", "FCST", "MAXDAYS")];
    for meta in &metas {
        let max_days = meta.max_days.to_string();
        lines.push(row(
            meta.id.as_str(),
            meta.display_name,
            key_label(meta),
            meta.network.as_str(),
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
    if meta.id.jwt_env().is_some() {
        lines.push(info_line(
            "store jwt:",
            format!("cirrocast key set {} --jwt", meta.id),
        ));
    }
    lines.extend([
        info_line("current:", yes_no(meta.current)),
        info_line("hourly:", yes_no(meta.hourly)),
        info_line("daily:", yes_no(meta.daily)),
        info_line("alerts:", provider_alerts(meta)),
        info_line("max days:", meta.max_days),
        info_line("locations:", locations),
        info_line("coverage:", meta.coverage),
        info_line("network:", meta.network.as_str()),
        info_line("history days:", meta.history_days),
        info_line("marine:", yes_no(meta.marine)),
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
        Cli, LocationTarget, Source, Sources, alert_request, configured_station, location_arg,
        location_args, provider_chain, request_days, resolve_all, station_chain, trim_one_newline,
        validate_query,
    };
    use crate::config::{CliOverrides, Config, Settings};
    use crate::error::Error;
    use crate::geo::LocationSpec;
    use crate::provider::ProviderId;
    use crate::render::Format;

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
        assert!(!sources.coordinates);
    }

    #[test]
    fn the_coordinate_flags_are_the_location_source() {
        // `--lat/--lon` spell the location themselves, so `-v` must not claim the config supplied
        // it; the argument's own tier stays separate for `validate_query`.
        let (_, sources) = parse(&["cirrocast", "--lat", "39.9", "--lon", "116.4"]);
        assert!(sources.coordinates);
        assert_eq!(sources.location, Source::Default);

        let (_, sources) = parse(&["cirrocast", "Beijing"]);
        assert!(!sources.coordinates);
        assert_eq!(sources.location, Source::CommandLine);
    }

    #[test]
    fn the_days_clamp_is_reported_once_and_silenceable() {
        // Open-Meteo's 16 days are beyond the flag's own 0..=14, so the clamp cannot bite here.
        let (days, warning) = request_days(14, &[ProviderId::OpenMeteo], true, false)
            .expect("open-meteo accepts days");
        assert_eq!(days, 14);
        assert_eq!(warning, None);

        // A station-only backend has no forecast at all: everything clamps to zero, with the
        // observation-specific wording rather than "supports at most 0 days".
        let (days, warning) = request_days(14, &[ProviderId::Metar], true, false)
            .expect("metar clamps instead of refusing");
        assert_eq!(days, 0);
        assert_eq!(
            warning.expect("the clamp is reported"),
            "warning: metar reports observations only; --days is ignored"
        );

        // A days value that came from the configuration or the built-in default is not a request
        // for a forecast, so dropping it is not worth a warning.
        let (days, warning) =
            request_days(3, &[ProviderId::Metar], false, false).expect("no refusal");
        assert_eq!(days, 0);
        assert_eq!(warning, None);

        // The clamp follows the *first* entry: a fallback cannot widen the request.
        let (days, warning) =
            request_days(14, &[ProviderId::Metar, ProviderId::OpenMeteo], true, false)
                .expect("no refusal");
        assert_eq!(days, 0);
        assert!(warning.is_some());

        // No chain at all is not a clamp case.
        assert_eq!(
            request_days(3, &[], true, false).expect("no chain"),
            (3, None)
        );
    }

    #[test]
    fn a_station_chain_contains_metar_or_is_auto() {
        for spec in [
            "auto",
            "AUTO",
            "metar",
            "METAR",
            "metar,open-meteo",
            "open-meteo,metar",
            " metar , smhi ",
        ] {
            assert!(station_chain(spec), "`{spec}` answers a station");
        }
        for spec in ["open-meteo", "", "smhi", "open-meteo,smhi"] {
            assert!(!station_chain(spec), "`{spec}` does not answer a station");
        }
    }

    #[test]
    fn a_station_selects_the_station_backend() {
        // Without a station, `auto` is the keyless place chain and nothing else.
        assert_eq!(
            provider_chain(&settings("auto"), None, Source::Default).expect("auto expands"),
            vec![ProviderId::OpenMeteo, ProviderId::MetNo]
        );

        // `--station` with the provider from the configuration or the built-in default prepends
        // `metar`, keeping that chain as the fallback rather than discarding it.
        assert_eq!(
            provider_chain(&settings("open-meteo"), Some("ZBAA"), Source::Default)
                .expect("a station prepends metar"),
            vec![ProviderId::Metar, ProviderId::OpenMeteo]
        );
        assert_eq!(
            provider_chain(&settings("open-meteo,smhi"), Some("ZBAA"), Source::Default)
                .expect("a station prepends metar"),
            vec![ProviderId::Metar, ProviderId::OpenMeteo, ProviderId::Smhi]
        );
        // `auto` from the configuration expands behind the prepended `metar`, exactly as the
        // command-line `auto` does.
        assert_eq!(
            provider_chain(&settings("auto"), Some("ZBAA"), Source::Default)
                .expect("a configured auto gains metar"),
            vec![ProviderId::Metar, ProviderId::OpenMeteo, ProviderId::MetNo]
        );

        // An explicit `auto` gains `metar` in front, because `auto` never contains a
        // station-only backend on its own.
        assert_eq!(
            provider_chain(&settings("auto"), Some("ZBAA"), Source::CommandLine)
                .expect("auto gains metar"),
            vec![ProviderId::Metar, ProviderId::OpenMeteo, ProviderId::MetNo]
        );

        // An explicit chain is used as written — the user asked for that order.
        assert_eq!(
            provider_chain(
                &settings("metar,open-meteo"),
                Some("ZBAA"),
                Source::CommandLine
            )
            .expect("an explicit chain"),
            vec![ProviderId::Metar, ProviderId::OpenMeteo]
        );
        assert_eq!(
            provider_chain(&settings("open-meteo"), None, Source::Default).expect("a place chain"),
            vec![ProviderId::OpenMeteo]
        );
    }

    #[test]
    fn a_configured_station_only_affects_metar() {
        let mut config = Config::default();
        config.providers.metar.station = "kjfk".to_owned();
        assert_eq!(
            configured_station(&config, &[ProviderId::Metar]).as_deref(),
            Some("KJFK")
        );
        assert_eq!(
            configured_station(&config, &[ProviderId::OpenMeteo, ProviderId::Metar]),
            None,
            "another backend is the default, so the station is not the location"
        );
        config.providers.metar.station = String::new();
        assert_eq!(configured_station(&config, &[ProviderId::Metar]), None);
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
    fn an_all_whitespace_location_argument_is_absent() {
        for args in [vec!["cirrocast", ""], vec!["cirrocast", "   "]] {
            let (cli, sources) = parse(&args);
            assert_eq!(location_arg(&cli.query), None, "{args:?}");
            assert_eq!(location_args(&cli.query).count(), 0, "{args:?}");
            assert_eq!(
                sources.location,
                Source::Default,
                "{args:?} is absent, not a command line value"
            );
        }

        // Absent means the configured default wins and the argument does not conflict with `--ip`.
        let (cli, sources) = parse(&["cirrocast", "", "--ip"]);
        validate_query(&cli.query, sources, &settings("open-meteo"))
            .expect("blank text is absent, so it does not conflict with --ip");
    }

    #[test]
    fn a_multi_location_pre_pass_resolves_in_argument_order() {
        let targets: Vec<LocationTarget> = ["Beijing", "Shanghai", "Guangzhou"]
            .into_iter()
            .map(|text| LocationTarget {
                text: text.to_owned(),
                spec: LocationSpec::Fuzzy(text.to_owned()),
            })
            .collect();
        let seen = std::cell::RefCell::new(Vec::new());
        let results = resolve_all(&targets, |target| {
            seen.borrow_mut().push(target.text.clone());
            if target.text == "Shanghai" {
                return Err(Error::LocationNotFound(
                    "no location found for Shanghai".to_owned(),
                ));
            }
            Ok(crate::provider::metar::placeholder_location("ZBAA"))
        });
        assert_eq!(*seen.borrow(), ["Beijing", "Shanghai", "Guangzhou"]);
        assert!(results[0].is_ok());
        assert!(results[2].is_ok());
        assert_eq!(
            results[1].as_ref().err().map(Error::exit_code),
            Some(5),
            "the failed slot keeps its error, in argument order"
        );
    }

    #[test]
    fn severity_cannot_be_combined_with_no_alerts() {
        let (cli, _) = parse(&[
            "cirrocast",
            "Beijing",
            "--no-alerts",
            "--severity",
            "severe",
        ]);
        let location = crate::provider::metar::placeholder_location("ZBAA");
        let request = alert_request(
            &cli.query,
            &Config::default(),
            &location,
            &[],
            Format::Plain,
            0,
        );
        let error = request.expect_err("severity is meaningless without alerts");
        assert_eq!(error.exit_code(), 2);
        assert!(
            error
                .to_string()
                .contains("--no-alerts cannot be combined with --severity"),
            "{error}"
        );
    }

    #[test]
    fn a_template_file_loses_exactly_one_trailing_newline() {
        assert_eq!(trim_one_newline("%l\n".to_owned()), "%l");
        assert_eq!(trim_one_newline("%l\r\n".to_owned()), "%l");
        assert_eq!(trim_one_newline("%l\n\n".to_owned()), "%l\n");
        assert_eq!(trim_one_newline("%l".to_owned()), "%l");
        assert_eq!(trim_one_newline("\n".to_owned()), "");
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

        // `--station` with the provider from the config or the default selects `metar`, so it is
        // not a conflict; an *explicit* chain without a station-capable entry is one.
        let (cli, sources) = parse(&["cirrocast", "--station", "ZBAA"]);
        validate_query(&cli.query, sources, &defaults).expect("`--station` selects metar");

        let (cli, sources) = parse(&["cirrocast", "--station", "ZBAA", "-p", "open-meteo"]);
        let error = validate_query(&cli.query, sources, &settings("open-meteo"))
            .expect_err("open-meteo cannot answer a station");
        assert_eq!(error.exit_code(), 2);
        assert!(
            error
                .to_string()
                .contains("--station ZBAA needs a station-capable provider"),
            "{error}"
        );

        let (cli, sources) = parse(&["cirrocast", "--station", "ZBAA", "-p", "metar"]);
        validate_query(&cli.query, sources, &settings("metar")).expect("metar answers a station");
        let (cli, sources) = parse(&["cirrocast", "--station", "ZBAA", "-p", "open-meteo,metar"]);
        validate_query(&cli.query, sources, &settings("open-meteo,metar"))
            .expect("a chain containing metar answers a station");
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
        assert_eq!(query.format.as_deref(), Some("one-line"));
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
