<!--
SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
SPDX-License-Identifier: GPL-3.0-or-later
-->

# QWeather fixtures

Recorded responses of QWeather's v1 APIs, used by `tests/air.rs`, `tests/provider_qweather.rs`,
`tests/geo_qweather.rs`, `tests/geo_chain.rs`, `tests/alerts.rs` (through
`fixtures/alerts/qweather-rainstorm.json`) and the unit tests of `src/air/qweather.rs`. Nothing here
is fetched by a test run. The files are the vendor's response bodies, so the directory carries the
`LicenseRef-Proprietary-API-Data` annotation in [`REUSE.toml`](../../../REUSE.toml); this README is
first-party text like the rest of the repository.

## GeoAPI (`geo/`)

Fetched 2026-10-13 from the account host with `GET {host}/geo/v2/city/lookup` and the API key in the
`X-QW-Api-Key` header, so the payloads carry the row shape the decoder and the probe decisions were
measured against:

| File | Request | What it pins |
|---|---|---|
| `xinxiangxian-zh.json` | `location=新乡县&number=10&lang=zh` | A district-level query the bundled table cannot answer: ten rows, the district first, `adm1` 河南省, the localized `country` 中国 with **no ISO code anywhere**, the zone, and `rank` as a string |
| `xinxiang-zh.json` | `location=新乡&number=10&lang=zh` | The same prefecture asked by its city name: 新乡 first, 新乡县 second — the service's own relevance order, which is why the decoder does not sort by `rank` |
| `beijing-en.json` | `location=Beijing&number=5&lang=en` | The Latin spelling: `Beijing`, `Haidian`, `Chaoyang`, … with `country` as `China` |
| `no-such-location.json` | `location=Zzzqqqnotaplace&number=5&lang=en` | The **HTTP 400** refusal that a query matching nothing produces: `{"error":{"status":400,"type":"…#no-such-location",…}}`, which the decoder turns into an empty hit list |

The measured 401 refusal (`…#unauthorized`) is the shared one at the root of this directory,
`error_401.json`. Hand-written edge cases (a missing or empty `location[]`, an unknown zone, an
out-of-range coordinate, a blank name, a non-numeric latitude) live inline in
`src/geo/qweather.rs`'s test module rather than here, so this directory stays a record of what the
service actually answered.

## Air quality (`air/`)

Fetched 2026-10-13 from the account host with
`GET {host}/airquality/v1/current/{latitude}/{longitude}` and the API key in the `X-QW-Api-Key`
header, so the payloads carry the per-pollutant units the decoder's table is measured against:

| File | Requested point | What it pins |
|---|---|---|
| `beijing-2026-10-13.json` | `39.9,116.4` | The Chinese national AQI alone (`cn-mee`) with all six pollutants, CO in `mg/m³` and the rest in `μg/m³`; the single attribution line |
| `berlin-2026-10-13.json` | `52.52,13.40` | `qaqi` beside the European standard (`eu-eea`): the ignored code and the mapped one in one response, all pollutants in `μg/m³` |
| `tokyo-2026-10-13.json` | `35.68,139.69` | No mapped index at all (`qaqi` + `jp-moe`), NO₂ in `ppb`, and the two pollutant codes the model does not carry (`no`, `nmhc` in `ppmC`) |

There is no observation time in the payload; the decoder stamps the fetch instant. Hand-written
edge cases (an unknown unit on a mapped pollutant, a fractional national index, blank attribution
lines, an empty envelope) live inline in `src/air/qweather.rs`'s test module rather than here, so
this directory stays a record of what the service actually answered.

## Forecast and credentials

The recorded forecast bodies (`current.json`, `hourly.json`, the `current-*.json` error pages) are
documented by the provider section of [`docs/providers.md`](../../../docs/providers.md#qweather).
`ed25519-test-key.pem` is a throwaway key generated for the step 27 test vector, declared
first-party in [`REUSE.toml`](../../../REUSE.toml).
