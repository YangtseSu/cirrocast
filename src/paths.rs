// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! XDG directory resolution.
//!
//! `cirrocast` keeps its configuration, cache and data under a single `cirrocast/` directory in
//! the XDG base directories. `XDG_CONFIG_HOME`, `XDG_CACHE_HOME` and `XDG_DATA_HOME` are honoured
//! via [`etcetera`]; when they are unset the user defaults apply:
//!
//! * config — `~/.config/cirrocast/`
//! * cache — `~/.cache/cirrocast/`
//! * data — `~/.local/share/cirrocast/`
//!
//! Resolving paths is side-effect free: this step does not create anything on disk.

use std::path::{Path, PathBuf};

use etcetera::BaseStrategy;

use crate::error::Result;

/// Directory name appended to each XDG base directory.
const APP_DIR: &str = "cirrocast";

/// The concrete directories and files `cirrocast` uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    /// `$XDG_CONFIG_HOME/cirrocast`
    pub config_dir: PathBuf,
    /// `config_dir/config.toml`
    pub config_file: PathBuf,
    /// `config_dir/keys.toml`
    pub keys_file: PathBuf,
    /// `$XDG_CACHE_HOME/cirrocast`
    pub cache_dir: PathBuf,
    /// `$XDG_DATA_HOME/cirrocast`
    pub data_dir: PathBuf,
}

impl Paths {
    /// Resolves every path from the current environment.
    ///
    /// Fails with [`crate::error::Error::Config`] when the home directory cannot be determined and
    /// no XDG override is set.
    pub fn resolve() -> Result<Self> {
        let strategy = etcetera::choose_base_strategy()?;
        Ok(Self::from_base_dirs(
            &strategy.config_dir(),
            &strategy.cache_dir(),
            &strategy.data_dir(),
        ))
    }

    /// Builds the application paths from already resolved XDG base directories.
    fn from_base_dirs(config: &Path, cache: &Path, data: &Path) -> Self {
        let config_dir = config.join(APP_DIR);
        Self {
            config_file: config_dir.join("config.toml"),
            keys_file: config_dir.join("keys.toml"),
            config_dir,
            cache_dir: cache.join(APP_DIR),
            data_dir: data.join(APP_DIR),
        }
    }
}
