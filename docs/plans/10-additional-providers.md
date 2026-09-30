<!--
SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Step 10 — Additional providers

Status: 🚧 in-progress
Depends on: 05, 06, 08
Touches: src/provider/{mod,openweathermap,weatherapi,worldweatheronline,pirateweather,qweather,smhi}.rs, src/render/mod.rs,
src/cli.rs, src/http.rs, tests/provider_*.rs, tests/fixtures/{owm,weatherapi,wwo,pirateweather,qweather,smhi}/, README.md,
docs/providers.md, docs/plans/01-project-scaffold.md (registry corrections)

## Goal
Add six real backends behind the existing `Provider` trait — OpenWeatherMap, WeatherAPI, World Weather Online,
PirateWeather, QWeather and SMHI — each with its endpoint, auth, native-code → WMO mapping, day-part aggregation,
declared limits and attribution, behind one shared HTTP helper and a contract-faithful fallback chain.

## Deliverables
- ✅ `docs/providers.md`: the verified provider reference (auth, endpoints, request parameters, response
      fields consumed, free-tier quotas with their wording, coverage, granularity, attribution/licence
      duties, caching ceilings, traps and unverified items, per backend and per location/IP service),
      plus the at-a-glance tables, the status-code behaviour table and the re-verification log. Authored
      here because the registry re-verification below needs a written record; `docs/plans/23-docs-and-guides.md`
      keeps the user-facing review of the same file.
- ✅ `provider/mod.rs`: the shared JSON helper landed as `provider::fetch_json(env, &JsonFetch { provider,
      request, key, ttl, what })` (naming amended: the low-level `http::HttpRequest` already exists, so the
      descriptor is `JsonFetch`, and the secret travels on the request instead of in a `key_secret` field —
      `HttpRequest::secret` marks a credential so the redaction reaches the `-v` lines, the error messages
      *and* the cache envelope, which `redacted_normalized()` keeps clean). The cache key is the caller's
      (`CacheKey::weather` for one-call backends, the new `CacheKey::weather_part` for a backend with more
      than one resource), because only the provider knows the location, the day count and the local date.
      Taxonomy as built: one helper for UA + `Accept-Encoding` (ureq's default `gzip` feature decompresses
      transparently), the cache policy from `cache.rs` (TTL, offline/refresh/no-cache), retries with backoff
      and `Retry-After` from `http.rs`, and JSON decode naming the provider. `401` → the new
      `Error::InvalidKey` (exit 6, "replace it with `cirrocast key set <id>`"); a `403` deliberately stays
      `Error::Upstream` (exit 3) because it carries quota/plan/host refusals whose body text is the actionable
      part — a provider that can tell "invalid key" apart refines it in its own decoder; `429`/`5xx`/timeout
      keep `Upstream`/`Network` so a chain continues.
- ⬜ Fallback chain semantics as contracted: only `Error::Upstream`/`Error::Network` continue to the next entry,
      while `Usage`/`Config`/`Location`/`MissingKey`/`InvalidKey` abort immediately with their own exit code. An
      exhausted chain reports every attempt: `error: all providers failed: open-meteo (upstream timeout after
      15s); smhi (out of coverage: 39.90,116.40 outside the PMP3g domain)`.
- ⬜ Capability-driven degradation, documented in `src/render/mod.rs` and tested: a provider with
      `daily == false` still renders `plain`/`one-line`/`json` (day sections collapse to current conditions) and
      `art-table` falls back to a single current-conditions block with a `--verbose` note — never an error.
- ⬜ `openweathermap.rs`: current `https://api.openweathermap.org/data/2.5/weather?lat=&lon=&units=metric&appid=`
      and forecast `https://api.openweathermap.org/data/2.5/forecast?lat=&lon=&units=metric&appid=` (3-hourly,
      40 slots ≈ 5 days); auth is the `appid` query param from `CIRROCAST_OPENWEATHERMAP_KEY`; free tier
      60 calls/min, 1,000,000 calls/month, **2** calls per fetch (a registry note). Mapping: an explicit
      `weather[0].id` table by group (`2xx` thunder, `3xx` drizzle, `5xx` rain and showers, `6xx` snow and
      snow-showers, `7xx` atmosphere → 45, `800`–`804` clear→overcast), else Unknown, exhaustiveness-tested.
      Aggregation: 3-hourly `list` slots into local 00–06/06–12/12–18/18–24 windows, summing
      `rain.3h`+`snow.3h`, averaging `main.feels_like`. Limits: `max_days: 5`, `hourly: false` (3 h),
      `uv_index: None`, visibility current-only, `requires_key: true`; the data licence is **ODbL** and the terms
      require visible attribution ("Weather data provided by OpenWeather" plus a link and the logo — see
      `docs/providers.md`). Traps pinned there: `rain`/`snow` are absent rather than zero and the key differs
      between endpoints (`rain.1h` vs `rain.3h`), `main.temp_min/max` are not daily extremes, and `511`/`616`
      carry snow icons upstream.
- ⬜ `weatherapi.rs`: `GET https://api.weatherapi.com/v1/forecast.json` with `key`, `q` (`lat,lon` or name),
      `days` and `lang=en` (pinned: provider-side translation is deliberately unused, our Fluent catalogs own the
      wording); auth `key` from `CIRROCAST_WEATHERAPI_KEY`; free tier **100,000 calls/month** with a 3-day
      forecast (1,000,000 was wrong; paid plans get 14 days), `Limited` air quality, 1 call per fetch. Mapping:
      explicit `condition.code` (**53 codes over 1000–1282**, not 1000–1087) table by family (sky 1000–1009,
      obscuration 1012–1048 + 1030, patchy 1063–1087, snow/fog 1114–1147, drizzle and rain 1150–1201,
      sleet/snow/ice pellets 1204–1237, showers 1240–1264, thunder 1273–1282); the canonical list at
      `weatherapi.com/docs/weather_conditions.json` is explicitly licensed for offline vendoring. Aggregation:
      `forecastday[].hour[]`, `day.mintemp_c/maxtemp_c`, `astro.sunrise/sunset` parsed from the 12-hour clock
      (`"04:31 PM"`; `"No moonrise"`/`"No moonset"` are possible), `day.uv`. Limits: `hourly: true`,
      `max_days: 3` on a free key (a paid key raises the cap; the step 08 clamp covers it), `requires_key: true`.
      Obligations: free keys must credit WeatherAPI.com by name or logo, caching caps are 60 min (current) and
      24 h (forecast), and the terms add a mandatory end-user disclaimer.
- ⬜ `worldweatheronline.rs`: `GET https://api.worldweatheronline.com/premium/v1/weather.ashx` with `key`,
      `q=lat,lon`, `format=json`, `num_of_days=` and `tp=3`. **Corrected 2026-09-30**: the `free/v1` path is not
      documented anywhere current and returns 403 — every official example and the pricing page's "All plans share
      the same core API suite" put a free key on `premium/v1`; `format=json` is mandatory because the documented
      default is **XML** (not pipe-delimited text); `num_of_days` is 1–14 at the endpoint (`0` = current only)
      while the FAQ says the free API gives 5 days and the pricing matrix ticks 14-day forecasts for Free; the
      quota is 100 requests/day per the pricing page and free terms, with a 500/month figure in the docs index
      callout (both recorded in `docs/providers.md`). Auth: `CIRROCAST_WORLDWEATHERONLINE_KEY`. Mapping: an explicit
      `weatherCode` table by family (sky, mist/fog, patchy/freezing/blowing, thunder, drizzle/sleet, rain, snow,
      showers, thunder-with-rain/snow), exhaustiveness-tested, from the 49-code feed. Caveat: WWO wraps scalars in
      single-element arrays (`hourly[0].weatherDesc[0].value`) and every number is a JSON **string**, so a `serde`
      helper `one<T>` (scalar or array) lives here and is unit-tested, and `hourly[].time` is an unpadded local-time
      string (`"700"`). Limits: `max_days: 5`, `hourly: false`, `requires_key: true`, `location_kinds: LatLon`.
      Obligations: free-tier credit is mandatory ("Weather Data by WorldWeatherOnline.com"), the terms add a
      mandatory end-user disclaimer, and caching caps are 60 min (current) / 24 h (forecast).
- ⬜ `pirateweather.rs`: `GET https://api.pirateweather.net/forecast/<key>/<lat>,<lon>` with
      `units=si&exclude=minutely,alerts&lang=en` (Dark-Sky-compatible payload; the key is a **path** segment and
      is masked in logs). **Corrected 2026-09-30**: there is no `tz` parameter (localise with the response's
      `timezone` name and `offset` hours); the free tier is 10,000 calls/month ($2/month → 20,000) with a
      per-key 1–4 req/s limit; auth `CIRROCAST_PIRATEWEATHER_KEY`. Mapping: an explicit `currently.icon` string
      table (`clear-day`/`clear-night`, `partly-cloudy-day`/`-night`, `cloudy`, `wind`, `fog`, `rain`, `snow`,
      `sleet`, `none`) refined by `precipIntensity` in **PirateWeather's own** mm/h bands
      (`0.02`/`0.4`/`2.5`/`10`, not Dark Sky's 0.4/3.4) into that family's sibling. Aggregation: `hourly.data[]`
      (unix `time`, localised) in six-hour windows; `precipAccumulation` is centimetres under `units=si` and must
      become millimetres — pinned by a fixture. Limits: `daily: true` (`daily.data[]`, 7 days), `hourly: true`
      (48 h; `extend=hourly` → 168 h), `max_days: 7`, `requires_key: true`, `location_kinds: LatLon`. No
      attribution is documented in the terms — do not invent a credit line for it (see `docs/providers.md`).
- ⬜ `qweather.rs`: **Corrected 2026-09-30 — decide v7 vs v1 before writing the module.** The city-based v7
      endpoints (`/v7/weather/now`, `/v7/weather/{3,7,10,15,30}d`, `/v7/weather/{24,72,168}h`, GeoAPI
      `/geo/v2/city/lookup`) are deprecated with an EOL in 2027 (the official pages disagree: 2027-06-01 in the
      deprecation table, 2027-02-01 in the v7 doc), and their successors are `/weather/v1/{current,hourly,daily}`
      (metric-only, m/s wind, metres visibility, RFC 7807 errors). Implementing v7 now buys at most a year; the
      step must either move to v1 after one more documentation pass or ship v7 with the EOL recorded and a
      migration task. `<host>` has no default and comes from config `[providers.qweather].host`; it is the
      **per-account** host issued by the console (`<id>.xy.qweatherapi.com` — the `<id>.re.qweather.com` form in
      the first draft of this plan is documented nowhere, and the legacy `api.`/`devapi.`/`geoapi.qweather.com`
      domains are being discontinued from 2026); a missing host while qweather is selected is `Error::Config`
      (exit 4) with `set [providers.qweather].host (see cirrocast provider info qweather)`. Auth:
      `X-QW-Api-Key: <key>` header or `key=` query parameter (never both) from `CIRROCAST_QWEATHER_KEY`; JWT is
      the recommended method but needs token minting and stays out of scope. Free tier: pay-as-you-go with the
      **first 50,000 requests/month at ¥0** (there is no "Standard" free plan), QPM 3,000; day ranges are not a
      billing dimension, so the plan does not cap `<nd>`. Mapping: an explicit
      `now.icon`/`iconDay`/`iconNight` table by family (clear, partly cloudy, overcast, rain, snow, fog/haze,
      extreme hot/cold, unknown), where `iconNight` selects night art and the published CSV's missing 150–153
      night family is passed through rather than mapped. Aggregation: day parts from `/v7/weather/24h`, else
      synthesized from `daily[]` with `warning: qweather has no hourly data on this plan; day parts show daily
      values`. Naming: the GeoAPI name is kept in both scripts and the display name picked from it (`zh-CN` →
      Chinese, `en-US` → English/pinyin), and GeoAPI results must not be bulk-cached. Limits: `daily: true`,
      `max_days: 7`, `requires_key: true`, `location_kinds: City|LatLon`; responses are gzip-compressed
      (transport default — send `Accept-Encoding: gzip`; there is no `gzip=y` parameter). Obligation: name
      QWeather + `https://www.qweather.com` wherever data is shown.
- ⬜ `smhi.rs`: **Corrected 2026-09-30 — the endpoint changed.** `GET
      https://opendata-download-metfcst.smhi.se/api/category/snow1g/version/1/geotype/point/lon/<lon>/lat/<lat>/data.json`
      (the old `category=pmp3g/version/2/geopoint/lat/…` path was decommissioned 2026-03-31 and returns 404; note
      `lon` before `lat`, and docs moved to `opendata.smhi.se/metfcst/snow1gv1`). Keyless, no auth, open data
      under **CC BY 4.0 SE** (name SMHI as the source; "Källa: SMHI" is the conventional rendering, not SMHI's
      own wording); no published quota, so the cache does the throttling, and SMHI's fair-use rules forbid
      re-fetching the same data or mass-downloading. Outside the valid-area polygon (Nordics and adjacent seas,
      roughly lon −18…44, lat 50…75) the service returns **HTTP 404 with an empty body** (the docs claim 400),
      which MUST surface as `Error::Upstream` (so `auto` falls through) and never as a hard failure or a silently
      empty report.
- ⬜ `smhi.rs` full `Wsymb2` (1–27) → WMO: `1`→0 clear, `2`→1 nearly clear, `3`→2 variable cloudiness, `4`→2
      halfclear, `5`→3 cloudy, `6`→3 overcast, `7`→45 fog, `8`→80 light rain showers, `9`→81 moderate rain
      showers, `10`→82 heavy rain showers, `11`→95 thunderstorm, `12`→66 light sleet showers, `13`→67 moderate
      sleet showers, `14`→67 heavy sleet showers, `15`→85 light snow showers, `16`→85 moderate snow showers,
      `17`→86 heavy snow showers, `18`→61 light rain, `19`→63 moderate rain, `20`→65 heavy rain, `21`→95 thunder,
      `22`→66 light sleet, `23`→67 moderate sleet, `24`→67 heavy sleet, `25`→71 light snowfall, `26`→73 moderate
      snowfall, `27`→75 heavy snowfall; sleet is deliberately lossy because WMO 4677 has no mixed-precipitation
      umbrella, so the `66/67` family carries it and the native value stays in `raw`. **Corrected 2026-09-30**:
      the table stands (SMHI's public symbol page numbers 1–27 with 5 absent), but the parameter is now
      `symbol_code` inside `timeSeries[].data` and arrives as an integer; the code→name mapping comes from that
      symbol page because the canonical table is client-rendered and unreachable (see `docs/providers.md`).
- ⬜ `smhi.rs` parsing: **Corrected 2026-09-30 for the SNOW1gv1 shape.** `timeSeries[].data` is a JSON object
      keyed by parameter name (`air_temperature`, `wind_speed`, `wind_from_direction`, `symbol_code`,
      `precipitation_amount_mean_deterministic`, `precipitation_amount_mean`, `visibility_in_air`,
      `cloud_area_fraction`, …) and must be read by key, never by index or order; the `parameters=` filter needs
      literal commas (`%2C` silently drops all but the first). Units: `m/s` wind, `km` visibility, `kg/m2`
      precipitation (numerically mm) accumulated over `[intervalParametersStartTime, time)`, and
      `cloud_area_fraction` in **oktas** (×12.5 for percent). `9999` is the in-band missing sentinel for every
      parameter (and `precipitation_frozen_part: -9` for "no precipitation"), so both map to `None`. The run
      carries 82 steps ≈ 10 days with the step widening 1 h → 6 h → 12 h, so the provider emits as many days as
      the payload contains, `max_days: 10` is an upper bound and fewer days than requested is not an error; no UV
      parameter exists (`uv_index: None`). Limits: `current: true` (nearest step), `hourly: true`, `daily: true`,
      `alerts: false`, `requires_key: false`, `key_env: None`, `location_kinds: LatLon`.
- ⬜ `provider info <ID>` prints id, display name, capabilities, auth mechanism and key variable, how to store
      the key (`cirrocast key set <id>`), attribution with URL and licence, rate limits, coverage, day-part
      granularity, docs URL and the registry row's `verified` date; `provider list` gains `key: yes|no` and
      `max days` columns, `--verbose` prints `attribution: <text> (<url>)` after every successful fetch (silenced
      by `-q`; OWM, WeatherAPI, WWO and QWeather require it by their terms, PirateWeather has no documented
      attribution duty — see `docs/providers.md`), and the README gains a provider table
      (id, key env var, free-tier note, coverage, granularity) plus one paragraph on the chain and attribution.
- ✅ Registry re-verification (done 2026-09-30, landed with `docs/providers.md`): every declared limit in the
      provider registry (`src/provider/mod.rs`, rows from step 01/06) was re-read against the provider's live
      documentation; each row gained `verified: 2026-09-30` and the wrong numbers were corrected in the registry
      **and** noted in `docs/plans/01-project-scaffold.md` in the same commit (SMHI's docs URL and horizon note,
      WWO's `max_days` 3 → 5 and its docs URL, QWeather's "China focused" note, WeatherAPI's, OWM's and
      PirateWeather's notes, the free-tier headlines). Quotas that cannot be pinned without an account (QWeather's
      console, WWO's conflicting pages, PirateWeather's paid tiers) are recorded as plan-dependent in
      `docs/providers.md` rather than guessed.
- ⬜ Fixtures (recorded, never live): `tests/fixtures/owm/{current,forecast,forecast-partial-last-window}.json`,
      `weatherapi/{forecast,forecast-key-error}.json`, `wwo/{weather-ashx,weather-ashx-single-element-arrays}.json`,
      `pirateweather/{forecast,forecast-null-values}.json`,
      `qweather/{now,7d,7d-no-hourly,error-401}.json` + `7d.json.gz` (or the v1 equivalents, if the v7/v1 decision
      goes to v1), `smhi/{point-2day,point-out-of-coverage}.json` (the SNOW1gv1 shape: `timeSeries[].data`); keys
      are scrubbed and a test greps the fixture tree for key-shaped strings.
- ⬜ `tests/provider_*.rs` (one per provider, offline): mapping exhaustiveness (anything unlisted → Unknown),
      aggregation boundaries (first/last window, a `Europe/Stockholm` DST transition, a partial trailing day), unit
      traps (PirateWeather centimetres, SMHI mm/h, WWO single-element arrays) and error mapping (401 → exit 6,
      429 → the chain continues, 500 and malformed JSON → exit 3).

## Design notes
Each provider is one file + one `ProviderId` variant + one registry row + fixtures, with no per-provider CLI flag;
`http_json` keeps retry/cache/timeout/UA/gzip/error taxonomy in one place, and aggregation stays in the providers
because only they know their native slot length (the renderer keeps the single unit-conversion point).
Lossy mappings are documented rather than modelled (sleet → `66/67`, wind/dust icons → `Overcast`, unknown →
`Unknown`): provider-specific pseudo-codes in `Condition` would leak provider vocabularies into the renderers, and
free-tier numbers belong in the registry with a `verified` date, not in code.

## Out of scope
- Any provider not listed here: `metar`/`taf` (step 11) and every other backend (roadmap in step 14); a new provider
  needs a registry row, fixtures and a `provider info` entry — never a CLI flag.
- Keyring storage of API keys, alert/warning rendering (roadmap in step 14), QWeather's legacy JWT auth,
  paid-tier-only, air-quality/indices and historical endpoints, radar/tiles, async I/O and provider racing.

## Verification
Fixtures: the per-provider directories above (current + forecast + one edge case each) and the `tests/fixtures/cli/*`
stubs that pin exit codes 3/4/6 with no network; the manual smoke run is live and keyed commands are `#[ignore]`d:
```
cargo run --release -- -p smhi --lat 59.33 --lon 18.06 --days 5 -v     # real SMHI data, no key, attribution line
cargo run --release -- -p smhi --lat 39.9 --lon 116.4 -v; echo $?      # out of coverage → exit 3, domain named
cargo run --release -- -p auto --lat 39.9 --lon 116.4 -v               # open-meteo serves it; -v lists attempts
CIRROCAST_QWEATHER_KEY=... cargo run --release -- -p qweather Beijing  # without host → exit 4 + hint
cirrocast provider list && cirrocast provider info smhi
```

## Exit criteria
- ⬜ `cargo fmt --check` / `cargo clippy --all-targets -- -D warnings` / `cargo test` / `reuse lint` all clean
- ⬜ `provider list`/`provider info <ID>` cover all seven providers with capabilities, auth, key variable, attribution, limits and verification date
- ⬜ SMHI renders real data for Stockholm without a key; an out-of-coverage request exits 3 and `-p auto` falls through (observed under `-v`)
- ⬜ Each provider renders from its fixtures offline with correct condition text/art and no key bytes in any fixture or log
- ⬜ 401/403 → exit 6, 429 → chain continues, 500/timeout → exit 3, missing QWeather host → exit 4 with the hint;
      `--days` beyond a provider maximum warns once and clamps
- ⬜ A provider with `daily == false` still renders `plain`/`one-line`/`json`, and `art-table` degrades to a
      single current-conditions block
- ✅ Every registry limit carries a `verified` date matching the live documentation at that date (done
      2026-09-30; `every_row_carries_a_verified_date` in `src/provider/mod.rs` keeps it from regressing)

## Risks
- Free-tier limits drift and upstream schemas change: the registry carries a `verified` date per row, `--days`
  clamps instead of erroring when a maximum shrinks, fixtures pin the parsed shape, and live smoke tests are opt-in.
- Aggregation and configuration bugs (a `Europe/Stockholm` DST day, partial trailing days, a QWeather host copied
  from the wrong console) are the likeliest failures, mitigated by fixed-instant boundary fixtures and by refusing
  to guess a host; key leakage is prevented by masking secrets and scrubbing fixtures.

## Progress log
- 2026-09-30 — step file written; endpoints, mapping strategies, aggregation rules, the SMHI `Wsymb2` table and
  the registry re-verification duty recorded.
- 2026-09-30 — `docs/providers.md` authored and the registry re-verification deliverable closed: all eight
  backends and the four location/IP services were checked against their live documentation in one pass
  (per-source reports, quotes and source URLs in the file). Corrections that changed this step's plan: SMHI's
  endpoint moved to `category=snow1g/version/1/geotype/point/lon/…/lat/…` with a flat `timeSeries[].data` object
  (the planned `pmp3g` path is decommissioned and 404s); QWeather's city v7 APIs are deprecated (EOL 2027, the
  official pages disagree on the month) and its free allowance is 50 000 requests/month pay-as-you-go, not a
  "Standard" plan — the v7/v1 decision is now an explicit prerequisite of the `qweather.rs` deliverable;
  WeatherAPI's free tier is 100 000 calls/month with 3 forecast days and its condition codes run to 1282, not
  1087; WWO's `free/v1` path is undocumented and 403s (free keys use `premium/v1`), its `format` default is XML,
  and its quota pages contradict each other (100/day vs 500/month); PirateWeather has no `tz` parameter and no
  documented attribution duty, and its intensity bands are its own 0.02/0.4/2.5/10 mm/h; OWM's data licence is
  ODbL with mandatory visible attribution. `ProviderMeta` gained `verified` (printed by `provider info`), the
  wrong registry rows were corrected in the same commit, and `docs/plans/01-project-scaffold.md` notes the
  correction. `docs/plans/23-docs-and-guides.md` keeps its review of the same file for the user-facing pass.
- 2026-10-01 — shared HTTP helper landed (`provider::JsonFetch` / `fetch_json`), with the redaction and
  taxonomy pieces it needs: `HttpRequest::secret`/`redacted_url`/`redacted_normalized` (and a `Debug` impl
  that cannot print a credential), `Error::InvalidKey` (exit 6), `CacheKey::weather_part` for multi-resource
  backends, `Cache::mode()`, and `Cache::read_or_fetch_json` naming the provider in decode errors. Open-Meteo
  and the three geocoders migrated to the new signatures in the same commit; `tests/provider_http.rs` covers
  401/403/429/5xx/malformed-body/offline/cache-hit/redaction. Naming and the 401-vs-403 split deviate from the
  text above and are recorded there.
