// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! `cirrocast` — a terminal weather client with pluggable backends.
//!
//! This crate is the library half of the binary: the command line surface ([`cli`]), the error and
//! exit-code contract ([`error`]), XDG directory resolution ([`paths`]), the backend registry
//! ([`provider`]), the canonical data model ([`model`]) and location resolution ([`geo`]). Weather
//! fetching arrives in later steps.

pub mod cli;
pub mod config;
pub mod error;
pub mod geo;
pub mod model;
pub mod paths;
pub mod provider;
