<!--
SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Step 09 — Localization

Status: ✅ done
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
- ✅ `src/i18n.rs`: `struct LanguageId(unic_langid::LanguageIdentifier)`, `enum LanguageRequest { Auto,
      Tag(String) }`, `struct I18n { bundle: FluentBundle<&'static str>, selected: LanguageId,
      chain: Vec<LanguageId>, notes: Vec<Note> }` and `enum Note { Fallback { requested: String,
      selected: LanguageId }, UnknownTag(String), MissingKey(String) }`.
- ✅ Embedded catalogs, no runtime filesystem dependency:
      `const CATALOGS: &[(&str, &str)] = &[("en-US", include_str!("../locales/en-US/main.ftl")),
      ("zh-CN", include_str!("../locales/zh-CN/main.ftl"))];` — `.ftl` takes `#` comments natively, so each
      file carries the SPDX header inline and no `REUSE.toml` annotation is needed.
- ✅ `I18n::load(req: LanguageRequest, env: &Env) -> I18n` — infallible: negotiate `--lang` > `LC_ALL` >
      `LC_MESSAGES` > `LANG` > `en-US`, normalising `zh_CN.UTF-8` → `zh-CN` (strip `.codeset` and `@modifier`,
      `_` → `-`) and mapping the `C`/`POSIX` pseudo-locales to `en-US`; `auto` means "walk the env chain".
- ✅ Fallback chain: explicit `zh-TW|zh-HK|zh-MO → zh-CN → en-US` plus `* → en-US`; built by adding
      resources to **one** `FluentBundle` in reverse priority (fallbacks first, selected last) because
      `get_message` scans resources last-added-first — a test pins that ordering.
- ✅ Robustness: `--lang bad-TAG` (unparsable or unsupported) prints
      `warning: unsupported language "<tag>", falling back to en-US` on stderr and exits 0 — never
      `Error::Usage`. `-v` prints one line:
      `i18n: requested zh-TW.UTF-8 → selected zh-CN (chain zh-TW → zh-CN → en-US)`.
- ✅ Lookup API: `fn text(&self, key: &str, args: &[(&str, FluentValue)]) -> Cow<'_, str>`,
      `fn condition(&self, c: Condition) -> Cow<'_, str>` (`cond-<code>`), `fn day_part(&self, p: DayPart)`,
      `fn weekday(&self, w: Weekday)`, `fn month(&self, m: Month)`, `fn uv_band(&self, uv: f32) -> Cow<'_, str>`,
      `fn format_temp(&self, temp_c: f32, units: UnitSystem) -> String`,
      `fn format_date(&self, d: NaiveDate, style: DateStyle) -> String` (`DateStyle::{Iso, Short, Today}`),
      `fn notes(&self) -> &[Note]`.
- ✅ Failure behaviour: a missing message returns the key itself, records `Note::MissingKey` once and is
      printed under `-v`; `bundle.set_use_isolating(false)` so no U+2068/U+2069 bidi marks leak into terminal
      output; formatting never panics on a bad pattern (pattern errors are collected and reported, not unwrapped).
- ✅ `locales/en-US/main.ftl` and `locales/zh-CN/main.ftl` carrying the complete key set: `cond-0` … `cond-99`
      (all 100 codes, unmapped ones = "Unknown"), `part-morning|noon|evening|night`, `weekday-mon`…`weekday-sun`,
      `month-1`…`month-12`, `date-iso|date-short|date-today`, `label-report`, `label-feels`, `label-wind`,
      `label-humidity`, `label-precip`, `label-pressure`, `label-visibility`, `label-uv`, `label-sunrise`,
      `label-sunset`, `label-location|updated|current|day|attribution` (the `plain` record keys),
      `uv-band-low|moderate|high|very-high|extreme`, `moon-na`, `na` (= `n/a`),
      `dir-n|nne|…|nnw` direction names, and the `format-*` unit and value messages.
      Sample form (both files, same keys): `cond-61 = Slight rain` / `cond-61 = 小雨`,
      `date-short = { $weekday }, { $month } { $day }` / `date-short = { $month }{ $day }日 { $weekday }`.
- ✅ `pub const RENDERER_KEYS: &[&str]` in `src/i18n.rs`: the single source of truth for every key the
      renderers and CLI ask for; renderer code references these constants (never raw literals), so a new label
      cannot be added without appearing in the completeness test.
- ✅ Number/date formatting: all numbers and dates go through Fluent messages (`num-1`, `date-*`), never
      through locale-dependent C formatting — `format!("{:.1}", x)` with a fixed `.` plus a message-provided
      decimal separator, and dates assembled from `weekday-*`/`month-*`/`date-*` messages rather than
      `chrono`'s locale names, which are never locale-aware in this project.
- ✅ Renderer integration: `art_table.rs`, `one_line.rs`, `plain.rs` take every label/condition/temperature
      string from `ctx.i18n`; `json.rs` stays machine-readable (`"condition": { "code": 61, "text": "…" }` uses
      the selected locale for `text` and never for keys); art blocks are language-independent.
- ✅ `src/render/art_table.rs` keeps boxes aligned for CJK: column widths are recomputed from
      `unicode_width::UnicodeWidthStr` on the localized strings (already the measuring function of step 07), and
      a `zh-CN` snapshot proves borders line up.
- ✅ `tests/i18n.rs` — completeness and negotiation: every code `0..=99` has `cond-<code>` in **every** catalog
      of `CATALOGS`; every key of `RENDERER_KEYS` exists in every catalog; each catalog's key set equals
      `en-US`'s exactly (no missing, no orphan); negotiation cases `zh-TW → zh-CN`, `de-DE → en-US`,
      `zh_CN.UTF-8 → zh-CN`, `C → en-US`, `bad-TAG → warning + en-US`; a key defined only in `en-US` resolves
      through the fallback chain; no output contains U+2068/U+2069.
- ✅ `tests/render_snapshots.rs` additions: hand-reviewed `zh-CN` snapshots for `art-table` (one 3-day, one
      stacked at width 40, one day/night pair), `one-line` (`@full` preset) and `plain`, all under
      `tests/snapshots/`; the existing `en-US` snapshots stay byte-identical, proving localization changed
      nothing for the default locale.
- ✅ `src/cli.rs`: `--lang <BCP-47|auto>` resolves through `I18n::load` (the flag itself is declared in
      step 08); `--verbose` surfaces `Note::Fallback`/`Note::UnknownTag`, `-q` silences the warning but keeps
      the fallback behaviour.
- ✅ `Cargo.toml`: `fluent-bundle = "0.16"` and `unic-langid = "0.9"` with the `macros` feature
      (justification below; `fluent-langneg` is dropped — see the log: its RFC 4647 matching answers a
      region-less `zh` request with the generic `zh`, which the explicit fallback map rejects, which made
      it an unused dependency); `README.md` gains a `## Languages` section documenting `en-US`/`zh-CN` and the add-a-language
      recipe; the remaining languages are listed as a roadmap entry in step 14.

## Design notes
Why Fluent instead of gettext or `rust-i18n`: `gettext` needs a `.mo` compile step (`build.rs` + `msgfmt`)
before anything can be embedded, and its plural/gender handling lives in C macros rather than the message file;
`rust-i18n` compiles into a fixed crate layout, parses YAML/TOML at runtime-ish boundaries and has weaker
plural-rule support. Fluent is the Mozilla standard, pure Rust, has plural/`select` expressions in the syntax
itself, needs no build script, and its catalogs are plain text — exactly what `include_str!` embedding wants.
`unic-langid` gives strict BCP-47 parsing (so `--lang bad-TAG` is a parse error, not a silent mismatch), and
the fallback map itself does the matching (`language_chain` plus the `zh` family rule), so no separate
negotiation crate is needed. Cost: two small pure-Rust crates, no `unsafe`, no build-time tooling.
The fallback map is explicit rather than purely algorithmic because `zh-TW` must land on `zh-CN` (not on the
generic `zh` region-less match an RFC 4647 negotiation would pick) and because a wrong guess silently shows Simplified
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
- ✅ `cargo fmt --check` / `cargo clippy --all-targets -- -D warnings` / `cargo test` / `reuse lint` all clean
- ✅ `--lang zh-CN` renders conditions, day parts, weekdays/months and labels in Chinese with the table still
      aligned (borders line up, no line exceeds the resolved width)
- ✅ `LANG=zh_CN.UTF-8` without `--lang` selects `zh-CN`; `--lang zh-TW` selects `zh-CN` and reports the chain under `-v`
- ✅ `--lang bad-TAG` warns on stderr, exits 0 and falls back to `en-US`
- ✅ `tests/i18n.rs` fails if any of the 100 condition keys or any `RENDERER_KEYS` key is missing from any catalog
- ✅ `en-US` snapshots are byte-identical to their step 07/08 revisions; new `zh-CN` snapshots are hand-reviewed
- ✅ README documents the shipped locales and the add-a-language recipe

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
- 2026-09-30 — implemented. Five points where the code had to leave the letter of this file, each
  because the plan met Fluent, `chrono` or step 07's snapshots:
  1. **Condition keys are `cond-<code>`, not `cond.<code>`.** An FTL message id may not contain a
     dot — `cond.0 = …` is a *syntax error* in the whole resource, so the catalog would not load at
     all. `src/i18n.rs` builds the key from the code, and `tests/i18n.rs` enumerates `0..=99`.
  2. **`dir-n … dir-nnw` replaces `arrow-north|east|…`, and `arrow-none` is gone.** The arrow is
     charset-dependent (`art::wind_arrow` draws ASCII for `TERM=dumb`), so it stays in the renderer;
     the catalog owns the direction *name* (`NE` / `东北风`). An unknown direction renders `n/a`,
     which leaves `arrow-none` unreachable — and a key nothing can ask for is dead weight.
  3. **`I18n::format_temp` takes `TempUnit`, not `UnitSystem`.** Renderers hold a `ResolvedUnits`
     (`[units] temp` can override `defaults.units`), so the per-quantity override is honoured instead
     of being recomputed from the system. `I18n::load` takes the environment as a closure rather than
     `&Env`: the language is resolved before any HTTP client or cache exists, and a closure is the
     seam that keeps negotiation testable without `set_var`.
  4. **The `date-*` messages take `$day`/`$month-number` (zero padded) *and* `$day-plain`.** `en-US`
     writes `Thu 01 Oct` — step 07's snapshots must stay byte-identical — while `zh-CN` writes
     `9月30日`; `date-iso` renders ISO in every catalog. Which spelling a language uses is the
     catalog's decision, so both are passed.
  5. **`plain` no longer lowercases a day-part label and its record keys come from the catalog.**
     `to_lowercase()` is not a localization strategy (it mangles a language with capitalised nouns);
     the label is printed as the catalog spells it. The record keys (`location:`, `updated:`, …) are
     `label-*` messages, lowercased by the renderer, so the greppable shape of step 08 is unchanged
     and a translation can rename a key without inventing a second format.
- 2026-09-30 — `LanguageId` shipped as an enum of the shipped catalogs (with `unic-langid` parsing
  every tag) rather than a newtype over `LanguageIdentifier`: it is `Copy`, `tag()` is free, and an
  enum cannot name a language this binary has no messages for. `MessageKey` is the newtype that keeps
  a renderer from inventing a key at a call site.
- 2026-09-30 — `tests/common/mod.rs` pins `LC_ALL=C.UTF-8` and clears `LANG`/`LC_MESSAGES`/
  `LC_CTYPE` for every sandboxed run, because `auto` now negotiates from the ambient locale and a
  test suite that inherited the developer's `LANG` would render in the developer's language. A test
  that is about a locale sets it itself.
- 2026-09-30 — known limit: a Chinese compass name (`东北风`, six display columns) does not fit the
  thirteen column metric cell of the `art-table` columns layout, so it degrades to `↗ 7.0km/h` there.
  That is step 07's documented ladder (cardinal dropped before the line is clipped), the arrow still
  names the sector, and the stacked layout and the current block print the full name.
- 2026-09-30 — `src/geo/mod.rs` needed no change: the two credit lines a place carries are the
  upstream licences' own words (CC-BY-4.0/ODbL ask for the credit, a translator rewriting it would put
  the attribution at risk), and the plan's own out-of-scope list rules out displaying any other
  upstream text. `label-data` is what a locale may translate in those lines, and `zh-CN` does.
- 2026-09-30 — `zh-CN` is hand-written without a native reviewer: `cond.53`=`小雨` follows the
  mainland convention of calling light drizzle "light rain", and `date-short` uses `9月30日 周三`
  (weekday last). A reviewer pass is still owed, as the risks section says.
- 2026-09-30 — manual smoke run against the live APIs (all of the file's `## Verification` commands):
  `--lang zh-CN` printed `天气报告： Beijing, Beijing Municipality, China (39.91, 116.40)` with aligned
  borders and `今天 9月30日 / 10月1日 周四` columns; `LANG=zh_TW.UTF-8 -v` printed
  `i18n: requested zh_TW.UTF-8 → selected zh-CN (chain zh-TW → zh-CN → en-US)`;
  `--lang zh-TW -v` printed `i18n: requested zh-TW → selected zh-CN (chain zh-TW → zh-CN → en-US)`;
  `--lang bad-TAG` warned `warning: unsupported language "bad-TAG", falling back to en-US` on stderr and
  exited 0 with English output; `-f json --lang zh-CN` carried `{"code": 0, "text": "晴"}` and
  `{"code": 2, "text": "多云"}` per part with `schema_version: 1` untouched;
  `-f one-line --template @full --lang zh-CN` printed
  `Beijing: *o* 晴 +17°C (+12°C) ↖ 11km/h 西北风 11% 0.0mm 1022hPa 无数据 17km 0 06:09 17:59 Asia/Shanghai`;
  `-f plain --lang zh-CN` printed the `地点:`/`更新:`/`当前:`/`逐日`/`来源:` records.
- 2026-09-30 — `en-US` snapshots verified byte-identical (`git status tests/snapshots/` shows only
  the five new `zh` files); `tests/i18n.rs` (11 tests) covers catalog completeness, key-set equality,
  bundle resolution with a full argument set, negotiation from `--lang`/env/config, the `zh-TW` chain,
  the `bad-TAG` warning with exit 0, `-q` silence, and the three document formats in Chinese.
