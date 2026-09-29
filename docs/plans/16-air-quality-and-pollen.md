<!--
SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Step 16 — air quality and pollen

Status: not-started
Depends on: 03, 08
Touches: `src/air/mod.rs`, `src/air/aqi.rs`, `src/render/{art_table,plain,one_line,json,color}.rs`,
`src/cli.rs`, `src/config/mod.rs`, `src/cache.rs`, `src/i18n.rs`, `locales/{en-US,zh-CN}/main.ftl`,
`tests/air.rs`, `tests/fixtures/air/`, `tests/fixtures/air/README.md`, `REUSE.toml`,
`docs/plans/README.md`, `CHANGELOG.md`

## Goal

`cirrocast --aqi Berlin` appends an air-quality panel (US AQI, European AQI, the six regulated
pollutants, and pollen where the source covers it) to the `art-table` and `plain` output, `--format
aqi` prints it standalone, `one-line` can expand it with `%q`, and `json` carries it as a typed
object. Values are passed through in µg/m³ exactly as the source reports them, the AQI category is
computed locally with an authored colour ramp, and a location outside the pollen domain degrades to
"pollen not covered here" under `--verbose` instead of failing.

## Deliverables

- [ ] `src/air/mod.rs`: `AirQuality { time: DateTime<FixedOffset>, aqi_us: Option<u16>,
      aqi_european: Option<u16>, pm2_5: Option<f64>, pm10: Option<f64>, o3: Option<f64>,
      no2: Option<f64>, so2: Option<f64>, co: Option<f64>, pollen: Option<Pollen>, source: AirSource }`
      and `Pollen { alder, birch, grass, mugwort, olive, ragweed: f64 }` (grains/m³);
      `AirSource::{OpenMeteo}` (an enum rather than a `&str`, so step 19 can add more sources
      without a schema change).
- [ ] `src/air/aqi.rs`: `AqiCategory` with the two documented scales — US: Good `0..=50`,
      Moderate `51..=100`, UnhealthyForSensitiveGroups `101..=150`, Unhealthy `151..=200`,
      VeryUnhealthy `201..=300`, Hazardous `301..=500` (and `>500` clamped to Hazardous plus the
      `beyond_index` flag); European: Good `0..=20`, Fair `21..=40`, Moderate `41..=60`,
      Poor `61..=80`, VeryPoor `81..=100`, ExtremelyPoor `>100`. `from_us(u16)`,
      `from_european(u16)`, `color(&ColorMode)`, `i18n_key()`.
- [ ] `src/air/open_meteo.rs`: `https://air-quality-api.open-meteo.com/v1/air-quality?latitude=<lat>
      &longitude=<lon>&current=us_aqi,european_aqi,pm2_5,pm10,ozone,nitrogen_dioxide,
      sulphur_dioxide,carbon_monoxide,alder_pollen,birch_pollen,grass_pollen,mugwort_pollen,
      olive_pollen,ragweed_pollen&timezone=auto` — keyless, no `&apikey`; parses `current.*`,
      `current_units.*` (asserted to be `μg/m³` / `grains/m³`, a unit mismatch is an `Error::Upstream`
      with the received unit in the message), `utc_offset_seconds`, `timezone`.
- [ ] Cache: reuse the report cache key with an `-air` suffix (`weather/open-meteo-air-<lat.2dp>-
      <lon.2dp>-<local-date>.json`), TTL = the existing weather TTL (600 s), `--no-cache`/`--refresh`/
      `--offline` semantics identical to step 05.
- [ ] CLI: `--aqi` (append panel), `--format aqi` (standalone view; implies `--aqi`),
      `--aqi-index <us|european>` (which index drives the category colour and the one-line summary;
      default `us`, configurable as `[air] index = "us"`), `%q` token in `one-line` (chosen because
      step 15 claims `%A`; documented in `--help` and in the README token table alongside `%A`).
- [ ] `json`: `"air": { "time", "source", "aqi_us", "aqi_european", "category": {"us","european"},
      "pm2_5", "pm10", "o3", "no2", "so2", "co", "pollen": null | {alder,birch,grass,mugwort,olive,
      ragweed}, "units": {"pollutants":"µg/m³","pollen":"grains/m³"} }` — additive relative to
      `schema_version` 2 from step 15, so no further bump.
- [ ] UV index: **not fetched here**. The panel prints the `uv_index` already present in the weather
      report (`src/model/mod.rs`, filled by step 06 from Open-Meteo's `uv_index` daily maximum) when
      the report carries it, labelled with its own provenance; the AQ module must compile and work
      with `uv_index: None`.
- [ ] Panel rendering with the width budget of step 07: 4-line compact panel ≥ 60 columns, stacked
      key/value lines below that, never a line wider than `ctx.width`; colour ramp Good→Hazardous
      (green → yellow → orange → red → purple → maroon) applied only to the category word and the
      value, `color = never` keeps the words.
- [ ] Graceful degradation, all three cases tested: all six pollen fields `null` ⇒ `pollen = None`
      plus one `--verbose` line `air: pollen forecast is not covered here (CAMS European domain
      only)`; a partially null pollen block ⇒ nulls read as `0.0` plus a `--verbose` note; a
      response without `current` ⇒ `Error::Upstream`, `--aqi` prints `air quality unavailable` on
      stderr and the run still exits 0 with the weather output intact.
- [ ] i18n: `aqi-category-{good,moderate,unhealthy-sensitive,unhealthy,very-unhealthy,hazardous,
      extremely-poor,fair,poor,very-poor}`, `aqi-panel-title`, `aqi-pollen-title`, `aqi-no-coverage`,
      `aqi-uv-label`, pollutant names (`pm2-5`, `pm10`, `o3`, `no2`, `so2`, `co`) and the six pollen
      species names.
- [ ] Tests (`tests/air.rs`): recorded fixtures for Berlin (all six pollen present), Sydney (all
      pollen `null`) and a malformed/truncated response; the boundary table for both scales at
      exactly `0, 50, 51, 100, 101, 150, 151, 200, 201, 300, 301, 500` (US) and
      `0, 20, 21, 40, 41, 60, 61, 80, 81, 100, 101` (European); missing pollen; out-of-coverage
      response; width-limited panel rendering at 59/60/80/120 columns (no line exceeds the width,
      no `NaN`, no `-0`).

## Design notes

* **Keyless single source.** Open-Meteo's Air Quality API covers the whole globe; adding a second
  pollutant provider would multiply the mapping work without adding coverage. Attribution is
  mandatory (CC BY 4.0): the panel footer and `--format aqi` footer print
  `Air quality data by Open-Meteo.com (CAMS ENSEMBLE)`; the same string goes to the README credits.
* **Measured behaviour drives the degradation design** (probes on 2026-09-29/30):
  Berlin `52.52,13.41` returns numbers for all six pollen fields; Sydney `-33.87,151.21` and the
  mid-Pacific point `0,-140` return `null` for all six; Reykjavík `64.15,-21.94` (inside the CAMS
  Europe domain) returns `0.0` for all six. "Zero" and "not covered" are therefore distinguishable
  only by null-ness, which is exactly what the `Option<Pollen>` rule encodes.
* **No unit conversion.** The API returns µg/m³ and grains/m³ (`current_units` proves it), and the
  contract stores SI; `--units us` does **not** change pollutant units, because AQI categories are
  not unit-dependent and mixing `µg/ft³` into a category scale is meaningless. Documented in the
  units table of step 03 as an explicit non-conversion.
* **Category vs index**: the raw index numbers are always shown; the category word is localised and
  coloured. `--aqi-index` selects which of the two scales drives the colour so EU users are not
  forced to read a US index.
* **No async, no new crates.** The fetch goes through `Env`'s `ureq` client and the existing cache;
  the new module has no dependencies beyond `serde`/`chrono` already in the tree.
* **`%q` over `%a`**: `%a` collides with wttr.in's "astronomy" slot semantics that step 17 fills with
  moon/solar tokens, so the AQI token is `%q` ("quality"), and both are listed in one token table.

## Out of scope

Pollutant *forecasts* (only `current` is requested), hourly AQ series in the graph renderer, indoor
IAQ, and the European AQI sub-indices (`european_aqi_pm2_5` …) — the consolidated index plus the raw
pollutants are enough to render the category and let advanced users read the numbers. A dedicated
`cirrocast air` subcommand is not added; `--aqi`/`--format aqi` cover the surface. Wildfire smoke
narratives and UV from the CAMS `uv_index` field are not duplicated — UV stays a weather-API field.

## Verification

```bash
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && reuse lint

cargo run -q -- --aqi Berlin                 # panel below art-table, AQI colour ramp, pollen block
cargo run -q -- --aqi Sydney -v              # pollen absent + one verbose "not covered" line
cargo run -q -- --aqi -f aqi --lat 52.52 --lon 13.41
#   standalone panel: us_aqi/european_aqi + 6 pollutants + 6 pollen + attribution footer
cargo run -q -- --aqi -f aqi --lat -33.87 --lon 151.21
#   pollen section replaced by "pollen: not covered at this location", exit 0
cargo run -q -- --aqi -f json Beijing | jq '.air.aqi_us, .air.pollen.birch'
#   number, number (or null outside the domain); jq -e '.air.units.pollutants == "µg/m³"'
COLUMNS=58 cargo run -q -- --aqi Beijing | awk '{ if (length($0) > 58) { print "TOO WIDE"; exit 1 } }'
cargo run -q -- --aqi --offline Beijing      # cached panel, no network syscalls (checked by the test)
```

## Exit criteria

- [ ] `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`, `reuse lint` clean.
- [ ] Boundary tests for both scales pass at the twelve/eleven values listed above.
- [ ] `--aqi` never changes the exit code of a successful weather run, including on an AQ error.
- [ ] No output line exceeds `--width`/`COLUMNS`, and the AQ panel contains no `NaN`, `inf` or `-0`.
- [ ] `%q` expands in `one-line` and is documented next to `%A` in `--help` and the README table.
- [ ] Fixtures are recorded responses; `tests/air.rs` passes with the network blocked.

## Risks

* Pollen availability is a CAMS-Europe property that can change without notice; the null-based rule
  and the verbose note are the mitigation, and the fixture set pins both branches.
* The 11 km European vs 45 km global grid mismatch means values on the domain border can jump; the
  panel always names the source and timestamp so a user can see what was sampled.
* `us_aqi`/`european_aqi` can disagree on the category for the same air; mitigated by showing both
  numbers and making the coloured one selectable rather than silently picking one.

## Progress log

- 2026-09-30 — step opened; live probes recorded Berlin (pollen present), Sydney and mid-Pacific
  (pollen `null`), Reykjavík (`0.0`), confirming the null-vs-zero rule in the design notes.
