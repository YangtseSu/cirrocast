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

## Status

Early scaffold (step 01 of 14 in [`docs/plans/`](docs/plans/README.md)) — the CLI skeleton, XDG path
resolution and the provider registry are in place. Weather fetching starts at step 06
(`docs/plans/06-open-meteo-provider.md`); the wttr.in-style renderer at step 07. See the plan index
for live per-step progress.

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

Keys are BYOK: never stored in `config.toml`, read from `CIRROCAST_<PROVIDER>_KEY` or from
`keys.toml` (mode `0600`) managed by `cirrocast key set|rm|list`.

## Configuration

`$XDG_CONFIG_HOME/cirrocast/config.toml` (default `~/.config/cirrocast/config.toml`), cache in
`$XDG_CACHE_HOME/cirrocast/`, data in `$XDG_DATA_HOME/cirrocast/`. Full schema: `docs/plans/README.md`.

## Licence

GPL-3.0-or-later — see [`LICENSE`](LICENSE). The repository is
[REUSE](https://reuse.software/)-compliant; per-file copyright and licence information lives in
SPDX headers and in [`REUSE.toml`](REUSE.toml).
