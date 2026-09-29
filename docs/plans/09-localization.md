<!--
SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Step 09 — Localization

Status: ⬜ not-started
Depends on: 03, 07
Touches: src/i18n.rs, src/main.rs, src/cli.rs, src/render/{mod,art_table,one_line,plain,json}.rs,
src/geo/mod.rs, locales/en-US/main.ftl, locales/zh-CN/main.ftl, tests/i18n.rs, tests/render_snapshots.rs,
tests/snapshots/, Cargo.toml, README.md

## Goal
Ship every user-visible weather string in the renderers through Fluent catalogs embedded in the binary:
condition names for all WMO codes, day-part/weekday/month names, table labels and the one-line/plain
vocabulary, with locale negotiation from `--lang` and the ambient locale, `en-US` + `zh-CN` complete, and a
completeness test that makes an untranslated condition or label a build failure.

## Deliverables
- ⬜ `src/i18n.rs`: `struct LanguageId(unic_langid::LanguageIdentifier)`, `enum LanguageRequest { Auto,
      Tag(String) }`, `struct I18n { bundle: FluentBundle<&'static str>, selected: LanguageId,
      chain: Vec<LanguageId>, notes: Vec<Note> }` and `enum Note { Fallback { requested: String,
      selected: LanguageId }, UnknownTag(String), MissingKey(String) }`.
- ⬜ Embedded catalogs, no runtime filesystem dependency:
      `const CATALOGS: &[(&str, &str)] = &[("en-US", include_str!("../locales/en-US/main.ftl")),
      ("zh-CN", include_str!("../locales/zh-CN/main.ftl"))];` — `.ftl` takes `#` comments natively, so each
      file carries the SPDX header inline and no `REUSE.toml` annotation is needed.
- ⬜ `I18n::load(req: LanguageRequest, env: &Env) -> I18n` — infallible: negotiate `--lang` > `LC_ALL` >
      `LC_MESSAGES` > `LANG` > `en-US`, normalising `zh_CN.UTF-8` → `zh-CN` (strip `.codeset` and `@modifier`,
      `_` → `-`) and mapping the `C`/`POSIX` pseudo-locales to `en-US`; `auto` means "walk the env chain".
- ⬜ Fallback chain: explicit `zh-TW|zh-HK|zh-MO → zh-CN → en-US` plus `* → en-US`; built by adding
      resources to **one** `FluentBundle` in reverse priority (fallbacks first, selected last) because
      `get_message` scans resources last-added-first — a test pins that ordering.
- ⬜ Robustness: `--lang bad-TAG` (unparsable or unsupported) prints
      `warning: unsupported language "<tag>", falling back to en-US` on stderr and exits 0 — never
      `Error::Usage`. `-v` prints one line:
      `i18n: requested zh-TW.UTF-8 → selected zh-CN (chain zh-TW → zh-CN → en-US)`.
- ⬜ Lookup API: `fn text(&self, key: &str, args: &[(&str, FluentValue)]) -> Cow<'_, str>`,
      `fn condition(&self, c: Condition) -> Cow<'_, str>` (`cond-<code>`), `fn day_part(&self, p: DayPart)`,
      `fn weekday(&self, w: Weekday)`, `fn month(&self, m: Month)`, `fn uv_band(&self, uv: f32) -> Cow<'_, str>`,
      `fn format_temp(&self, temp_c: f32, units: UnitSystem) -> String`,
      `fn format_date(&self, d: NaiveDate, style: DateStyle) -> String` (`DateStyle::{Iso, Short, Today}`),
      `fn notes(&self) -> &[Note]`.
- ⬜ Failure behaviour: a missing message returns the key itself, records `Note::MissingKey` once and is
      printed under `-v`; `bundle.set_use_isolating(false)` so no U+2068/U+2069 bidi marks leak into terminal
      output; formatting never panics on a bad pattern (pattern errors are collected and reported, not unwrapped).
- ⬜ `locales/en-US/main.ftl` and `locales/zh-CN/main.ftl` carrying the complete key set: `cond-0` … `cond-99`
      (all 100 codes, unmapped ones = "Unknown"), `part-morning|noon|evening|night`, `weekday-mon`…`weekday-sun`,
      `month-1`…`month-12`, `date-iso|date-short|date-today`, `report-header`, `label-feels-like`, `label-wind`,
      `label-humidity`, `label-precip`, `label-pressure`, `label-visibility`, `label-uv`, `label-sunrise`,
      `label-sunset`, `label-updated`, `label-attribution`, `uv-band-low|moderate|high|very-high|extreme`,
      `moon-na`, `na` (= `n/a`), `arrow-north|east|south|west` composite direction names.
      Sample form (both files, same keys): `cond-61 = Light rain` / `cond-61 = 小雨`,
      `date-short = { $weekday }, { $month } { $day }` / `date-short = { $month }{ $day }日 { $weekday }`.
- ⬜ `pub const RENDERER_KEYS: &[&str]` in `src/i18n.rs`: the single source of truth for every key the
      renderers and CLI ask for; renderer code references these constants (never raw literals), so a new label
      cannot be added without appearing in the completeness test.
- ⬜ Number/date formatting: all numbers and dates go through Fluent messages (`num-1`, `date-*`), never
      through locale-dependent C formatting — `format!("{:.1}", x)` with a fixed `.` plus a message-provided
      decimal separator, and dates assembled from `weekday-*`/`month-*`/`date-*` messages rather than
      `chrono`'s locale names, which are never locale-aware in this project.
- ⬜ Renderer integration: `art_table.rs`, `one_line.rs`, `plain.rs` take every label/condition/temperature
      string from `ctx.i18n`; `json.rs` stays machine-readable (`"condition": { "code": 61, "text": "…" }` uses
      the selected locale for `text` and never for keys); art blocks are language-independent.
- ⬜ `src/render/art_table.rs` keeps boxes aligned for CJK: column widths are recomputed from
      `unicode_width::UnicodeWidthStr` on the localized strings (already the measuring function of step 07), and
      a `zh-CN` snapshot proves borders line up.
- ⬜ `tests/i18n.rs` — completeness and negotiation: every code `0..=99` has `cond-<code>` in **every** catalog
      of `CATALOGS`; every key of `RENDERER_KEYS` exists in every catalog; each catalog's key set equals
      `en-US`'s exactly (no missing, no orphan); negotiation cases `zh-TW → zh-CN`, `de-DE → en-US`,
      `zh_CN.UTF-8 → zh-CN`, `C → en-US`, `bad-TAG → warning + en-US`; a key defined only in `en-US` resolves
      through the fallback chain; no output contains U+2068/U+2069.
- ⬜ `tests/render_snapshots.rs` additions: hand-reviewed `zh-CN` snapshots for `art-table` (one 3-day, one
      stacked at width 40, one day/night pair), `one-line` (`@full` preset) and `plain`, all under
      `tests/snapshots/`; the existing `en-US` snapshots stay byte-identical, proving localization changed
      nothing for the default locale.
- ⬜ `src/cli.rs`: `--lang <BCP-47|auto>` resolves through `I18n::load` (the flag itself is declared in
      step 08); `--verbose` surfaces `Note::Fallback`/`Note::UnknownTag`, `-q` silences the warning but keeps
      the fallback behaviour.
- ⬜ `Cargo.toml`: `fluent-bundle = "0.16"`, `fluent-langneg = "0.13"`, `unic-langid = "0.9"` (justification
      below); `README.md` gains a `## Languages` section documenting `en-US`/`zh-CN` and the add-a-language
      recipe; the remaining languages are listed as a roadmap entry in step 14.

## Design notes
Why Fluent instead of gettext or `rust-i18n`: `gettext` needs a `.mo` compile step (`build.rs` + `msgfmt`)
before anything can be embedded, and its plural/gender handling lives in C macros rather than the message file;
`rust-i18n` compiles into a fixed crate layout, parses YAML/TOML at runtime-ish boundaries and has weaker
plural-rule support. Fluent is the Mozilla standard, pure Rust, has plural/`select` expressions in the syntax
itself, needs no build script, and its catalogs are plain text — exactly what `include_str!` embedding wants.
`unic-langid` gives strict BCP-47 parsing (so `--lang bad-TAG` is a parse error, not a silent mismatch), and
`fluent-langneg` supplies the RFC 4647 style matching used with our explicit fallback map. Cost: three small
pure-Rust crates, no `unsafe`, no build-time tooling.
The fallback map is explicit rather than purely algorithmic because `zh-TW` must land on `zh-CN` (not on the
generic `zh` region-less match `fluent-langneg` would pick) and because a wrong guess silently shows Simplified
Chinese to Traditional readers; the chain is printed under `-v` so it is never a secret.
Embedding with `include_str!` means adding a language touches `CATALOGS` (one line) and nothing else. The
rejected alternative was a `build.rs` that globs `locales/*/main.ftl` and generates the table: it would make
"drop a file, no code change" literally true, but it adds a build script (REUSE and `cargo deny` surface, and a
second place where file content becomes code). Recorded as a possible step-13 improvement, not done here.
Completeness is enforced over all 100 WMO codes, not over the codes the current providers emit, so a new
provider adding a new condition code cannot ship a silently English/Chinese-missing string.
`n/a` is a message (`na`) rather than a hard-coded string, so translators can localize it; the moon phase itself
does not exist yet and renders `n/a` until the roadmap item lands.

## Out of scope
- `--help`, clap usage errors, warning and log lines stay English (localizing clap's own strings is an upstream
  concern; recorded in the backlog of step 14).
- Provider-supplied text is never displayed: `condition.text` from an upstream payload is only kept in
  `Attribution`/`raw` for debugging, so a provider cannot inject untranslated or untrusted strings into output.
- RTL layout mirroring (Arabic, Hebrew, Farsi): the art table is LTR by construction; the two shipped locales
  are LTR. Deferred to the step 14 roadmap with a note that `art` placement — not just text — must be mirrored.
- Translating the art blocks themselves (they are language-independent pictograms by design).
- Additional locales beyond `en-US`/`zh-CN` (roadmap; the recipe above is the whole mechanism).

## Verification
Fixtures: `tests/fixtures/report/beijing-3d-day.json`, `beijing-night.json`, `current-only.json` (reused from
step 07/08) and `tests/fixtures/report/all-conditions.json` for the per-condition localized-name sweep.
Manual smoke run:
```
cargo run -- --lang zh-CN --days 3 Beijing                        # Chinese labels; borders still aligned
LANG=zh_CN.UTF-8 cargo run -- -v Beijing | head -3                # auto-negotiates zh-CN, -v prints the chain
cargo run -- --lang zh-TW Beijing -v | head -3                    # zh-TW → zh-CN, chain reported
cargo run -- --lang bad-TAG Beijing; echo $?                      # warning on stderr, exit 0, en-US output
cargo run -- --format one-line --template '@full' --lang zh-CN Beijing   # one line, Chinese vocabulary
cargo run -- --format json --lang zh-CN Beijing | jq -r .current.condition.text
cargo test --test i18n                                            # completeness passes for every catalog
```

## Exit criteria
- ⬜ `cargo fmt --check` / `cargo clippy --all-targets -- -D warnings` / `cargo test` / `reuse lint` all clean
- ⬜ `--lang zh-CN` renders conditions, day parts, weekdays/months and labels in Chinese with the table still
      aligned (borders line up, no line exceeds the resolved width)
- ⬜ `LANG=zh_CN.UTF-8` without `--lang` selects `zh-CN`; `--lang zh-TW` selects `zh-CN` and reports the chain under `-v`
- ⬜ `--lang bad-TAG` warns on stderr, exits 0 and falls back to `en-US`
- ⬜ `tests/i18n.rs` fails if any of the 100 condition keys or any `RENDERER_KEYS` key is missing from any catalog
- ⬜ `en-US` snapshots are byte-identical to their step 07/08 revisions; new `zh-CN` snapshots are hand-reviewed
- ⬜ README documents the shipped locales and the add-a-language recipe

## Risks
- CJK width: a single miscounted double-width glyph shifts every border. Mitigated by measuring with
  `unicode-width` (step 07's `fit`) and by a `zh-CN` width-40 snapshot plus the existing property test rerun for
  the `zh-CN` locale.
- Ambient-locale flakiness: tests must pass under `LC_ALL=C`, `LANG=zh_CN.UTF-8` and an empty environment, so the
  test helper sets the locale environment explicitly per case instead of inheriting it.
- Catalog drift as keys are added: the completeness test runs over every catalog in `CATALOGS`, so a new key with
  a missing `zh-CN` translation fails immediately; a new locale is likewise validated the moment it is listed.
- Fluent version churn (`fluent-bundle` 0.16 line): pinned exactly, and the fallback-ordering test is the
  canary — if `get_message` resource precedence ever changes, that test fails instead of silently preferring the
  wrong language.
- `zh-CN` is maintained by hand without a native reviewer on the team: the PR checklist requires a second reader
  for new `zh-CN` strings, and the roadmap lists `zh-TW` as the next locale so Traditional users are not served
  by a Simplified fallback forever.

## Progress log
- 2026-09-30 — step file written; Fluent choice, fallback map and completeness test recorded.
