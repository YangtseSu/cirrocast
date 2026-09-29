// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The command line surface.
//!
//! Scope: global flags, the `config`/`key`/`provider` subcommands. Weather query flags
//! (`--provider`, `--format`, `--days`, ...) and the location argument are added by the steps that
//! implement them, so that a flag never exists before the behaviour behind it.

use std::io::{IsTerminal as _, Read as _};
use std::process::Command as StdCommand;

use clap::{ArgAction, Args, Parser, Subcommand};

use crate::config::Config;
use crate::config::keys::{KeySource, KeyStore};
use crate::error::{Error, Result};
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
        }
    }
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
