<!--
SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Step 29 — QWeather air quality as a second air source

Status: ⬜ not-started
Depends on: `16-air-quality-and-pollen.md` (the `AirQuality` model, the `AirSource` dispatch, the panel and the `%q` token), `27-qweather-jwt-auth.md` (the qweather credential and `providers.qweather.host`), `15-alerts-and-severity.md` (the provider-bound source precedent: credential, host and error taxonomy)
Touches: `src/air/{mod,qweather}.rs` (new file), `src/air/aqi.rs`, `src/model/air.rs`, `src/model/mod.rs`, `src/config/mod.rs`, `src/cli.rs`, `src/i18n.rs`, `locales/{en-US,zh-CN}/main.ftl`, `src/render/{air,json,one_line,plain,art_table}.rs`, `tests/air.rs`, `tests/fixtures/qweather/`, `docs/providers.md`, `docs/configuration.md`, `docs/schema.md`, `README.md`, `docs/plans/README.md`, `CHANGELOG.md`

## Goal

`cirrocast --aqi` gains a second source. For a location where the qweather provider is on the run's
chain and a credential resolves, the panel reports **QWeather's own index set — including the
Chinese national AQI (`cn-mee`, 环境空气质量指数 GB 3095-2012)** — and its six pollutants, instead of
Open-Meteo's two consolidated indices. Open-Meteo stays the keyless default everywhere else, so a
default install still answers `--aqi` without a key.

The point of the step is the *index*, not the pollutants: a reader in China recognises 优/良/轻度污染
from the national scale, and today's panel can only print the US or European number. Everything else
(every pollutant in μg/m³, the panel, the JSON object, the cache key, the credit line) already
exists in step 16 and is reused.

## Deliverables

- ⬜ **Measured endpoint facts, recorded in the design notes below** (probe the account host live
      before writing code, and correct this file if the service differs from its docs):
      `GET {host}/airquality/v1/current/{latitude}/{longitude}` with an optional `lang`; the
      envelope is `metadata` + `indexes[]` + `pollutants[]` + `stations[]`; `indexes[]` carries at
      most two entries (QWeather's own QAQI plus the local standard), each
      `{code,name,aqi,aqiDisplay,category,color,primaryPollutant,health}`; `pollutants[]` carries
      `{code,name,fullName,concentration{value,unit},subIndexes}` with the **unit varying per
      pollutant** (the published example: PM in `μg/m3`, NO₂ in `ppb`, CO in `ppm`). Record for
      every pollutant the unit the account actually returns, and whether the payload carries an
      observation time at all.
- ⬜ `src/air/qweather.rs`: the request and the decoder, through the shared `fetch_json` path with
      `CacheKey::air("qweather", …)` and `cache.weather_ttl_secs` — the credential and host come
      from `providers.qweather.host` plus whichever of the API key / JWT the key store resolves
      (`QWeatherAuth`, step 27), exactly as `src/alerts/qweather.rs` borrows them; a `401` maps to
      the same `InvalidKey`/`InvalidCredential` taxonomy, a missing host is a configuration error
      naming the console, and a missing credential is the provider's `MissingCredential` error.
- ⬜ The pollutant mapping: `pm2p5`/`pm10`/`o3`/`no2`/`so2`/`co` → the six model fields, with the
      unit policy decided and pinned by tests — the payload's per-pollutant unit is either the
      canonical μg/m³ or a **documented conversion** (the factors and their source written down
      beside the table), never a silent rescale; a unit outside the table fails the reading the way
      `src/air/open_meteo.rs` fails a mismatched one.
- ⬜ The index mapping: `cn-mee` → the new Chinese scale, `us-epa` → `aqi_us`, `eaqi` → the
      European scale; every other documented code (`qaqi`, `cn-mee-1h`, `daqi`, `jp`, `mo`, `th`,
      `tw`, `us-epa-nc`) is ignored with a `--verbose` note rather than guessed into a scale, and an
      index the point does not carry stays `None`.
- ⬜ **The attribution duty, which is not optional.** QWeather's terms
      (`https://dev.qweather.com/docs/terms/attribution/`) require the response's
      `metadata.attributions` displayed **in full and unmodified** wherever air-quality data is
      shown, exactly as they do for warnings. The decoder carries the lines on the reading
      (`AirQuality.credit: Vec<String>`, the same shape step 15 gave `Alert::credit`), the panel and
      `plain` print them after the source's credit line, and `-f json` exposes them in the `air`
      object; a blank line is dropped, a duplicate printed once, and nothing is trimmed or reworded.
      Recorded here because the air panel is the second place the clause bites.
- ⬜ `AqiIndex::Cn` in `src/air/aqi.rs`: the GB 3095-2012 breakpoints (优 `0..=50`, 良 `51..=100`,
      轻度污染 `101..=150`, 中度污染 `151..=200`, 重度污染 `201..=300`, 严重污染 above 300), the
      category words in both catalogs, `--aqi-index cn` / `[air] index = "cn"` through the existing
      parser and validator, and the colour ramp the panel paints with. `AirQuality` gains
      `aqi_cn: Option<u16>` beside the two existing indices.
- ⬜ Source selection: `AirSource::QWeather` plus `[air] source = auto | open-meteo | qweather`
      (config key + `--air-source` flag + `--help` epilogue + `docs/configuration.md`). `auto`
      prefers QWeather when the point is in mainland China, the qweather provider is on the run's
      chain and a credential resolves; otherwise Open-Meteo — the same "coverage selects the
      national service, the keyless one is the floor" rule the alert registry uses. An explicit
      `qweather` without host or credential is the documented error, never a silent fallback.
- ⬜ Renderer and JSON: the source label and credit line (`licence_line("qweather")`), the CN index
      row in the compact and stacked panels, `%q` following `[air] index` (so `cn` works there
      too), the `air` JSON object gaining `aqi_cn` and the `source` value `qweather` — a
      **non-breaking** addition under `docs/schema.md`'s compatibility rule ("a new key appears →
      unchanged"), so the key index and `docs/schema/json-v2.json` are updated while
      `schema_version` stays `2` and the frozen v1 document is untouched.
- ⬜ Tests: unit decode tests over a recorded payload (both index codes present, an absent index, a
      pollutant with a converted unit and one with an unknown unit), the category table at every
      boundary, an integration test that renders the panel from a seeded cache entry, the width
      tests at 59/60/80/120 columns with the longer CN label, and a `#[ignore]`d live probe beside
      the existing ones.
- ⬜ Fixtures: recorded responses under `tests/fixtures/qweather/` (already annotated as recorded
      QWeather API data in `REUSE.toml`) with the request URLs recorded in
      `tests/fixtures/alerts/README.md`'s sibling for the provider or in a new
      `tests/fixtures/qweather/README.md` row.
- ⬜ Docs: `docs/providers.md` gains a `### qweather air quality` block (endpoint, index codes, the
      unit table, coverage, the credit obligation, the one-request cost) and its obligations table
      row; the README air section names the second source; `CHANGELOG.md` gains the entry;
      `docs/plans/README.md` marks this step.

## Design notes

* **Why this source at all.** The panel's whole value for a Chinese reader is the number they see in
  every other app, and that number is the national AQI. Open-Meteo's CAMS data cannot produce it.
  QWeather publishes it under `cn-mee` and the same free tier (first 50 000 requests/month at ¥0)
  already pays for the forecast, so the marginal cost of a `--aqi` run is one request.
* **QAQI is not a substitute.** QWeather's own universal index is explicitly *not available for
  China* ("QAQI 暂时不适用于中国地区"), so a Chinese point must be read from `cn-mee`, and the two
  must never be conflated into one field.
* **The unit policy.** `src/air/open_meteo.rs` refuses a non-canonical unit because that API reports
  μg/m³ natively and a mismatch means upstream changed something. QWeather reports **different units
  per pollutant by design** (ppb/ppm for the gases), so refusal alone would drop half the panel:
  this source converts, with the factor table, its source and its assumptions (reference
  temperature/pressure for the ppm→μg/m³ step) written into the code and pinned by a test, and
  still refuses anything outside the table.
* **Observation time.** `AirQuality.time` is required. If the payload carries no timestamp (verify
  in the probe), use the fetch instant and say so in the module docs, the way the forecast backend
  records that its `observed_at` is the fetch instant.
* **One extra request, one cache entry.** The reading is cached under
  `weather/qweather-air-<lat.2dp>-<lon.2dp>-<local date>.json`, beside the Open-Meteo reading rather
  than replacing it, so switching sources (or losing the credential) never serves the other
  source's numbers.
* **The auto rule is deliberately narrow.** A keyless default install must keep answering `--aqi`
  offline of any credential; QWeather is chosen only when the user has already put the provider on
  the chain, which is also when the host and credential are known to work.
* **No new dependency.** Everything reuses `serde`, the shared `HttpClient`, the cache and the
  existing unit helpers; `deny.toml` is untouched.

## Out of scope

* The hourly and daily air-quality forecasts (`/airquality/v1/hourly|daily`) and the `stations[]`
  list: the panel is "now" only, as step 16 decided.
* The health-advice block (`health.effect`) and `primaryPollutant`: no surface exists for prose
  advice, and the panel's job is the number.
* The remaining national standards (`daqi`, `jp`, `mo`, `th`, `tw`, `cn-mee-1h`): one new scale per
  step, and none of them is the reason for this one.
* Pollen: QWeather's air API does not publish it.
* A standalone `air` subcommand: `--aqi` / `--format aqi` remain the surface.

## Verification

```bash
# the live probe, with the account configured (host + credential):
cargo run -q -- --aqi -p qweather Beijing -f json | jq '.air | {source, aqi_cn, pm2_5, no2}'
#   → "source": "qweather", a "aqi_cn" number, pollutants in μg/m³ (converted, not raw ppb)
cargo run -q -- --aqi -p qweather Beijing          # the panel, with the CN label and its credit line
cargo run -q -- --aqi --air-source open-meteo Beijing -f json | jq '.air.source'   # → "open-meteo"
cargo run -q -- --aqi-index cn --aqi Beijing       # the colour ramp and the category word
cargo run -q -- --aqi --air-source qweather @0,0   # the documented "needs a credential/host" error
cargo test --workspace --locked                    # unit + integration, no network
cargo clippy --workspace --all-targets --locked -- -D warnings && reuse lint
```

## Exit criteria

- ⬜ `--aqi` against a Chinese point with the qweather provider on the chain prints the national
      index, its category word and the six pollutants in μg/m³, verified live and pasted into the
      progress log.
- ⬜ Every index code and pollutant unit the service can return is either mapped or explicitly
      ignored with a `-v` note — nothing is guessed into a scale, and the unit table has a test per
      row.
- ⬜ The response's attribution lines are printed verbatim with the air data in every format that
      shows it (the terms' requirement), proven by a test and by a live run pasted into the log.
- ⬜ `--aqi` without any credential still works exactly as before (Open-Meteo, keyless), proven by
      the existing air tests passing unchanged.
- ⬜ `docs/providers.md`, `docs/configuration.md`, `docs/schema.md`, the README and the CHANGELOG
      describe the second source, the new scale and the new key; `reuse lint` is clean.

## Risks

* **The unit table is a measurement, not a promise.** If the service changes a pollutant's unit the
  reading fails loudly (the desired outcome), but a *new* pollutant or index code appearing is a
  `-v` note rather than a failure — the plan should keep those two behaviours distinct.
* **The auto rule can surprise a CN user** who adds a QWeather key and sees `--aqi` switch sources.
  The `source` field in the panel and the JSON makes the switch visible; if it proves confusing, the
  fallback is to make `auto` mean Open-Meteo and require the explicit flag.
* **Terms.** The provider's terms row already requires naming QWeather with the link wherever its
  data is shown; the air panel must carry that line like the forecast does.

## Progress log

- 2026-10-07 — step created from the QWeather free-tier audit (the user's question about which
  services of the 50 000-request plan are actually wired up). Endpoint, envelope, index codes and
  the per-pollutant unit mix read from the published docs (2026-10-07) and recorded above as
  deliverables rather than as verified facts: the live probe is the first deliverable on purpose.
