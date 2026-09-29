<!--
SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Step 08 — CLI surface and formats

Status: not-started
Depends on: 06, 07
Touches: src/cli.rs, src/main.rs, src/error.rs, src/render/{mod,one_line,plain,json}.rs, Cargo.toml, README.md,
tests/cli_flags.rs, tests/render_one_line.rs, tests/render_json.rs, tests/snapshots/, tests/fixtures/report/

## Goal
Make the CLI the final documented contract — full flag matrix with its conflict rules, the precedence table
(flag > env > config > builtin), the exit codes in `--help` — and add the three remaining formats (`one-line`
tokens, `plain`, versioned JSON), shell completions and the man page, all from the same clap definitions.

## Deliverables
- [ ] `src/cli.rs`: `struct Cli` (clap derive) carrying the whole flag matrix, `enum Command { Config, Key,
      Provider, Cache, Location, Completion, Man }` and the positional `[LOCATION]`; subcommand *shapes* live
      here, their logic stays with the owning step (02/04/05/10/11).
- [ ] Flags with their env attributes: `-p/--provider <ID[,ID...]>` (`CIRROCAST_PROVIDER`), `-f/--format
      <art-table|one-line|plain|json|dumb>`, `-d/--days <0..=14>` (`CIRROCAST_DAYS`), `-u/--units
      <metric|us|uk>` (`CIRROCAST_UNITS`), `--lang <BCP-47|auto>` (`CIRROCAST_LANG`), `--lat/--lon <DEG>`,
      `--ip`, `--station <ICAO>`, `--no-cache`, `--refresh`, `--offline`, `--timeout <SECS>`
      (`CIRROCAST_TIMEOUT`), `--color`, `--width <COLS>`, `-q/--quiet`, `-v/--verbose`, `-h`, `-V`,
      `--bin-name` (completion/man only). `-p open-meteo,smhi` parses to an ordered `Vec<ProviderId>` via
      `ProviderId::from_str`; unknown or duplicated ids are `Error::Usage` (exit 2); `auto` expands to
      `open-meteo,smhi`, plus `metar` appended when `--station` is given.
- [ ] Conflict rules — clap `requires`/`conflicts_with` where static, `validate(&Cli) -> Result<()>` where
      provider-dependent: `--lat` requires `--lon` and vice versa; `--lat/--lon`, `--ip`, `--station` and
      `LOCATION` are mutually exclusive; `--offline` conflicts with `--refresh` and with `--no-cache` (nothing
      could be served); `-q` conflicts with `-v`; `--station` is valid only when the chain starts with `metar`
      or is `auto`, else `error: --station requires --provider metar (or auto)`, exit 2.
- [ ] `--days` clamping against the primary provider's `Capabilities::max_days`, printing exactly one
      `warning: <provider> supports at most <n> days; --days <m> clamped to <n>` on stderr (silenced by `-q`,
      never an error); `--days 0` means current conditions only.
- [ ] Precedence from the parse source, not from guesses: `Cli::command().get_matches()` then
      `matches.value_source("<id>")` classifies each setting as `CommandLine`, `EnvVariable` or `DefaultValue`,
      merged flag > env > config > builtin (config never overrides an env value). The precedence table and the
      exit-code table are replicated verbatim in the `--help` epilog (`after_long_help`).
- [ ] `src/error.rs`: `Error::exit_code(&self) -> i32` — 0 success, 1 generic, 2 usage, 3 network/upstream,
      4 config/state, 5 location not found, 6 missing/invalid key; `main` prints `error: <msg>` plus the cause
      chain under `-v`, never panics, `--help`/`--version` exit 0, clap usage errors stay 2.
- [ ] `src/render/one_line.rs`: `struct OneLineRenderer` and `fn expand(template: &str, report: &Report,
      ctx: &RenderContext<'_>) -> Result<String>` over a `&'static [(char, Token)]` table implementing every
      token in the table below; `%m` renders the literal `n/a` in this step and until the moon-phase roadmap
      item exists.
- [ ] `src/render/one_line.rs` escapes: `%%` → `%`; `\n`, `\t`, `\\` unescaped before expansion;
      `%{<text>}` emits `<text>` verbatim without token expansion (`\}` escapes a closing brace); unknown `%X`
      emits `%X` literally and logs one `-v` warning with its position; a trailing lone `%` is literal; an empty
      or whitespace-only template is `Error::Usage` (exit 2). Presets via `--format one-line --template
      <TEMPLATE|@PRESET>`: `default` = `%l: %c %C %t (%f), %w, %h, %p, %P, %v`, `short` = `%c %t`,
      `full` = `%l: %c %C %t (%f) %w %h %p %P %m %v %u %S %s %Z`, `uv` = `%l: UV %u (%U)`,
      `sun` = `%l: sunrise %S sunset %s (%z %Z)`; no `--template` means `default`; an unknown `@name` prints the
      preset list and exits 2; the list also appears in `--help`.
- [ ] `src/render/plain.rs`: box-free, ANSI-free, greppable output reusing the one-line value formatters (no
      second conversion path): `location: <name> (<lat>, <lon>) <tz>`, `updated: <ISO local time> <offset>`,
      `current: <condition> <temp> (feels <feels>) wind <speed> <dir> humidity <h>% precip <p>mm pressure
      <p>hPa visibility <v>km`, then one `day <YYYY-MM-DD>: morning <…> | noon <…> | evening <…> | night <…>`
      line per day and finally `attribution: <provider> <url>`.
- [ ] `src/render/json.rs`: `serde_json` renderer emitting the schema below with `schema_version: 1`, keys
      always present (`null` for missing values, never omitted), `days` ascending from location-local today,
      ISO-8601 timestamps with the location offset, and canonical metric numeric suffixes (`_c`, `_kmh`, `_mm`,
      `_hpa`, `_km`, `_pct`), plus the stability promise documented in the module doc comment and the README:
      within `schema_version 1` changes are additive only (new keys may appear, existing keys keep
      name/type/unit), a breaking change bumps `schema_version` with a `CHANGELOG.md` entry, consumers must
      ignore unknown keys, and `--units` has no effect on JSON (noted under `-v`).
- [ ] `cirrocast completion <bash|zsh|fish|elvish|powershell>` via `clap_complete::generate` and
      `cirrocast man` via `clap_mangen::Man::new(cmd).render(&mut out)`, both to stdout, both generated from the
      same `Cli::command()`, `--bin-name` defaulting to `cirrocast`; `Cargo.toml` gains only
      `clap_complete = "4"` and `clap_mangen = "0.3"` — `serde_json` and `ureq` already arrive with step 05, so
      this step adds no HTTP/JSON dependency.
- [ ] `README.md`: a `## Usage` section with the flag table, both epilog tables, the token list (`%m` note), the presets and the completion/man one-liners.
- [ ] `tests/cli_flags.rs`: `assert_cmd` matrix over every flag (short and long), every conflict rule (exit 2
      **and** the exact stderr fragment), the one-time `--days` warning with and without `-q`, the precedence
      tiers (flag/env/config/builtin), the exit-code table 0/1/2/3/4/5/6 via `tests/fixtures/cli/*.json` stubs,
      and the completions smoke test.
- [ ] `tests/render_one_line.rs` token-by-token unit tests (both unit systems, `%%`/unknown/`%{...}`/trailing-`%`
      escapes, every preset) and `tests/render_json.rs` (hand-reviewed `insta` snapshot, key-set completeness,
      byte-identity of `-u us` vs `-u metric`).

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

JSON schema, version 1 (stable):
```json
{
  "schema_version": 1,
  "location": { "name": "Beijing, China", "lat": 39.9, "lon": 116.4, "timezone": "Asia/Shanghai" },
  "current": { "time": "2026-09-30T08:12:00+08:00", "condition": { "code": 1, "text": "Mainly clear" },
               "temp_c": 22.0, "feels_like_c": 23.0, "humidity_pct": 56, "precip_mm": 0.0, "pressure_hpa": 1013.0,
               "visibility_km": 10.0, "wind_kmh": 12.0, "wind_dir_deg": 45, "uv_index": 5.0, "is_day": true },
  "days": [ { "date": "2026-09-30", "sunrise": "06:05", "sunset": "17:58", "min_c": 15.0, "max_c": 25.0,
              "parts": { "morning": { "condition": { "code": 1, "text": "Mainly clear" }, "temp_c": 18.0,
                          "feels_like_c": 17.0, "wind_kmh": 8.0, "wind_dir_deg": 270, "precip_mm": 0.0,
                          "humidity_pct": 60, "pressure_hpa": 1012.0, "visibility_km": 10.0 },
                          "noon": null, "evening": null, "night": null } } ],
  "attribution": { "provider": "open-meteo", "url": "https://open-meteo.com/", "notice": "…",
                   "retrieved_at": "2026-09-30T00:11:00Z" }
}
```
Conflicts are declared in clap wherever possible because clap then owns the message and the exit code; only
provider-dependent rules need `validate()`, and they still map to `Error::Usage` so the code stays 2. Precedence
is read from `ArgMatches::value_source` rather than re-implementing "was it given?" checks — with the `env`
feature enabled that is the only reliable way to distinguish an env value from a flag value.
`--format json` ignores `--units`, `--width` and `--color` (documented; `--units` is asserted by the byte-identity
test), so a JSON document stays unit- and width-independent.
`clap_complete` is the official companion of the pinned clap line: five shells from one derive source of truth,
and hand-written completions would drift on the first flag rename. `clap_mangen` follows the same argument for the
man page (official, tiny, compile-time only); both are reached only from `completion`/`man`.
`-v` levels: `-v` = resolved settings (provider, width, colour, i18n match), cache hit/miss, attribution line;
`-vv` = HTTP request line with the API key masked, status, bytes, duration; `-vvv` = response headers plus a 2 KiB
truncated payload echo. Keys are masked at every level; `-q` suppresses warnings and notes, never errors.

## Out of scope
- Translating `--help`, clap usage errors, warnings or log lines: step 09 localizes the weather vocabulary and
  renderer labels only; clap's own strings stay English.
- The bodies of `config`, `key`, `cache`, `location` (steps 02/04/05) and the `provider info` content (step 10):
  this step defines their clap shapes and dispatch only.
- `--format` plugins, `--output <file>`, jq-style filtering, `--csv`, TUI modes (roadmap or never in v1) and
  installing completions/man pages into distro directories (step 13).

## Verification
Fixtures: `tests/fixtures/report/{beijing-3d-day,beijing-1d,beijing-night,current-only}.json` plus the CLI-level
stubs `tests/fixtures/cli/{upstream-500,location-not-found,missing-key,unreadable-config}.json` forcing exit 3/5/6/4.

Manual smoke run:
```
cirrocast --help | sed -n '/PRECEDENCE/,/EXIT CODES/p'   # both tables present, in full
cirrocast -f one-line --template '@full' Beijing         # one line, every token populated, %m says n/a
cirrocast -f one-line --template '%l:%{%}%c %t' Beijing  # the literal % survives the escape
cirrocast -f json Beijing | jq -r '.schema_version'      # 1
cirrocast -f json -u us Beijing | cmp - <(cirrocast -f json Beijing)   # identical bytes
cirrocast --lat 39.9 Beijing; echo $?                    # 2, mutual-exclusion message
cirrocast --station ZBAA -p open-meteo; echo $?          # 2, "--station requires --provider metar (or auto)"
cirrocast completion bash | head -5                      # non-empty, mentions cirrocast
cirrocast man | head -3                                  # roff starting with .TH
```

## Exit criteria
- [ ] `cargo fmt --check` / `cargo clippy --all-targets -- -D warnings` / `cargo test` / `reuse lint` all clean
- [ ] `--help` prints the flag matrix and both tables; every conflict rule exits 2 with its documented message; `--days` clamps once
- [ ] `one-line` renders every token (including `%%` and `%{...}`), all five presets and both unit systems, `%m` = `n/a`
- [ ] `json` snapshot matches, carries `schema_version: 1`, and is byte-identical under `-u metric` and `-u us`
- [ ] `completion bash|zsh|fish|elvish|powershell` and `man` produce non-empty output and exit 0
- [ ] README `## Usage` documents flags, precedence, exit codes, tokens and presets

## Risks
- clap/env precedence bugs hide easily: mitigated by driving the merge from `value_source` and one test per tier
  (flag > env > config > builtin) asserting the winning provider through `-f json` output.
- wttr.in compatibility drift: only the documented token subset is implemented, unknown tokens are logged and
  never invented; the token table above is the authority and is unit-tested token by token.
- Renaming a JSON key would silently break consumers: the completeness assertions (key set + unit suffixes) plus
  the snapshot make any rename fail loudly.
- `clap_complete`/`clap_mangen` skew is handled by pinning all three to the same minor line and bumping together;
  `README.md` is shared with other steps, so this step appends to `## Usage` only.

## Progress log
- 2026-09-30 — step file written; flag matrix, precedence mechanism, token table and JSON schema recorded.
