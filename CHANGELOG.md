<!--
SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Changelog

All notable, user-visible changes to `cirrocast` are documented here. The format follows
[Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/), and the version numbers follow
[Semantic Versioning 2.0.0](https://semver.org/spec/v2.0.0.html). The crate version, the JSON
output schema version and the config schema version move independently; [`docs/schema.md`](docs/schema.md)
records how each of them changes and which changes are breaking.

## [Unreleased]

### Added

* **Offline city database (step 18).** A `GeoNames` `cities15000` snapshot (CC BY 4.0, dump date and
  input checksum in `src/geo/data/SNAPSHOT`) is embedded in the binary — about 3.2 MiB compressed,
  decoded lazily on the first name lookup and never written — so a plain city name resolves to
  coordinates, a time zone and a country code with no network at all. Folding is NFKD-based, so
  `São Paulo`/`Sao Paulo`, `MÜNCHEN`/`munchen`, `北京`/`Beijing`/`Peking` and `Wien`/`Vienna` all
  reach their city, and the offline and network paths rank through one shared function. New:
  `--offline[=<weather|geo|all>]` (`weather` = cache-only forecast with live geocoding, `geo` =
  bundled names with live weather, bare/`all` = no socket at all) and its default
  `[network] offline`; `[geo] strategy` = `auto` (bundled table first, network on a miss — the new
  default), `bundled` or `network`; `location search --all` prints the ranked candidate table and
  `--exact` is the flag spelling of `:query`. Offline-resolved places print
  `Location data by GeoNames (CC BY 4.0) — https://www.geonames.org/`, and a build with
  `--no-default-features` drops the table (`offline-geo` is a default feature) and falls back to
  the network geocoder.
* `cirrocast`'s workspace gained the dev-only `build/geo-table` builder that produces
  `src/geo/data/*.bin.gz` from a `cities15000.txt` (`cargo run -p geo-table -- <file> src/geo/data`);
  its output is byte-for-byte deterministic. `scripts/refresh-city-data.sh` wraps the refresh —
  download, extract, build, then run the canary tests that pin rows of the committed snapshot — and
  refuses to hide a data change behind a passing build.

### Changed

* A plain location name (`Beijing`, `:Beijing`) is now resolved by the bundled city table first and
  only falls back to the Open-Meteo geocoding API when the table has no hit. The table carries the
  ISO country code rather than the country name the geocoder reports, and no admin-1 division, so a
  default run's location line reads `Beijing, CN (…)` instead of `Beijing, Beijing Municipality,
  China (…)`; `[geo] strategy = "network"` restores the previous path.
* `--offline` is no longer a boolean: bare `--offline` still means "no socket at all" (now spelled
  `--offline=all`), and the cache-only-weather behaviour of previous releases is `--offline=weather`.
  `--offline=geo` is new. The empty-cache message now reads `offline: no cached <provider> forecast
  for <place> at <key path>`, followed by the rerun hint.

## [1.1.0] - 2026-10-04

Phase D's first three steps — severe-weather alerts (15), air quality (16) and moon/astro (17) —
on top of the 2026-10-02 review fixes
([`docs/reviews/02-review-01-fixes-2026-10-02.md`](docs/reviews/02-review-01-fixes-2026-10-02.md)).
The v1 CLI contract is unchanged; the JSON document moves to `schema_version` 2 because the
`alerts` array arrived with the alerts work (see below), and the config document stays at
`schema_version` 1 — the `[alerts]` and `[air]` tables are additive.

### Added

* **Severe-weather alerts (step 15).** `cirrocast` now fetches official warnings by default —
  `--no-alerts` opts out, `--alerts` forces them, `--alerts-from nws,meteoalarm,…` names the sources —
  and renders them as a severity-coloured banner above `art-table`/`one-line`, as `alert:` records in
  `plain`, as the new `--format alerts` listing and as the `alerts` array in `json`. Sources are
  selected by coverage: NWS (US and territories), MeteoAlarm (EUMETNET members, optional
  `CIRROCAST_METEOALARM_KEY`), HKO (Hong Kong), QWeather (China, on its provider's chain) and the two
  global aggregators WMO SWIC and FPAS (self-hostable through `[alerts] fpas_url`). Warnings are
  normalised to CAP 1.2, expired ones are dropped (`ends`, else `expires`), duplicates across sources
  are collapsed, and strongest comes first. The threshold is `[alerts] severity_threshold`/`--severity`
  (default `minor`); the cache namespace `alerts/` has a 300 s TTL and `--offline` replays the last
  set. One-line's `%A` expands to the strongest alert's event (empty when none).
* JSON output: `schema_version` is now **2**, adding the `alerts` array and `alert_credits`
  (`docs/schema.md`); version 1 documents still parse. `provider info` gained the alert row
  (`provider info qweather` → `alerts: qweather`, `provider info smhi` → `alerts: none`).
* `[alerts]` configuration table (`enabled`, `severity_threshold`, `sources`, `fpas_url`,
  `cache_ttl_secs`) with `config get`/`set` support.
* **Air quality (step 16).** `--aqi` appends an air-quality panel to `art-table` and `plain` — the
  US and European AQI, the six regulated pollutants in μg/m³ and, inside the CAMS European domain,
  the six pollen species in grains/m³ — `--format aqi` prints it standalone, `one-line` gains the
  `%q` token, and `json` carries an `air` object (additive within `schema_version` 2). The reading
  is one keyless request to Open-Meteo's Air Quality API for the location the run already resolved,
  cached under `weather/open-meteo-air-<lat>-<lon>-<local-date>.json` with `cache.weather_ttl_secs`
  and the usual `--no-cache`/`--refresh`/`--offline` semantics. Categories are computed locally from
  the published breakpoints; `--aqi-index` (`[air] index`, default `us`) picks the scale that drives
  the category colour and `%q`; `--units` leaves the pollutant values alone by design. The fetch is
  best-effort: a failure prints `warning: air quality unavailable: …` on stderr and never changes
  the run's exit code, and a location outside the pollen domain says `not covered at this location`
  instead of inventing a zero.
* **Moon phase and astronomy (step 17).** `--moon` appends a locally computed moon/sun block to
  `art-table` and `plain`, `--format moon` prints the standalone view, `one-line` gains `%m` (the
  phase's art glyph) and `%M` (the phase's name), and `json` carries an `astro` object (additive
  within `schema_version` 2). Nothing is fetched: the phase, the geocentric illuminated fraction,
  the age, moonrise/moonset, the next four phase instants and — when the backend sends no sun
  times — sunrise/sunset/daylight are computed from the truncated Meeus series (ELP-2000/82 and
  solar, ΔT from the Espenak–Meeus fits). The sun block prefers the provider's own times and records
  where they came from in `astro.sun.source`; inside the polar circles the state is named
  (`polar day`/`polar night`) instead of clamping to `00:00`, and an event a day does not have
  prints `—`. `%m`/`%M` are always available; `--moon` is a usage error for the formats that have
  no astro surface (`one-line`, `alerts`, `aqi`).

### Changed

* `network.proxy` accepts only `http://` and `https://` URLs. A SOCKS URL is refused by the config
  validator, naming the key, instead of reaching `ureq` — which is built without a SOCKS connector and
  panicked on a hand-written setting.
* `providers.qweather.host` must be the account's HTTPS host, `https://<account-id>.re.qweatherapi.com`.
  A legacy shared host or a plain-`http://` value now fails validation on every run: the legacy hosts
  answer `403 Invalid Host`, and cleartext would leak the key.
* `config show` prints the values `config get` reports, `CIRROCAST_*` overrides included, and exits 4
  when an override is invalid; it previously ignored the environment.
* `config edit` rejects unknown keys like `config validate`; `config set` validates only the key it
  writes, so an unrelated invalid value no longer blocks it.
* `config init`, and `config edit` on a missing file, seed the new file with the *effective*
  configuration when a system document is shadowed (that one case writes canonical TOML without the
  commented template; with no other source the template is unchanged).
* A whitespace-only `CIRROCAST_*` override counts as unset, like a whitespace-only config value.
* `--lang` accepts POSIX spellings (`zh_CN.UTF-8` → `zh-CN`), and `en-*`/`zh-*` tags resolve through
  their family chain (`en-GB → en-US`) without the fallback warning.
* `art-table`: the `dumb`/ASCII arrows are `,` (SW) and `` ` `` (NW) — the previous keypad digits read
  as part of the speed — and the arrow sector follows the 16-point compass, so arrow and label always
  turn together.
* `art-table` at 20–36 columns: the stacked ladder's rungs keep the precipitation and wind fields
  instead of silently dropping them (at ≥37 columns the output is unchanged).
* `-f dumb` is always plain, escapes included: `--color always` no longer paints the ASCII table.
* `-f one-line`: `%w` prints the speed alone when the direction is absent, instead of `n/a`.
* `-f json`: `-0.0` is written as `0.0`, like every other display path.
* `-v`: the missing-key dump runs after the forecast and reports each missing key once per run; `-vv`
  request logs redact secrets in their percent-encoded spelling too.
* Messages: an unknown location no longer promises a candidate list it does not print, an unknown
  station points at `@lat,lon`, `--lat/--lon` report the command-line source, and the help epilogue
  spells the precedence as `LOCATION CIRROCAST_LOCATION`.

### Fixed

* A reading that is `NaN` or `inf` is refused with `Error::Upstream` in `Provider::fetch` — the one
  path every backend's answer takes — before it can be cached or rendered.
* QWeather: precipitation probability is read as the percent upstream sends (a `40` came out as `100%`
  through the fraction helper), and code 515 maps to WMO 56 (freezing drizzle), not a fog variant.
* Open-Meteo: an absent or truncated `precipitation_probability` array means "no probability", not an
  upstream error.
* Pirate Weather: `-999` sentinels in humidity, cloud cover and probability are missing values, not
  readings.
* WorldWeatherOnline: the `{"data":{"error":[…]}}` envelope is an upstream error carrying the message,
  not a decode failure.
* METAR: an unmapped obscuration falls back to the sky condition, `IC` maps to WMO 79 (ice pellets,
  the nearest described family) instead of failing, and conversions keep the exact value rather than a
  pre-rounded one.
* A value just below a `.5` tie no longer rounds to the wrong side (`fmt_int` on large readings,
  `fmt_small` on a negative reading).
* An extreme timestamp in an upstream payload produces a typed error instead of overflowing.
* A failed cache write logs at `-vv` and still serves the fetched answer.
* Nominatim requests are sent once, without retry, so its 1 req/s policy is never breached by backoff.
* A non-absolute `XDG_CONFIG_DIRS` entry is ignored instead of read.
* A `days` array whose parts are not exactly `[Morning, Noon, Evening, Night]` is rejected at
  deserialisation, so no renderer can show a part under another part's label.
* `defaults.format` accepts `moon`: the validator's list stopped at `aqi` and the template comment
  at `dumb`, so `config set defaults.format moon` failed and a hand-written `format = "moon"` was
  refused on every run although `-f moon` and `CIRROCAST_FORMAT=moon` worked. The three lists are
  now pinned to each other by a test.

## [1.0.0] - 2026-10-02

The v1 acceptance release: the whole A–C surface proved by execution in step 14 — the eight
backends against all five formats, the side-by-side against `wttr.in`, the four failure exit codes,
the packaged install — and `1.0.0` freezes the CLI, the JSON schema v1 and the config schema v1 for
the v1 scope. No new surface; the acceptance run produced three fixes:

### Changed

* `art-table` and its `dumb` twin: the day-cell tail pairs the precipitation amount with the
  precipitation *probability* (`0.0mm 20%`), matching the `plain` document's `0.0mm (0%)` and
  wttr.in's `0.0 mm | 0%`; humidity stays in the current-conditions block above the table (step 07).
* `--help` and the man page describe `-v`/`-vv` as they really behave: `-v` prints the upstream
  request behind the answer, `-vv` adds every HTTP attempt and the cache decisions (step 12).

### Fixed

* `--ip` with both location services failing names every attempt
  (`all IP location services failed: ipwho.is (…); ipapi.co (…)`) instead of only the last service
  (step 05).

## [0.1.0] - 2026-10-01

The first release: the whole v1 surface, shipped as `0.x` while phases A–C of `docs/plans/`
landed. `1.0.0` freezes it.

### Added

* Weather backends behind `--provider` / `CIRROCAST_PROVIDER` / `[defaults].provider`: the keyless
  `open-meteo` (default), `smhi` (Nordics and adjacent seas) and `metar` (station observations,
  selected by `--station` or `@lat,lon`); the BYOK backends `openweathermap`, `weatherapi`,
  `worldweatheronline`, `pirateweather` and `qweather`; `auto` expands to the keyless chain, and a
  chain falls through to the next entry only on a transport or upstream failure. `provider list`
  and `provider info <ID>` print the registry rows.
* Location resolution in four spellings, all documented with the winning place echoed on stderr:
  fuzzy geocoding (`Beijing`), exact-name (`:Beijing`), OpenStreetMap (`~Tsinghua`, one request per
  second, cached 30 days, base URL swappable through `network.nominatim_url`), coordinates
  (`@39.9,116.4`, `--lat`/`--lon`) and a METAR station (`--station`); the public-IP lookup
  (`--ip`, or implicit when nothing is configured) uses ipwho.is with an ipapi.co fallback and
  caches its answer for at most 24 hours. `location search` resolves a name without printing
  weather.
* Output formats behind `-f art-table|one-line|plain|json|dumb` (`art-table` is the default; `dumb`
  is the ASCII art table and engages automatically for `TERM=dumb` or a non-UTF-8 locale);
  `--template` takes a `%`-token string or a preset (`@default`, `@short`, `@full`, `@uv`,
  `@sun`); `plain` and `json` never truncate a record; `-f json` emits the stable document
  documented in [`docs/schema.md`](docs/schema.md) with `schema_version: 1`.
* Unit systems `-u metric|us|uk` with per-quantity overrides under `[units]`; every value is stored
  and cached canonically metric, so switching units never refetches.
* Output languages: `--lang`, `CIRROCAST_LANG` and `defaults.language` with the embedded `en-US`
  and `zh-CN` catalogs, `auto` following the ambient locale, and a documented fallback chain that
  warns instead of failing.
* Configuration and state under the XDG directories: `config init|show|get|set|edit|validate`,
  `cache stat|clean`, and BYOK keys in `keys.toml` (`key set|rm|list`, mode 0600, environment
  variable first) so secrets never enter `config.toml`, argv or any log.
* Cache control with `--no-cache`, `--refresh` and `--offline` (cache-only, never touches the
  network) over the weather, geocoding, IP and station namespaces.
* Terminal integration: `completion bash|zsh|fish|elvish|powershell` and `man` generate the shell
  completion and the manual page from the binary; colour honours `NO_COLOR`, `CLICOLOR_FORCE` and
  `--color auto|always|never` with a 16-colour fold when the terminal advertises no more, and
  `--width`/`COLUMNS` drive a stacked layout below 60 columns.
* Attribution is part of the output: every format carries the credits its data licences require
  (`plain` and `json` in the document, `art-table` in the footer, `one-line` on stderr).
* Exit codes `0`–`6` documented in `--help` and the README, with `error: …` on stderr and the
  cause chain under `-v`.

[Unreleased]: https://github.com/YangtseSu/cirrocast/compare/v1.1.0...HEAD
[1.1.0]: https://github.com/YangtseSu/cirrocast/releases/tag/v1.1.0
[1.0.0]: https://github.com/YangtseSu/cirrocast/releases/tag/v1.0.0
[0.1.0]: https://github.com/YangtseSu/cirrocast/releases/tag/v0.1.0
