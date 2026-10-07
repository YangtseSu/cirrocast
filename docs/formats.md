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
| `normals` | the month's climate normal against the forecast (`--normals` implies the fetch) | wraps to `--width` |

`--format` also accepts the other one-line preset names (`default`, `short`, `uv`, `sun`) and any
key of the `[templates]` table; those render as `one-line` with that template. The resolution order
is: built-in format, built-in preset, `[templates]` key, otherwise a usage error listing all three
namespaces.

`cirrocast status` is the one subcommand where `-f/--format` means something else: its whole output
is one `%`-template, so `--format` (and its synonym `--template`) takes the template itself —
`--format '%c %t'` — and the format names above are not accepted there. The probe's line, colour
and exit-code contract is in [`docs/ecosystem.md`](ecosystem.md).

## Icon sets

The condition and moon art has two opt-in glyph sets beside the default hand-drawn blocks:

| `--icons` | What it draws | Needs |
|---|---|---|
| `blocks` (default) | the four-line unicode/ASCII blocks | nothing beyond a Latin monospace |
| `emoji` | one Unicode emoji per key | a font with the emoji it covers |
| `nerd` | one Weather Icons glyph per key (U+E300–U+E3E3) | a [Nerd Font](https://github.com/ryanoasis/nerd-fonts) |

The value is an **ordered chain**: `--icons nerd,emoji` draws the Nerd Font glyph wherever the
Weather Icons set has one and falls through to the emoji elsewhere, with `blocks` appended
implicitly as the last resort — a terminal cannot be asked whether its font carries a glyph, so
"mixed" can only mean "try in order". A terminal that cannot draw UTF-8 (`TERM=dumb`, a non-UTF-8
locale, `--format dumb`) always gets the blocks, whatever the chain says. The preference is a flag,
`CIRROCAST_ICONS`, then `[render] icons`; `-v` prints the resolved chain.

```text
$ cirrocast --icons emoji -d 1 Beijing       # the table, borders unchanged
Weather report: Beijing, CN (39.91, 116.40)

        Clear sky
  🌙    +18°C (+18°C)
        ↙ 1.0km/h SW
        59% 1019hPa 15km 0.0mm

┌───────────────────────┐
│ Today, Oct 08         │
├───────────────────────┤
│         Morning       │
│   ☀️    +17°C (+17°C) │
│         ↑ 2.4km/h N   │
│         0.0mm 0%      │
├───────────────────────┤
│         Night         │
│   🌙    +17°C (+17°C) │
│         ← 1.1km/h W   │
│         0.0mm 0%      │
└───────────────────────┘
```

The same cells with `--icons nerd`, quoted as their codepoints (a Nerd Font terminal draws the
glyphs; `\ue32b` is `weather-night_clear`):

```text
$ cirrocast --icons nerd -f one-line --template '%l: %c %t'
Beijing: \ue32b +18°C
```

`%c` follows the chain; **`%x` never does** — it is the plain 7-bit symbol whatever `--icons` says,
so a status bar that greps a template keeps working on a terminal without the font. `plain` and
`json` carry no art and do not change with the setting. The palette of a glyph is the colour its
block has (`ArtStyle`), so rain stays blue in every set.

## Captured examples

Every block below is a real capture from the built binary run against Beijing, stored as plain text
with no colour or escape sequences. The full transcripts live in
[`docs/screenshots/`](screenshots/), one file per format; each block names the file it came from.
`full` and `minimal` are `one-line` presets, so they share the `one-line` example below with a
different template.

### `art-table` (the default)

The head and tail of [`art-table.txt`](screenshots/art-table.txt); the 33-line file continues with
the Morning, Noon, Evening and Night rows for each of the three days:

```text
Weather report: Beijing, CN (39.91, 116.40)

  · * · Clear sky
   (●)  +20°C (+16°C)
  * · * ↙ 11km/h SW
        29% 1021hPa 18km 0.0mm

┌───────────────────────┬───────────────────────┬───────────────────────┐
│ Today, Oct 06         │ Wed 07 Oct            │ Thu 08 Oct            │
├───────────────────────┼───────────────────────┼───────────────────────┤
```

The file ends with the day grid's closing rule and the two attribution lines:

```text
└───────────────────────┴───────────────────────┴───────────────────────┘

Location data by GeoNames (CC BY 4.0) — https://www.geonames.org/
Data: Open-Meteo.com (CC BY 4.0) — https://open-meteo.com/
```

### `one-line`

[`one-line.txt`](screenshots/one-line.txt), the `default` preset with `%c`'s blocks art:

```text
Beijing: *o* Clear sky +20°C (+16°C), ↙ 11km/h SW, 29%, 0.0mm, 1021hPa, 18km
```

### `plain`

[`plain.txt`](screenshots/plain.txt), one `label: value` record per line:

```text
location: Beijing, CN (39.91, 116.40) Asia/Shanghai
updated: 2026-10-06T21:30:00+08:00
current: Clear sky 20°C (feels 16°C) wind 11km/h SW humidity 29% precip 0.0mm pressure 1021hPa visibility 18km
day 2026-10-06: Morning Clear sky 18°C 0.0mm (0%) wind 7.0km/h N | Noon Clear sky 26°C 0.0mm (0%) wind 4.2km/h NE | Evening Clear sky 20°C 0.0mm (0%) wind 11km/h SW | Night Clear sky 14°C 0.0mm (0%) wind 8.8km/h NNW
day 2026-10-07: Morning Clear sky 17°C 0.0mm (0%) wind 1.5km/h NE | Noon Clear sky 27°C 0.0mm (0%) wind 13km/h S | Evening Clear sky 21°C 0.0mm (0%) wind 7.4km/h S | Night Clear sky 17°C 0.0mm (0%) wind 0.9km/h W
day 2026-10-08: Morning Clear sky 17°C 0.0mm (0%) wind 3.1km/h NNE | Noon Clear sky 26°C 0.0mm (0%) wind 9.4km/h SSE | Evening Clear sky 22°C 0.0mm (0%) wind 5.1km/h SSE | Night Clear sky 17°C 0.0mm (0%) wind 1.0km/h NNW
Location data by GeoNames (CC BY 4.0) — https://www.geonames.org/
Data: Open-Meteo.com (CC BY 4.0) — https://open-meteo.com/
attribution: open-meteo https://api.open-meteo.com/v1/forecast
```

### `json`

The head and tail of [`json.txt`](screenshots/json.txt), 261 lines of pretty-printed
`schema_version: 2` document; the key index is in [`docs/schema.md`](schema.md):

```text
{
  "schema_version": 2,
  "location": {
    "name": "Beijing",
    "admin1": null,
    "country": "CN",
    "country_code": "CN",
    "lat": 39.9075,
    "lon": 116.39723,
    "timezone": "Asia/Shanghai",
    "elevation_m": null,
    "source": "offline",
    "station": null
  },
```

The document ends with the credits block:

```text
    "url": "https://api.open-meteo.com/v1/forecast",
    "notice": "Open-Meteo.com (CC BY 4.0) — https://open-meteo.com/",
    "location_notice": "Location data by GeoNames (CC BY 4.0) — https://www.geonames.org/",
    "retrieved_at": "2026-10-06T13:43:52Z"
  },
  "alert_credits": []
}
```

### `dumb`

[`dumb.txt`](screenshots/dumb.txt), the same layout in 7-bit ASCII:

```text
Weather report: Beijing, CN (39.91, 116.40)

  . * . Clear sky
   (o)  +20C (+16C)
  * . * , 11km/h SW
        29% 1021hPa 18km 0.0mm

+-----------------------+-----------------------+-----------------------+
| Today, Oct 06         | Wed 07 Oct            | Thu 08 Oct            |
+-----------------------+-----------------------+-----------------------+
|    \|/  Morning       |    \|/  Morning       |    \|/  Morning       |
|   -(o)- +18C (+14C)   |   -(o)- +17C (+16C)   |   -(o)- +17C (+16C)   |
|    /|\  ^ 7.0km/h N   |    /|\  / 1.5km/h NE  |    /|\  / 3.1km/h NNE |
|         0.0mm 0%      |         0.0mm 0%      |         0.0mm 0%      |
+-----------------------+-----------------------+-----------------------+
|    \|/  Noon          |    \|/  Noon          |    \|/  Noon          |
|   -(o)- +26C (+22C)   |   -(o)- +27C (+23C)   |   -(o)- +26C (+24C)   |
|    /|\  / 4.2km/h NE  |    /|\  v 13km/h S    |    /|\  v 9.4km/h SSE |
|         0.0mm 0%      |         0.0mm 0%      |         0.0mm 0%      |
+-----------------------+-----------------------+-----------------------+
|    \|/  Evening       |    \|/  Evening       |    \|/  Evening       |
|   -(o)- +20C (+17C)   |   -(o)- +21C (+20C)   |   -(o)- +22C (+21C)   |
|    /|\  , 11km/h SW   |    /|\  v 7.4km/h S   |    /|\  v 5.1km/h SSE |
|         0.0mm 0%      |         0.0mm 0%      |         0.0mm 0%      |
+-----------------------+-----------------------+-----------------------+
|   . * . Night         |   . * . Night         |   . * . Night         |
|    (o)  +14C (+11C)   |    (o)  +17C (+15C)   |    (o)  +17C (+16C)   |
|   * . * ^ 8.8km/h NNW |   * . * < 0.9km/h W   |   * . * ^ 1.0km/h NNW |
|         0.0mm 0%      |         0.0mm 0%      |         0.0mm 0%      |
+-----------------------+-----------------------+-----------------------+

Location data by GeoNames (CC BY 4.0) -- https://www.geonames.org/
Data: Open-Meteo.com (CC BY 4.0) -- https://open-meteo.com/
```

### `alerts`

[`alerts.txt`](screenshots/alerts.txt); no warning was in force for Beijing at capture time, so this
is the empty state:

```text
no active weather alerts
```

### `aqi`

[`aqi.txt`](screenshots/aqi.txt), the standalone air-quality panel:

```text
Beijing, CN (39.91, 116.40) Asia/Shanghai
updated 2026-10-06T21:00:00+08:00 · Open-Meteo
Air quality: US AQI 145 (Unhealthy for sensitive groups) · European AQI 76 (Poor)
PM2.5 78.9 · PM10 91 · O3 0 · NO2 92.4 · SO2 11.9 · CO 753 μg/m³
Pollen: not covered at this location
UV 0 (low) · weather data
Air quality data by Open-Meteo.com (CAMS ENSEMBLE)
```

### `moon`

[`moon.txt`](screenshots/moon.txt), the standalone moon/sun view (computed locally, no request):

```text
Beijing, CN (39.91, 116.40) Asia/Shanghai
computed locally (no network) at 2026-10-06T21:43:53+08:00
 █▒░░░  Moon: Waning Crescent
█▒░░░░░ 18% illuminated (geocentric) · age 25.4 d
█▒░░░░░ Moonrise 01:03 · Moonset 15:36
 █▒░░░  Sunrise 06:15 · Sunset 17:49 · daylight 11h 34m
Next phases:
  New Moon Sat 10 Oct 23:49
  First Quarter Mon 19 Oct 00:12
  Full Moon Mon 26 Oct 12:11
  Last Quarter Mon 02 Nov 04:28
```

### `normals`

[`normals.txt`](screenshots/normals.txt), the month's normal against the forecast:

```text
Beijing, CN (39.91, 116.40) Asia/Shanghai
Climate normals: 1991–2020 · BEIJING, CH (CHM00054511) 10 km · high 19.4°C (+6.4°C) · low 8.8°C
(+4.9°C) · precip 29.1 mm/mo (-100%) · 22 years
Climate normals computed from NOAA NCEI Global Summary of the Month (public domain)
```

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
| `%c` | condition art, day/night aware, from the icon chain | `%C` | condition text |
| `%x` | condition art in plain 7-bit text, whatever `--icons` says | `%l` | place name |
| `%t` `%f` | temperature / feels-like, signed | `%H` `%L` | today's high / low |
| `%e` | dew point, computed from temperature and humidity | `%w` | wind `↗ 12km/h NE` |
| `%h` | humidity `56%` | `%p` `%P` | precipitation `0.0mm` / pressure `1013hPa` |
| `%v` | visibility `10km` | `%u` `%U` | UV `5` / `5 (moderate)` |
| `%d` `%D` | `2026-09-30` / `Thu 01 Oct` | `%T` | local time `15:04` |
| `%Z` `%z` | `Asia/Shanghai` / `+0800` | `%S` `%s` | sunrise / sunset `06:05` |
| `%m` `%M` | moon glyph (chain-aware) / phase name | `%A` | strongest alert's event, empty when none |
| `%q` | air-quality index on the selected scale | | |

A value the provider does not report prints `n/a` (localised) — never an invented number. Two
tokens are computed rather than read: `%e` (the Magnus-formula dew point of the reported
temperature and humidity, `n/a` when either is missing) and the astro tokens.

### Width and precision

`%[-][0][<width>][.<prec>]X`:

* `<width>` is a minimum in characters; the value is padded with spaces on the left, or on the
  right with `-`.
* `0` pads a **numeric** token with zeros between its sign and its digits (`%08.1t` → `+019.7°C`);
  text tokens pad with spaces, because `000Cloudy` is never what was meant.
* `.<prec>` truncates text from the right in characters and rounds a numeric token to that many
  decimals inside its unit suffix (`%.1t` → `+19.7°C`, `%.4C` → `Clea`).
* A width above 200 is clamped to 200 and a precision above 6 to 6, so an absurd specifier widens
  the line rather than failing.
* The numeric tokens are `%t %f %H %L %e %u %h %p %P %v`; every other token — `%w` included, whose
  value is arrow plus speed plus direction — is text.

One specifier of each kind, captured live (`%-12l` right-pads `Beijing` to twelve characters and
`%.4C` truncates the condition text):

```
$ cirrocast -f one-line --template '%l|%08.1t|%.1t|%-12l|%.4C|%10.2p' --lang en --color never Beijing
Beijing|+019.7°C|+19.7°C|Beijing     |Clea|    0.00mm
```

### Escapes

* `%%` is a literal `%`; a trailing lone `%` and an incomplete specifier (`%12`) are literal.
* `\n`, `\t` and `\\` are unescaped before the tokens are read.
* `%{…}` prints its content verbatim — no token expansion — and `\}` escapes the closing brace.
  When the content is exactly one known token letter it *is* that token (`%{c}`), which is how a
  token is written next to text that would glue onto its letter.

### Unknown tokens

An unknown `%X` is reported with its 1-based position. On the CLI it is an `Error::Usage` (exit 2)
because the user is authoring a template and wants the typo; both the `one-line` renderer and
`status`'s `--format`/`--template` go through the same gate, so they report the same position and
exit code:

```console
$ cirrocast -f one-line --template '%l %Q' --offline Beijing
error: unknown template token `%Q` at position 4; known tokens: cCxtfHLwhpPeuUmMvldDTZzSsAq
$ cirrocast status --location Beijing --format '%l %Q' --offline
error: unknown template token `%Q` at position 4; known tokens: cCxtfHLwhpPeuUmMvldDTZzSsAq
```

The wttr.in compatibility service (B01) serves the same `TOKENS` table with literal passthrough
instead, because a script written against wttr.in's larger token set must not break; that asymmetry
lives at the service boundary, never as a second parser.

## Stability

The one-line token meanings above are a frozen output contract: changing one is a breaking change
that gets a minor version bump and a `CHANGELOG.md` entry naming the old and the new meaning.
Step 19 is the worked example: `%L` used to print the location's coordinates and now prints today's
low temperature, matching wttr.in's documented `H`/`L` pair; the coordinates remain available in
`art-table`'s header and in `plain`'s `location:` record.

[`docs/ecosystem.md`](ecosystem.md) is the whole of that policy — the `json`, `one-line` and
`plain` contracts, the breaking-change procedure and the tests that enforce them — and the
`status` probe's own contract (one line, colour, exit codes, freshness, privacy).
