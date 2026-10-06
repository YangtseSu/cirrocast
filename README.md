<!--
SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
SPDX-License-Identifier: GPL-3.0-or-later
-->

# cirrocast

Terminal weather client. Pluggable weather backends, wttr.in-style output, city-name geocoding,
optional IP-based location, configurable units and output language, XDG-compliant state.

```
$ cirrocast Beijing
Weather report: Beijing, Beijing Municipality, China (39.91, 116.40)

  · * · Clear sky
   (●)  +18°C (+13°C)
  * · * ↖ 13km/h NW
        11% 1021hPa 17km 0.0mm

┌───────────────────────┬───────────────────────┬───────────────────────┐
│ Today, Sep 30         │ Thu 01 Oct            │ Fri 02 Oct            │
├───────────────────────┼───────────────────────┼───────────────────────┤
│    \│/  Morning       │    \│/  Morning       │    \│/  Morning       │
│  ╭───╮  +17°C (+11°C) │   ─(●)─ +16°C (+13°C) │   ─(●)─ +16°C (+13°C) │
│ (     ) ↑ 19km/h NNW  │    /│\  ← 0.0km/h W   │    /│\  ↗ 6.0km/h NNE │
│  ╰───╯  0.0mm 18%     │         0.0mm 13%     │         0.0mm 27%     │
… the noon, evening and night rows of each day, then the credits …
```

```
$ cirrocast Beijing --format plain
location: Beijing, CN (39.91, 116.40) Asia/Shanghai
updated: 2026-09-30T19:45:00+08:00
current: Clear sky 18°C (feels 13°C) wind 13km/h NW humidity 11% precip 0.0mm pressure 1021hPa visibility 17km
day 2026-09-30: Morning Partly cloudy 17°C 0.0mm (0%) wind 19km/h NNW | Noon Overcast 20°C 0.0mm (0%) wind 19km/h NW | Evening … | Night …
… one `day` line per forecast day …
Location data by GeoNames (CC BY 4.0) — https://www.geonames.org/
Data: Open-Meteo.com (CC BY 4.0)
attribution: open-meteo https://api.open-meteo.com/v1/forecast
```

```
$ cirrocast Beijing -f one-line --template @short
*o* +18°C
```

An observation instead of a forecast, keyless and station-based:

```
$ cirrocast --station KJFK
Weather report: New York/JF Kennedy Intl, NY, US (40.64, -73.76)

 ╭───╮  Overcast
╭╯   ╰╮ +18°C
(     ) ↘ 7.4km/h SE
╰─────╯ 88% 1019hPa 16km 0.0mm
observed 23:51Z · 54 min ago
no forecast: METAR is an observation

Data: aviationweather.gov (NOAA/NWS, public domain)
```

`--station` selects the `metar` backend when no `--provider` is given, resolves the identifier
through the embedded 55-station table (or a 30-day cached `stationinfo` lookup for anything else),
and decodes the report itself: wind in knots or m/s, visibility in metres or statute miles, present
weather into WMO codes, cloud layers, temperature and dew point, altimeter in hPa or inHg. `-v`
adds the raw METAR and, when the station issues one, its TAF.

Those are real runs — 2026-09-30, 19:45 local, `COLUMNS=120`, `TERM=xterm-256color` — with the middle
rows elided. When the name is ambiguous a `note: … 10 candidates …` line goes to stderr, so stdout
stays pipeable; `-q` silences it and `:Beijing` demands an exact name.

## Status

Steps 01–21 of 29 in [`docs/plans/`](docs/plans/README.md) — 18b included — are in place: the CLI
skeleton, XDG path resolution, the provider registry, the typed configuration with its
`config`/`key` subcommands, the canonical model, location resolution, the shared HTTP/cache layer,
the Open-Meteo forecast, the wttr.in-style `art-table` renderer, the full flag matrix with the
`one-line`/`plain`/`json`/`dumb` formats, shell completions and the man page, localization
(`en-US` + `zh-CN`) and eight selectable backends — three of them keyless, including the
station-based `metar` observation. Steps 12–14 hardened the error contract and made it releasable:
`v1.0.0` (2026-10-02) tags the accepted v1 surface, with the release workflow shipping three native
archives, the AUR package published, and the JSON and config schemas written out in
[`docs/schema.md`](docs/schema.md) with the versioning policy in [Versioning](#versioning) and the
history in [`CHANGELOG.md`](CHANGELOG.md). Phase D is complete as of `v1.2.0`: `v1.1.0` added
severe-weather alerts (step 15), the air-quality panel (step 16) and the locally computed moon/sun
block (step 17), and `v1.2.0` adds the offline city database (step 18) with user-installed table
updates (18b), multi-location runs with the shared `%`-token template engine (step 19) and the
interactive location picker (step 20). Step 21 turned the performance and resource promises into
enforced numbers — one harness, one committed baseline, one dispatch-only CI workflow — recorded in
[`docs/performance.md`](docs/performance.md). Phase E's remaining item, the `status` probe and the
ecosystem recipes (step 22), is next.

## Install

```bash
cargo install --locked cirrocast            # from crates.io (1.2.0 is published; see Publishing)
cargo install --locked --path .             # from this checkout

paru -S cirrocast                           # Arch: the AUR package (yay, or a manual git clone +
                                            # makepkg); https://aur.archlinux.org/packages/cirrocast

# Prebuilt archives for Linux (x86_64, aarch64) and macOS (Apple silicon), one per release:
version=v1.2.0 target=x86_64-unknown-linux-gnu
curl -LO "https://github.com/YangtseSu/cirrocast/releases/download/$version/cirrocast-$version-$target.tar.gz"
sha256sum -c "cirrocast-$version-$target.tar.gz.sha256"
tar xzf "cirrocast-$version-$target.tar.gz"
install -Dm755 "cirrocast-$version-$target/cirrocast" ~/.local/bin/cirrocast
```

Every release is cut only after the bundled city table has been refreshed and verified against the
official `GeoNames` dump (`cargo run -p geo-table -- <dump> --check` runs locally before the tag),
so an installed release ships the snapshot it was cut from — and a newer one can be installed
without waiting for the next release (see [Updating the city data](#updating-the-city-data)).

`cargo install` places the binary and nothing else — no crate may write outside its own prefix — so
the man page and the completions are generated by the binary itself:

```bash
cirrocast man > ~/.local/share/man/man1/cirrocast.1
cirrocast completion bash > ~/.local/share/bash-completion/completions/cirrocast  # bash-completion ≥ 2.9
cirrocast completion zsh  > ~/.local/share/zsh/site-functions/_cirrocast          # add that directory to $fpath
cirrocast completion fish > ~/.config/fish/completions/cirrocast.fish
```

[Packaging and release](#packaging-and-release) has the full picture: what each path installs, how
the archives are built and verified, and the checklist a release follows.

## Usage

```
cirrocast [OPTIONS] [LOCATION]...

  -p, --provider <ID[,ID...]>   open-meteo | smhi | metar | openweathermap | weatherapi
                                | worldweatheronline | pirateweather | qweather | auto
  -f, --format <NAME>           art-table | one-line | plain | json | dumb | alerts | aqi | moon,
                                or a one-line preset: full | minimal | short | default | uv | sun,
                                or a [templates] key
  -d, --days <N>                0..=14, clamped to what the provider serves
  -u, --units <SYSTEM>          metric | us | uk
      --lang <TAG>              BCP-47, or "auto"
      --lat <DEG> --lon <DEG>   coordinates instead of a location argument
      --ip                      locate from the public IP
      --station <ICAO>          METAR station; selects --provider metar when no provider is given
      --alerts | --no-alerts    force / suppress severe-weather warnings (they are on by default)
      --alerts-from <LIST>      explicit alert sources: nws, meteoalarm, qweather, hko, wmoswic, fpas
      --severity <LEVEL>        lowest alert severity shown: unknown | minor | moderate | severe | extreme
      --aqi                     append the air-quality panel (US/European AQI, pollutants, pollen)
      --aqi-index <SCALE>       us | european: the AQI scale behind the panel colour and %q
      --moon                    append the locally computed moon/sun block (no request)
      --template <TEMPLATE>     one-line template or @PRESET
      --template-file <PATH>    read the template from a file; `-` reads standard input
      --no-cache | --refresh | --offline[=<weather|geo|all>]
      --timeout <SECS>
      --color <WHEN>            auto | always | never
      --width <COLS>            layout width for the table formats, 1..=500
      --pick | --yes            ask which candidate a name means / take the ranked winner (conflict)
  -q, --quiet    -v, --verbose (repeat `-vv` for every HTTP attempt, its status and the cache decisions)
  -h, --help     -V, --version
```

Several `LOCATION` arguments are one run: they are fetched at most four at a time and printed in
argument order whatever order the network answers in. A location that fails keeps its slot (a
one-line `error: <query>: <message>` on stdout, the full error on stderr) while the others still
print, and the process exits with the numerically largest mapped code among the failures — a
missing key (6) outranks a location miss (5). Above one location `json` becomes an array (a failed
slot is an `{"schema_version": 2, "query": …, "error": {"code": …, "message": …}}` object) and
`art-table` draws a combined summary for 2–4 locations before falling back to the full tables.
`--lat/--lon`, `--ip` and `--station` describe one place and are refused with several.

```bash
cirrocast Beijing Shanghai Tokyo -f one-line     # three lines, argument order
cirrocast Beijing Shanghai Nope-9x ; echo $?     # 5; the third slot carries the error
cirrocast Beijing Shanghai -f json | jq length   # 2
```

```
cirrocast config     <path|init [--force]|show|get|set|edit|validate [--offline]>
cirrocast key        <set [--stdin]|rm|list>
cirrocast provider   <list|info>
cirrocast cache      <stat|clean [--all] [--offline]>
cirrocast status     [--format <TEMPLATE>] [--location <SPEC>] [--max-age <SECS>] [--offline]
                     [--placeholder <TEXT>] [--color never|always]
cirrocast location   <search [--offline] [--all] [--exact] [--ip] [--limit <N>] [--timeout <SECS>]>
cirrocast location   update-data [--from <PATH|URL>] [--check] [--timeout <SECS>]
cirrocast completion <bash|zsh|fish|elvish|powershell> [--bin-name NAME]
cirrocast man [--bin-name NAME]
```

Location syntax: `Beijing` (fuzzy), `:Beijing` (exact name), `~Tsinghua` (OpenStreetMap),
`@39.9,116.4` (coordinates), `@home` (a `[locations]` alias), empty (config default, else public
IP). A single location argument,
`--lat/--lon`, `--ip` and `--station` are mutually exclusive; with several arguments those three
flags are refused (they describe one place), and when the argument comes from
`CIRROCAST_LOCATION` instead of the command line, a flag on one of the other forms wins by
precedence rather than conflicting. `--station KJFK` (case-insensitive, four ICAO characters) makes
the station the location and `[providers.metar] station` is the same thing for a bare `cirrocast`
when `metar` answers it; a `@lat,lon` pair reaches the nearest embedded station, which `-v` names
with its distance.

### Precedence

Highest first: **command line flag → `CIRROCAST_*` environment variable → `config.toml` → built-in
default**. The configuration file is consulted only for settings neither the flag nor the variable
supplied, so an environment value is never overridden by the file. `-v` prints every setting with the
tier it came from.

| Setting | Flag | Variable | Config key |
|---|---|---|---|
| Provider chain | `-p, --provider` | `CIRROCAST_PROVIDER` | `defaults.provider` |
| Format | `-f, --format` | `CIRROCAST_FORMAT` | `defaults.format` |
| Days | `-d, --days` | `CIRROCAST_DAYS` | `defaults.days` |
| Units | `-u, --units` | `CIRROCAST_UNITS` | `defaults.units` |
| Language | `--lang` | `CIRROCAST_LANG` | `defaults.language` |
| Location | `[LOCATION]` | `CIRROCAST_LOCATION` | `location.default` |
| Timeout | `--timeout` | `CIRROCAST_TIMEOUT` | `network.timeout_secs` |
| Colour | `--color` | — | `render.color` |
| Width | `--width` | — | `render.width` |
| AQI scale | `--aqi-index` | — | `air.index` |
| Offline policy | `--offline[=<weather\|geo\|all>]` | — | `network.offline` |
| Candidate pick | `--pick` / `--yes` | `CIRROCAST_LOCATION_PICK` | `location.pick` |
| Geo strategy | — | — | `geo.strategy` |
| City table source | — | — | `geo.data` |
| Freshness note | — | — | `geo.update` |
| Update source | `--from` (`update-data`) | — | `geo.update_url` |

### Exit codes

| Code | Meaning |
|---|---|
| 0 | success |
| 1 | generic failure (an unreadable file, an editor that will not run) |
| 2 | usage: unknown flag or value, conflicting options, an empty template |
| 3 | network or upstream failure |
| 4 | configuration or state on disk |
| 5 | location not found |
| 6 | missing or invalid API key |

Errors go to stderr as `error: …`, with the cause chain under `-v`; `-q` suppresses warnings and
notes, never errors.

### Formats

The reference for every format, the token contract and the multi-location output rules is
[`docs/formats.md`](docs/formats.md).

| Format | What it is | Width |
|---|---|---|
| `art-table` | the wttr.in-style coloured day-part table (default) | honours `--width`, degrades to a stacked layout below 60 columns |
| `dumb` | the same table in 7-bit ASCII, no colour; automatic for `TERM=dumb` or a non-UTF-8 locale | as above |
| `one-line` | one line driven by `%` tokens, for a prompt or status bar | fixed |
| `full` `minimal` | one-line presets (`@full` / `@minimal`), the same renderer | fixed |
| `plain` | box-free `label: value` lines, one record per line | ignores `--width`: a record is never truncated |
| `json` | the stable machine-readable document (`schema_version: 2`) | ignores `--width` and `--units` |
| `alerts` | the full severe-weather warning listing for the location, strongest first; `no active weather alerts` when there are none | ignores `--width` |
| `aqi` | the standalone air-quality panel (implies `--aqi`); `air quality unavailable` when the reading could not be fetched | wraps to `--width` |
| `moon` | the standalone moon/sun view: phase, illumination, age, moonrise/moonset, sunrise/sunset and the next four phase instants, all computed locally | wraps to `--width` |

`one-line` takes a template with `--template` (or `--template-file <PATH>`, `-` for stdin), either
a literal string or a preset. `--format` accepts the preset names directly too, so
`-f minimal` and `-f one-line --template @minimal` are the same run.

| Token | Output | Token | Output |
|---|---|---|---|
| `%c` | condition art, day/night aware | `%C` | condition text |
| `%x` | condition art in plain 7-bit text | `%l` | place name |
| `%t` `%f` | temperature / feels-like | `%H` `%L` | today's high / low |
| `%e` | dew point, computed from temperature and humidity | `%w` | wind `↗ 12km/h NE` |
| `%h` | humidity `56%` | `%p` `%P` | precipitation `0.0mm` / pressure `1013hPa` |
| `%v` | visibility `10km` | `%u` `%U` | UV `5` / `5 (moderate)` |
| `%d` `%D` | `2026-09-30` / `Thu 01 Oct` | `%T` | local time `15:04` |
| `%Z` `%z` | `Asia/Shanghai` / `+0800` | `%S` `%s` | sunrise / sunset `06:05` |
| `%m` `%M` | moon glyph / phase name, e.g. `◕` / `Waning Gibbous` | `%A` | strongest alert's event, empty when nothing is in force |
| `%q` | air-quality index on the selected scale, e.g. `US AQI 43 (Good)` | | |

A token may carry a width and precision specifier, `%[-][0][<width>][.<prec>]X`: the width pads
(right-aligned unless `-`; zero-padded for numeric tokens), `.prec` truncates text from the right
and rounds a numeric token to that many decimals (`%.1t` → `+18.4°C`, `%-12C` → the condition text
left-aligned in 12 columns). `%%` is a literal `%`, a trailing lone `%` is one too, `%{…}` is
verbatim unless its content is exactly one token letter (`%{c}` is the token, `%{x}` is literal
text), and `\n`/`\t`/`\\` are unescaped. An unknown `%X` is a usage error (exit 2, naming the
position and the known tokens) — the wttr.in compatibility service keeps it literal instead. A
value the provider does not report prints `n/a` — never an invented number.

The presets are `@default` (`%l: %c %C %t (%f), %w, %h, %p, %P, %v`), `@short` (`%c %t`),
`@minimal` (`%c%t`), `@full` (`%l: %c %C %t (%f) %w %h %p %P %m %v %u %S %s %Z`), `@uv`
(`%l: UV %U`) and `@sun` (`%l: sunrise %S sunset %s (%z %Z)`); a `[templates]` key can be used
through `--template @name` or `--format name`. `--template` with a non-one-line format or a
preset-selecting one, `--template` together with `--template-file`, an empty template, an unknown
`@name` and an unknown token are usage errors.

### Status bars

`cirrocast status` is the probe a bar or a prompt runs on a timer: one line, no colour unless asked,
and **never a non-zero exit because the network was down** (the placeholder `n/a`, or
`[status] placeholder`, takes the reading's place). It fetches the alert set only when the template
shows `%A` and the air reading only when it shows `%q`, so the default costs one request.

```bash
cirrocast status                                  # `%c %t` for the location from the config
cirrocast status --format '%l %t' --max-age 900   # at most one fetch per 15 minutes
cirrocast status --format '%c%t' --offline -q     # cache only, no socket, no credits
```

The exit codes are 0 for a reading *and* for a transient failure, 2 for a usage mistake and 4 for a
configuration problem (a missing location among them: the probe never performs the public-IP
lookup). The full contract — freshness, offline behaviour, privacy, and the runnable waybar,
polybar, i3blocks, tmux, starship and bash/zsh recipes in [`contrib/statusbar/`](contrib/statusbar/)
— is [`docs/ecosystem.md`](docs/ecosystem.md), which is also where the `json`, `one-line` and
`plain` **output contracts** and the breaking-change policy live.

`json` is the scripting surface: keys are always present (`null` when the provider has no value), all
numbers are canonical metric with the unit in the key (`temp_c`, `wind_kmh`, `precip_mm`,
`pressure_hpa`, `visibility_km`), timestamps carry the location's offset, and `attribution` carries
the credits. Within `schema_version` changes are additive only — new keys may appear and existing
ones keep their name, type and unit — so a consumer must ignore keys it does not know; a breaking
change bumps the version and is named in [`CHANGELOG.md`](CHANGELOG.md). Version 2 added the
`alerts` array (id, source, event, severity/urgency/certainty, onset, expires, areas, headline,
description, instruction, sender) and `alert_credits`; version 1 documents still parse. The
top-level type follows the number of locations: one location is a plain object, several are an
array of the same objects in argument order, and a location that failed is
`{"schema_version": 2, "query": …, "error": {"code": …, "message": …}}`. Every key, its type and its
unit are listed in [`docs/schema.md`](docs/schema.md).

### Alerts

Severe-weather warnings are fetched by default (`[alerts] enabled = true`). The sources covering
the location are chosen automatically — the national services first (`nws` for the US and its
territories, `meteoalarm` for the EUMETNET members, `hko` for Hong Kong, `qweather` for China when
that provider is on the chain) and the two global aggregators last (`wmoswic`, the WMO Severe
Weather Information Centre, and `fpas`, the FOSS Public Alert Server, which answer anywhere).
Warnings are normalised to CAP 1.2 and appear as a severity-coloured banner above `art-table` and
`one-line` (the strongest one is also `%A`), as `alert: …` records in `plain`, as the full listing
under `--format alerts`, and as the `alerts` array in `json`. Alerts whose end (`ends`, else
`expires`) has passed are never shown, duplicates from two sources are collapsed, the banner caps at
three lines with a `… and N more` tail, and the strongest is first.

| Flag | Effect |
|---|---|
| `--no-alerts` | skip the extra requests for this run |
| `--alerts` | force the fetch even if `[alerts] enabled = false` |
| `--alerts-from <LIST>` | query exactly these sources, comma separated; one that does not cover the point is a usage error |
| `--severity <LEVEL>` | show only warnings at or above the level (default `minor`, from `[alerts] severity_threshold`) |

MeteoAlarm's endpoints need a token in `CIRROCAST_METEOALARM_KEY`; without one that source is
skipped with a `-v` note and the aggregators still answer. Its portal states access is for members
and re-distributors, and cached warnings are for the local user only — not for redistribution.
Alert responses are cached for `[alerts] cache_ttl_secs` (300 s) under
`$XDG_CACHE_HOME/cirrocast/alerts/`, so a repeated run is served from disk; `--offline` replays the
last set with a `-v` staleness note. A self-hosted FPAS is configured with `[alerts] fpas_url`.

### Air quality

`--aqi` appends an air-quality panel to the table and `plain` output, carries it as the `air`
object in `json`, and makes `%q` expand in `one-line`; `--format aqi` prints the panel standalone.
The reading is one extra keyless request to Open-Meteo's Air Quality API for the location the run
already resolved — the two consolidated indices (US and European AQI), the six regulated
pollutants in μg/m³ and, inside the CAMS European domain, the six pollen species in grains/m³:

```text
Air quality: US AQI 43 (Good) · European AQI 42 (Good)
PM2.5 8.2 · PM10 13.3 · O3 38 · NO2 27.9 · SO2 3 · CO 251 μg/m³
Pollen: alder 0 · birch 0 · grass 0 · mugwort 0 · olive 0 · ragweed 0 grains/m³
UV 5 (moderate) · weather data
Air quality data by Open-Meteo.com (CAMS ENSEMBLE)
```

The category is computed locally from the raw index with the published breakpoints; `--aqi-index`
(`[air] index`, default `us`) selects which scale drives its colour and `%q`, while both raw
numbers are always shown. `--units` does **not** convert these values: an AQI category is not unit
dependent, so the panel keeps the source's μg/m³ exactly as reported (the JSON `air.units` object
says so). The panel's UV line is the `uv_index` already present in the weather report, labelled
`weather data` — the air API's own UV field is deliberately not fetched twice (the UV work in step
17 extends that reading, not this panel).

Below 60 columns the panel switches to one key per line and every line is wrapped to the resolved
width, so it never widens the table. The fetch is **best-effort**: a failure prints `warning: air
quality unavailable: …` on stderr (`-q` silences it), leaves the weather output and the exit code
untouched, and `--format aqi` alone then prints `air quality unavailable`. Outside the pollen
domain the pollen line reads `not covered at this location`; under `-v` the run says why. The
response is cached in the `weather` namespace under
`weather/open-meteo-air-<lat>-<lon>-<local-date>.json` with `cache.weather_ttl_secs`, and
`--no-cache`/`--refresh`/`--offline` behave exactly as they do for the forecast.

### Moon and sun

`--moon` appends a moon/sun block to the table and `plain` output, carries it as the `astro` object
in `json`, and `--format moon` prints the standalone view; `one-line` shows the same numbers through
the `%m` (glyph) and `%M` (phase name) tokens, which need no flag. Everything is **computed on this
machine** — no request, no cache entry, no key:

```text
 ██▒░░  Moon: Last Quarter
████▒░░ 47% illuminated (geocentric) · age 22.7 d
████▒░░ Moonrise 23:49 · Moonset 14:24
 ██▒░░  Sunrise 06:13 · Sunset 17:52 · daylight 11h 39m
```

The moon block is always local: phase (one of eight 45° windows of the synodic elongation), the
geocentric illuminated fraction, the age since the last new moon, moonrise/moonset for the
location-local day, and the next four phase instants (shown by `--format moon`, carried by
`json`). The sun block prefers the backend's own sunrise/sunset and computes them locally only when
the backend sends none — `astro.sun.source` and a `-v` line say which happened. Days without an
event print `—` rather than a clamped `00:00`: inside the polar circles the sun line becomes
`polar day`/`polar night`, and a lunar day (24 h 50 m) can miss a rise or a set. Sunrise, sunset and
the polar state are computed for the location-local calendar day, so a 23-hour spring-forward day
stays 23 hours long and the printed clocks are the location's own.

The arithmetic is the truncated ELP-2000/82 and solar series of Meeus' *Astronomical Algorithms*
(chapters 22, 25, 47 and the chapter-15 event method), with ΔT from the Espenak–Meeus fits. Against
JPL Horizons DE441 the illuminated fraction is within 0.14 pp over the 2026 fixtures and the 1977
and 2044 phase instants of Meeus' examples are reproduced to 32 s and 23 s; the supported range is
1900–2100, where the truncated series stays inside the tolerances the test suite pins.

### Weather

```bash
cirrocast Beijing                       # default format (art-table) and units (metric)
cirrocast Beijing --format plain        # box-free lines, for pipes and logs
cirrocast Beijing -f one-line --template @short
cirrocast Beijing -f json | jq -r '.current.temp_c'
cirrocast @39.9042,116.4074 -f plain    # coordinates: no geocoding request at all
cirrocast --lat 39.9042 --lon 116.4074 -f plain   # the same, as flags
cirrocast Beijing -u us                 # °F, mph, inHg, mi, in
cirrocast Beijing --days 0              # current conditions only
cirrocast Beijing --offline             # bundled names + cached weather, no socket at all
cirrocast Beijing --offline=geo         # resolve the name from the bundle, fetch live weather
cirrocast Beijing --offline=weather     # cache-only weather, live geocoding
cirrocast Beijing --refresh             # ignore the cache and replace it
cirrocast completion bash > ~/.local/share/bash-completion/completions/cirrocast
cirrocast man > cirrocast.1
```

A query resolves the location first (the same four forms as `location search`, with the winning place
echoed on stderr when the name was ambiguous), then walks the provider chain — `--provider` takes an
ordered list, and `auto` expands to the implemented keyless backends that answer for a resolved
place (`metar` is never in it: a station has to be named, and `--station` selects the station
backend — or is prepended to `auto`) — and renders the first report that comes back. A chain entry that fails at the transport
or upstream level falls through to the next one with a `warning:` line; a usage, key or location
error stops the walk. `--days` is clamped to the primary provider's horizon with one `warning:` line.
Every forecast is cached for `cache.weather_ttl_secs` under
`$XDG_CACHE_HOME/cirrocast/weather/<provider>-<lat>-<lon>-<days>-<local-date>.json`, keyed by the
location's own calendar date; a station-based backend keys the same namespace by identifier
(`weather/metar-<ICAO>-current.json`) and keeps the station's metadata under
`$XDG_CACHE_HOME/cirrocast/station/<ICAO>.json` for 30 days. Geocoder answers live under `geocode/`
and `ip/`, the air reading under `weather/<source>-air-<lat>-<lon>-<local-date>.json`, and the OSM
one-request-per-second stamp under `ratelimit/nominatim.json` — `cache stat` reports the five data
namespaces. Providers are also requested in metric, and the renderer converts into
the display units, so a cache entry is unit-independent.

The `one-line` format is a single line by contract, so the credits its licences require go to stderr;
`plain` and `json` carry them in the document itself, and `art-table` in a footer.

## Languages

The output language follows `--lang`, then `CIRROCAST_LANG`, then `defaults.language`, and when all
three say `auto` the ambient locale does: `LC_ALL`, `LC_MESSAGES`, `LANG`, in that order. `en-US` is
the fallback of every chain and the only catalog that must exist.

```bash
cirrocast Beijing --lang zh-CN                # 天气报告：北京…
LANG=zh_CN.UTF-8 cirrocast Beijing            # the same, from the environment
cirrocast Beijing --lang zh-TW -v             # zh-TW → zh-CN → en-US, chain printed
cirrocast Beijing --lang de-DE                # warning on stderr, English output, exit 0
```

Two catalogs ship in the binary (embedded, no files to install): `en-US` and `zh-CN` (Simplified
Chinese). A tag this build cannot serve is a warning, never an error: the run continues in English
and says so on stderr — `-q` silences the warning without changing the output, `-v` prints the
negotiated chain. `zh-TW`, `zh-HK` and `zh-MO` resolve to `zh-CN` through that chain (a deliberate
choice, and one the `-v` line discloses: Traditional readers are not silently served Simplified text
without a note); every other tag falls back to `en-US`.

What is translated: condition names for all 100 WMO codes, day-part, weekday and month names, the
date formats, the measurement labels, the UV bands, the sixteen compass directions, the
air-quality panel (AQI categories, pollutant and pollen names, units) and the
`one-line` vocabulary (`%C`, `%w`, `%U`, `%D`, `%m`, `%q`). What is not: `--help` and the other clap
strings, the art blocks (they are pictograms), and the attribution lines of the data licences, which
stay verbatim next to their data. The `json` format translates `condition.text` and nothing else —
keys stay the machine-readable contract.

Numbers and dates are assembled from catalog messages (`format-*`, `date-*`), so the decimal
separator, the unit spelling and the phrase order of a date (`Thu 01 Oct` vs `9月30日 周三`) are the
translator's, not `chrono`'s English.

### Adding a language

1. `cp -r locales/en-US locales/<tag>` and translate every value in `main.ftl` — the keys and the
   argument names stay as they are.
2. Add one line to `CATALOGS` in `src/i18n.rs`:
   `("<tag>", include_str!("../locales/<tag>/main.ftl")),`.
3. `cargo test --test i18n` — it fails if any of the 100 condition keys, any renderer key or any
   other message is missing, if the catalog has a key `en-US` does not, or if a pattern does not
   render.

No other code changes: a language is a file and one line. The catalogs are embedded with
`include_str!`, so nothing is read from disk at runtime. A new `zh` variant joins the fallback chain
in `language_chain`.

## Location

Four spellings, one deterministic result — the chosen place is always echoed, so a script that runs
the same command twice gets the same answer:

| Argument | Meaning |
|---|---|
| `Beijing` | fuzzy search: the bundled city table first, the keyless Open-Meteo geocoding API on a miss |
| `:Beijing` | only a candidate whose name matches exactly (diacritics and punctuation folded) |
| `~Tsinghua` | OpenStreetMap/Nominatim, cached for 30 days and throttled to one request per second |
| `@39.9042,116.4074` | coordinates; no geocoding request at all |
| *(empty)* | `location.default`, else the public-IP lookup |

Multiple fuzzy candidates are ranked by exact name, then a name prefix, then population, then the
source's own order. When more than one survives, a terminal run asks which one to use — the ranked
list goes to stderr (`[1]`…`[N]`, the winner marked `*`) and one line is read from stdin: an index,
Enter for the winner, or `q` to give up (exit 5; three invalid answers are a usage error, exit 2).
`--pick` forces the prompt even without a terminal, `--yes` takes the ranked winner without asking,
and `[location] pick = "never"` makes that the default; a piped or redirected run never prompts.
The pick is echoed as `selected: Beijing, Beijing Municipality, China — use @39.9042,116.4074 to
skip the prompt`, so the next run can skip the ranking entirely. Without a prompt the ambiguity is
reported once on stderr (suppressed by `-q`) with the winning place and the same `--pick`/`--yes`
hints; add `:` to demand an exact name (`location search` has `--exact`, the weather query does
not — only the `:` prefix narrows a query to an exact name). Coordinates and `~` results carry a
provisional time zone until the forecast response supplies the location's real one, and `~` output
prints `Location data © OpenStreetMap contributors` (ODbL).

```console
$ cirrocast Beijing --pick
[1] * Beijing, Beijing Municipality, China (39.91, 116.40) Asia/Shanghai (pop. 18960744)
[2]   Basingstoke, GB (51.26, -1.09) Europe/London (pop. 107642)
[3]   Beckingen, DE (49.40, 6.70) Europe/Berlin (pop. 15983)
choose a location [1-3, Enter=1, q=quit]: 2
selected: Basingstoke, GB — use @51.26249,-1.08708 to skip the prompt
…the Basingstoke forecast follows on stdout…
```

**Offline names.** A `GeoNames` `cities15000` snapshot is embedded in the binary (about 3.3 MiB
compressed, decoded lazily and never written), so a plain name resolves with no network at all:
folding is NFKD-based, which is why `São Paulo`/`Sao Paulo`, `MÜNCHEN`/`munchen`,
`北京`/`Beijing`/`Peking` and `Wien`/`Vienna` all reach their city. `[geo] strategy` picks the order —
`auto` (the default: the table first, the geocoder on a miss), `bundled` (the table only) or
`network` (the geocoder only) — and the offline path ranks with the same function as the network
path. The table carries the ISO country code rather than the country name the geocoder reports, and
no admin-1 division; `-v` says which source answered. Refresh the snapshot with
`cargo run -p geo-table -- <path-or-url>` — the same fetch/extract/build code as
`location update-data`, writing into `src/geo/data` — and `cargo run -p geo-table -- <path-or-url>
--check` reports whether the committed snapshot still matches a dump (exit 1 when it does not)
without writing anything; run `cargo test --workspace` afterwards, because the suite pins rows of
the committed data, so a refresh is a reviewable diff, never a silent one (the `SNAPSHOT` file
records the dump date and the dump's SHA-256). A user who does not maintain the repository can install a
newer table for their own account instead — see [Updating the city data](#updating-the-city-data).

**Offline countries.** A second embedded dataset answers *which country* a coordinate is in:
Natural Earth's 1:50m admin-0 shapes, quantised to ~110 m (about 407 KiB compressed). It is what
lets `@lat,lon` be named from the bundled tables alone, and it is public domain (CC0-1.0). Refresh
it with `cargo run -p geo-table -- --countries <path-or-url>` (the pinned source is in
`src/geo/data/COUNTRIES`) and check it with the same command plus `--check`.

### Updating the city data

The bundled table is the default and the fallback; a newer dump can be installed per user without
waiting for a release:

```bash
cirrocast location update-data                 # the official GeoNames cities15000 dump
cirrocast location update-data --from ~/Downloads/cities15000.zip   # a local .txt or .zip
cirrocast location update-data --check         # only report whether it would change (exit 1 if so)
```

The command fetches through the same HTTP stack as everything else (proxy, timeout, retries and the
`CIRROCAST_FORBID_NETWORK` guard apply), builds the table with the tool's own encoder, proves it
decodes and installs it atomically under `$XDG_DATA_HOME/cirrocast/geo/`. Name resolution then
prefers it — `-v` names the table and its dump date — while the bundled table stays the fallback:

| `[geo] data` | Behaviour |
|---|---|
| `auto` (default) | the user table when one is installed and valid, else the bundled table |
| `bundled` | always the bundled table (the user table is ignored) |
| `user` | the user table only; a missing or corrupt one is an error naming `location update-data` |

A corrupt user table is diagnosed once, with a warning, and the run falls back to the bundled table
unless `data = "user"` was set. The source is `[geo] update_url` when set (a mirror), else the
official dump; `--from` overrides both for one run.

**Automatic updates are yours to schedule.** Nothing in a query ever fetches city data:
`[geo] update = "check"` only prints a once-a-day note (silenced by `-q`) when the answering table
is older than `update_interval_days` (default 90). To refresh on a schedule, run the idempotent
command from a timer — a run against an unchanged dump rewrites identical bytes:

```ini
# ~/.config/systemd/user/cirrocast-data.service
[Unit]
Description=Refresh cirrocast's city table

[Service]
Type=oneshot
ExecStart=/usr/bin/cirrocast location update-data
```
```ini
# ~/.config/systemd/user/cirrocast-data.timer
[Unit]
Description=Weekly cirrocast city-table refresh

[Timer]
OnCalendar=weekly
Persistent=true

[Install]
WantedBy=timers.target
```

(The cron equivalent is `0 4 * * 1 cirrocast location update-data`. The `GeoNames` credit prints for
either table: `Location data by GeoNames (CC BY 4.0) — https://www.geonames.org/`.)

Non-Latin names are searched in their own script — `新乡`, `Москва`, `Αθήνα`, `القاهرة`, `תל אביב`,
`กรุงเทพ` — because the geocoding service indexes place names per language and an English request
cannot match them. Coverage still differs per source: for some Chinese cities the OpenStreetMap route
is the reliable one (`~新乡市` resolves the city, a bare `新乡` only finds the villages GeoNames
indexes under that name), so both routes are worth trying when a name comes back wrong.

```bash
cirrocast location search Beijing          # Beijing, CN (39.91, 116.40) Asia/Shanghai
cirrocast location search :Beijing         # same line, no ambiguity note
cirrocast location search --all Beijing    # the ranked candidates, numbered, with populations
cirrocast location search --offline sao paulo   # bundled table only, no socket
cirrocast location search '~Tsinghua University' --limit 5
cirrocast location search @39.9042,116.4074
```

**Privacy:** the public-IP lookup is the only request that reveals anything about *you* rather than
about a place you asked for, and it is never implicit — it runs only with `--ip` or when no location
is configured anywhere (`location.default` empty and no positional argument). It sends the public IP
to `ipwho.is`, falling back to `ipapi.co` and `IP.SB` (`CIRROCAST_IP_SERVICE=auto|ipwhois|ipapi|ipsb`),
caches the answer for 24 hours and names the service it used on stderr. An offline policy that
silences the geo scope (`--offline`, `--offline=geo`) refuses the lookup instead, and never opens a
socket.

## Data sources, limits and licences

`cirrocast` ships no data of its own — every place, address and forecast comes from a donated or open
service, each with its own limits and licence. The per-provider record (endpoints, request parameters,
response fields consumed, quotas with their exact wording, caching ceilings and traps) lives in
[`docs/providers.md`](docs/providers.md); what the tool does to stay inside those limits:

| Source | Used for | Limits the service sets | Licence / attribution |
|---|---|---|---|
| [Open-Meteo](https://open-meteo.com/) geocoding | `Beijing`, `:Beijing` | free tier is **non-commercial**, < 10 000 calls/day, 5 000/hour, 600/minute; `name` needs ≥ 2 characters | data CC-BY-4.0; the CLI prints `Location data based on GeoNames (CC-BY-4.0) via Open-Meteo` with the service link |
| [Open-Meteo](https://open-meteo.com/) forecast | every weather query | free tier is **non-commercial**, < 10 000 calls/day; `forecast_days` ≤ 16 | data CC-BY-4.0; the rendered report ends with `Data: Open-Meteo.com (CC BY 4.0)` |
| [Open-Meteo Air Quality](https://open-meteo.com/en/docs/air-quality-api) | `--aqi`, `--format aqi` | same free tier as the forecast API; one keyless request per run; pollen comes from the CAMS European domain only (elsewhere it is absent, which the panel states) | CAMS ENSEMBLE data, CC BY 4.0; the panel ends with `Air quality data by Open-Meteo.com (CAMS ENSEMBLE)` |
| [SMHI](https://opendata.smhi.se/metfcst/snow1gv1) open data | `-p smhi` (Nordics) | no published quota; SMHI's fair-use rules forbid mass downloads and re-fetching the same data | data CC BY 4.0 SE; the rendered report ends with `Data: SMHI (CC BY 4.0 SE)` |
| [aviationweather.gov](https://aviationweather.gov/) (NOAA/NWS) | `-p metar`, `--station` | 100 requests/minute, at most 400 entries per response; an unknown station answers `204 No Content`; a custom `User-Agent` is required | US government work, public domain (no credit mandated); the report still names the source: `Data: aviationweather.gov (NOAA/NWS, public domain)` |
| [OpenWeatherMap](https://openweathermap.org/) | `-p openweathermap` | free tier: 60 calls/minute, 1 000 000 calls/month; two calls per fetch (current + 5-day/3-hourly forecast); a fresh key needs up to 2 hours to activate | data ODbL 1.0; visible attribution required — the rendered report ends with `Data: OpenWeather (ODbL 1.0) — https://openweathermap.org/` |
| [WeatherAPI.com](https://www.weatherapi.com/) | `-p weatherapi` | free tier: 100 000 calls/month, 3-day forecast; `lang=en` is pinned and translation is ours | data proprietary; free keys must credit WeatherAPI.com — the rendered report ends with `Data: WeatherAPI.com (free-tier attribution) — https://www.weatherapi.com/`; caching ceilings 60 min (current) / 24 h (forecast) |
| [World Weather Online](https://www.worldweatheronline.com/) | `-p worldweatheronline` | free tier: 100 requests/day, up to 5 forecast days per its FAQ; `format=json` is sent explicitly | data proprietary; free keys must credit WorldWeatherOnline.com — the rendered report ends with `Data: WorldWeatherOnline.com (free-tier attribution) — https://www.worldweatheronline.com/` |
| [Pirate Weather](https://pirateweather.net/) | `-p pirateweather` | free tier: 10 000 calls/month, 1–4 requests/second; `extend=hourly` is sent for the 7-day horizon | data proprietary; no attribution is mandated — the rendered report ends with `Data: Pirate Weather — https://pirateweather.net/` |
| [QWeather](https://www.qweather.com/) | `-p qweather` | free allowance: first 50 000 requests/month at ¥0, QPM 3 000; needs the account API host in `[providers.qweather].host` | data proprietary; the rendered report ends with `Data: QWeather — https://www.qweather.com/` |
| [GeoNames](https://www.geonames.org/) | the data behind Open-Meteo's geocoding, and the city tables: the bundled one (`src/geo/data`, snapshot `cities15000`, dump date in `SNAPSHOT`) and any the user installs with `location update-data` under `$XDG_DATA_HOME/cirrocast/geo/` | — | CC-BY-4.0; both paths print `Location data by GeoNames (CC BY 4.0) — https://www.geonames.org/` |
| [Nominatim](https://nominatim.openstreetmap.org/) / OpenStreetMap | `~Tsinghua` | ≤ 1 request/second, an identifying `User-Agent`, results must be cached, no autocomplete and no bulk geocoding | data ODbL; `Location data © OpenStreetMap contributors (ODbL)` is printed; the service is switchable through `network.nominatim_url` without a code change, which the policy requires |
| [api.weather.gov](https://api.weather.gov/) (NOAA/NWS) | alerts for the US and territories | a descriptive `User-Agent` (sent); public-domain data | no credit mandated; the source is named in the listing |
| [MeteoAlarm](https://api.meteoalarm.org/) | alerts for EUMETNET members | a bearer token (`CIRROCAST_METEOALARM_KEY`) that the portal issues to members and re-distributors; the source is optional | cached warnings are for the local user only, not for redistribution |
| [WMO SWIC](https://severeweather.wmo.int/) | global alert aggregator | keyless; the WFS index and one CAP document per warning are cached | credit printed with the warnings: `Warnings by the WMO Severe Weather Information Centre (severeweather.wmo.int), © the issuing agencies` |
| [FPAS](https://alerts.kde.org/) | global alert aggregator, self-hostable | keyless donated instance (`[alerts] fpas_url` points at your own) | credit printed with the warnings: `Warnings via the FOSS Public Alert Server (<host>)` |
| [Hong Kong Observatory](https://data.weather.gov.hk/) | alerts for Hong Kong | keyless open data | credit printed with the warnings: `Warnings by the Hong Kong Observatory` |
| [ipwho.is](https://ipwho.is/) | `--ip` (primary) | free endpoint: 1 000 requests/day per client IP, then `429` + `Retry-After` | personal or internal use, no redistribution |
| [ipapi.co](https://ipapi.co/) | `--ip` (fallback) | free tier: up to 1 000 requests/day | internal use, no resale; its terms allow keeping an answer for **at most 24 hours**, which is why `cache.ip_ttl_secs` is capped there |
| [IP.SB](https://ip.sb/) | `--ip` (last fallback) | keyless public endpoint, worldwide, no documented quota; a 429 is honoured like the others' | no attribution required; answers are cached under the same 24-hour cap |

Alert output carries its own credits: WMO SWIC names the issuing agencies and FPAS the instance,
printed in the `alerts` listing, the `art-table`/`plain` footers, the `json` `alert_credits` array
and, for `one-line`, on stderr. The air-quality credit (`Air quality data by Open-Meteo.com (CAMS
ENSEMBLE)`) travels inside the air panel — the `art-table` footer area, the `plain` document, the
`aqi` listing and the `json` `air` object — because the numbers it describes are air data, not
forecast data. The weather output carries the credit the data licence asks for — `Data: Open-Meteo.com (CC BY 4.0)` or
`Data: SMHI (CC BY 4.0 SE)` — plus a provenance line (`attribution: open-meteo
https://api.open-meteo.com/v1/forecast`), both taken from the provider registry rather than
hard-coded, and a resolved place adds the matching GeoNames line next to them (`via Open-Meteo` for
the geocoder, `by GeoNames` for the bundled table). Where the credit travels
depends on the format: the `art-table` footer, the `plain` document and the `json` `attribution`
object carry it themselves, while `one-line` — one line by contract — prints it to stderr.

Nothing from these services is redistributed: responses are cached under
`$XDG_CACHE_HOME/cirrocast/` with the TTLs in `[cache]` (10 minutes for weather, 24 hours for an IP
location, 30 days for geocoding) and `--no-cache`, `--refresh` and `--offline` decide what that cache
is used for. The handful of recorded responses used as test fixtures keep their upstream licence —
`CC-BY-4.0` for GeoNames/Open-Meteo, `ODbL-1.0` for OpenStreetMap — declared per path in
[`REUSE.toml`](REUSE.toml).

## Backends

| ID | Key | Coverage | Observation | Forecast | Max days |
|---|---|---|---|---|---|
| `open-meteo` | none | global | yes | yes | 16 |
| `met-no` | none | global | yes | yes | 9 |
| `smhi` | none | Nordics and adjacent seas | yes | yes | 10 |
| `nws` | none | the US and its territories | no | yes | 7 |
| `brightsky` | none | Germany (DWD open data) | yes | yes | 10 |
| `metar` | none | worldwide stations | yes | no | — (observation) |
| `open-meteo-archive` | none | global | no | archive | — (1940-01-01 onward, `--date`/`--history`) |
| `open-meteo-marine` | none | coastal waters | supplement | supplement | 8 (waves and swell, `--marine`) |
| `openweathermap` | `CIRROCAST_OPENWEATHERMAP_KEY` | global | yes | yes | 5 |
| `weatherapi` | `CIRROCAST_WEATHERAPI_KEY` | global | yes | yes | 3 |
| `worldweatheronline` | `CIRROCAST_WORLDWEATHERONLINE_KEY` | global | yes | yes | 5 |
| `pirateweather` | `CIRROCAST_PIRATEWEATHER_KEY` | global | yes | yes | 7 |
| `qweather` | `CIRROCAST_QWEATHER_KEY` | global | yes | yes | 10 |
| `visualcrossing` | `CIRROCAST_VISUALCROSSING_KEY` | global | yes | yes | 15 |

`auto` is the keyless chain that answers for a resolved place, **ranked by coverage**: an exact
country match first (`nws` for a US point, `brightsky` for a German one), then a containing bounding
box (`smhi` in the Nordics), then the global entries (`open-meteo`, `met-no`), each tier in registry
order — `-v` prints the chain that was chosen, and a multi-location run ranks each slot for its own
place. `metar` is prepended for a `--station` run. `open-meteo-marine` is **supplementary** — it is never a chain entry, and
`--marine` is what requests it; `open-meteo-archive` is archive-only and answers `--date`/
`--history` instead of a forecast.

A backend is admitted only when it publishes a documented, public API with a stated licence, an
identity/attribution policy and a machine-readable payload that fills all four day parts. Two
candidate classes are refused on that test: **scraped HTML sites** (gismeteo-class pages publish no
data licence, forbid extraction, and an HTML parser breaks silently on a layout change — for a
weather tool that means confidently wrong output), and **`wttr.in` used as a data source** (it is
this project's *layout* reference, and it is itself an aggregator rather than a data origin). The
same rule refuses credentials that would have to ship inside the binary and APIs reachable only
through reverse-engineered private routes.


Declared capabilities only, each row carrying the date it was last checked against the provider's live
documentation (`provider info <ID>` prints it). The full record — endpoints, request parameters,
response fields consumed, free-tier quotas with their exact wording, licence duties, caching ceilings
and the traps — lives in [`docs/providers.md`](docs/providers.md); `cirrocast provider list` /
`provider info <ID>` print the machine-readable subset from the binary itself. The 2026-09-30
re-verification corrected this table (SMHI's endpoint and horizon, WWO's 5-day free horizon,
QWeather's global coverage and v1 horizon) and is recorded in that file's log.

A chain falls through to the next backend **only** when the failure is transport-level or comes from
upstream (a timeout, a `429`, a `5xx`, an out-of-coverage point); a usage, configuration, location or
credential error stops the walk, because retrying another backend cannot fix it. `--verbose` prints
one line per attempt plus `attribution: <credit> (<request URL>)` after the fetch that answered, so
the exact upstream call is visible without a packet capture. Every rendered report carries the credit
its data licence asks for — `Data: Open-Meteo.com (CC BY 4.0)`, `Data: SMHI (CC BY 4.0 SE)`,
`Data: OpenWeather (ODbL 1.0) — https://openweathermap.org/` and so on, all taken from the provider
registry rather than hard-coded — and `provider info <ID>` prints the same line under `credit:` along
with the provider's auth mechanism, coverage, granularity and documented limits.

Keys are BYOK and stored outside `config.toml`, in `keys.toml` (mode `0600`) or in the provider's
environment variable — see [Configuration § API keys](#api-keys).

## Configuration

Everything lives under the XDG directories: the configuration document in
`$XDG_CONFIG_HOME/cirrocast/config.toml` (default `~/.config/cirrocast/config.toml`), API keys in
`$XDG_CONFIG_HOME/cirrocast/keys.toml`, the cache in `$XDG_CACHE_HOME/cirrocast/` and data in
`$XDG_DATA_HOME/cirrocast/`. When no user file exists, `$XDG_CONFIG_DIRS` (default `/etc/xdg`) is
searched for a system wide one. `cirrocast config path` prints the user file — the one `init`/`set`
write and the first one read — and `config validate` reports the file that was actually read.

| Key | Default | Values |
|---|---|---|
| `defaults.provider` | `open-meteo` | provider id, comma separated chain, or `auto` |
| `defaults.format` | `art-table` | `art-table`, `one-line`, `plain`, `json`, `dumb`, `alerts`, `aqi`, `moon`, a one-line preset (`full`, `minimal`, `short`, `default`, `uv`, `sun`) or a `[templates]` key |
| `defaults.units` | `metric` | `metric`, `us`, `uk` |
| `defaults.days` | `3` | `0..=14`, clamped per provider |
| `defaults.language` | `auto` | `auto` or a BCP-47 tag such as `zh-CN` |
| `location.default` | empty | `Beijing`, `:Beijing`, `@39.9,116.4`, `~Tsinghua`, `@home` (an alias) |
| `location.pick` | `auto` | `auto` (ask on a terminal when a name has several candidates), `never` |
| `locations.<NAME>` | none | any location argument; define `@NAME` (values may name other aliases) |
| `templates.<NAME>` | none | a literal `%`-token template for `--template @NAME` / `--format NAME` |
| `units.temp` | unset | `c`, `f` |
| `units.wind` | unset | `kmh`, `mph`, `mps`, `knots` |
| `units.pressure` | unset | `hpa`, `inhg`, `mmhg` |
| `units.distance` | unset | `km`, `mi` |
| `units.precip` | unset | `mm`, `in` |
| `network.timeout_secs` | `15` | `1..=300` |
| `network.retries` | `3` | `0..=10` |
| `network.proxy` | empty | `scheme://host[:port]` or `host:port`; empty = direct |
| `network.nominatim_url` | empty | Nominatim base URL for `~name` searches; empty = the public OpenStreetMap service |
| `cache.enabled` | `true` | `true`, `false` |
| `cache.weather_ttl_secs` | `600` | `> 0` (10 minutes) |
| `cache.ip_ttl_secs` | `86400` | `> 0` (24 hours; larger values are capped — see [Data sources, limits and licences](#data-sources-limits-and-licences)) |
| `cache.geocode_ttl_secs` | `2592000` | `> 0` (30 days) |
| `render.color` | `auto` | `auto`, `always`, `never` |
| `render.width` | `0` | `0` (detect from the terminal) or `40..=500` |
| `alerts.enabled` | `true` | `true`, `false` |
| `alerts.severity_threshold` | `minor` | `unknown`, `minor`, `moderate`, `severe`, `extreme` |
| `alerts.sources` | `["auto"]` | `["auto"]` or source ids: `nws`, `meteoalarm`, `qweather`, `hko`, `wmoswic`, `fpas` |
| `alerts.fpas_url` | empty | FOSS Public Alert Server base URL; empty = `https://alerts.kde.org` |
| `alerts.cache_ttl_secs` | `300` | `> 0` (5 minutes) |
| `air.index` | `us` | `us`, `european` — the AQI scale behind the panel colour and `%q` |
| `providers.metar.station` | empty | ICAO identifier, e.g. `ZBAA` |
| `providers.qweather.host` | empty | your QWeather API host, from <https://console.qweather.com/setting> (e.g. `https://<account-id>.re.qweatherapi.com`) |

The `[units]` overrides are per quantity and optional: an absent (or empty) key follows
`defaults.units`, so switching that one value to `us` moves every quantity that was not pinned.

`[locations]` and `[templates]` are free-form tables whose keys are yours: an alias maps `@name`
to any location argument (including another alias; chains are cycle-checked, a bad chain is a
config error at load), and a named template is a literal `%`-token string that `--format` and
`--template @name` both understand. `config.toml` is the only place to edit them; `config get`/`set`
address single keys only.

```toml
schema_version = 2

[locations]
home = "@39.9042,116.4074"      # cirrocast @home
office = ":Shanghai"            # aliases may chain: office = "@home" also works

[templates]
compact = "%c%t"                # cirrocast -f compact  /  --template @compact
```

Precedence, highest first: **command line flag → `CIRROCAST_*` environment variable → `config.toml`
→ built-in default**. The variables are `CIRROCAST_PROVIDER`, `CIRROCAST_FORMAT`,
`CIRROCAST_UNITS`, `CIRROCAST_DAYS`, `CIRROCAST_LANG`, `CIRROCAST_LOCATION`,
`CIRROCAST_LOCATION_PICK`, `CIRROCAST_TIMEOUT`, `CIRROCAST_NOMINATIM_URL` and
`CIRROCAST_IP_SERVICE`; API keys use their own
`CIRROCAST_<PROVIDER>_KEY` namespace (below). `cirrocast config get <KEY>` prints the effective
value, environment override included.

```bash
cirrocast config path                              # where the file lives (creates nothing)
cirrocast config init [--force]                    # annotated default document
cirrocast config get defaults.days                 # 3
cirrocast config set defaults.days 5 && cirrocast config show
cirrocast config validate                          # ok: ~/.config/cirrocast/config.toml
cirrocast config edit                              # $VISUAL/$EDITOR, re-validated afterwards
```

`config set` rewrites the whole document in canonical form, so hand-written comments do not survive
it; `config init --force` writes the fully annotated document back. The file is `0644` — it is meant
to be pasted into bug reports — unknown keys from a newer release are ignored, and a
`schema_version` above the supported one is refused instead of guessed.

### API keys

Provider keys are BYOK and never enter `config.toml` (which is world readable, shown by `config show`
and hand edited). First hit wins: `CIRROCAST_<PROVIDER>_KEY` (provider id upper-cased, `-` → `_`,
e.g. `CIRROCAST_OPENWEATHERMAP_KEY`) → `keys.toml` in the configuration directory. There is no
third tier: a key in neither place is simply missing (OS keyring storage is explicitly out of scope
for v1).

The MeteoAlarm alert token is the one credential outside the provider key store: it is read from
`CIRROCAST_METEOALARM_KEY` only (the service is not a weather provider, so `key set` does not know
it), and `[alerts] sources`/`--alerts-from` decide whether that source is used at all.

```bash
printf %s "$CIRROCAST_OPENWEATHERMAP_KEY" | cirrocast key set openweathermap
cirrocast key list        # openweathermap  sk-t…56  (file)
cirrocast key rm openweathermap
```

`key set` reads the secret from stdin only — never from the command line, where the process list and
the shell history would see it — prompting on the terminal with echo disabled when stdin is a
terminal. The stored file is mode `0600`; a file that group or other can read is refused with the
`chmod 600` fix instead of being used. `key list` prints masked values only, and `keys.*` is not a
configuration namespace: `config set keys.openweathermap …` fails as an unknown key.

## Licence

GPL-3.0-or-later — see [`LICENSE`](LICENSE). The repository is
[REUSE](https://reuse.software/)-compliant; per-file copyright and licence information lives in
SPDX headers and in [`REUSE.toml`](REUSE.toml).

## Versioning

Four version numbers move independently; only the first three are visible to a consumer, and
[`docs/schema.md`](docs/schema.md) is where their change rules and their fields live.

| Number | Where it appears | Now | Changes when |
|---|---|---|---|
| crate version | `Cargo.toml`, `--version`, release tags | `1.2.0` | any release, following SemVer |
| JSON schema version | `"schema_version"` in every `-f json` document | `2` | a key is removed, renamed, retyped, changes unit, or changes between always-present and nullable |
| config schema version | `schema_version` in `config.toml` | `2` | an existing key's meaning, type or validity changes (a new key with a default is additive and does not) |
| cache envelope version | `cache_schema_version` in cache files | `1` | internal only: a mismatch is a cache miss, never an error |

The release schedule, matching [`docs/plans/README.md`](docs/plans/README.md):

* **`0.x`** while phases A–C landed — `0.1.0` (2026-10-01) was the first release;
* **`1.0.0`** (2026-10-02) freezes the CLI, the configuration schema and the JSON schema for the v1
  scope (phases A–C, steps 01–14);
* **`1.1.0`** (2026-10-04) starts phase D — severe-weather alerts, air quality and moon/astro
  (steps 15–17), with the JSON document at `schema_version` 2;
* **`1.2.0`** (2026-10-05) completes phase D — the offline city database and user-installed table
  updates (steps 18/18b), multi-location output with the shared template engine (19) and the
  interactive location picker (20), with the configuration document at `schema_version` 2;
* **`1.3.0`** for phase E (steps 21–22: performance and resource budgets, the `status` probe and the
  ecosystem recipes);
* **`1.4.0`** for phases F and G (steps 23–28: additional backends, coverage-aware `auto`,
  second-generation location sources, climate normals, QWeather JWT, the documentation set). The
  backlog (B01–B02: the wttr-compatible local service, the multi-platform packaging matrix) is not
  scheduled.

Within a major version, adding a key to the JSON document or adding a config key with a built-in
default is a **minor** change: a consumer keeps working if it ignores what it does not know.
Removing, renaming, retyping or re-meaning anything is a **major** change, bumps `schema_version`
and is named in [`CHANGELOG.md`](CHANGELOG.md).

## Packaging and release

| Install path | What lands on the system | Built from |
|---|---|---|
| `cargo install --locked cirrocast` (or `--path .`) | the binary in `$CARGO_HOME/bin`; the man page and completions are generated by it on demand | crates.io, or this checkout |
| AUR [`cirrocast`](https://aur.archlinux.org/packages/cirrocast) | `/usr/bin/cirrocast`, `cirrocast.1`, the bash/zsh/fish completions, and `LICENSE` under `/usr/share/licenses/cirrocast/` | the `vX.Y.Z` tag tarball from GitHub |
| Release archive `cirrocast-vX.Y.Z-<target>.tar.gz` | `cirrocast-vX.Y.Z-<target>/` containing the binary, `README.md`, `LICENSE`, `CHANGELOG.md`, `cirrocast.1` and `completions/` — nothing is installed for you | the release workflow (`.github/workflows/release.yml`) |

Archives are built natively on `ubuntu-26.04` (x86_64), `ubuntu-26.04-arm` (aarch64) and `macos-26`
(Apple silicon), one job per target, with no cross-compilation; a Linux archive therefore needs that
image's glibc or newer. On an older distribution the AUR package is the better fit — it builds from
the tag tarball against the system's own glibc. Windows is not packaged in v1: `cargo install`
covers it, and further ecosystem packages are deferred to the backlog (B02). Every archive ships
next to the `.sha256` the workflow computed:

```bash
sha256sum -c cirrocast-v1.2.0-x86_64-unknown-linux-gnu.tar.gz.sha256   # macOS: shasum -a 256 -c
```

`cargo package --list --locked` and `cargo publish --dry-run --locked` run on every pull request
(the CI `package` job below), so the file set a release uploads is reviewed before any tag exists.

### Release checklist

1. The four gates plus `cargo deny check` are green on the commit to be tagged, and CI is green on it.
2. The bundled city data is current: `cargo run -p geo-table -- <path-or-url> --check` reports every
   file `unchanged` against the official dump. If it reports `CHANGED`, refresh the snapshot
   (`cargo run -p geo-table -- <path-or-url>`), run `cargo test --workspace` (the suite pins rows of
   the committed data), re-record the size/timing numbers in
   [`docs/plans/21-perf-and-resource-budget.md`](docs/plans/21-perf-and-resource-budget.md), and
   commit all of it before tagging.
3. The bundled country layer is current too: `cargo run -p geo-table -- --countries
   https://raw.githubusercontent.com/nvkelso/natural-earth-vector/v5.1.2/geojson/ne_50m_admin_0_countries.geojson
   --check` reports both files `unchanged`. `CHANGED` → rebuild without `--check`, run
   `cargo test --workspace` (the layer's canary pins coordinates and the size budget), and commit
   `src/geo/data`.
4. `version` bumped in `Cargo.toml`; `CHANGELOG.md` gets its dated section and compare link; both
   committed.
5. `cargo package --list --locked` reviewed (`src/`, `locales/`, `Cargo.toml`, `Cargo.lock`,
   `LICENSE`, `README.md`, `CHANGELOG.md` — and nothing else).
6. A signed tag is pushed: `git tag -s vX.Y.Z -m "cirrocast vX.Y.Z" && git push origin vX.Y.Z`. The
   workflow refuses a tag that does not match `Cargo.toml`'s version.
7. The run is watched: three archives, three `.sha256` files, and a GitHub release with generated
   notes. One archive is inspected (`tar tzf`) and verified (`sha256sum -c`).
8. The `publish` job is approved in the `crates-io` environment — `cargo publish` cannot be undone.
9. The AUR package is bumped, which is only possible once the tag exists because the checksums come
   from the tag tarball:

```bash
git clone ssh://aur@aur.archlinux.org/cirrocast.git   # the packaging files live only there
cd cirrocast                                          # edit pkgver= in PKGBUILD, then:
updpkgsums                                            # recompute sha256sums from the tag tarball
makepkg --printsrcinfo > .SRCINFO                     # AUR metadata, regenerated in the same commit
makepkg --printsrcinfo | diff - .SRCINFO              # must print nothing
namcap PKGBUILD
makepkg -f && namcap cirrocast-*.pkg.tar.zst
git commit -am "upgpkg: cirrocast X.Y.Z-1" && git push
```

```bash
# what a bump is verified with, on a clean machine or in a chroot
pkgctl build                         # devtools clean chroot; makechrootpkg -c does the same
sudo pacman -U cirrocast-1.2.0-1-x86_64.pkg.tar.zst
cirrocast --version && man -w cirrocast
pacman -Ql cirrocast | grep -E 'completions/cirrocast$|site-functions/_cirrocast$|vendor_completions\.d/cirrocast\.fish$' | wc -l   # 3
```

(A plain `pacman -Ql cirrocast | grep -c completions` prints 4: the two completion *directories* match
as well, which is why the three file paths are matched explicitly.)

The AUR repository is the only home of the packaging files — this tree keeps no copy, which is why
the checklist edits them there (the reasoning is in
[`docs/plans/13-packaging-and-release.md`](docs/plans/13-packaging-and-release.md)). `check()` in the
PKGBUILD compares `cargo metadata`'s version with `$pkgver`, so a stale PKGBUILD fails the build
instead of shipping a mislabelled package.

## Publishing

The crate is prepared for crates.io, but publishing is a deliberate act rather than a side effect of
tagging: it runs in the release workflow's `publish` job, behind the protected `crates-io`
environment, and `cargo publish` cannot be undone.

```bash
cargo package --list --locked        # the exact file set that would be uploaded
cargo publish --dry-run --locked     # builds the packaged crate; uploads nothing
cargo doc --no-deps --all-features   # what docs.rs renders
```

Checklist, executed 2026-10-01 before the first release:

* `cirrocast` is free on crates.io (checked against the registry API);
* `cargo package --list --locked` shows 46 files — `src/**`, `locales/**`, `Cargo.toml` (plus
  cargo's own `Cargo.toml.orig`), `Cargo.lock`, `LICENSE`, `README.md`, `CHANGELOG.md` — and no
  `docs/`, `tests/`, `examples/` or `.github/`;
* `cargo publish --dry-run --locked` succeeds, building the crate from the packaged file set;
* `cargo doc --no-deps --all-features` builds the library target without warnings, and
  `[package.metadata.docs.rs] all-features = true` is set;
* `license = "GPL-3.0-or-later"` with no `license-file` (cargo forbids both);
* `Cargo.lock` is committed, because every release path builds with `--locked`.

`cirrocast 1.0.0` was published on 2026-10-02 with `cargo publish --locked` from the tagged tree
(the `v1.0.0` release workflow's `publish` job had skipped with its notice, because that was before
the crate moved to trusted publishing). The crate's crates.io settings now enable **"Require trusted
publishing for all new versions"**, so token-based publishing is refused from here on.

Since 2026-10-02 the release workflow publishes with **crates.io trusted publishing** (OIDC): no
token is stored in GitHub. The `publish` job runs inside the `crates-io` environment with
`id-token: write` and exchanges the workflow's OIDC identity for a short-lived crates.io token via
[`rust-lang/crates-io-auth-action`](https://github.com/rust-lang/crates-io-auth-action), pinned by
commit SHA in `.github/workflows/release.yml`. The crate's trusted-publisher record names three
things — the repository (`YangtseSu/cirrocast`), the workflow file (`release.yml`) and the
environment (`crates-io`) — and the exchange fails if any of them changes. `gh workflow run
release.yml` executes only the `verify` job, which proves that record matches without cutting a
release; a version that is already on crates.io is skipped by the publish job's own check.

## Development and CI

The check matrix is the same locally and in CI ([`.github/workflows/ci.yml`](.github/workflows/ci.yml)). The performance budget is the exception: it lives in [`.github/workflows/perf.yml`](.github/workflows/perf.yml), which is dispatch-only, because it builds the release profile and re-records the baseline on request:

| Job | Command | Notes |
|---|---|---|
| `fmt` | `cargo fmt --check` | stable toolchain |
| `clippy` | `cargo clippy --workspace --all-targets --locked -- -D warnings` | warnings are errors; the workspace flag also covers the `geo-table` builder |
| `test` | `cargo test --workspace --locked`, then `cargo test --workspace --no-default-features --locked` | one leg, `ubuntu-26.04` on stable, with `CIRROCAST_FORBID_NETWORK=1`; the second command is the reduced build (city search via the network geocoder) |
| `package` | `cargo package --list --locked`, `cargo publish --dry-run --locked` | the crate's file set and the packaged build, reviewed on every pull request; nothing is uploaded |
| `gates` | `python3 scripts/check-render-imports.py`, `cmp LICENSE LICENSES/GPL-3.0-or-later.txt` | repository invariants that no compiler enforces |
| `reuse` | `reuse lint` | every file carries SPDX information |
| `deny` | `cargo deny check` | licences, advisories, bans, sources |
| `audit` | `cargo audit` | independent advisory check beside `deny` |

* **Toolchain**: `rust-version` in `Cargo.toml` names the stable toolchain the crate builds with
  (`1.99` at `v1.2.x`). The project tracks stable, holds no floor below it and supports no older
  toolchain — CI runs a single test leg — so there is no MSRV job to keep in sync. macOS is covered
  where it ships: the release workflow builds and tests the release profile on `macos-26` before it
  packs an archive.
* **The performance budget** is `.github/workflows/perf.yml` behind an explicit dispatch: it runs
  `scripts/bench/run.sh` and holds the fresh medians against the committed baseline with
  `scripts/bench/compare.py` — the timing metrics against their budgets, the size and RSS metrics
  against the 20 % ratio as well. `record_baseline` re-records the baseline instead of comparing,
  and `audit` adds the `cargo bloat`/`cargo llvm-lines` dependency-weight report. Nothing in it runs
  on a push or a pull request.
* **No test may open a network connection.** `CIRROCAST_FORBID_NETWORK=1` makes `src/http.rs`
  refuse every non-loopback request before DNS or connect, and the test job exports it for the
  whole suite, so a network-dependent test fails loudly. Live smoke tests are `#[ignore]`d and run
  manually (`CIRROCAST_LIVE_TESTS=1 cargo test -- --ignored`).
* **Dependency policy** lives in [`deny.toml`](deny.toml): an audited licence allow list (GPL-3.0-or-later
  admits the permissive licences, MPL-2.0 and the data licences `Unicode-3.0`, `CDLA-Permissive-2.0`
  and `ODbL-1.0`), duplicate versions and wildcard requirements denied, crates.io as the only
  source, and every advisory ignore carrying a reason plus an expiry date.
* **Dependabot** checks both ecosystems once a day ([`.github/dependabot.yml`](.github/dependabot.yml)):
  `cargo` for `Cargo.toml` and the committed `Cargo.lock`, and `github-actions` for the action pins
  in the workflows — it moves each pinned commit SHA together with its `# vX.Y.Z` comment. Minor and
  patch bumps arrive as one grouped pull request per ecosystem; a major bump keeps its own, because
  the `test`, `deny` and `audit` gates are what decide whether it can land. Dependabot alerts and
  security updates are enabled in the repository settings.
* **Reproducibility**: every third-party action is pinned to a commit SHA and the runner images are
  named explicitly (`ubuntu-26.04`, `macos-26`, `ubuntu-26.04-arm`) instead of `<os>-latest`; the
  release matrix ([`.github/workflows/release.yml`](.github/workflows/release.yml)) follows the same
  rule and asserts that the tag equals the crate version.

## Acknowledgements

* [wttr.in](https://wttr.in)
* [wego](https://github.com/schachmat/wego)
