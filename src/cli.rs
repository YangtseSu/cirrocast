// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The command line surface.
//!
//! Scope of step 01: global flags plus the `config` and `provider` introspection commands. Weather
//! query flags (`--provider`, `--format`, `--days`, ...) and the location argument are added by the
//! steps that implement them, so that a flag never exists before the behaviour behind it.

use clap::{ArgAction, Args, Parser, Subcommand};

use crate::error::Result;
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
    /// Inspect where cirrocast keeps its files.
    Config(ConfigArgs),

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
    /// Print the resolved configuration directory.
    Path,
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
            Command::Config(args) => match &args.command {
                ConfigCommand::Path => {
                    println!("{}", Paths::resolve()?.config_dir.display());
                    Ok(())
                }
            },
            Command::Provider(args) => match &args.command {
                ProviderCommand::List => {
                    for line in provider_table() {
                        println!("{line}");
                    }
                    Ok(())
                }
                ProviderCommand::Info(info) => {
                    // An unknown id is a usage problem, not a lookup failure.
                    let id: ProviderId = info.id.parse()?;
                    for line in provider_details(&id.metadata()) {
                        println!("{line}");
                    }
                    Ok(())
                }
            },
        }
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
