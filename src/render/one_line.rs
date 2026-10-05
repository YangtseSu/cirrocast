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

use super::{RenderContext, Renderer};
use crate::error::Result;
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
        let mut out = String::with_capacity(4096);
        for line in super::alerts::banner(&report.alerts, ctx.term.charset(), ctx) {
            out.push_str(&line.text);
            out.push('\n');
        }
        out.push_str(&template::expand(&self.template, report, ctx)?);
        Ok(out)
    }

    /// One line per location: the blocks stack on consecutive lines, without a blank line between.
    fn slot_separator(&self) -> &'static str {
        "\n"
    }
}
