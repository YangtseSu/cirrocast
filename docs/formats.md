<!--
SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Output formats

`cirrocast` renders one of the formats below from the canonical report. Every renderer converts
units for display only (`src/render/` is the single conversion point) and never reads the
environment: width, colour and charset are resolved once by the CLI and travel in `RenderContext`.

| `--format` | What it is | Width |
|---|---|---|
| `art-table` | the wttr.in-style coloured day-part table (default) | honours `--width`; stacked below 60 columns |
| `dumb` | `art-table` in 7-bit ASCII, no colour; automatic for `TERM=dumb` or a non-UTF-8 locale | as above |
| `one-line` | one line driven by `%` tokens | fixed (never truncated) |
| `full`, `minimal` | `one-line` with the `@full` / `@minimal` preset | fixed |
| `plain` | box-free `label: value` records | ignores `--width` |
| `json` | the stable machine-readable document | ignores `--width` and `--units` |
| `alerts` | the full severe-weather warning listing, strongest first | ignores `--width` |
| `aqi` | the standalone air-quality panel (implies `--aqi`) | wraps to `--width` |
| `moon` | the standalone moon/sun view | wraps to `--width` |

`--format` also accepts the other one-line preset names (`default`, `short`, `uv`, `sun`) and any
key of the `[templates]` table; those render as `one-line` with that template. The resolution order
is: built-in format, built-in preset, `[templates]` key, otherwise a usage error listing all three
namespaces.

## Multiple locations

Several positional arguments are one run:

* Locations are fetched at most four at a time (never more than the machine's parallelism) and
  printed in **argument order**, whatever order the network answers in.
* A failed location keeps its slot: `error: <query>: <message>` on stdout (no colour, one line) and
  the full `error: …` line on stderr. Every other location is rendered as usual and the process
  exits with the numerically largest mapped code among the failures — a missing key (6) outranks a
  location miss (5).
* `json` is a plain object for one location and an **array** above one. A failed slot is
  `{"schema_version": 2, "query": "<as typed>", "error": {"code": <exit code>, "message": "…"}}`.
  The top-level type therefore depends on the number of locations; pass exactly one location, or
  read `.[]` after `jq -s`.
* `art-table` (and `dumb`) draw a combined summary for 2–4 locations: one header line per location
  and one aligned grid row (condition art and text, temperature, today's high/low), a blank line
  between blocks. A narrow terminal, a report without a current condition, or five locations falls
  back to the full per-location tables; above four the fallback also logs
  `note: art-table summary layout is limited to 4 locations` to stderr once (`-q` silences it).
* `--lat/--lon`, `--ip` and `--station` describe a single place and are refused with more than one
  location argument.

## The `%`-token template

The engine is `src/template.rs`; the exported `TOKENS` table is the single binding of a `%` letter
to a meaning and is enumerated by the tests and by the `--help` epilogue.

| Token | Output | Token | Output |
|---|---|---|---|
| `%c` | condition art, day/night aware, terminal charset | `%C` | condition text |
| `%x` | condition art in plain 7-bit text | `%l` | place name |
| `%t` `%f` | temperature / feels-like, signed | `%H` `%L` | today's high / low |
| `%e` | dew point, computed from temperature and humidity | `%w` | wind `↗ 12km/h NE` |
| `%h` | humidity `56%` | `%p` `%P` | precipitation `0.0mm` / pressure `1013hPa` |
| `%v` | visibility `10km` | `%u` `%U` | UV `5` / `5 (moderate)` |
| `%d` `%D` | `2026-09-30` / `Thu 01 Oct` | `%T` | local time `15:04` |
| `%Z` `%z` | `Asia/Shanghai` / `+0800` | `%S` `%s` | sunrise / sunset `06:05` |
| `%m` `%M` | moon glyph / phase name | `%A` | strongest alert's event, empty when none |
| `%q` | air-quality index on the selected scale | | |

A value the provider does not report prints `n/a` (localised) — never an invented number. Two
tokens are computed rather than read: `%e` (the Magnus-formula dew point of the reported
temperature and humidity, `n/a` when either is missing) and the astro tokens.

### Width and precision

`%[-][0][<width>][.<prec>]X`:

* `<width>` is a minimum in characters; the value is padded on the left, or on the right with `-`.
* `0` pads a **numeric** token with zeros between its sign and its digits (`%08.1t` → `+021.5°C`);
  text tokens pad with spaces, because `000Cloudy` is never what was meant.
* `.<prec>` truncates text from the right in characters and rounds a numeric token to that many
  decimals inside its unit suffix (`%.1t` → `+18.4°C`, `%.4C` → `Main`).
* The numeric tokens are `%t %f %H %L %e %u %h %p %P %v`; every other token — `%w` included, whose
  value is arrow plus speed plus direction — is text.

### Escapes

* `%%` is a literal `%`; a trailing lone `%` and an incomplete specifier (`%12`) are literal.
* `\n`, `\t` and `\\` are unescaped before the tokens are read.
* `%{…}` prints its content verbatim — no token expansion — and `\}` escapes the closing brace.
  When the content is exactly one known token letter it *is* that token (`%{c}`), which is how a
  token is written next to text that would glue onto its letter.

### Unknown tokens

An unknown `%X` is reported with its 1-based position. On the CLI it is an `Error::Usage` (exit 2)
because the user is authoring a template and wants the typo. The wttr.in compatibility service
(B01) serves the same `TOKENS` table with literal passthrough instead, because a script written
against wttr.in's larger token set must not break; that asymmetry lives at the service boundary,
never as a second parser.

## Stability

The one-line token meanings above are a frozen output contract: changing one is a breaking change
that gets a minor version bump and a `CHANGELOG.md` entry naming the old and the new meaning.
Step 19 is the worked example: `%L` used to print the location's coordinates and now prints today's
low temperature, matching wttr.in's documented `H`/`L` pair; the coordinates remain available in
`art-table`'s header and in `plain`'s `location:` record.
