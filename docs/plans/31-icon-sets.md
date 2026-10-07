<!--
SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Step 31 — icon sets (emoji and Nerd Font)

Status: ⬜ not-started
Depends on: `07-art-table-renderer.md` (the art corpus, `ART_W`/`ART_LINES`, the charset split), `08-cli-surface-and-formats.md` (the format and flag surface, the config keys), `17-moon-phase-and-astro.md` (the moon blocks and `%m`), `19-multi-location-and-templates.md` (`%c` and the template engine), `09-localization.md` (the `one-line` presets)
Touches: `src/render/{mod,art,art_table,one_line,moon,plain}.rs`, `src/model/condition.rs` (only if a key gains an icon variant), `src/config/mod.rs`, `src/cli.rs`, `src/template.rs`, `tests/{render_icons,render_art_table,render_one_line,cli}.rs`, `tests/snapshots/`, `docs/configuration.md`, `README.md`, `docs/plans/README.md`, `CHANGELOG.md`

## Goal

`cirrocast --icons emoji` and `--icons nerd` (with `[render] icons = "emoji" | "nerd" | "nerd,emoji"`
and `CIRROCAST_ICONS` as the config and environment tiers) draw the condition and moon art from a
glyph set instead of the hand-drawn blocks: one emoji per condition from Unicode, or one Weather
Icons glyph from the set that ships inside [Nerd Fonts](https://github.com/ryanoasis/nerd-fonts).
The default stays `blocks` — today's art, which needs no font beyond a Latin monospace — so a
default install, a `TERM=dumb` shell and a non-UTF-8 locale are all unchanged.

The value is an **ordered chain**, not a single choice: `--icons nerd,emoji` tries each set per
glyph and falls back to the next one, with `blocks` always last. A terminal cannot be asked whether
its font carries a glyph, so the chain is the honest form of "mix": the first set that *has* the
glyph wins, and a key the set lacks degrades to the next set instead of printing tofu. There is no
alias for the common order — the comma spelling *is* the mechanism, so one syntax covers every
subset and order (`nerd`, `nerd,emoji`, `emoji,nerd`) and no second vocabulary has to be explained.

The switch is the same axis wttr.in offers as `v2` (emoji) versus `v2d`/`v2n` ("if you prefer Nerd
Fonts instead of Emoji"), and it closes the one honest difference in the `%c` token: wttr's `%c` is
an emoji, ours is a 7-bit three-character glyph.

## Deliverables

- ⬜ The icon chain in the render layer (`src/render/mod.rs`), resolved once per run beside
      `Charset`: an ordered list of sets — `blocks` (default) | `emoji` | `nerd` — spelled as a
      comma-separated `--icons` value, with `blocks` appended implicitly as the last resort.
      Precedence is the usual `--icons` → `CIRROCAST_ICONS` → `[render] icons` → the built-in
      default. It travels in `RenderContext`/`TermCaps` exactly like `Charset` does, so renderers
      stay pure functions of the context and snapshots stay deterministic.
- ⬜ Per-glyph resolution, pinned by tests: the first set in the chain that carries a glyph for the
      key wins, `blocks` ends every chain (so a key no set covers renders as art and prints one `-v`
      note, never tofu and never a panic). An empty list, an empty element or an unknown name is a
      usage error listing the three names; the chain is normalised (duplicates dropped, order kept)
      so `nerd,nerd,emoji` and `nerd,emoji` behave identically. No alias is defined for any order:
      the comma syntax is the mechanism, and a name like `auto` would promise a detection the client
      cannot perform.
- ⬜ The forced-fallback rule, pinned by tests: a `Charset::Ascii` run (`--format dumb`, `TERM=dumb`,
      a non-UTF-8 locale) renders `blocks` in its ASCII form whatever `--icons` says — an icon set
      cannot be drawn in 7-bit — and `-v` says so once, like the existing charset note.
- ⬜ The emoji corpus: one glyph per art key (41 keys) and per moon phase (8), the sequences pinned
      in the table — emoji presentation spelled explicitly (`☀️` is U+2600 U+FE0F, `☁️` U+2601
      U+FE0F, `⛈️` U+26C8 U+FE0F; `🌧️`, `🌨️`, `🌫️`, `🌪️`, `🌡️` are single codepoints), the moon
      `🌑🌒🌓🌔🌕🌖🌗🌘` (U+1F311–U+1F318) — with each sequence's `unicode-width` measurement
      recorded next to it (2 columns for the emoji-presentation sequences, which is what the layout
      pads for).
- ⬜ The Nerd Font corpus: one glyph per art key and per moon phase from the **Weather Icons** set
      inside Nerd Fonts, range **U+E300–U+E3E3** (228 glyphs; `weather-*` names in the project's
      `glyphnames.json`): `weather-day_sunny` U+E30D, `weather-night_clear` U+E32B,
      `weather-cloudy` U+E312, `weather-fog` U+E313, `weather-rain` U+E318, `weather-showers`
      U+E319, `weather-snow` U+E31A, `weather-sleet` U+E3AD, `weather-hail` U+E314,
      `weather-thunderstorm` U+E31D, `weather-strong_wind` U+E34B, `weather-tornado` U+E351,
      `weather-hurricane` U+E36C, `weather-sandstorm` U+E37A, `weather-dust` U+E35D,
      `weather-smoke` U+E35C, the `weather-moon_*` phases U+E38D–U+E3A8, and the day/night siblings
      (`weather-day_*` / `weather-night_alt_*`) for the keys that carry a `-night` suffix. Every key
      maps; the coverage test below is what keeps that true when a condition is added.
- ⬜ Renderer integration, all through the resolved chain:
      * `art_table` — the glyph centred in the block's 7×[`ART_LINES`] cell, padded by its measured
        width, so the table geometry (`ART_W`, `GAP`, `METRICS_W`, the borders) does not move;
      * the stacked narrow layout — the glyph in the `GLYPH_W` column;
      * `one_line` — `%c` prints the set's glyph; **`%x` keeps printing the 7-bit ASCII glyph**
        whatever the set (today the two tokens are identical because the one-line glyph is
        deliberately 7-bit; this makes `%x` what its own doc already claims);
      * the `moon` format and `%m` — the set's moon glyph;
      * `plain` and `json` — untouched: they carry no art.
- ⬜ Config and CLI surface: `[render] icons` in `RenderConfig` with validation through the same
      parser as the flag, `--icons <SETS>` in `--help` (the three names, the comma syntax, the Nerd
      Font prerequisite), `config get/set render.icons`, the environment override,
      `docs/configuration.md` and the README (a short example block per set, the `nerd,emoji` chain
      as the recommended Nerd Font setup, and the line "Nerd Fonts: install a patched font from
      ryanoasis/nerd-fonts; the default needs no font").
- ⬜ Tests: per-set coverage (every `Condition::art_key` and every `MoonPhase` has a glyph in every
      set, in both directions like the existing art test), the chain's resolution (first set with the
      glyph wins, `blocks` is appended and wins when nothing else has it, duplicates collapse, an
      unknown name is a usage error), width invariants (no rendered line wider
      than its cell; every padded line exactly `ART_W`/`GLYPH_W` columns by `unicode-width`), the
      forced-ASCII rule, the precedence chain (flag beats env beats config), snapshots per set for
      `art-table`, `one-line` and `moon` at 80 and 120 columns, and a `--icons nerd` snapshot whose
      bytes are asserted to be exactly the expected codepoints (so a wrong codepoint cannot pass as
      "some PUA glyph").
- ⬜ Docs: README (the flag, the two sets, the font prerequisite, what stays default), the CHANGELOG
      entry, and `docs/plans/README.md` marking this step. `docs/schema.md` and the JSON document do
      not change: no data, only drawing.

## Design notes

* **Why this is an A-series step, not a backlog wish.** It adds a rendering axis that every text
  format's output depends on (the corpus, the layout padding, the config, the help text), and it is
  the only wttr.in parity item the 2026-10-07 audit found missing. It is not a data feature: no
  provider, no request, no cache key.
* **Mixing is a chain, not a detection.** A terminal cannot be asked whether its font carries a
  glyph — the escape sequences that exist (OSC 50 and friends) report a font *name*, not coverage —
  so "mixed" cannot mean "pick the one that works". It means "try in order": `nerd,emoji` renders
  Nerd Font glyphs wherever the Weather Icons set has one and falls through to emoji elsewhere, with
  `blocks` always appended so the output is never tofu and never a panic. The one signal that *is*
  reliable — a non-UTF-8 locale or `TERM=dumb` — keeps forcing `blocks`. No alias is defined for any
  order: `auto` would promise a detection the client cannot perform, `mixed`/`both` would hide the
  order, and the comma spelling already covers every subset and order with no second vocabulary.
* **Per-surface mixing is a different feature.** "Nerd Font conditions, emoji moon" would need a
  second key (`[render] moon_icons` or similar) and a second resolution per art family; this step
  keeps one chain for all art and records the idea as out of scope, so the surface stays one flag.
* **What is deliberately not built.** No font is bundled, ever: we emit codepoints and the user
  installs the font. **qweather-icons is rejected** — its icons are CC BY 4.0 with a terms clause
  requiring the response's attribution list shown in full and unmodified wherever the icons appear,
  which is a licence burden a terminal client should not take on for decoration.
* **Why the Weather Icons family inside Nerd Fonts.** It is the weather-specific set (day and night
  variants, every precipitation family, moon phases, wind, thermometer), it lives in one contiguous
  PUA block (U+E300–U+E3E3) that is trivial to pin by test, and rendering its glyphs carries no
  attribution duty — Nerd Fonts and the Weather Icons set are permissive (SIL OFL 1.1 for the icons),
  and nothing is redistributed. Material Design Icons' `md-weather_*` family (U+F0590+) is the
  alternative if a set ever needs replacing; the corpus table is the one place that would change.
* **Width is the whole risk.** Emoji are two columns in `unicode-width` 0.2.2 when the sequence
  carries emoji presentation, Nerd Font glyphs are one; the padding uses the measurement, never a
  hard-coded assumption, and the block stays 7 columns wide so the table's borders cannot drift.
  Terminals disagree about emoji width (some render them one column, tmux may disagree with the
  terminal), which is exactly why `blocks` stays the default and `--icons` is opt-in.
* **`%c` follows the set, `%x` never does.** A status bar that greps a `one-line` template must keep
  working on a terminal with no emoji font; `%x` is that promise, and it is the token wttr.in also
  keeps as plain text.
* **No new dependency.** `unicode-width` 0.2.2 already measures the sequences; the tables are
  `&'static str`; `deny.toml` is untouched.

## Out of scope

* Bundling or patching fonts, and any runtime font detection (a terminal cannot be asked whether it
  has a glyph — the user chooses the set, we document the prerequisite).
* `qweather-icons` (rejected above) and any icon set whose terms demand per-display attribution.
* Image protocols (sixel, kitty, iTerm2 inline images) and any raster art: the contract is text.
* Per-condition colour changes: the art's `ArtStyle` colours stay as they are in every set.
* Changing `plain`/`json` (they carry no art) or the alert/air panels.

## Verification

```bash
cargo run -q -- --icons emoji Beijing            # the table with emoji, borders still aligned
cargo run -q -- --icons nerd  Beijing            # needs a Nerd Font; codepoints only otherwise
cargo run -q -- --icons nerd,emoji Beijing       # the chain: Nerd Font first, emoji where it lacks
cargo run -q -- --icons emoji -f one-line --template '%l %c %t'
cargo run -q -- --icons emoji -f one-line --template '%l %x %t'   # unchanged 7-bit glyph
cargo run -q -- --icons emoji --moon Beijing     # the moon block from the set
TERM=dumb cargo run -q -- --icons emoji Beijing  # forced back to blocks, one -v note
cargo run -q -- --icons nerd -f json Beijing | jq '.current'      # identical to any other set
CIRROCAST_ICONS=emoji cargo run -q -- Beijing | head -12          # the environment tier
cargo run -q -- --icons wat Beijing              # usage error listing blocks|emoji|nerd
cargo test --workspace --locked && cargo clippy --workspace --all-targets --locked -- -D warnings
reuse lint
```

## Exit criteria

- ⬜ `--icons emoji` and `--icons nerd` render the table, the stacked layout, `%c`, the `moon` format
      and `%m` from their set, verified by running the binary and pasting the output into the log —
      with the borders aligned in a Nerd Font terminal and in an emoji-capable one.
- ⬜ The default output (no flag, no config, `TERM=dumb`, `--format dumb`, non-UTF-8 locale) is
      byte-identical to the current build, proven by the existing snapshots passing unchanged.
- ⬜ Every art key and every moon phase has a glyph in both sets, asserted in both directions by a
      test that fails when a condition or a set entry is added without its counterpart, and the chain
      resolves per glyph (a set that lacks one falls through, `blocks` last) — proven by a test that
      removes a glyph from a test-only table rather than by hoping.
- ⬜ `docs/configuration.md`, the README and `--help` name the sets, the precedence and the Nerd Font
      prerequisite; `reuse lint` is clean (no new third-party file enters the repository).

## Risks

* **Terminal emoji width disagreement.** The mitigation is the default (`blocks`), the `%x` token and
  the documented caveat; a user whose terminal renders emoji one column wide sees a misaligned table
  and can switch back with one flag.
* **Nerd Font not installed.** The output is tofu for that user; the flag's help text says the font
  is required, the README repeats it, and the default never needs it.
* **Corpus drift.** A new condition key must gain a glyph in both sets; the coverage test is the
  gate, and the missing-glyph fallback keeps a release build renderable even if the test is skipped.
* **A tempting licence creep.** Some icon sets (notably qweather-icons) come with display-time
  attribution duties; the plan records the rejection so a later "just add one more set" does not
  quietly import it.

## Progress log

- 2026-10-07 — step created at the maintainer's request, scoped to `emoji` and `nerd` only;
  `qweather-icons` was considered and rejected (CC BY 4.0 icons with a mandatory full-attribution
  clause). Facts gathered the same day: Nerd Fonts' `glyphnames.json` carries 228 `weather-*` glyphs
  in U+E300–U+E3E3 (Weather Icons) and 31 `md-weather_*` in U+F0590+ (Material Design Icons); the
  repository already depends on `unicode-width` 0.2.2, which measures the emoji-presentation
  sequences this step needs; the art corpus is 41 condition keys plus 8 moon phases, and wttr.in's
  `v2`/`v2d`/`v2n` views are the precedent for the emoji-versus-Nerd-Font split.
- 2026-10-07 — the either/or question was settled as a chain: the value is a comma-separated list
  (`--icons nerd,emoji`), each glyph resolved by the first set that carries it with `blocks` always
  last. Automatic font detection is impossible (a terminal reports a font *name*, not glyph
  coverage), so no `auto`-style value exists; the placeholder alias `mixed` was dropped in favour of
  the one syntax that generalises, and the deliverable list, the design notes, the verification block
  and the exit criteria were updated in place.
