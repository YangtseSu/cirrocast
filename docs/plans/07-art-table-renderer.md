<!--
SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Step 07 — Art table renderer

Status: not-started
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
- [ ] `src/render/mod.rs` (extend the module step 06 created — do **not** redeclare the trait or
      `RenderContext`): add the two contract fields still missing, `lang: LanguageId` and `i18n: &'a I18n`, to
      the existing `RenderContext<'a>` (`units, color, width, term, now, tz` already exist), and implement the
      art-table renderer plus width/colour resolution in the same module.
- [ ] `src/render/mod.rs`: `Format` gains `ArtTable`, `OneLine`, `Json`, `Dumb` on top of step 06's `Plain`,
      with `FromStr`/`as_str`/`ALL: &[Format]` completed; step 08 only maps the clap `--format` value onto it.
- [ ] `src/render/mod.rs`: `renderer_for(format, caps) -> Box<dyn Renderer>` gains the `art-table` and `dumb`
      arms (the `one-line`/`json` arms land in step 08, not as stubs here).
- [ ] `src/render/mod.rs`: extend step 06's `TermCaps { is_tty, color, dumb }` with `utf8: bool`,
      `term: TermKind` and `depth: ColorDepth` and test the extension `detect()`, built from `std::io::IsTerminal::is_terminal(std::io::stdout())`, `TERM`,
      `LC_ALL`/`LC_CTYPE`/`LANG` (UTF-8 iff the value contains `utf-8` or `utf8`, case-insensitive) and
      `COLORTERM` (`truecolor`/`24bit` ⇒ `Ansi256`, the deepest palette this step implements).
- [ ] `src/render/mod.rs`: `fn resolve_width(flag: Option<usize>) -> usize` implementing
      `--width` > `COLUMNS` > `tcgetwinsize` > `80`; unparsable/zero sources are skipped, a result below `20`
      is raised to `20` with a `--verbose` note, and the chosen source is recorded for `--verbose`.
      Add `rustix = { version = "1", features = ["termios"] }` to `[dependencies]` (justification below).
- [ ] `src/render/mod.rs`: `fn resolve_color(mode: ColorMode, caps: &TermCaps) -> ColorMode` implementing the
      ladder: `always` wins over everything (escapes are emitted even into a pipe), `never` wins over
      everything except nothing, then `NO_COLOR` (present with any value, including empty) disables,
      `CLICOLOR_FORCE` set and not `"0"` enables, `TERM=dumb` disables, non-tty stdout disables, else enable;
      when both `NO_COLOR` and `CLICOLOR_FORCE` are set, `CLICOLOR_FORCE` wins. Unit-tested as a truth table.
- [ ] `src/render/art.rs`: `pub struct Block { unicode: [&'static str; 4], ascii: [&'static str; 4],
      style: ArtStyle }`, `pub fn art(key: &str) -> Option<&'static Block>`,
      `pub fn one_line_art(key: &str) -> &'static str` (the 2–3 column glyph used by `%c` in step 08) and
      `pub fn night_variant(key: &'static str) -> &'static str` (day key → its `-night` sibling, used when the
      report's observation is at night). The keys are exactly the `&'static str` vocabulary that step 03's
      `Condition::art_key()` returns — art.rs consumes it and never defines a second `art_key()`.
- [ ] Art corpus: one block per key in that vocabulary (~23 shaped keys plus their `-night` siblings and the
      `unknown`/`n/a` block), each with a unicode and an ASCII block of exactly four lines, every line padded
      with spaces to `ART_W = 7` **display columns**; only `clear` and `partly-cloudy` need distinct night art.
- [ ] Coverage assertion (the anti-drift guard with step 03): a test iterates every code `0..=99`, calls
      `Condition::art_key()`, and asserts `art(key).is_some()` and `one_line_art(key)` is non-empty — so a new
      condition key in step 03 fails the build until art exists for it.
- [ ] The 23 unicode blocks are authored by hand in this commit (see `## Risks` on provenance). The ASCII
      blocks are a 7-bit transcription of the same shapes (e.g. clear day `"   \\ | /"`, `"  - ( ) -"`,
      `"   / | \\"`, `"         "`); a unit test asserts every ASCII line is in `0x20..=0x7E` or a trailing pad.
- [ ] `src/render/art.rs`: a unit test asserts the block table is total over the vocabulary (no key without a
      block, no block without a key) and that every unicode block is exactly four lines of ≤ `ART_W` columns.
- [ ] `src/render/color.rs`: `temp_fg(temp_c: f32) -> u8` from stops
      `-20→21, -10→27, 0→51, 5→45, 10→47, 15→118, 20→226, 25→214, 30→208, 35→196` (nearest lower stop, clamped
      at both ends), `precip_fg(mm: f32) -> u8` (`0 →` default, `>0 → 39`, `>=2.5 → 33`, `>=7.5 → 27`),
      `wind_fg(kmh: f32) -> u8`, `humidity_fg(pct: u8) -> u8`, `art_fg(c: Condition) -> u8` (sun 220, cloud
      250, rain 33, snow 255, thunder 129, fog 245), `ansi16_from_256(n: u8) -> u8` for `ColorDepth::Ansi16`,
      and `paint(s: &str, fg: u8) -> Cow<'_, str>` emitting `\x1b[38;5;<n>m…\x1b[0m` while returning the input
      borrowed (zero allocation) when colour is disabled — `Mono` short-circuits before any formatting.
- [ ] `src/render/art_table.rs`: layout constants `ART_W = 7`, `ART_LINES = 4`, `GAP = 1`, `METRICS_W = 13`,
      `METRICS_W_NARROW = 10`, `CELL_W = ART_W + GAP + METRICS_W = 21`, `CELL_W_NARROW = 18`, `PAD = 1`.
- [ ] `src/render/art_table.rs`: header line `Weather report: <display name> (<lat>, <lon>)` followed by the
      current-conditions block — four art lines with, to the right, the condition text, `+22°C (+23°C)`
      temperature/feels-like, `↗ 12km/h NE` wind arrow + speed + cardinal direction on line 3, and
      `56% 1013hPa 10km 0.0mm` (humidity, pressure, visibility, precipitation) on line 4.
- [ ] `src/render/art_table.rs`: day columns with `cells_per_row = clamp((width - 1) / (cell_w + 3), 1, 3)`,
      `cell_w = CELL_W` when `width >= 74`, else `CELL_W_NARROW`; 7 days render as 3 + 3 + 1 column rows,
      `--days 1` renders exactly one column, and cells in one screen row are joined with `│`, rows separated
      by `├───┼───┤` runs of `─` sized from the same cell width.
- [ ] `src/render/art_table.rs`: day-part cell contract (documented in the module doc comment and asserted by
      a unit test) — exactly four lines per part: `art[k]` in columns `0..7`, then the localized part label
      (`part-morning`), `+22°C (+23°C)`, `↗ 12km/h NE`, `0.0mm 56%`; the one-line day header above the four
      rows is the localized `date-short` message (`Tue 30 Sep`, `Today, Sep 30` for day 0).
- [ ] `src/render/art_table.rs`: stacked layout for `width < 60` — one section per day (blank line, day
      heading, then four `Morning│<art>│<temp>│<wind>│<precip>` lines), same content, no horizontal joining.
- [ ] `src/render/art_table.rs`: `dumb` mode = ASCII charset + box drawing `+ - |` + forced `ColorMode::Never`;
      selected explicitly by `Format::Dumb` and automatically (with a `--verbose` note) when
      `caps.term == TermKind::Dumb` or `!caps.utf8`.
- [ ] Width invariant: every emitted line goes through `fn fit(line: &str, width: usize) -> String`, which
      measures with `unicode_width::UnicodeWidthStr`, truncates on a char boundary with `…` (preferring to
      drop the metrics tail before the art), and is backed by `debug_assert!` plus the property test in
      `tests/render_width.rs` over widths `20..=200` × days `1|3|7`. Add `unicode-width = "0.2"`.
- [ ] `tests/common/mod.rs`: `fn fixture_report(name: &str) -> Report` loading `tests/fixtures/report/*.json`
      via `serde_json` (added by step 03/06) and fixing `now`/`tz` so snapshots never depend on the clock.
- [ ] `tests/render_snapshots.rs` + `tests/snapshots/`: `insta` snapshots (`snapshot_path` set to
      `snapshots`, `prepend_module_to_snapshot = false`) for metric vs us units, days 1/3/7, width 80/60/40,
      `--color always` vs `--color never`, day vs night, `TERM=dumb`, a current-only report (no `days`) and an
      `art_gallery` snapshot rendering all 23 blocks. Snapshots are committed and reviewed by hand in the PR
      (`cargo insta review`); CI runs `INSTA_UPDATE=no` so drift fails instead of silently rewriting.
- [ ] Locale-independence test: the same fixture rendered with `--lang en-US` under `LANG=C`,
      `LANG=zh_CN.UTF-8` and `LC_ALL=tr_TR.UTF-8` must be byte-identical — pins that step 07 emits en-US
      literals and that no float/date formatting leaks in from the ambient locale.
- [ ] REUSE: `[[annotations]]` entry for `tests/snapshots/**` in `REUSE.toml` (insta `.snap` files cannot
      carry comment headers); amend the `docs/plans/README.md` width-resolution line to name the `rustix`
      tier used here.
- [ ] `Cargo.toml`: dev-dependency `insta = { version = "1", features = ["filters"] }` plus the runtime
      additions above, each justified in the one-liners below.

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
- [ ] `cargo fmt --check` / `cargo clippy --all-targets -- -D warnings` / `cargo test` / `reuse lint` all clean
- [ ] `--days 3 --width 80` prints three boxed day columns with no line wider than 80 display columns; `--days 1`
      prints exactly one column; `--days 7` wraps as 3 + 3 + 1
- [ ] `--color always` piped through `cat -v` shows `38;5;` escapes; `NO_COLOR=1` with `--color always` still
      shows them; `--color never` shows none
- [ ] `COLUMNS=40` degrades to the stacked layout and `TERM=dumb` emits zero bytes outside `0x20..=0x7E`
- [ ] `tests/snapshots/` holds the committed, hand-reviewed snapshots for units 2 × days 3 × width 3 × colour 2,
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
