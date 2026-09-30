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
cirrocast — Beijing, China (39.9042, 116.4074)
  ...wttr.in-style art-table output lands at step 07 of docs/plans...
```

```
$ cirrocast Beijing --format plain
Beijing, Beijing Municipality, China (39.91, 116.40) Asia/Shanghai
Now: 31°C (feels 36°C), Partly cloudy, wind 12 km/h SE, humidity 66%, pressure 1004 hPa, visibility 10 km, 0.0 mm
2026-07-15  min 25°C  max 35°C  sunrise 04:58  sunset 19:42
  Morning  29°C  Clear sky     precip 0.0 mm (0%)   wind 2.5 km/h N
  Noon     35°C  Clear sky     precip 0.0 mm (0%)   wind 4.7 km/h SW
  Evening  30°C  Overcast      precip 0.0 mm (0%)   wind 13 km/h SW
  Night    26°C  Overcast      precip 0.0 mm (0%)   wind 6.0 km/h SW
Data: Open-Meteo.com (CC BY 4.0)
```

The `plain` shape above is the recorded-response run from the test suite
(`tests/fixtures/open_meteo/forecast_beijing_2026-07-15.json`), so the numbers are the weather of that
day, not of today.

## Status

Steps 01–06 of 24 in [`docs/plans/`](docs/plans/README.md) are in place: the CLI skeleton, XDG path
resolution, the provider registry, the typed configuration with its `config`/`key` subcommands, the
canonical model, location resolution (`cirrocast location search`), the shared HTTP/cache layer and
the first end-to-end forecast (`cirrocast Beijing --format plain`, Open-Meteo through the provider
chain). The wttr.in-style `art-table` renderer is step 07, and the rest of the flag matrix
(`--units`, `--lang`, `--color`, `--width`, `--lat/--lon`, `--station`, the non-plain formats,
`provider list|info`, `completion`, `man`) arrives in step 08 — until then `--format` accepts `plain`
only and those flags are rejected by the command line. See the plan index for live per-step progress.

## Install

```bash
cargo install --path .        # from a checkout
# AUR packages and prebuilt release archives: planned, step 13 (docs/plans/13-packaging-and-release.md)
```

## Usage

```
cirrocast [OPTIONS] [LOCATION]

  -p, --provider <ID[,ID...]>   open-meteo | openweathermap | weatherapi | worldweatheronline
                                | pirateweather | qweather | smhi | metar | auto
  -f, --format <NAME>           art-table | one-line | plain | json | dumb
  -d, --days <N>                0..=14, clamped per provider
  -u, --units <metric|us|uk>
      --lang <TAG>              BCP-47, or "auto"
      --lat <DEG> --lon <DEG>   explicit coordinates
      --ip                      locate from the public IP
      --station <ICAO>          METAR station
      --no-cache | --refresh | --offline
      --color <auto|always|never>   --width <COLS>   --timeout <SECS>
  -q, --quiet    -v, --verbose

cirrocast config   <path|init|show|get|set|edit|validate>
cirrocast key      <set|rm|list>
cirrocast provider <list|info>
cirrocast cache    <stat|clean>
cirrocast location <search>
cirrocast completion <shell>   cirrocast man
```

Location syntax: `Beijing` (fuzzy), `:Beijing` (exact name), `~Tsinghua` (OpenStreetMap),
`@39.9,116.4` (coordinates), empty (config default, else public IP).

### Weather

```bash
cirrocast Beijing                       # the configured default format (art-table arrives in step 07)
cirrocast Beijing --format plain        # box-free lines, for pipes and logs
cirrocast @39.9042,116.4074 -f plain    # coordinates: no geocoding request at all
cirrocast Beijing -f plain --days 0     # current conditions only
cirrocast Beijing -f plain --offline    # cached answer only, never the network
cirrocast Beijing -f plain --refresh    # ignore the cache and replace it
cirrocast -p open-meteo -f plain Beijing
```

A query resolves the location first (the same four forms as `location search`, with the winning place
echoed on stderr when the name was ambiguous), then walks the provider chain — `--provider` takes an
ordered list, and `auto` expands to the implemented keyless backends — and renders the first report
that comes back. A chain entry that fails at the transport or upstream level falls through to the
next one with a `warning:` line; a usage, key or location error stops the walk. Every forecast is
cached for `cache.weather_ttl_secs` under
`$XDG_CACHE_HOME/cirrocast/weather/<provider>-<lat>-<lon>-<days>-<local-date>.json`, keyed by the
location's own calendar date. Providers are also requested in metric, and the renderer converts into
the display units, so a cache entry is unit-independent.

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
service, each with its own limits and licence. What the tool does to stay inside them:

| Source | Used for | Limits the service sets | Licence / attribution |
|---|---|---|---|
| [Open-Meteo](https://open-meteo.com/) geocoding | `Beijing`, `:Beijing` | free tier is **non-commercial**, < 10 000 calls/day, 5 000/hour, 600/minute; `name` needs ≥ 2 characters | data CC-BY-4.0; the CLI prints `Location data based on GeoNames (CC-BY-4.0) via Open-Meteo` with the service link |
| [Open-Meteo](https://open-meteo.com/) forecast | every weather query | free tier is **non-commercial**, < 10 000 calls/day; `forecast_days` ≤ 16 | data CC-BY-4.0; the rendered report ends with `Data: Open-Meteo.com (CC BY 4.0)` |
| [GeoNames](https://www.geonames.org/) | the data behind Open-Meteo's geocoding | — | CC-BY-4.0 |
| [Nominatim](https://nominatim.openstreetmap.org/) / OpenStreetMap | `~Tsinghua` | ≤ 1 request/second, an identifying `User-Agent`, results must be cached, no autocomplete and no bulk geocoding | data ODbL; `Location data © OpenStreetMap contributors (ODbL)` is printed; the service is switchable through `network.nominatim_url` without a code change, which the policy requires |
| [ipwho.is](https://ipwho.is/) | `--ip` (primary) | free endpoint: 1 000 requests/day per client IP, then `429` + `Retry-After` | personal or internal use, no redistribution |
| [ipapi.co](https://ipapi.co/) | `--ip` (fallback) | free tier: up to 1 000 requests/day | internal use, no resale; its terms allow keeping an answer for **at most 24 hours**, which is why `cache.ip_ttl_secs` is capped there |

Weather output carries the credit Open-Meteo's licence asks for: a rendered report ends with
`Data: Open-Meteo.com (CC BY 4.0)`, taken from the provider registry rather than hard-coded, and a
geocoded place adds the GeoNames line below it.

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
| `smhi` | none | Nordics | yes | yes | 10 ⚠ |
| `metar` | none | stations | yes | no | — |
| `openweathermap` | `CIRROCAST_OPENWEATHERMAP_KEY` | global | yes | yes | 5 |
| `weatherapi` | `CIRROCAST_WEATHERAPI_KEY` | global | yes | yes | 3 |
| `worldweatheronline` | `CIRROCAST_WORLDWEATHERONLINE_KEY` | global | yes | yes | 3 |
| `pirateweather` | `CIRROCAST_PIRATEWEATHER_KEY` | global | yes | yes | 7 |
| `qweather` | `CIRROCAST_QWEATHER_KEY` | China-first | yes | yes | 7 |

⚠ `smhi`'s legacy host (`opendata-download-metfcst.smhi.se`) answered `404` on 2026-09-30, so the
endpoint must be re-derived before that backend can ship; the correction is tracked in
`docs/plans/19-more-providers.md`.

Declared capabilities only — each row is re-verified against the provider's live documentation in
step 10 (`docs/plans/10-additional-providers.md`) and corrected there if wrong. `cirrocast provider
list` / `provider info <ID>` prints the same data from the binary itself.

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
