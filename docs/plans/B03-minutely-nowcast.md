<!--
SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
SPDX-License-Identifier: GPL-3.0-or-later
-->

# B03 — minutely precipitation nowcast (backlog)

Status: ⏸ backlog — deferred on 2026-10-07: no scheduled phase asks for it, and the feature needs a
new model type, a new renderer surface and at least one new upstream request. Scheduling it means
renumbering it into the A-series tail first (`docs/plans/README.md`).
Depends on: 03 (canonical model and units), 06 (Open-Meteo provider), 10 (PirateWeather, the BYOK precedent), 24 (coverage-aware `auto` and the keyless national backends), 16/26 (the "supplementary fetch beside the forecast" pattern: `air`, `marine`, `normals`)
Touches: `src/model/{minutely,mod}.rs` (new), `src/provider/{mod,open_meteo,pirateweather,met_no}.rs`, `src/cache.rs`, `src/cli.rs`, `src/render/{mod,art_table,plain,one_line,json}.rs`, `src/i18n.rs`, `locales/{en-US,zh-CN}/main.ftl`, `tests/{minutely,provider_open_meteo,provider_pirateweather}.rs`, `tests/fixtures/{open-meteo,pirateweather,met-no}/`, `docs/providers.md`, `docs/schema.md`, `README.md`, `CHANGELOG.md`

## Goal

`cirrocast --minutely Beijing` answers the question the hourly table cannot: *is it about to rain, and
when does it stop.* A short series (per-minute or per-15-minute precipitation, SI mm/h) is fetched
beside the forecast — never as a chain entry, exactly like the air, marine and normals blocks — and
rendered as a compact nowcast line in `art-table`, a `minutely` block in `plain`, a `%`-token for
`one-line`, a standalone `--format minutely` view, and a typed `minutely` object in `json`. A
backend that carries no nowcast data degrades to "not available", never to "no rain".

## Source survey (2026-10-07, official docs + live probes)

| Source | Coverage | Span / resolution | Auth | Free tier | Licence | Repo status |
|---|---|---|---|---|---|---|
| **Open-Meteo** `minutely_15=` on `/v1/forecast` | global; native 15-min only inside HRRR (N. America) and ICON-D2/AROME (Central Europe), interpolated from hourly elsewhere | 288 steps (~3 local days) by default, `forecast_minutely_15=<n>` narrows it; 15-min | keyless | 10 000/day, 5 000/h, 600/min; non-commercial | CC BY 4.0, link beside data | **same host, same request, same cache key** — one query parameter plus a column-array decode; `minutely_15` is already documented as an unused parameter in `docs/providers.md` |
| **MET Norway Nowcast 2.0** `/weatherapi/nowcast/2.0/complete` | Norway, Sweden, Finland, Denmark (radar-dependent: `radar_coverage: ok｜temporarily unavailable｜no coverage`) | 2 h, **5-min**, refreshed every 5 min | keyless, the mandatory descriptive User-Agent | 20 req/s; re-request only after `Expires` | CC BY 4.0 | same host/UA/cache, **different endpoint and schema** → new request + decoder, coverage-gated like `smhi`/`nws` |
| **PirateWeather** `minutely` block | global (15-min accuracy inside the HRRR domain) | 60 min, 1-min steps | BYOK `CIRROCAST_PIRATEWEATHER_KEY` | 10 000 calls/month | none documented | **already excluded by `exclude=minutely,alerts`** — drop the token, decode the block; same URL, key and cache entry |
| **Bright Sky `/radar`** (DWD RADOLAN RV) | Germany, 1 km² pixels | past 6 h + **2 h forecast, 5-min**; base64+zlib 2-byte ints | keyless | none published | DWD open data (CC BY 4.0) / Bright Sky credit | same host, but a new endpoint and a binary grid decoder — the most work for the narrowest coverage |
| **QWeather minutely** `/v7/minutely/5m` | **China only**, 1 km grid | 2 h, 5-min; `minutely[].precip` mm/5 min + a vendor `summary` | BYOK | 50 000 req/month at ¥0 | proprietary; name + link | the repo speaks **v1** on the per-account host; minutely exists only on **v7** (EOL 2027-02-01) and whether the account host serves it is unverified — not worth a decoder today |
| OpenWeatherMap One Call (minutely) | global | 60 min, 1-min | BYOK, **paid "One Call by Call" subscription** | card required; not the free plan the repo uses | ODbL 1.0 + logo | reject: different product, paid tier |
| WeatherAPI.com `tp=15` | global | 15-min **Enterprise only** | BYOK | free tier is hourly | proprietary | reject: nothing on the free tier |
| Apple WeatherKit `forecastNextHour` | global `[INFERENCE]` | next hour, 1-min | ES256 developer token, Apple Developer account | pay-per-call | attribution mandatory | reject for v1: heavy auth for one data set |
| AccuWeather MinuteCast / Google Weather `forecast:minutes` / Caiyun 彩云天气 | global / global / China | 1–2 h, 1-min | BYOK | trial-only / pay-per-call / enterprise-only | proprietary | reject: no free tier |

Not offering any nowcast: `nws`, `smhi`, `visualcrossing`, `worldweatheronline`, `metar`,
`open-meteo-archive`, `open-meteo-marine`.

Sources: the vendors' own documentation and live probes — `open-meteo.com/en/docs` (and the
`minutely_15` probe), `api.met.no/weatherapi/nowcast/2.0/documentation`,
`docs.pirateweather.net` (data blocks), `api.brightsky.dev/openapi.json`, the QWeather OpenAPI
(`github.com/qwd/dev-site`), `openweathermap.org/api/one-call-3`, `developer.accuweather.com`,
`developers.google.com/maps/documentation/weather/minute-forecast`,
`docs.caiyunapp.com/weather-api`. Full list in this step's creation log entry.

## Deliverables

- ⬜ `src/model/minutely.rs`: `Minutely { samples: Vec<MinuteSample { at, precip_mm_per_h, probability_pct, kind }>, summary: Option<String>, source, span_minutes }`, SI only, `kind` limited to rain/snow/sleet (the one thing every source agrees on); `Report.minutely: Option<Minutely>` as a `serde(default)` field so pre-existing JSON documents still parse.
- ⬜ `src/provider/open_meteo.rs`: request `minutely_15=precipitation,precipitation_probability` and decode the column arrays; a response without the block is `None` (not an empty series), and the interpolation caveat (native only inside HRRR/ICON-D2/AROME) is documented in the module and in `docs/providers.md`.
- ⬜ `src/provider/pirateweather.rs`: drop `minutely` from `exclude` and decode the 60-step block (`precipIntensity`, `precipProbability`); the existing `-999`/absent-field rules apply unchanged.
- ⬜ `src/provider/met_no.rs`: the Nowcast 2.0 endpoint as a second request, coverage-gated on the location's country (`NO`/`SE`/`FI`/`DK`) and honouring the `Expires`/`If-Modified-Since` handshake the provider already implements; a `radar_coverage` of `no coverage` is "not available", not an error.
- ⬜ `src/cache.rs`: a `CacheKey::weather_part(<provider>, "minutely", …)` entry for the sources with their own endpoint; Open-Meteo rides its existing weather entry (its key does not encode parameters).
- ⬜ Registry: `minutely: bool` in `ProviderMeta`/`Capabilities`, printed by `provider info` and `provider list`; no new provider id (all three are existing rows).
- ⬜ Renderers: `--minutely` flag, `--format minutely`, the compact `art-table` line (start/stop/steps), a `%`-token in `src/template.rs` with a `TokenSpec` row and a `tests/templates.rs` row, the `plain` block, and the typed `json` object with `docs/schema.md` + `docs/schema/json-v2.json` updated (additive, `schema_version` stays).
- ⬜ Wording: derive "rain stops in N min" from the series in the catalogs rather than printing a vendor's English `summary`; the vendor string, when present, is kept only in the JSON object as `summary`.
- ⬜ Tests: column-array and block decoders over recorded fixtures, the "block absent → `None`" case, the coverage gate for met.no, the renderer at 59/60/80/120 columns, the template token count assertion, and `#[ignore]`d live probes for each source.
- ⬜ Docs: `docs/providers.md` gains a nowcast block per source (endpoint, resolution, coverage, caveats) and its at-a-glance rows; the README documents `--minutely` and `--format minutely` with an example block; `CHANGELOG.md` gains the entry.

## Design notes

* **Order the work by cost, not by glamour.** Open-Meteo first (a parameter and a decode, keyless, works for every install), PirateWeather second (one request-token change, BYOK), MET Norway third (a new endpoint behind a coverage gate). Bright Sky's radar grid is a fourth step only if a real raster/sparkline surface is wanted.
* **Nowcast is not a chain entry.** Like `air`, `marine` and `normals`, the fetch is supplementary: a failure is a `--verbose` note, never an exit code, and the forecast renders regardless.
* **The cache TTL is fine.** `cache.weather_ttl_secs` (600 s) is short relative to a 60–120 minute nowcast; met.no keeps its own `Expires` duty. A stale entry must degrade to `None` when the series' last sample is already in the past.
* **Coverage before capability.** met.no's nowcast covers four countries; the source selection must use the same country-coverage rule the keyless national backends already use, so a Beijing run never asks Norway for rain.
* **Rejected on principle:** every paid-only or enterprise-only source (One Call, WeatherAPI 15-min, WeatherKit, MinuteCast, Google, Caiyun) and QWeather's China-only v7 endpoint, which is EOL 2027-02-01 and unverified on the account host the repo actually uses.

## Out of scope

* A radar map, tiles or any raster output: the terminal surface is text and a sparkline at most.
* Severe-weather or lightning nowcasts: alerts are step 15's registry.
* Sub-minute or per-second interpolation, and any client-side model blending between sources.
* QWeather minutely until it appears on the v1 host (a follow-up probe can revisit it).

## Verification

```bash
cargo run -q -- --minutely Berlin                    # the compact nowcast line, Open-Meteo
cargo run -q -- --minutely Oslo -p met-no -v         # the 5-min radar series, coverage gate visible
cargo run -q -- --minutely Beijing -f json | jq '.minutely | {source, span_minutes, samples: (.samples|length)}'
cargo run -q -- --minutely Beijing -p open-meteo-archive   # "not available", exit 0
cargo run -q -- -f minutely Beijing                  # the standalone view
cargo test --workspace --locked && cargo clippy --workspace --all-targets --locked -- -D warnings
reuse lint
```

## Exit criteria

- ⬜ A keyless default install answers `--minutely` for any point on Earth (Open-Meteo), and the output says when the precipitation starts and stops in the reader's units and language.
- ⬜ A backend without nowcast data renders "not available" (exit 0) instead of an empty or zero series, proven by a test per source.
- ⬜ MET Norway's nowcast is selected only inside its four countries, and its `Expires`/`If-Modified-Since` duty is honoured.
- ⬜ The JSON object, its schema file and the template token table are documented and tested; `reuse lint` is clean.

## Risks

* **Interpolation masquerading as observation.** Open-Meteo's 15-minute series is model-interpolated outside the HRRR/ICON-D2/AROME domains; the output must not present it as radar data. The module docs and `docs/providers.md` say so, and the renderer labels the source.
* **A nowcast that is already stale** (cache hit near the end of the window) would print rain that has passed; the liveness rule above (drop samples before the run's instant, degrade to `None` when none remain) is the mitigation.
* **Scope creep into a map.** The raster temptation is the reason Bright Sky is last and conditional.

## Progress log

- 2026-10-07 — backlog item created at the user's request, together with a source survey. The survey was run against the vendors' official documentation and live probes on 2026-10-07: Open-Meteo (`minutely_15` parameter, probe with `forecast_minutely_15=8`), MET Norway Nowcast 2.0 (probe for Oslo), PirateWeather (data-block docs and the repo's own `exclude=minutely` line), Bright Sky `/radar`, the QWeather OpenAPI and minutely docs, OpenWeatherMap One Call 3/4 pricing, WeatherAPI's published spec, WeatherKit's `forecastNextHour`, AccuWeather MinuteCast, Google's minute forecast and Caiyun's 2-minutely product. No code exists for this item.
