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

/// File name of the configuration document inside [`APP_DIR`].
const CONFIG_FILE: &str = "config.toml";

/// Environment variable holding the XDG config search path.
const XDG_CONFIG_DIRS: &str = "XDG_CONFIG_DIRS";

/// Fallback for [`XDG_CONFIG_DIRS`] when it is unset or empty.
#[cfg(unix)]
const DEFAULT_CONFIG_DIRS: &str = "/etc/xdg";

/// No system wide search path on platforms without the XDG base directory spec.
#[cfg(not(unix))]
const DEFAULT_CONFIG_DIRS: &str = "";

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
            config_file: config_dir.join(CONFIG_FILE),
            keys_file: config_dir.join("keys.toml"),
            config_dir,
            cache_dir: cache.join(APP_DIR),
            data_dir: data.join(APP_DIR),
        }
    }

    /// Every configuration file candidate, in XDG search order: the user's
    /// `$XDG_CONFIG_HOME/cirrocast/config.toml` first, then one
    /// `$XDG_CONFIG_DIRS/cirrocast/config.toml` per entry (default `/etc/xdg`). The first entry
    /// that exists is the one `cirrocast` reads; duplicates are dropped. The XDG base directory
    /// spec says a path in `XDG_CONFIG_DIRS` must be absolute, and that a relative one is
    /// invalid and must be ignored — so a `XDG_CONFIG_DIRS=.` cannot make the process read a
    /// `./cirrocast/config.toml` from the working directory.
    pub fn config_file_candidates(&self) -> Vec<PathBuf> {
        let mut candidates = vec![self.config_file.clone()];
        let dirs = std::env::var(XDG_CONFIG_DIRS).unwrap_or_default();
        let dirs = if dirs.is_empty() {
            DEFAULT_CONFIG_DIRS
        } else {
            &dirs
        };
        for dir in dirs
            .split(':')
            .filter(|entry| !entry.is_empty())
            .filter(|entry| Path::new(entry).is_absolute())
        {
            let candidate = Path::new(dir).join(APP_DIR).join(CONFIG_FILE);
            if !candidates.contains(&candidate) {
                candidates.push(candidate);
            }
        }
        candidates
    }
}
