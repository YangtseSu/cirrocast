<!--
SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
SPDX-License-Identifier: GPL-3.0-or-later
-->

# cirrocast

Terminal weather client. Pluggable weather backends, wttr.in-style output, city-name geocoding,
optional IP-based location, configurable units and output language, XDG-compliant state.

`cirrocast` is a from-scratch replacement for [`wego`](https://github.com/chubin/wego) (unmaintained,
stale backends, docs that do not match the code). It takes [`wttr.in`](https://wttr.in) as the
*output* reference and shares no code, art or data with either project.

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
│ (     ) ↖ 19km/h NNW  │    /│\  ← 0.0km/h W   │    /│\  ↗ 6.0km/h NNE │
│  ╰───╯  0.0mm 18%     │         0.0mm 13%     │         0.0mm 27%     │
… the noon, evening and night rows of each day, then the credits …
```

```
$ cirrocast Beijing --format plain
location: Beijing, Beijing Municipality, China (39.91, 116.40) Asia/Shanghai
updated: 2026-09-30T19:45:00+08:00
current: Clear sky 18°C (feels 13°C) wind 13km/h NW humidity 11% precip 0.0mm pressure 1021hPa visibility 17km
day 2026-09-30: Morning Partly cloudy 17°C 0.0mm (0%) wind 19km/h NNW | Noon Overcast 20°C 0.0mm (0%) wind 19km/h NW | Evening … | Night …
… one `day` line per forecast day …
Location data based on GeoNames (CC-BY-4.0) via Open-Meteo — https://open-meteo.com/
Data: Open-Meteo.com (CC BY 4.0)
attribution: open-meteo https://api.open-meteo.com/v1/forecast
```

```
$ cirrocast Beijing -f one-line --template @short
*o* +18°C
```

Those are real runs — 2026-09-30, 19:45 local, `COLUMNS=120`, `TERM=xterm-256color` — with the middle
rows elided. When the name is ambiguous a `note: … 10 candidates …` line goes to stderr, so stdout
stays pipeable; `-q` silences it and `:Beijing` demands an exact name.

## Status

Steps 01–08 of 24 in [`docs/plans/`](docs/plans/README.md) are in place: the CLI skeleton, XDG path
resolution, the provider registry, the typed configuration with its `config`/`key` subcommands, the
canonical model, location resolution, the shared HTTP/cache layer, the Open-Meteo forecast, the
wttr.in-style `art-table` renderer and the full flag matrix with the four other output formats
(`one-line`, `plain`, `json`, `dumb`), shell completions and the man page. Localization (step 09),
the remaining backends (10, 11) and the quality gates (12–14) are still ahead, so `--lang` accepts
`auto`/`en-US` only and `--provider` only names implemented backends. See the plan index for live
per-step progress.

## Install

```bash
cargo install --path .        # from a checkout
# AUR packages and prebuilt release archives: planned, step 13 (docs/plans/13-packaging-and-release.md)
```

## Usage

```
cirrocast [OPTIONS] [LOCATION]

  -p, --provider <ID[,ID...]>   open-meteo | smhi | metar | openweathermap | weatherapi
                                | worldweatheronline | pirateweather | qweather | auto
  -f, --format <NAME>           art-table | one-line | plain | json | dumb
  -d, --days <N>                0..=14, clamped to what the provider serves
  -u, --units <SYSTEM>          metric | us | uk
      --lang <TAG>              BCP-47, or "auto"
      --lat <DEG> --lon <DEG>   coordinates instead of a location argument
      --ip                      locate from the public IP
      --station <ICAO>          METAR station (needs --provider metar or auto)
      --template <TEMPLATE>     one-line template or @PRESET
      --no-cache | --refresh | --offline
      --timeout <SECS>
      --color <WHEN>            auto | always | never
      --width <COLS>            layout width for the table formats, 1..=500
  -q, --quiet    -v, --verbose
  -h, --help     -V, --version

cirrocast config     <path|init|show|get|set|edit|validate>
cirrocast key        <set|rm|list>
cirrocast provider   <list|info>
cirrocast cache      <stat|clean>
cirrocast location   <search>
cirrocast completion <bash|zsh|fish|elvish|powershell> [--bin-name NAME]
cirrocast man [--bin-name NAME]
```

Location syntax: `Beijing` (fuzzy), `:Beijing` (exact name), `~Tsinghua` (OpenStreetMap),
`@39.9,116.4` (coordinates), empty (config default, else public IP). A location argument,
`--lat/--lon`, `--ip` and `--station` are mutually exclusive; when the argument comes from
`CIRROCAST_LOCATION` instead of the command line, a flag on one of the other forms wins by
precedence rather than conflicting.

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

| Format | What it is | Width |
|---|---|---|
| `art-table` | the wttr.in-style coloured day-part table (default) | honours `--width`, degrades to a stacked layout below 60 columns |
| `dumb` | the same table in 7-bit ASCII, no colour; automatic for `TERM=dumb` or a non-UTF-8 locale | as above |
| `one-line` | one line driven by `%` tokens, for a prompt or status bar | fixed |
| `plain` | box-free `label: value` lines, one record per line | ignores `--width`: a record is never truncated |
| `json` | the stable machine-readable document (`schema_version: 1`) | ignores `--width` and `--units` |

`one-line` takes a template with `--template`, either a literal string or a preset:

| Token | Output | Token | Output |
|---|---|---|---|
| `%c` | condition art, day/night aware | `%d` `%D` | `2026-09-30` / `Thu 01 Oct` |
| `%C` | condition text | `%Z` `%z` | `Asia/Shanghai` / `+0800` |
| `%t` `%f` | temperature / feels-like | `%u` `%U` | `5` / `5 (moderate)` |
| `%w` | wind `↗ 12km/h NE` | `%S` `%s` | sunrise / sunset `06:05` |
| `%h` | humidity `56%` | `%l` `%L` | name / `39.90,116.40` |
| `%p` | precipitation `0.0mm` | `%m` | moon phase — `n/a` until the moon step lands |
| `%P` | pressure `1013hPa` | `%v` | visibility `10km` |

`%%` is a literal `%`, a trailing lone `%` is one too, `%{…}` is verbatim (`\}` escapes the brace),
`\n`/`\t`/`\\` are unescaped, and an unknown `%X` prints literally and is reported once under `-v`.
A value the provider does not report prints `n/a` — never an invented number. The presets are
`@default` (`%l: %c %C %t (%f), %w, %h, %p, %P, %v`), `@short` (`%c %t`),
`@full` (`%l: %c %C %t (%f) %w %h %p %P %m %v %u %S %s %Z`), `@uv` (`%l: UV %U`) and
`@sun` (`%l: sunrise %S sunset %s (%z %Z)`); `--template` without a value, `-f plain/json/…` next to
`--template`, and an unknown `@name` are usage errors.

`json` is the scripting surface: keys are always present (`null` when the provider has no value), all
numbers are canonical metric with the unit in the key (`temp_c`, `wind_kmh`, `precip_mm`,
`pressure_hpa`, `visibility_km`), timestamps carry the location's offset, and `attribution` carries
the credits. Within `schema_version: 1` changes are additive only — new keys may appear and existing
ones keep their name, type and unit — so a consumer must ignore keys it does not know; a breaking
change bumps the version (the changelog lands with step 13).

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
cirrocast Beijing --offline             # cached answer only, never the network
cirrocast Beijing --refresh             # ignore the cache and replace it
cirrocast completion bash > ~/.local/share/bash-completion/completions/cirrocast
cirrocast man > cirrocast.1
```

A query resolves the location first (the same four forms as `location search`, with the winning place
echoed on stderr when the name was ambiguous), then walks the provider chain — `--provider` takes an
ordered list, and `auto` expands to the implemented keyless backends (plus `metar` when `--station`
is given) — and renders the first report that comes back. A chain entry that fails at the transport
or upstream level falls through to the next one with a `warning:` line; a usage, key or location
error stops the walk. `--days` is clamped to the primary provider's horizon with one `warning:` line.
Every forecast is cached for `cache.weather_ttl_secs` under
`$XDG_CACHE_HOME/cirrocast/weather/<provider>-<lat>-<lon>-<days>-<local-date>.json`, keyed by the
location's own calendar date. Providers are also requested in metric, and the renderer converts into
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
date formats, the measurement labels, the UV bands, the sixteen compass directions and the
`one-line` vocabulary (`%C`, `%w`, `%U`, `%D`, `%m`). What is not: `--help` and the other clap
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
| `Beijing` | fuzzy search through the keyless Open-Meteo geocoding API |
| `:Beijing` | only a candidate whose name matches exactly (case-insensitively) |
| `~Tsinghua` | OpenStreetMap/Nominatim, cached for 30 days and throttled to one request per second |
| `@39.9042,116.4074` | coordinates; no geocoding request at all |
| *(empty)* | `location.default`, else the public-IP lookup |

Multiple fuzzy candidates are ranked by exact name, then population, then upstream order, and the
ambiguity is reported once on stderr (suppressed by `-q`) with the winning place; add `:` to demand
an exact name. Coordinates and `~` results carry a provisional time zone until the forecast response
supplies the location's real one, and `~` output prints `Location data © OpenStreetMap contributors`
(ODbL).

Non-Latin names are searched in their own script — `新乡`, `Москва`, `Αθήνα`, `القاهرة`, `תל אביב`,
`กรุงเทพ` — because the geocoding service indexes place names per language and an English request
cannot match them. Coverage still differs per source: for some Chinese cities the OpenStreetMap route
is the reliable one (`~新乡市` resolves the city, a bare `新乡` only finds the villages GeoNames
indexes under that name), so both routes are worth trying when a name comes back wrong.

```bash
cirrocast location search Beijing          # Beijing, Beijing Municipality, China (39.91, 116.40) Asia/Shanghai
cirrocast location search :Beijing         # same line, no ambiguity note
cirrocast location search '~Tsinghua University' --limit 5
cirrocast location search @39.9042,116.4074
```

**Privacy:** the public-IP lookup is the only request that reveals anything about *you* rather than
about a place you asked for, and it is never implicit — it runs only with `--ip` or when no location
is configured anywhere (`location.default` empty and no positional argument). It sends the public IP
to `ipwho.is`, falling back to `ipapi.co` (`CIRROCAST_IP_SERVICE=auto|ipwhois|ipapi`), caches the
answer for 24 hours and names the service it used on stderr. `--offline` serves the cached answer and
touches no network.

## Data sources, limits and licences

`cirrocast` ships no data of its own — every place, address and forecast comes from a donated or open
service, each with its own limits and licence. The per-provider record (endpoints, request parameters,
response fields consumed, quotas with their exact wording, caching ceilings and traps) lives in
[`docs/providers.md`](docs/providers.md); what the tool does to stay inside those limits:

| Source | Used for | Limits the service sets | Licence / attribution |
|---|---|---|---|
| [Open-Meteo](https://open-meteo.com/) geocoding | `Beijing`, `:Beijing` | free tier is **non-commercial**, < 10 000 calls/day, 5 000/hour, 600/minute; `name` needs ≥ 2 characters | data CC-BY-4.0; the CLI prints `Location data based on GeoNames (CC-BY-4.0) via Open-Meteo` with the service link |
| [Open-Meteo](https://open-meteo.com/) forecast | every weather query | free tier is **non-commercial**, < 10 000 calls/day; `forecast_days` ≤ 16 | data CC-BY-4.0; the rendered report ends with `Data: Open-Meteo.com (CC BY 4.0)` |
| [SMHI](https://opendata.smhi.se/metfcst/snow1gv1) open data | `-p smhi` (Nordics) | no published quota; SMHI's fair-use rules forbid mass downloads and re-fetching the same data | data CC BY 4.0 SE; the rendered report ends with `Data: SMHI (CC BY 4.0 SE)` |
| [OpenWeatherMap](https://openweathermap.org/) | `-p openweathermap` | free tier: 60 calls/minute, 1 000 000 calls/month; two calls per fetch (current + 5-day/3-hourly forecast); a fresh key needs up to 2 hours to activate | data ODbL 1.0; visible attribution required — the rendered report ends with `Data: OpenWeather (ODbL 1.0) — https://openweathermap.org/` |
| [GeoNames](https://www.geonames.org/) | the data behind Open-Meteo's geocoding | — | CC-BY-4.0 |
| [Nominatim](https://nominatim.openstreetmap.org/) / OpenStreetMap | `~Tsinghua` | ≤ 1 request/second, an identifying `User-Agent`, results must be cached, no autocomplete and no bulk geocoding | data ODbL; `Location data © OpenStreetMap contributors (ODbL)` is printed; the service is switchable through `network.nominatim_url` without a code change, which the policy requires |
| [ipwho.is](https://ipwho.is/) | `--ip` (primary) | free endpoint: 1 000 requests/day per client IP, then `429` + `Retry-After` | personal or internal use, no redistribution |
| [ipapi.co](https://ipapi.co/) | `--ip` (fallback) | free tier: up to 1 000 requests/day | internal use, no resale; its terms allow keeping an answer for **at most 24 hours**, which is why `cache.ip_ttl_secs` is capped there |

Weather output carries the credit the data licence asks for — `Data: Open-Meteo.com (CC BY 4.0)` or
`Data: SMHI (CC BY 4.0 SE)` — plus a provenance line (`attribution: open-meteo
https://api.open-meteo.com/v1/forecast`), both taken from the provider registry rather than
hard-coded, and a geocoded place adds the GeoNames line next to them. Where the credit travels
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
| `smhi` | none | Nordics and adjacent seas | yes | yes | 10 |
| `metar` | none | stations | yes | no | — |
| `openweathermap` | `CIRROCAST_OPENWEATHERMAP_KEY` | global | yes | yes | 5 |
| `weatherapi` | `CIRROCAST_WEATHERAPI_KEY` | global | yes | yes | 3 |
| `worldweatheronline` | `CIRROCAST_WORLDWEATHERONLINE_KEY` | global | yes | yes | 5 |
| `pirateweather` | `CIRROCAST_PIRATEWEATHER_KEY` | global | yes | yes | 7 |
| `qweather` | `CIRROCAST_QWEATHER_KEY` | global | yes | yes | 7 |

Declared capabilities only, each row carrying the date it was last checked against the provider's live
documentation (`provider info <ID>` prints it). The full record — endpoints, request parameters,
response fields consumed, free-tier quotas with their exact wording, licence duties, caching ceilings
and the traps — lives in [`docs/providers.md`](docs/providers.md); `cirrocast provider list` /
`provider info <ID>` print the machine-readable subset from the binary itself. The 2026-09-30
re-verification corrected this table (SMHI's endpoint and horizon, WWO's 5-day free horizon,
QWeather's global coverage) and is recorded in that file's log.

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
| `defaults.format` | `art-table` | `art-table`, `one-line`, `plain`, `json`, `dumb` |
| `defaults.units` | `metric` | `metric`, `us`, `uk` |
| `defaults.days` | `3` | `0..=14`, clamped per provider |
| `defaults.language` | `auto` | `auto` or a BCP-47 tag such as `zh-CN` |
| `location.default` | empty | `Beijing`, `:Beijing`, `@39.9,116.4`, `~Tsinghua` |
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
| `providers.metar.station` | empty | ICAO identifier, e.g. `ZBAA` |
| `providers.qweather.host` | empty | your QWeather API host |

The `[units]` overrides are per quantity and optional: an absent (or empty) key follows
`defaults.units`, so switching that one value to `us` moves every quantity that was not pinned.

Precedence, highest first: **command line flag → `CIRROCAST_*` environment variable → `config.toml`
→ built-in default**. The variables are `CIRROCAST_PROVIDER`, `CIRROCAST_FORMAT`,
`CIRROCAST_UNITS`, `CIRROCAST_DAYS`, `CIRROCAST_LANG`, `CIRROCAST_LOCATION`,
`CIRROCAST_TIMEOUT`, `CIRROCAST_NOMINATIM_URL` and `CIRROCAST_IP_SERVICE`; API keys use their own
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
e.g. `CIRROCAST_OPENWEATHERMAP_KEY`) → `keys.toml` in the configuration directory → the OS keyring
(feature-gated, later release).

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
