<!--
SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Air-quality fixtures

Responses of the Open-Meteo Air Quality API
(`https://air-quality-api.open-meteo.com/v1/air-quality`), used by `tests/air.rs` and the unit
tests of `src/air/open_meteo.rs`. Nothing here is fetched by a test run.

## Recorded

Fetched 2026-10-03 with the exact request the adapter sends (`current=us_aqi,european_aqi,pm2_5,
pm10,ozone,nitrogen_dioxide,sulphur_dioxide,carbon_monoxide,alder_pollen,birch_pollen,
grass_pollen,mugwort_pollen,olive_pollen,ragweed_pollen`, `timezone=auto`, no unit parameters), so
the payloads carry the upstream units (`μg/m³` with the Greek mu, `grains/m³`) the decode asserts.

| File | What it pins |
|---|---|
| `berlin-2026-10-03.json` | All six pollen fields present (zero in October, but present); both AQI indices and all six pollutants |
| `sydney-2026-10-03.json` | Outside the CAMS European pollen domain: all six pollen fields `null`, everything else present |
| `reykjavik-2026-10-03.json` | Inside the domain with a zero count: all six pollen fields `0.0` — `Some(Pollen)` with zeros, not `None` |

## Hand-written

Edge cases the live API would not produce on demand. They keep the recorded response's shape and
differ only in the field under test.

| File | What it pins |
|---|---|
| `truncated.json` | A body that is not valid JSON: the cache re-fetch path and the decode error |
| `no-current.json` | A valid envelope with `"current": null`: `Error::Upstream` naming the missing block |
| `unit-mismatch.json` | `pm2_5` in `mg/m³`: the unit assertion fails with the received unit in the message |
