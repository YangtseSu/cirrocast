<!--
SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Configuration

`cirrocast` reads one TOML document, `config.toml`. This file is the authoritative reference for
it: every key, its type, its built-in default and what changing it does. The versioned schema and
the annotated document `config init` writes live in [schema.md](schema.md); the keys in the two
tables that name output formats and alert sources are owned by [formats.md](formats.md) and
[providers.md](providers.md) respectively.

## Where the file lives

Everything is under the XDG directories, in a single `cirrocast/` directory each:

| What | Path | Mode |
|---|---|---|
| configuration | `$XDG_CONFIG_HOME/cirrocast/config.toml` (`~/.config/cirrocast/config.toml`) | `0644` |
| API keys | `$XDG_CONFIG_HOME/cirrocast/keys.toml` | `0600` |
| cache | `$XDG_CACHE_HOME/cirrocast/` (`~/.cache/cirrocast/`) | — |
| data | `$XDG_DATA_HOME/cirrocast/` (`~/.local/share/cirrocast/`) | — |

`cirrocast config path` prints the user file — the one `config init`/`set`/`edit` write and the
first one read — without creating anything:

```console
$ cirrocast config path
/home/yangtse/.config/cirrocast/config.toml
```

### Search order

The first configuration file that exists is the one that is read. The candidate list is built the
same way the XDG base directory spec prescribes:

1. `$XDG_CONFIG_HOME/cirrocast/config.toml` (the user file `config path` prints);
2. one `cirrocast/config.toml` under **each** entry of `$XDG_CONFIG_DIRS`, colon-separated, in
   order — default `/etc/xdg` when the variable is unset or empty.

A path in `XDG_CONFIG_DIRS` must be absolute; a relative entry is ignored, so `XDG_CONFIG_DIRS=.`
can never make the process read a `./cirrocast/config.toml` from the working directory. Duplicate
candidates are dropped. Because the user entry comes first, a user file shadows every system file;
a system file is read only when no user file exists. Parsing, a missing `schema_version` and
migration all behave identically whichever file wins. `config validate` prints the file it actually
read (`ok: <path>`), and a fresh install with no file at all yields the built-in defaults.

Writes always target the user path: `config set` loads the effective file, applies one change and
saves the whole document to `$XDG_CONFIG_HOME/cirrocast/config.toml`, never to a system file.

## Precedence

Highest first:

| Tier | What | Notes |
|---|---|---|
| 1 | command-line flag | e.g. `--days 5` |
| 2 | `CIRROCAST_*` environment variable | e.g. `CIRROCAST_DAYS=5` |
| 3 | user `config.toml` | the first file found above |
| 4 | system `config.toml` | an `XDG_CONFIG_DIRS` entry |
| 5 | built-in default | the `Default` column of every table below |

The configuration file is consulted only when neither the flag nor the variable is set, so an
environment value is never overridden by `config.toml`. `cirrocast config get <KEY>` applies the
same override, and `cirrocast config show` renders the *effective* configuration — the file with
every `CIRROCAST_*` override applied — so the two never disagree about a key.

| Flag | Environment variable | Config key it overrides |
|---|---|---|
| `--provider` | `CIRROCAST_PROVIDER` | `defaults.provider` |
| `--format` | `CIRROCAST_FORMAT` | `defaults.format` |
| `--units` | `CIRROCAST_UNITS` | `defaults.units` |
| `--days` | `CIRROCAST_DAYS` | `defaults.days` |
| `--lang` | `CIRROCAST_LANG` | `defaults.language` |
| `--tz` | `CIRROCAST_TZ` | `location.tz` |
| `--timeout` | `CIRROCAST_TIMEOUT` | `network.timeout_secs` |
| `LOCATION` (positional) | `CIRROCAST_LOCATION` | `location.default` |

The `--help` epilogue additionally lists variables that have no flag. Each still overrides exactly
one config key, except the last two, which have no config key:

| Environment variable only | Overrides |
|---|---|
| `CIRROCAST_LOCATION_PICK` | `location.pick` |
| `CIRROCAST_NOMINATIM_URL` | `network.nominatim_url` |
| `CIRROCAST_GEO_SEARCH` | `geo.search` |
| `CIRROCAST_GEO_REVERSE` | `geo.reverse` |
| `CIRROCAST_NORMALS_PERIOD` | `normals.period` |
| `CIRROCAST_NORMALS_MAX_DISTANCE_KM` | `normals.max_distance_km` |
| `CIRROCAST_IP_SERVICE` | **no config key** — selects the public-IP location service chain: `auto` (default), `ipwhois`, `ipapi`, `ipsb` |
| `CIRROCAST_GEONAMES_USER` | **no config key** — the GeoNames account *name*, resolved through the key store as a named credential |

`CIRROCAST_LOCATION` names **one** location — the environment tier supplies a single spec, so a
comma inside the value is part of that spec (a coordinate pair, or a `City, Region` name), never a
separator. A multi-location run lists its locations as separate arguments on the command line.

An empty environment variable counts as unset.

### Credentials are resolved separately

API keys are **not** part of this schema: `config.toml` is world readable, meant to be pasted into
bug reports and dumped by `config show`. Provider keys are BYOK and resolved by the key store,
first hit wins: `CIRROCAST_<PROVIDER>_KEY` (the provider id upper-cased, `-` → `_`) → `keys.toml`
under `[keys]`. There is no third tier, and `keys.*` is not a configuration namespace —
`config set keys.openweathermap …` fails as an unknown key. The key handling rules (the `0600`
refusal, `key set` reading stdin only, `key list` masking, the QWeather JWT quartet, the GeoNames
named credential) are in [getting-started.md](getting-started.md) and the README's *API keys*
section.

## Key reference

`schema_version` is the document's one top-level scalar; every other key belongs to a table.

| Key | Type | Default | Example | Effect |
|---|---|---|---|---|
| `schema_version` | integer | `2` | `2` | Schema version of the document. Do not edit; see [Schema version](#schema-version-and-migration). |

### `[defaults]`

| Key | Type | Default | Example | Effect |
|---|---|---|---|---|
| `defaults.provider` | string | `"open-meteo"` | `"open-meteo,met-no"` | Backend id, a comma-separated fallback chain, or `auto` (the keyless chain). See [providers.md](providers.md). |
| `defaults.format` | string | `"art-table"` | `"one-line"` | Output format, a one-line preset (`full`, `minimal`, `short`, `default`, `uv`, `sun`), or a `[templates]` key. See [formats.md](formats.md). |
| `defaults.units` | string | `"metric"` | `"us"` | Unit system: `metric`, `us` or `uk`. |
| `defaults.days` | integer | `3` | `7` | Forecast days, `0..=14`; each provider clamps to its own maximum. |
| `defaults.language` | string | `"auto"` | `"en-US"` | `auto` or a BCP-47 tag such as `zh-CN`. See [i18n.md](i18n.md). |
| `defaults.normals` | boolean | `false` | `true` | Fetch the climate-normals comparison on every run; `--normals` forces it for one run. |

### `[location]`

| Key | Type | Default | Example | Effect |
|---|---|---|---|---|
| `location.default` | string | `""` (empty) | `"Beijing"` | Location argument used when none is given; empty means ask for the IP location. Any argument form is allowed, including `@name` for an alias. See [location.md](location.md). |
| `location.pick` | string | `"auto"` | `"never"` | `auto` asks which candidate to use on a terminal when a name matches several places; `never` always takes the ranked winner. |
| `location.tz` | string | `""` (empty) | `"Asia/Shanghai"` | IANA zone the report's times are expressed in; empty means the zone the location resolves. Mostly for `@lat,lon`, whose zone the bundled tables may not know; `--tz` overrides it. See [location.md](location.md). |

### `[geo]`

| Key | Type | Default | Example | Effect |
|---|---|---|---|---|
| `geo.strategy` | string | `"auto"` | `"bundled"` | `auto` (bundled table first, network on a miss), `bundled` (table only) or `network` (geocoder only). |
| `geo.search` | string | `"auto"` | `"geonames"` | Name-search sources: `auto` (open-meteo, then geonames when an account name is set, then nominatim), or one of `open-meteo`, `geonames`, `nominatim`. |
| `geo.reverse` | string | `"auto"` | `"offline"` | Naming a coordinate: `auto` (bundled tables, then Nominatim), `offline` (no socket) or `off` (never). |
| `geo.data` | string | `"auto"` | `"user"` | Which city table answers: `auto` (the user table when present), `bundled` or `user`. |
| `geo.update` | string | `"off"` | `"check"` | `off`, or `check` for the once-a-day note when the table is older than `update_interval_days`; the tool never fetches by itself. |
| `geo.update_interval_days` | integer | `90` | `180` | The `check` note's threshold in days, `1..=3650`. |
| `geo.update_url` | string | `""` (empty) | `"https://download.geonames.org/export/dump/"` | Source for `cirrocast location update-data`; empty uses the official GeoNames dump. Must be empty or an `http(s)` URL. |

### `[units]`

Per-quantity overrides on top of `defaults.units`. An absent (or empty) key follows the unit
system; the accepted spellings are the exact lowercase names below, verified against
`src/model/units.rs`.

| Key | Type | Default | Example | Effect |
|---|---|---|---|---|
| `units.temp` | string | unset | `"f"` | Temperature: `c` or `f`. |
| `units.wind` | string | unset | `"knots"` | Wind speed: `kmh`, `mph`, `mps` or `knots`. |
| `units.pressure` | string | unset | `"inhg"` | Pressure: `hpa`, `inhg` or `mmhg`. |
| `units.distance` | string | unset | `"mi"` | Distance: `km` or `mi`. |
| `units.precip` | string | unset | `"in"` | Precipitation: `mm` or `in`. |

The per-quantity key wins over the system, and `config validate` says so on stderr:

```console
$ cirrocast config validate
note: units.temp = "f" overrides defaults.units = "metric" for temperature; the per-quantity key wins
ok: /tmp/tmp.waRpzhKOdZ/config/cirrocast/config.toml
```

### `[network]`

| Key | Type | Default | Example | Effect |
|---|---|---|---|---|
| `network.timeout_secs` | integer | `15` | `30` | Per-request timeout, `1..=300` seconds. |
| `network.retries` | integer | `3` | `5` | Retry attempts for retriable transport and upstream failures, `0..=10`. |
| `network.proxy` | string | `""` (empty) | `"http://127.0.0.1:8080"` | HTTP(S) proxy; empty connects directly. Accepts `http(s)://host[:port]` or `host:port`; a SOCKS scheme is rejected. |
| `network.nominatim_url` | string | `""` (empty) | `"https://nominatim.example"` | Nominatim base URL for `~name` searches and reverse naming; empty uses the public OpenStreetMap service. |
| `network.offline` | string | `"off"` | `"weather"` | Default offline policy: `off`, `weather` (cache-only weather), `geo` (bundled names, live weather) or `all` (never open a socket). `--offline` overrides it for one run. |

### `[cache]`

| Key | Type | Default | Example | Effect |
|---|---|---|---|---|
| `cache.enabled` | boolean | `true` | `false` | Whether the on-disk cache is used at all. |
| `cache.weather_ttl_secs` | integer | `600` | `1200` | Weather response TTL in seconds; must be `> 0`. |
| `cache.ip_ttl_secs` | integer | `86400` | `43200` | Public-IP lookup TTL in seconds; must be `> 0`, and larger values are capped at 24 h by the service's terms. |
| `cache.geocode_ttl_secs` | integer | `2592000` | `86400` | Geocoding result TTL in seconds; must be `> 0` (default 30 days). |

### `[render]`

| Key | Type | Default | Example | Effect |
|---|---|---|---|---|
| `render.color` | string | `"auto"` | `"never"` | Colour mode: `auto`, `always` or `never`. |
| `render.width` | integer | `0` | `100` | `0` detects the terminal width, otherwise `1..=500` columns. |

### `[alerts]`

| Key | Type | Default | Example | Effect |
|---|---|---|---|---|
| `alerts.enabled` | boolean | `true` | `false` | Fetch warnings automatically when a source covers the location. |
| `alerts.severity_threshold` | string | `"minor"` | `"severe"` | Lowest severity shown: `unknown`, `minor`, `moderate`, `severe` or `extreme`. |
| `alerts.sources` | array of strings | `["auto"]` | `["nws", "meteoalarm"]` | `["auto"]` (coverage-selected) or an explicit list of source ids: `nws`, `meteoalarm`, `qweather`, `hko`, `wmoswic`, `fpas`, `visualcrossing`. `auto` cannot be mixed with explicit ids. |
| `alerts.fpas_url` | string | `""` (empty) | `"https://alerts.example"` | FOSS Public Alert Server base URL; empty uses `https://alerts.kde.org`. |
| `alerts.cache_ttl_secs` | integer | `300` | `600` | Alert response TTL in seconds; must be `> 0`. |

### `[air]`

| Key | Type | Default | Example | Effect |
|---|---|---|---|---|
| `air.index` | string | `"us"` | `"european"` | AQI scale that drives the air panel's colour and the `%q` token: `us` or `european`. |

### `[normals]`

| Key | Type | Default | Example | Effect |
|---|---|---|---|---|
| `normals.period` | string | `"1991-2020"` | `"1981-2010"` | Reference window the normal is averaged over: two four-digit years, the earlier one first. |
| `normals.max_distance_km` | integer | `60` | `120` | Farthest NOAA NCEI station that still answers, `1..=500` km. |

### `[status]`

| Key | Type | Default | Example | Effect |
|---|---|---|---|---|
| `status.placeholder` | string | `"n/a"` | `"—"` | What `cirrocast status` prints when it has no reading to show. The probe's line and exit-code contract is in [ecosystem.md](ecosystem.md). |

### `[providers.*]`

Per-backend settings, one sub-table per backend that has one. Auth material is never here.

| Key | Type | Default | Example | Effect |
|---|---|---|---|---|
| `providers.metar.station` | string | `""` (empty) | `"ZBAA"` | Default ICAO station identifier for the `metar` backend. |
| `providers.qweather.host` | string | `""` (empty) | `"https://<account-id>.re.qweatherapi.com"` | Your QWeather API host, from <https://console.qweather.com/>. |

## `[locations]` — aliases

The `[locations]` table is free-form: its keys are your own `@name` aliases, and a value is **any**
location argument — a fuzzy name, `:exact`, `~osm`, `@lat,lon`, another alias as `@name`, or
anything else the positional argument accepts ([location.md](location.md)). The argument `@name` is
expanded through this table until a non-alias argument is left.

```toml
[locations]
home    = "@39.9042,116.4074"   # @home  -> a coordinate
work    = "@home"               # @work  -> @home -> the coordinate
office  = ":Shanghai"           # @office -> an exact-name query
```

Chains are expanded with cycle detection and an eight-hop cap; a cycle is a **configuration error**
(exit 4) named with its chain, and a name that does not exist is a usage error (exit 2) with the
closest configured names suggested. `config validate` expands every alias at load time, so a bad
chain never reaches a run:

```console
$ cirrocast config validate
error: config error: locations.a: location alias cycle: @a -> @b -> @a
```

A worked run, with `home` and `compact` from the sections here:

```console
$ cirrocast --lang en-US -f compact @home
Location data by GeoNames (CC BY 4.0) — https://www.geonames.org/
Data: Open-Meteo.com (CC BY 4.0) — https://open-meteo.com/
Beijing: *o* +20°C
```

## `[templates]` — named one-line templates

The `[templates]` table is free-form too: each key names a literal `%`-token template, usable as
`--format <NAME>`, as `--template @NAME`, or as `defaults.format`. The token vocabulary, width and
precision syntax, escapes and the unknown-token policy are owned by [formats.md](formats.md).

```toml
[templates]
compact = "%l: %c %t"   # cirrocast -f compact   /   --template @compact
chain   = "@compact"    # a body may itself be @other; resolved to a fixed point
```

An unknown `%X` in a template body is rejected when the template is used (exit 2, with its
position). `defaults.format` may name a `[templates]` key, and `config set defaults.format` checks
the same three namespaces as a run — a name in none of them is refused:

```console
$ cirrocast config set defaults.format compact
error: config error: defaults.format: `compact` is not a format (art-table, one-line, plain, json, dumb, alerts, aqi, moon, normals), a template preset (default, short, minimal, full, uv, sun) or a `[templates]` key
```

## Managing the file

```console
$ cirrocast config path                       # where the file lives (creates nothing)
$ cirrocast config init [--force]             # write the annotated default document
$ cirrocast config show                       # the effective configuration as TOML
$ cirrocast config get <KEY>                  # one effective value, env override included
$ cirrocast config set <KEY> <VALUE>          # change one value in the user file
$ cirrocast config edit                       # $VISUAL/$EDITOR, re-validated afterwards
$ cirrocast config validate [--offline]       # parse and check the file
```

* **`config get <KEY>`** takes a dotted key such as `defaults.days` and prints the effective value,
  applying the `CIRROCAST_*` override for keys that have one. Free-form keys (`locations.home`,
  `templates.compact`) are not addressable — `config.toml` is the only place to edit them. An
  unknown key is a usage error (exit 2) listing every known key.

* **`config set <KEY> <VALUE>`** parses and validates the one value, then rewrites the **whole**
  document in canonical form to the user file, so hand-written comments are lost; `config init
  --force` writes the fully annotated document back. It creates the user file when none exists, and
  when a system file supplied the values it saves the merged document to the user path. A rejected
  value is a configuration error (exit 4) naming the key and the fix.

* **`config validate`** first rejects the first key the schema does not define, then checks every
  value, then (with `--offline`) also rejects the combination `cache.enabled = false` plus a policy
  that needs the cache. It prints `ok: <path>` for the file it read, or the user path when no file
  exists, and any unit-override notes on stderr. It never touches the network. `config edit`
  validates the same way after the editor exits.

## Schema version and migration

`schema_version` (currently `2`) stamps the document. The compatibility rule and the migration
hooks are specified in [schema.md](schema.md); in summary:

* a document **without** `schema_version` is read as the current version (a hand-written file may
  leave it out);
* version `1` migrates to `2` by stamping the version — the tables the bump added (`[locations]`,
  `[templates]`) are optional, and absent means empty;
* version `0` is refused with the `cirrocast config init --force` hint; a version **above** the
  supported one is refused with both numbers;
* a key this build does not know is **ignored on load** (so a document from a newer release keeps
  working) but rejected by `config validate`, which exists to catch typos;
* adding a key with a built-in default does not bump the version; changing an existing key's
  meaning, type or validity does.

## Validation errors and exit codes

Configuration problems are the `Config` error class, which always exits **4** — a bad value, an
unknown key caught by `validate`, a parse error, an unsupported `schema_version`, an alias cycle,
an unreadable file, or `config init` refusing to overwrite an existing file. A **usage** problem
(exit **2**) is distinct: an unknown key passed to `config get`/`set`, or a typo the command line
itself rejects. The full ladder is in `cirrocast --help`; command failures are covered in
[ecosystem.md](ecosystem.md) and [troubleshooting.md](troubleshooting.md).

Messages carry the dotted key and either the offending value or the file position:

```console
$ cirrocast config validate
error: config error: defaults.days: 99 is out of range 0..=14
```

A syntax error adds the position:

```console
$ cirrocast config validate
error: config error: /tmp/tmp.waRpzhKOdZ/config/cirrocast/config.toml:2:10: unclosed table, expected `]`
```
