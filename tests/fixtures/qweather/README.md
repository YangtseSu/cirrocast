<!--
SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
SPDX-License-Identifier: GPL-3.0-or-later
-->

# QWeather fixtures

Recorded responses of QWeather's v1 APIs, used by `tests/air.rs`, `tests/provider_qweather.rs`,
`tests/alerts.rs` (through `fixtures/alerts/qweather-rainstorm.json`) and the unit tests of
`src/air/qweather.rs`. Nothing here is fetched by a test run. The files are the vendor's response
bodies, so the directory carries the `LicenseRef-Proprietary-API-Data` annotation in
[`REUSE.toml`](../../../REUSE.toml); this README is first-party text like the rest of the repository.

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
