<!--
SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Step 07 — Art table renderer

Status: ✅ done
Depends on: 03, 06
Touches: src/render/{mod,art,art_table,color,plain}.rs, src/main.rs, Cargo.toml, REUSE.toml, tests/render_snapshots.rs,
tests/render_width.rs, tests/snapshots/, tests/fixtures/report/, tests/common/mod.rs, docs/plans/README.md

## Goal
Turn a canonical `Report` into the wttr.in-style coloured table: a location header, a current-conditions
block (art + temperature + wind arrow + humidity/precipitation/visibility/pressure), and one boxed column
per day with four `Morning/Noon/Evening/Night` rows joined horizontally, laid out so that no emitted line is
wider than the resolved terminal width. The step also lands the `Renderer`/`RenderContext` plumbing, the
256-colour palette, the re-authored condition art corpus (unicode + ASCII) and deterministic snapshot coverage.

## Deliverables
- ✅ `src/render/mod.rs` (extend the module step 06 created — do **not** redeclare the trait or
      `RenderContext`): the two contract fields still missing, `lang: LanguageId` and `i18n: &'a I18n`, were added
      to the existing `RenderContext<'a>` together with the width/colour resolution; the renderer itself lives in
      `art_table.rs` (below).
- ✅ `src/i18n.rs` (not listed in `Touches`, added because the contract puts `ctx.i18n` in
      `RenderContext`): `LanguageId` + `I18n` with the `part-*`/`label-*` messages, condition text through the WMO
      table and `date_short` (`Today, Sep 30` / `Tue 30 Sep`). `auto`, `en` and `en-US` resolve to the built-in
      language; any other tag is refused with exit 2 instead of silently rendering English. Step 09 replaces the
      table with Fluent catalogs and adds the environment negotiation — no renderer changes then.
- ✅ `src/render/mod.rs`: `Format` gains `ArtTable`, `OneLine`, `Json`, `Dumb` on top of step 06's `Plain`,
      with `FromStr`/`as_str`/`ALL: &[Format]` completed; step 08 only maps the clap `--format` value onto it.
- ✅ `src/render/mod.rs`: `renderer_for(format, caps) -> Box<dyn Renderer>` gains the `art-table` and `dumb`
      arms (the `one-line`/`json` arms land in step 08, not as stubs here).
- ✅ `src/render/mod.rs`: `TermCaps { is_tty, term: TermKind, utf8: bool, depth: ColorDepth,
      color_pref: ColorPreference }`. The two step 06 booleans were **replaced**, not extended: `term`/`depth` say
      what `dumb`/`color` said with one representation per fact, and `NO_COLOR`/`CLICOLOR_FORCE` collapsed into one
      three-state `ColorPreference` because the variables interact rather than coexist. `read()` takes the
      environment as a parameter (edition 2024 makes `std::env::set_var` unsafe, so an injectable seam is the only
      way the rules stay testable) and is built from `IsTerminal::is_terminal(stdout)`, `TERM`,
      `LC_ALL`/`LC_CTYPE`/`LANG` (UTF-8 iff the value contains `utf-8` or `utf8`, case-insensitive) and
      `COLORTERM` (`truecolor`/`24bit` ⇒ `Ansi256`, the deepest palette this step implements).
- ✅ `src/render/mod.rs`: `fn resolve_width(explicit: Option<usize>) -> Width` implementing
      `--width`/`[render] width` > `COLUMNS` > `tcgetwinsize` > `80`; unparsable/zero sources are skipped, a
      result below `20` is raised to `20` with a `--verbose` note, and the result carries
      `{ columns, source, raised_from }` so that note can name the source — a bare `usize` cannot.
      Add `rustix = { version = "1", features = ["termios"] }` to `[dependencies]` (justification below).
- ✅ `src/render/mod.rs`: `fn resolve_color(mode: ColorMode, caps: &TermCaps) -> ColorMode` implementing the
      ladder: `always` wins over everything (escapes are emitted even into a pipe), `never` wins over
      everything except nothing, then `NO_COLOR` (present with any value, including empty) disables,
      `CLICOLOR_FORCE` set and not `"0"` enables, `TERM=dumb` disables, non-tty stdout disables, else enable;
      when both `NO_COLOR` and `CLICOLOR_FORCE` are set, `CLICOLOR_FORCE` wins. Unit-tested as a truth table.
- ✅ `src/render/art.rs`: `pub struct Block { unicode: [&str; 4], ascii: [&str; 4], style: ArtStyle }`,
      `pub fn art(key: &str) -> Option<&'static Block>` (with `n/a` accepted as a spelling of `unknown`),
      `pub fn one_line_art(key: &str) -> &'static str` (the 3 column ASCII glyph shared by both charsets, used by
      the stacked layout and by `%c` in step 08), `pub fn night_variant(key: &str) -> &str` and
      `pub fn wind_arrow(deg: u16, charset) -> &'static str`. The corpus owns its own geometry (`ART_W`,
      `ART_LINES`, `NO_BLOCK`), which `art_table.rs` imports. The keys are exactly the vocabulary step 03's
      `Condition::art_key()` returns — art.rs consumes it and never defines a second `art_key()`.
- ✅ Art corpus: **33 blocks** — the 29 keys the WMO table actually describes (not ~23), the three sky night
      siblings the night rule needs (`clear-night`, `mainly-clear-night`, `partly-cloudy-night`: a mainly clear
      *night* must not draw a daytime sun) and `unknown`. Each has a unicode and an ASCII block of exactly four
      lines, stored **right-trimmed** and at most `ART_W = 7` display columns wide; the renderer does the
      padding, and the unit test asserts the bound in both charsets.
- ✅ Coverage assertion (the anti-drift guard with step 03): a test iterates every code `0..=99`, calls
      `Condition::art_key()`, and asserts `art(key).is_some()` and `one_line_art(key)` is non-empty — so a new
      condition key in step 03 fails the build until art exists for it.
- ✅ The 23 unicode blocks are authored by hand in this commit (see `## Risks` on provenance). The ASCII
      blocks are a 7-bit transcription of the same shapes (e.g. clear day `"   \\ | /"`, `"  - ( ) -"`,
      `"   / | \\"`, `"         "`); a unit test asserts every ASCII line is in `0x20..=0x7E` or a trailing pad.
- ✅ `src/render/art.rs`: a unit test asserts the block table is total over the vocabulary (no key without a
      block, no block without a key) and that every unicode block is exactly four lines of ≤ `ART_W` columns.
- ✅ `src/render/color.rs`: `temp_fg(temp_c: f32) -> u8` from stops
      `-20→21, -10→27, 0→51, 5→45, 10→47, 15→118, 20→226, 25→214, 30→208, 35→196` (nearest lower stop, clamped
      at both ends), `precip_fg(mm: f32) -> u8` (`0 →` default, `>0 → 39`, `>=2.5 → 33`, `>=7.5 → 27`),
      `wind_fg(kmh: f32) -> u8`, `humidity_fg(pct: u8) -> u8`, `art_fg(style: ArtStyle) -> u8` and its
      `condition_fg(c: Condition) -> u8` (sun 220, cloud 250, rain 33, snow 255, thunder 129, fog 245),
      `ansi16_from_256(n: u8) -> u8` for `ColorDepth::Ansi16`, and
      `paint(text: &str, fg: u8, depth: ColorDepth) -> Cow<'_, str>` emitting `\x1b[38;5;<n>m…\x1b[0m` while
      returning the input borrowed (zero allocation) when colour is disabled or the text is empty — `Mono`
      short-circuits before any formatting. The depth is a parameter because it decides the escape: the same
      function serves the 256 colour palette and its ANSI fold.
- ✅ `src/render/art_table.rs`: layout constants `GAP = 1`, `METRICS_W = 13`, `METRICS_W_NARROW = 10`,
      `CELL_W = ART_W + GAP + METRICS_W = 21`, `CELL_W_NARROW = 18`, `PAD = 1` (`ART_W`/`ART_LINES` come from the
      corpus) plus the thresholds `WIDE_FROM = 74`, `CELLS_PER_ROW = 3` and `STACKED_BELOW = 60`.
- ✅ `src/render/art_table.rs`: header line `Weather report: <display name> (<lat>, <lon>)` followed by the
      current-conditions block — four art lines with, to the right, the condition text, `+22°C (+23°C)`
      temperature/feels-like, `↗ 12km/h NE` wind arrow + speed + cardinal direction on line 3, and
      `56% 1013hPa 10km 0.0mm` (humidity, pressure, visibility, precipitation) on line 4.
- ✅ `src/render/art_table.rs`: day columns with `cells_per_row = clamp((width - 1) / (cell_w + 3), 1, 3)`,
      `cell_w = CELL_W` when `width >= 74`, else `CELL_W_NARROW`; 7 days render as 3 + 3 + 1 column rows,
      `--days 1` renders exactly one column, and cells in one screen row are joined with `│`, rows separated
      by `├───┼───┤` runs of `─` sized from the same cell width.
- ✅ `src/render/art_table.rs`: day-part cell contract (documented in the module doc comment and asserted by
      a unit test) — exactly four lines per part: `art[k]` in columns `0..7`, then the localized part label
      (`part-morning`), `+22°C (+23°C)`, `↗ 12km/h NE`, `0.0mm 56%`; the one-line day header above the four
      rows is the localized `date-short` message (`Tue 30 Sep`, `Today, Sep 30` for day 0).
- ✅ `src/render/art_table.rs`: stacked layout for `width < 60` — one section per day (blank line, day
      heading, then four `Morning │ <art> │ <temp> │ <wind> │ <precip>` lines), same content, no horizontal
      joining. It degrades by value rather than by luck: the ladder drops the humidity, the apparent temperature,
      the cardinal direction (the arrow already names the sector) and finally the glyph before anything is
      truncated.
- ✅ `src/render/art_table.rs`: `dumb` mode = ASCII charset + box drawing `+ - |` + forced `ColorMode::Never`;
      selected explicitly by `Format::Dumb` and automatically (with a `--verbose` note) when
      `caps.term == TermKind::Dumb` or `!caps.utf8`.
- ✅ Width invariant: every emitted line goes through `fn fit(line: &str, width: usize, charset: Charset) -> String`, which
      measures with `unicode_width::UnicodeWidthStr`, copies escape sequences verbatim (they take no columns)
      and closes an unterminated colour before the ellipsis, truncates on a char boundary (`…` in unicode,
      `...` in ASCII), and is backed by `debug_assert!` plus the property test in `tests/render_width.rs` over
      widths `20..=200` × days `1|3|7` in both charsets. Metrics are sized to their column before the row is
      built, so a truncation can only ever drop a border at widths below the layout's minimum, never half an art
      block.
- ✅ `tests/common/mod.rs`: `fn fixture_report(name: &str) -> Report` loading `tests/fixtures/report/*.json`
      via `serde_json` (added by step 03/06) and fixing `now`/`tz` so snapshots never depend on the clock.
- ✅ `tests/render_snapshots.rs` + `tests/snapshots/`: 18 committed `insta` snapshots
      (`prepend_module_to_snapshot = false`, in the default `tests/snapshots` directory next to the test) —
      metric and us units at widths 80/60/40 with days 1/3/7, the two coloured cases, day vs night, `dumb`,
      current-only in both unit systems, a week of every condition and the `art_gallery` of all 33 blocks. Every
      one of them was reviewed by eye. CI runs `INSTA_UPDATE=no`, so drift fails instead of silently
      rewriting.
- ✅ Locale-independence test: the real binary, offline over a seeded cache, renders the same fixture
      byte-identically under `LANG=C.UTF-8`, `LANG=zh_CN.UTF-8` and `LC_ALL=tr_TR.UTF-8` — pins that the language
      comes from the configuration and that no float or date formatting leaks in from the ambient locale. The
      plan's `LANG=C` run is asserted separately, because there the *character set* legitimately switches to
      ASCII: that run must be ASCII and keep the layout (same line count). `--lang` itself is step 08's flag; the
      test drives `defaults.language`.
- ✅ REUSE: `[[annotations]]` entries for `tests/snapshots/**` and `tests/fixtures/report/**` (insta `.snap`
      files and JSON cannot carry comment headers); the `docs/plans/README.md` rendering contract now names the
      `rustix::termios::tcgetwinsize` tier, the 20 column minimum, the `CLICOLOR_FORCE`-over-`NO_COLOR` rule and
      the `dumb` charset switch.
- ✅ `Cargo.toml`: `rustix = { version = "1", features = ["termios"] }`, `unicode-width = "0.2"` and the
      dev-dependency `insta`, each justified in the design notes below.

## Design notes
- Width tier 3: the README lists `ioctl(TIOCGWINSZ)`; it is called through `rustix::termios::tcgetwinsize`
  instead of hand-written `libc` FFI because `[lints.rust] unsafe_code = "forbid"` in `Cargo.toml` cannot be
  overridden locally and `libc::ioctl` needs `unsafe`. `rustix` 1.1 + `termios` is a thin, dependency-light
  syscall wrapper exposing the same request as safe code; `crossterm`/`terminal_size` were rejected as 10× the
  code for one call, and `COLUMNS`-only was rejected because most shells do not export it, pinning every run to
  80 columns. Non-Unix targets compile the `rustix` tier out (`#[cfg(unix)]`) and fall back to `COLUMNS` → 80.
- `Condition::art_key()` is step 03's (`src/model/condition.rs`, `&'static str` vocabulary) and is **not**
  redeclared here: `render/art.rs` maps that vocabulary to blocks and asserts total coverage over `0..=99`, so
  the two steps cannot drift and a second inherent method cannot collide.
- `dumb` is not a separate renderer: it is `ArtTableRenderer { charset: Charset::Ascii, color: ColorMode::Never }`,
  so the layout math, the width invariant and the tests exist once.
- `unicode-width` is required to size boxes by display columns; without it `zh-CN` labels (step 09) misalign
  every border and the width invariant becomes unprovable.
- Dependency licences and MSRV, as the policy requires: `rustix` (MIT OR Apache-2.0), `unicode-width`
  (MIT OR Apache-2.0) and `insta` (Apache-2.0) are all GPL-3.0-or-later compatible, none of them pulls TLS or
  an async runtime, and all three build on the pinned Rust 1.85 toolchain. `insta` is dev-only and never
  linked into the binary; the `filters` feature is deliberately not enabled because no snapshot is filtered.
- `insta` is justified over hand-rolled string equality: multi-line table diffs are unreadable without
  structured snapshots, and the review workflow plus `INSTA_UPDATE=no` in CI makes accidental rewrites
  impossible. It is dev-only and never linked into the binary.
- Night handling and truncation order are fixed: art selection uses the report's own `is_day`/`DayPart::Night`
  data (never the wall clock), and truncation drops the metrics tail first, never the art.

## Out of scope
- `one-line`, `plain` and `json` bodies (step 08 mints only the `Format` variants and the dispatch arms).
- Any translation of labels/conditions: step 09 supplies the catalogs; here the strings are en-US literals
  fetched through `ctx.i18n` lookups that step 09 fills in (the lookup indirection exists now, the catalogs do not).
- Moon phase art and `%m` (roadmap section of step 14), UV-aware art variants, 24-bit truecolor, sixel/image
  output, animated art, per-provider art skins and `--art=<file>` overrides (never in v1).

## Verification
Fixtures (canonical `Report` JSON, loaded by `tests/common/mod.rs::fixture_report`):
`tests/fixtures/report/beijing-3d-day.json`, `beijing-1d.json`, `beijing-7d.json`, `beijing-night.json`,
`current-only.json` (no `days`, provider with `daily == false`), `all-conditions.json` (23 conditions × 4 parts,
feeds `art_gallery`). Live upstream payload fixtures stay owned by step 06.

Manual smoke run (hits the network once; `--verbose` prints provider/width/colour resolution):
```
cargo run --release -- --provider open-meteo --days 3 --width 80 Beijing | awk '{print length}' | sort -rn | head -1   # <= 80
cargo run --release -- --days 3 --color always Beijing | cat -v | grep -c '38;5;'                                    # > 0 when piped
COLUMNS=40 cargo run --release -- --days 3 Beijing                                                                   # stacked, <= 40 cols
TERM=dumb cargo run --release -- --days 3 Beijing | LC_ALL=C grep -c '[^ -~]'                                       # 0
cargo test --test render_snapshots -- --nocapture                                                                    # snapshots match, INSTA_UPDATE=no
```

## Exit criteria
- ✅ `cargo fmt --check` / `cargo clippy --all-targets -- -D warnings` / `cargo test` / `reuse lint` all clean
- ✅ `--days 3 --width 80` prints three boxed day columns with no line wider than 80 display columns; `--days 1`
      prints exactly one column; `--days 7` wraps as 3 + 3 + 1
- ✅ `--color always` piped through `cat -v` shows `38;5;` escapes; `NO_COLOR=1` with `--color always` still
      shows them; `--color never` shows none
- ✅ `COLUMNS=40` degrades to the stacked layout and `TERM=dumb` emits zero bytes outside `0x20..=0x7E`
- ✅ `tests/snapshots/` holds the committed, hand-reviewed snapshots for units 2 × days 3 × width 3 × colour 2,
      day/night, dumb, current-only and `art_gallery`

## Risks
- Art provenance: all 23 unicode blocks are written in this commit; nothing may be traced to wego or wttr.in
  output. The blocks are drawn from the WMO 4677 description of each phenomenon, and the progress log records
  author and date so a later reviewer can audit the claim.
- Night detection relies on the provider reporting `is_day`; if a provider cannot, the table must use the day
  block rather than infer night from local time — pinned by `beijing-night.json`.
- Snapshot churn: one padding or palette tweak invalidates many snapshots. Mitigated by keeping geometry in
  `art_table.rs` and colours in `color.rs`, so an intended change is a two-file diff plus a reviewed refresh.
- Width edge cases (garbage `COLUMNS`, 1-column pty, very narrow `--width`) are covered by `resolve_width` unit
  tests and the `tests/render_width.rs` property test rather than by snapshots.

## Progress log
- 2026-09-30 — step file written; design decisions (width tier, art home, dumb-as-charset) recorded.
- 2026-09-30 — step executed and verified. The divergences from the list above are recorded where they
  happen: `TermCaps` replaced step 06's two booleans with `term`/`depth`/`color_pref`; `resolve_width` returns a
  `Width` struct because a `usize` cannot name its own source; `paint` takes the colour depth; the corpus is 33
  blocks (29 described codes + 3 night siblings + `unknown`) and stores right-trimmed lines; `src/i18n.rs` was
  added here because the contract puts `ctx.i18n` in `RenderContext`; `UnitStyle` and `format_temp_signed` were
  added to `src/model/units.rs` so the compact cell spelling still has exactly one conversion point; the stacked
  layout drops secondary values before it truncates.
- 2026-09-30 — art provenance: all 33 blocks were drawn in this session (2026-09-30) from the WMO 4677
  descriptions of the phenomena — a sun with rays, a cloud that grows with the cloud cover, drops that get denser
  with the precipitation, stars for snow, a bolt for thunder and bars for fog. Nothing is traced to wego,
  wttr.in or any other client's artwork, and `art_gallery.snap` is the reviewable record of the whole corpus.
- 2026-09-30 — snapshot review: only two snapshots are coloured (metric days 3, us days 1); the rest of the
  colour matrix is covered by `colour_changes_no_layout`, which strips the escapes from every coloured render
  (units 2 × days 3 × width 3) and requires the monochrome bytes. Eighteen escape-only `.snap` files would be
  churn rather than coverage.
- 2026-09-30 — manual smoke run over live upstream data (Open-Meteo, Beijing, deps on the network cleared
  first): the widest line is 80 columns at `COLUMNS=80` (the credit line, clipped) and 40 at `COLUMNS=40`, which
  stacks; `CLICOLOR_FORCE=1` piped through `cat -v` shows 20 `38;5;` sequences, `NO_COLOR=1` shows none,
  `NO_COLOR=1 CLICOLOR_FORCE=1` shows them again, and `TERM=dumb` emits zero bytes outside `0x20..=0x7e`.
  `--width`/`--color` are step 08's flags, so the run reached the same resolution ladder through
  `COLUMNS`/`CLICOLOR_FORCE`/`NO_COLOR`/`TERM`; `-v` printed the resolved width, its source, the raised-from
  note, the palette and the language.
