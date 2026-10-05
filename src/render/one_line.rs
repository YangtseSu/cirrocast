// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The `one-line` format: a single line driven by `%` tokens, wttr.in-compatible.
//!
//! This is the format a prompt, a status bar or a shell function consumes, so it is a *template*
//! language rather than a fixed sentence: `--template '%c %t'` prints the condition art and the
//! temperature, `%l` the place, and a preset (`@full`) is nothing but a longer template.
//!
//! The engine itself — the [`crate::template::TOKENS`] table, the presets, the escapes and the
//! width/precision rules — lives in [`crate::template`], shared with the `full`/`minimal` presets,
//! step 22's `status` probe and the wttr.in compatibility surface. This module is only the
//! renderer: it wraps one expansion with the alert banner that belongs above it.

use std::fmt::Write as _;

use super::{RenderContext, Renderer};
use crate::error::Result;
use crate::i18n::keys;
use crate::model::Report;
use crate::template;

/// The renderer for the `one-line` format.
#[derive(Debug, Clone)]
pub struct OneLine {
    template: String,
}

impl OneLine {
    /// A renderer for `template`, which is already resolved (see
    /// [`crate::template::resolve_template`]) and may carry any token the table defines.
    #[must_use]
    pub fn new(template: impl Into<String>) -> Self {
        Self {
            template: template.into(),
        }
    }

    /// The template this renderer expands.
    #[must_use]
    pub fn template(&self) -> &str {
        &self.template
    }
}

impl Renderer for OneLine {
    fn render(&self, report: &Report, ctx: &RenderContext<'_>) -> Result<String> {
        // The banner travels above the one-liner: it is data, not a credit, and `%A` alone would
        // drop the until/instruction details. The glyph follows the terminal's charset.
        let mut banner = String::with_capacity(256);
        for line in super::alerts::banner(&report.alerts, ctx.term.charset(), ctx) {
            banner.push_str(&line.text);
            banner.push('\n');
        }
        // `one-line` stays one line per location, so the archive label is a prefix on the line
        // itself (never on the banner above it): `2026-09-14 · archive Beijing: …`.
        let mut out = String::with_capacity(4096);
        if let Some(span) = super::archive_span(report) {
            let label = ctx.i18n.text(&keys::MODE_ARCHIVE);
            let _ = write!(out, "{span} · {label} ");
        }
        out.push_str(&template::expand(&self.template, report, ctx)?);
        out.push('\n');
        out.push_str(&banner);
        // The trailing separator belongs to the line, not to the banner: `render_slots` joins
        // slots with `slot_separator`, so exactly one `\n` ends each location.
        out.pop();
        Ok(out)
    }

    /// One line per location: the blocks stack on consecutive lines, without a blank line between.
    fn slot_separator(&self) -> &'static str {
        "\n"
    }
}
