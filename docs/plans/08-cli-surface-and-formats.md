<!--
SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Step 08 — CLI surface and formats

Status: ✅ done
Depends on: 06, 07
Touches: src/cli.rs, src/main.rs, src/error.rs, src/render/{mod,one_line,plain,json}.rs, src/model/mod.rs,
src/provider/open_meteo.rs, src/i18n.rs, Cargo.toml, REUSE.toml, README.md, tests/cli_flags.rs,
tests/render_one_line.rs, tests/render_json.rs, tests/render_plain.rs, tests/snapshots/, tests/fixtures/report/

## Goal
Make the CLI the final documented contract — full flag matrix with its conflict rules, the precedence table
(flag > env > config > builtin), the exit codes in `--help` — and add the three remaining formats (`one-line`
tokens, `plain`, versioned JSON), shell completions and the man page, all from the same clap definitions.

## Deliverables
- ✅ `src/cli.rs`: `struct Cli` (clap derive) carrying the whole flag matrix, `enum Command { Config, Key,
      Provider, Cache, Location, Completion, Man }` and the positional `[LOCATION]`; subcommand *shapes* live
      here, their logic stays with the owning step (02/04/05/10/11).
- ✅ Flags with their env attributes: `-p/--provider <ID[,ID...]>` (`CIRROCAST_PROVIDER`), `-f/--format
      <art-table|one-line|plain|json|dumb>`, `-d/--days <0..=14>` (`CIRROCAST_DAYS`), `-u/--units
      <metric|us|uk>` (`CIRROCAST_UNITS`), `--lang <BCP-47|auto>` (`CIRROCAST_LANG`), `--lat/--lon <DEG>`,
      `--ip`, `--station <ICAO>`, `--template`, `--no-cache`, `--refresh`, `--offline`, `--timeout <SECS>`
      (`CIRROCAST_TIMEOUT`), `--color <auto|always|never>`, `--width <COLS>`, `-q/--quiet`, `-v/--verbose`,
      `-h`, `-V`, `--bin-name` (completion/man only). `-p open-meteo,smhi` parses to an ordered `Vec<ProviderId>`
      via `select`; unknown or duplicated ids are `Error::Usage` (exit 2); `auto` expands to `open-meteo`, plus
      `metar` when `--station` is given.
- ✅ Conflict rules — clap `requires`/`conflicts_with` where static, `validate_query` where the *source* of a
      value or the provider chain decides: `--lat` requires `--lon` and vice versa; `--lat/--lon`, `--ip` and
      `--station` are mutually exclusive in clap; a `LOCATION` argument conflicts with all three through
      `validate_query`, because clap counts an environment value as "present" (probed: `ArgMatches::contains_id`
      is true for `env`-backed args, so a static conflict would make `CIRROCAST_LOCATION` beat `--lat`);
      `--offline` conflicts with `--refresh` and with `--no-cache`; `-q` conflicts with `-v`; `--station` is
      valid only when the chain starts with `metar` or is `auto`, else `error: --station requires --provider
      metar (or auto)`, exit 2.
- ✅ `--days` clamping against the primary provider's `Capabilities::max_days`, printing exactly one
      `warning: <provider> supports at most <n> days; --days <m> clamped to <n>` on stderr (silenced by `-q`,
      never an error); `--days 0` means current conditions only. `request_days` returns the clamped value plus
      the warning, so the rule is unit-tested (the open-meteo horizon is 16 and the flag caps at 14, so with
      the only implemented backend the clamp cannot be reached from the command line — the same limitation
      step 06 recorded; step 10's shorter-horizon backends exercise it end to end).
- ✅ Precedence from the parse source, not from guesses: `Cli::command().get_matches()` in `main`, then
      `Sources::read(&matches)` classifies each setting as `CommandLine`, `Environment` or `Default`, and the
      typed `Cli` is built from the same matches (`Cli::from_arg_matches`). Merged flag > env > config >
      builtin (config never overrides an env value — clap's `env` attribute already resolves the first two
      tiers). The precedence table and the exit-code table are replicated in the `--help` epilog
      (`after_long_help`), and `-v` prints every setting with the tier that supplied it.
- ✅ `src/error.rs`: `Error::exit_code(&self)` — 0 success, 1 generic, 2 usage, 3 network/upstream, 4
      config/state, 5 location not found, 6 missing/invalid key; `main` prints `error: <msg>` plus the cause
      chain under `-v`, never panics, `--help`/`--version` exit 0, clap usage errors stay 2. The return type
      stays `u8` (not `i32`): `ExitCode::from` consumes a `u8`, and a process exit status is a `u8`; the
      documented values are unchanged.
- ✅ `src/render/one_line.rs`: `struct OneLine` and `fn expand(template, report, ctx) -> Result<String>` over
      the `TOKENS: &[(char, Token)]` table implementing every token below; `%m` renders the literal `n/a`;
      `warnings(template)` reports unknown tokens (pure, so the CLI prints them under `-v` and the renderer
      stays free of verbosity flags); a token whose value the provider lacks prints `n/a` too.
- ✅ `src/render/one_line.rs` escapes: `%%` → `%`; `\n`, `\t`, `\\` unescaped before expansion;
      `%{<text>}` emits `<text>` verbatim without token expansion (`\}` escapes a closing brace); unknown `%X`
      emits `%X` literally and is logged once under `-v` with its character position; a trailing lone `%` is
      literal; an empty or whitespace-only template is `Error::Usage` (exit 2). Presets via `--format one-line
      --template <TEMPLATE|@PRESET>`: `default` = `%l: %c %C %t (%f), %w, %h, %p, %P, %v`, `short` = `%c %t`,
      `full` = `%l: %c %C %t (%f) %w %h %p %P %m %v %u %S %s %Z`, `uv` = `%l: UV %U`, `sun` = `%l: sunrise %S
      sunset %s (%z %Z)`; no `--template` means `default`; an unknown `@name` prints the preset list and exits
      2; the list also appears in `--help`.
- ✅ `src/render/plain.rs`: box-free, ANSI-free, greppable output reusing the one-line value formatters (no
      second conversion path): `location: <name> (<lat>, <lon>) <tz>`, `updated: <ISO local time> <offset>`,
      `current: <condition> <temp> (feels <feels>) wind <speed> <dir> humidity <h>% precip <p>mm pressure
      <p>hPa visibility <v>km`, then one `day <YYYY-MM-DD>: morning <…> | noon <…> | evening <…> | night <…>`
      line per day and finally `attribution: <provider> <url>`. The width is deliberately *not* applied: a
      clipped record would silently drop values.
- ✅ `src/render/json.rs`: `serde_json` renderer emitting the schema below with `schema_version: 1`, keys
      always present (`null` for missing values, never omitted), `days` ascending from location-local today,
      ISO-8601 timestamps with the location offset, and canonical metric numeric suffixes (`_c`, `_kmh`, `_mm`,
      `_hpa`, `_km`, `_pct`), plus the stability promise (module doc and README).
- ✅ `cirrocast completion <bash|zsh|fish|elvish|powershell>` via `clap_complete::generate` and
      `cirrocast man` via `clap_mangen::Man`, both to stdout, both generated from the same `Cli::command()`,
      `--bin-name` defaulting to `cirrocast`; `Cargo.toml` gains only `clap_complete = "4.6"` and
      `clap_mangen = "0.3"` (both non-async, no TLS, GPL-3.0-or-later compatible).
- ✅ `README.md`: a `## Usage` section with the flag table, both epilog tables, the token list (`%m` note), the
      presets, the format table with the width rule, and the completion/man one-liners.
- ✅ `tests/cli_flags.rs`: `assert_cmd` matrix over every flag (short and long), every conflict rule (exit 2
      **and** the exact stderr fragment), the precedence tiers (flag/env/config/builtin), the exit-code table
      0/1/2/3/4/5/6, the one-line token warnings and `-q` suppression, and the completions/man smoke tests.
      The one-time `--days` warning is covered by the `request_days` unit test (see above); exit 6 is asserted
      on `Error::exit_code` because no key-requiring backend exists before step 10.
- ✅ `tests/render_one_line.rs` token-by-token tests (both unit systems, `%%`/unknown/`%{...}`/trailing-`%`
      escapes, every preset as a snapshot) and `tests/render_json.rs` (hand-reviewed `insta` snapshot, key-set
      completeness, byte-identity of `-u us` vs `-u metric`).

## Design notes
Token table (authoritative; `%c` comes from `render::art::one_line_art`, everything else from the canonical
model, converted only here):
| token | output | token | output |
|---|---|---|---|
| `%c` | condition art, day/night aware | `%d` `%D` | ISO date / `Wed 30 Sep` |
| `%C` | localized condition text | `%Z` `%z` | timezone name / `+0800` |
| `%t` `%f` | temp / feels-like, unit-converted | `%u` `%U` | UV `5` / `5 (moderate)` |
| `%w` | wind `↗ 12km/h NE` | `%S` `%s` | sunrise / sunset `06:05` |
| `%h` | humidity `56%` | `%l` `%L` | name / `39.90,116.40` |
| `%p` | precipitation `0.0mm` | `%m` | moon phase → `n/a` (roadmap) |
| `%P` | pressure `1013hPa` | `%v` | visibility `10km` |

JSON schema, version 1 (stable) — the emitted document of `tests/fixtures/report/beijing-1d.json` is the
authoritative copy (`tests/snapshots/json_beijing_1d.snap`):
```json
{
  "schema_version": 1,
  "location": { "name": "Beijing", "admin1": "Beijing", "country": "China", "country_code": "CN",
                "lat": 39.9042, "lon": 116.4074, "timezone": "Asia/Shanghai", "elevation_m": 44.0,
                "source": "geocoder" },
  "current": { "time": "2026-09-30T12:15:00+08:00", "condition": { "code": 1, "text": "Mainly clear" },
               "temp_c": 21.5, "feels_like_c": 22.0, "humidity_pct": 52, "precip_mm": 0.0,
               "pressure_hpa": 1015.0, "visibility_km": 14.0, "wind_kmh": 10.0, "wind_dir_deg": 30,
               "wind_gust_kmh": null, "cloud_cover_pct": 25, "uv_index": 5.0, "is_day": true },
  "days": [ { "date": "2026-09-30", "sunrise": "06:05", "sunset": "17:58", "min_c": 14.0, "max_c": 24.0,
              "parts": { "morning": { "condition": { "code": 0, "text": "Clear sky" }, "temp_c": 17.0,
                          "feels_like_c": 16.0, "precip_mm": 0.0, "precip_prob_pct": 10,
                          "humidity_pct": 58, "visibility_km": 12.0, "wind_kmh": 7.0, "wind_dir_deg": 100 },
                          "noon": { … }, "evening": { … }, "night": { … } } } ],
  "attribution": { "provider": "open-meteo",
                   "url": "https://api.open-meteo.com/v1/forecast",
                   "notice": "Open-Meteo.com (CC BY 4.0)",
                   "location_notice": "Location data based on GeoNames (CC-BY-4.0) via Open-Meteo — https://open-meteo.com/",
                   "retrieved_at": "2026-09-30T04:11:00Z" }
}
```
The document is built from `#[derive(Serialize)]` structs rather than `serde_json::Value`: `serde` writes the
fields in declaration order (so `schema_version` leads), and every `Option` is written as `null` (no
`skip_serializing_if`), which is what makes "keys always present" true by construction. `attribution.url` is
the request endpoint with the query string stripped — the service that answered — not the site homepage.

Conflicts are declared in clap wherever the value source cannot change the answer; the location-argument rules
live in `validate_query` because clap treats an `env`-backed argument as present for `conflicts_with`, which
would let `CIRROCAST_LOCATION` beat `--lat` instead of losing to it. Precedence is read from
`ArgMatches::value_source` (with the `env` feature enabled that is the only reliable way to tell an env value
from a flag value), used for the `-v` report and for that one rule.

`--format json` ignores `--units`, `--width` and `--color` (documented, and asserted byte-for-byte by the
renderer test). `plain` and `json` also ignore `--width` for the same reason `json` does: they are record
formats, and truncating a record deletes data instead of laying it out.

`clap_complete` is the official companion of the pinned clap line: five shells from one derive source of truth,
and hand-written completions would drift on the first flag rename. `clap_mangen` follows the same argument for
the man page (official, tiny, compile-time only). Both are reached only from `completion`/`man`.

`println!` is not used for output any more: it panics when the reader is gone, and `cirrocast … | head` (and
every smoke run in this repo) closes the pipe early. `print_line`/`print_text` write through
`io::Write` and treat `BrokenPipe` as success; `completion`/`man` write into a sink with the same rule.

### Corrections to the step file, made while implementing it
* `%u`/`%U` (and the JSON `current.uv_index` the schema already specified) need a UV reading the canonical
  model did not have. Step 16 assumed step 06 had filled it; it had not. `Current` gains
  `uv_index: Option<f32>`, the Open-Meteo request and decoder carry it, the four recorded forecast fixtures
  were re-recorded with it (`REUSE.toml` states the new observation time), and the six hand-written report
  fixtures gained a value.
* `@uv` was specced as `%l: UV %u (%U)`, which doubles the band parentheses because `%U` already includes
  them; the preset is `%l: UV %U`.
* `Error::exit_code` keeps the `u8` return type (see the deliverable above).
* The sample JSON schema carried `parts.<part>.pressure_hpa`, which the canonical model has no per-part value
  for; the emitted schema drops it. `days[].parts` is always an object per part — the model cannot represent a
  missing part — with the keys nullable for consumers that read documents from other producers.
* `one_line::expand` is the documented signature; the warnings for unknown tokens come from the separate pure
  `one_line::warnings`, so the renderer never consults `-v`/`-q` (a renderer may not read the environment).
* The step named four CLI-level stubs under `tests/fixtures/cli/`; the exit-code test uses the fixtures that
  already existed (`config/bad-syntax.toml`, `geo/open_meteo_geocode_no_hits.json`) plus a seeded cache, so no
  duplicate fixture directory was added.

## Out of scope
- Translating `--help`, clap usage errors, warnings or log lines: step 09 localizes the weather vocabulary and
  renderer labels only; clap's own strings stay English.
- The bodies of `config`, `key`, `cache`, `location` (steps 02/04/05) and the `provider info` content (step 10):
  this step defines their clap shapes and dispatch only. `--station` is validated here and reaches the backend
  in step 11 (until then a `metar` chain fails with `provider `metar` is not implemented yet`).
- `--format` plugins, `--output <file>`, jq-style filtering, `--csv`, TUI modes and installing
  completions/man pages into distro directories (step 13).

## Verification
Fixtures: `tests/fixtures/report/{beijing-3d-day,beijing-1d,beijing-night,current-only}.json` (the renderer
fixtures, now carrying `uv_index`) and the CLI-level failure sources `tests/fixtures/config/bad-syntax.toml`,
`tests/fixtures/geo/open_meteo_geocode_no_hits.json` and `…_ambiguous.json`, driven through a seeded cache so
no test opens a socket.

Manual smoke run — on 2026-09-30 with `COLUMNS=120 TERM=xterm-256color LANG=en_US.UTF-8`, a throwaway XDG
sandbox and direct TLS to Open-Meteo:

```sh
tmp=$(mktemp -d); export XDG_CONFIG_HOME=$tmp/config XDG_CONFIG_DIRS=$tmp/system \
  XDG_CACHE_HOME=$tmp/cache XDG_DATA_HOME=$tmp/data COLUMNS=120 TERM=xterm-256color LANG=en_US.UTF-8
cirrocast --help | sed -n '/PRECEDENCE/,/EXIT CODES/p'   # both tables present, in full
cirrocast Beijing                                        # art-table: 3 days × 4 parts + credits
cirrocast -f one-line --template '@full' Beijing
# Beijing: *o* Clear sky +18°C (+13°C) \ 13km/h NW 11% 0.0mm 1021hPa n/a 17km 0 06:09 17:59 Asia/Shanghai
cirrocast -f one-line --template '%l:%{%}%c %t' Beijing  # 39.9…: % *o* +18°C — the literal % survives
cirrocast -f one-line --template @uv Beijing             # Beijing: UV 0 (low)
cirrocast -f plain Beijing                               # location/updated/current/3 day lines/credits
cirrocast -f json Beijing | jq -r '.schema_version'      # 1
cirrocast -f json -u us Beijing | cmp - <(cirrocast -f json Beijing)   # identical bytes (exit 0)
cirrocast -f plain -u us Beijing | sed -n 3p             # current: Clear sky 64°F (feels 55°F) …
cirrocast -f plain --days 0 Beijing                      # header, updated, current and the credits only
cirrocast -f dumb Beijing | head -4                      # ASCII art, no escapes
cirrocast --color always Beijing | cat -v | grep -c '38;5;'  # 20 when piped
cirrocast --lat 39.9 Beijing; echo $?                    # 2, clap: --lon <DEG> is required
cirrocast --station ZBAA -p open-meteo; echo $?          # 2, "--station requires --provider metar (or auto)"
cirrocast completion bash | head -5                      # "_cirrocast() {", exit 0 despite the closed pipe
cirrocast man | head -3                                  # roff with ".TH cirrocast 1"
cirrocast man --bin-name weather | head -3               # ".TH weather 1"
```

## Exit criteria
- ✅ `cargo fmt --check` / `cargo clippy --all-targets -- -D warnings` / `cargo test` / `reuse lint` all clean
- ✅ `--help` prints the flag matrix and both tables; every conflict rule exits 2 with its documented message;
      `--days` clamps once (unit-tested: no implemented backend has a horizon below the flag's cap)
- ✅ `one-line` renders every token (including `%%` and `%{...}`), all five presets and all three unit systems,
      `%m` = `n/a`
- ✅ `json` snapshot matches, carries `schema_version: 1`, and is byte-identical under `-u metric` and `-u us`
- ✅ `completion bash|zsh|fish|elvish|powershell` and `man` produce non-empty output and exit 0
- ✅ README `## Usage` documents flags, precedence, exit codes, tokens and presets

## Risks
- clap/env precedence bugs hide easily: mitigated by driving the merge from `value_source` and one test per tier
  (flag > env > config > builtin), asserting both the winning value and the tier the `-v` report prints.
- wttr.in compatibility drift: only the documented token subset is implemented, unknown tokens are logged and
  never invented; the token table above is the authority and is unit-tested token by token.
- Renaming a JSON key would silently break consumers: the completeness assertions (key set + unit suffixes) plus
  the snapshot make any rename fail loudly.
- `clap_complete`/`clap_mangen` skew is handled by pinning to the same clap line (`4.6`/`0.3`, both bumped
  together); `README.md` is shared with other steps, so this step appended to `## Usage` only.

## Progress log
- 2026-09-30 — step file written; flag matrix, precedence mechanism, token table and JSON schema recorded.
- 2026-09-30 — UV end to end, before the CLI work, because three deliverables depend on it: `Current` gains
  `uv_index`, Open-Meteo requests and decodes it, the four recorded forecast fixtures were re-recorded from
  `historical-forecast-api` with the widened parameter list (`REUSE.toml` updated: observation at 2026-09-30
  11:30Z), the provider test's current-block assertions moved with it, and the six report fixtures gained the
  field so the renderers have a value to print.
- 2026-09-30 — renderers landed: `render/one_line.rs` (token table, escapes, presets, pure `warnings`),
  `render/json.rs` (serde structs in declaration order, `null` for missing values), `plain` rewritten to the
  documented record lines with the width deliberately ignored, `renderer_for` extended with the template and
  refusing it outside `one-line`, and the five UV band labels in the catalog.
- 2026-09-30 — CLI landed: the full flag matrix with env attributes, clap conflicts for the static rules and
  `validate_query` for the source-dependent ones, `Sources`/`Source` read from `ArgMatches::value_source` with
  a `-v` report per setting, `provider_chain` (`auto` + `metar` for a station) and `request_days`,
  `completion`/`man` from the same `Cli::command()`, and the `after_long_help` epilog carrying the precedence,
  exit-code and token tables. Output goes through `print_line`/`print_text`/`StdoutSink`, so a closed pipe is
  no longer a panic.
- 2026-09-30 — tests and docs: `tests/cli_flags.rs` (14 cases), `tests/render_one_line.rs` (5 presets as
  snapshots), `tests/render_json.rs` (snapshot + 89 key paths + unit-independence), `tests/render_plain.rs`
  rewritten for the new record shape, README `## Usage` rewritten with the flag/precedence/exit-code/format/
  token tables, and the rendering contract in `docs/plans/README.md` scoped to the table formats.
- 2026-09-30 — step done. Gates: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`
  (22 binaries, 0 failures) and `reuse lint` clean; the smoke run above was executed and observed against the
  live Open-Meteo API, including the byte-identical `-u us` JSON comparison.
