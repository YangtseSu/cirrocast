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

Nothing yet.

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

[Unreleased]: https://github.com/YangtseSu/cirrocast/compare/v1.0.0...HEAD
[1.0.0]: https://github.com/YangtseSu/cirrocast/releases/tag/v1.0.0
[0.1.0]: https://github.com/YangtseSu/cirrocast/releases/tag/v0.1.0
