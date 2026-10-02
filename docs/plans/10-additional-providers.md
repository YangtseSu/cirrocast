<!--
SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Step 10 — Additional providers

Status: ✅ done
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
- ✅ Fallback chain semantics as contracted (the `fetch_chain` walk already existed from step 06): only
      `Error::Upstream`/`Error::Network` continue to the next entry, while `Usage`/`Config`/`Location`/`MissingKey`/
      `InvalidKey` abort immediately with their own exit code. The exhausted-chain message landed as
      `Error::Chain { attempts }` (exit 3), rendered as `error: all providers failed: <id> (<reason>); <id>
      (<reason>)` with `chain_reason`'s `network:`/`upstream:` classifier — observed live:
      `-p smhi,open-meteo --offline @39.9,116.4` printed both offline-miss reasons.
- ✅ Capability-driven degradation: the renderers already handle a report with no days (`art-table` guards the
      table with `!report.days.is_empty()`, `one-line` falls back to `current`, and `plain`/`json` keep their
      keys), and the fixtures cover it (`tests/fixtures/report/current-only.json` is rendered by the json,
      snapshot, width and plain suites). The `--verbose` note (`note: <provider> reports no forecast days;
      rendering current conditions only`) landed in the CLI. The end-to-end pin through a real `daily == false`
      backend moves to step 11, whose `metar` is the first one — there is no such provider in this step.
- ✅ `openweathermap.rs` (landed 2026-10-01): current `https://api.openweathermap.org/data/2.5/weather?lat=&lon=&units=metric&appid=`
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
- ✅ `weatherapi.rs` (landed 2026-10-01): `GET https://api.weatherapi.com/v1/forecast.json` with `key`, `q` (`lat,lon` or name),
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
- ✅ `worldweatheronline.rs` (landed 2026-10-01): `GET https://api.worldweatheronline.com/premium/v1/weather.ashx` with `key`,
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
- ✅ `pirateweather.rs` (landed 2026-10-01): `GET https://api.pirateweather.net/forecast/<key>/<lat>,<lon>` with
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
- ✅ `qweather.rs` (landed 2026-10-01 on **v1**, per the decision recorded below): `GET
      https://<host>/weather/v1/current/<lat>/<lon>` and `…/weather/v1/hourly/<lat>/<lon>?hours=<n>` with
      `lang=en`, where `<host>` is the **per-account** host from `[providers.qweather].host` (a missing host is
      `Error::Config`, exit 4, with `set providers.qweather.host (see cirrocast provider info qweather)`; the
      legacy `api.`/`devapi.`/`geoapi.qweather.com` domains answer `403 Invalid Host` with a valid key, probed
      2026-10-01). Auth: `X-QW-Api-Key` header from `CIRROCAST_QWEATHER_KEY`. The v7 city endpoints
      (`/v7/weather/…`) are deprecated (EOL 2027), so the module speaks v1: measure objects
      (`{"value":…,"unit":…}`) whose units are **checked** (`°C`, `m/s`, `mm`, `hPa`, `m`) instead of assumed,
      UTC instants in the minutes form (`2026-10-01T00:00Z`), `hours` ≤ 240 and `days` ≤ 10 (both probed),
      metric-only (`unit=i` ignored), RFC 7807 errors (401 → exit 6 through the shared helper). Findings pinned
      by tests: the hourly series is anchored to the **next UTC midnight** (so the location-local today is
      usually incomplete and skipped — the first fully covered day is emitted, like SMHI/OWM/WWO), the current
      block carries **no observation time** (the fetch instant stands in), and the payload has **no time zone**
      (a provisional location is refused with a usage error). The `days[]` block is deliberately not consumed
      (its day/night split does not map onto the four parts). Limits: `daily: true`, `max_days: 10`,
      `requires_key: true`, `location_kinds: City|LatLon`. Obligation: name QWeather +
      `https://www.qweather.com` wherever data is shown (GeoAPI is not called, so its no-bulk-cache rule does
      not apply).
- ✅ `smhi.rs` (landed 2026-10-01, see the corrections below): `GET
      https://opendata-download-metfcst.smhi.se/api/category/snow1g/version/1/geotype/point/lon/<lon>/lat/<lat>/data.json`
      (the old `category=pmp3g/version/2/geopoint/lat/…` path was decommissioned 2026-03-31 and returns 404; note
      `lon` before `lat`, and docs moved to `opendata.smhi.se/metfcst/snow1gv1`). Keyless, no auth, open data
      under **CC BY 4.0 SE** (name SMHI as the source; "Källa: SMHI" is the conventional rendering, not SMHI's
      own wording); no published quota, so the cache does the throttling, and SMHI's fair-use rules forbid
      re-fetching the same data or mass-downloading. The `parameters=` filter is deliberately not sent (its
      separator handling is literal-comma only, `%2C` silently drops all but the first parameter, and the
      unfiltered answer for a point is a few kilobytes). Outside the valid-area polygon (Nordics and adjacent
      seas, roughly lon −18…44, lat 50…75) the service returns **HTTP 404 with an empty body** (the docs claim
      400), which surfaces as `Error::Upstream("out of coverage: 39.90,116.40 is outside the SMHI valid area")`
      so `auto` falls through; an HTML error body is named as such instead of being pasted into the message.
      Two limitations, both recorded in `docs/providers.md`: the series starts at the current hour, so a
      location-local today whose night hours are already past cannot fill four `DayPart`s and is skipped (the
      backend emits the first fully covered days, usually starting tomorrow), and the payload carries no time
      zone or daylight flag, so a provisional (UTC) location is refused with a usage error and `is_day` comes
      from the local civil day until step 17 computes real sun times.
- ✅ `smhi.rs` full `Wsymb2` (1–27) → WMO: `1`→0 clear, `2`→1 nearly clear, `3`→2 variable cloudiness, `4`→2
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
- ✅ `smhi.rs` parsing: **Corrected 2026-09-30 for the SNOW1gv1 shape.** `timeSeries[].data` is a JSON object
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
- ✅ `provider info <ID>` prints id, display name, status, auth mechanism, key variable and the `cirrocast key
      set <id>` hint, capabilities, locations, coverage, granularity, documented limits, the credit line, docs URL
      and the registry row's `verified` date (`ProviderMeta` gained `auth`, `coverage`, `granularity` and `limits`
      for this, filled from the verified documentation); `provider list` keeps its `KEY`/`MAXDAYS` columns (they
      answer the `key: yes|no` and `max days` questions with more information), `--verbose` prints
      `attribution: <credit> (<request URL>)` after the fetch that answered, and the README gains a provider table
      (id, key env var, free-tier note, coverage, granularity) plus one paragraph on the chain and attribution.
- ✅ Registry re-verification (done 2026-09-30, landed with `docs/providers.md`): every declared limit in the
      provider registry (`src/provider/mod.rs`, rows from step 01/06) was re-read against the provider's live
      documentation; each row gained `verified: 2026-09-30` and the wrong numbers were corrected in the registry
      **and** noted in `docs/plans/01-project-scaffold.md` in the same commit (SMHI's docs URL and horizon note,
      WWO's `max_days` 3 → 5 and its docs URL, QWeather's "China focused" note, WeatherAPI's, OWM's and
      PirateWeather's notes, the free-tier headlines). Quotas that cannot be pinned without an account (QWeather's
      console, WWO's conflicting pages, PirateWeather's paid tiers) are recorded as plan-dependent in
      `docs/providers.md` rather than guessed.
- ✅ Fixtures (recorded, never live): `tests/fixtures/owm/{current,forecast,error_401}.json`,
      `weatherapi/{forecast,forecast-key-error}.json`, `wwo/{weather-ashx,weather-ashx-single-element-arrays}.json`,
      `pirateweather/{forecast,forecast-null-values}.json`,
      `qweather/{current,hourly,error_401}.json` (the v1 shapes), `smhi/{point-stockholm,point-out-of-coverage}`
      plus a hand-written `point_sentinel` (the SNOW1gv1 shape: `timeSeries[].data`); keys are scrubbed and
      `tests/cli_offline.rs` scans the fixture tree against the keys in the local `keys.toml`.
- ✅ `tests/provider_*.rs` (one per backend, offline): mapping exhaustiveness (anything unlisted → Unknown),
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
- ✅ `cargo fmt --check` / `cargo clippy --all-targets -- -D warnings` / `cargo test` / `reuse lint` all clean
      (29 suites green, REUSE 157/157 at the time of the run)
- ✅ `provider list`/`provider info <ID>` cover all seven providers with capabilities, auth, key variable, credit,
      limits and verification date
- ✅ SMHI renders real data for Stockholm without a key; an out-of-coverage request exits 3 and falls through
      (observed under `-v`: `-p smhi,open-meteo @39.9,116.4` → `smhi failed (upstream: out of coverage: 39.90,116.40 is
      outside the SMHI valid area); falling back to open-meteo`. `auto` puts open-meteo first, so the fall-through is
      demonstrated with the explicit chain)
- ✅ Each provider renders from its recorded fixtures offline with the right condition text and credit line, and
      no fixture carries a key: `tests/cli_offline.rs` seeds the cache with the recorded payloads and runs the real
      binary (`--offline -p <id> Beijing -f plain`) for all six implemented backends, plus a scan of
      `tests/fixtures/**` against the keys in the local `keys.toml`
- ✅ 401 → exit 6 (tested per backend and in `tests/provider_http.rs`), 429/500/timeout → exit 3 and the chain
      continues (same suite), `--days` beyond a provider maximum warns once and clamps (observed: `-p weatherapi -d 7`
      → one warning, 3 days), and the missing-QWeather-host path is exit 4 with the hint (unit-tested in
      `tests/provider_qweather.rs`). Deviation kept on purpose: a `403` stays exit 3 because it carries quota, plan
      and host refusals whose body text is the actionable part (documented in the helper and the provider reference)
- ✅ A report with no days still renders `plain`/`one-line`/`json` and `art-table` degrades to a single
      current-conditions block: pinned by `tests/fixtures/report/current-only.json` through the json, snapshot
      (art-table at width 80), width and plain suites, plus the `-v` note the CLI prints. The end-to-end pin through
      a real `daily == false` backend lands with `metar` in step 11 — this step has no such provider
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
- 2026-10-01 — SMHI backend landed (`src/provider/smhi.rs`, `tests/provider_smhi.rs`, recorded fixtures under
  `tests/fixtures/smhi/`, REUSE annotations for the CC BY 4.0 SE payloads). The day-part aggregation moved to
  `provider::dayparts` so SMHI and Open-Meteo share it, `Current.feels_like_c` became optional (SMHI publishes
  no apparent temperature), the registry row turned `implemented: true` with the `SMHI (CC BY 4.0 SE)` credit
  line, and `auto` now expands to `open-meteo,smhi`. Live smoke: `-p smhi Stockholm` printed real data with the
  credit, an out-of-coverage point exited 3 with `out of coverage: …`, and an in-coverage coordinate exited 2
  with the place-name hint. Deviations recorded in the deliverable above and in `docs/providers.md`: a partial
  location-local today is skipped, and a provisional zone is refused rather than aggregated in UTC.
- 2026-10-01 — OpenWeatherMap landed (`src/provider/openweathermap.rs`, `tests/provider_owm.rs`, recorded
  fixtures under `tests/fixtures/owm/` with the key scrubbed, ODbL-1.0 REUSE annotation). Two calls per fetch
  cached under `weather/openweathermap-{current,forecast}-…`; `units=metric` means m/s for wind; `rain`/`snow`
  are absent rather than zero and use different accumulation windows per endpoint; day extremes come from the
  slots; `weather[0].id` maps through the explicit family table (unit-tested over every published id);
  `weather[0].icon`'s trailing letter is the day/night flag. Deviations recorded in the deliverable: a
  provisional zone is refused before any request (the payload carries only a UTC offset), a partial
  location-local today is skipped, and a missing `deg` (calm wind) becomes 0. Live smoke: `-p openweathermap
  Beijing` printed real data with `Data: OpenWeather (ODbL 1.0) — https://openweathermap.org/`.
- 2026-10-01 — WeatherAPI landed (`src/provider/weatherapi.rs`, `tests/provider_weatherapi.rs`, recorded
  forecast and 401 fixtures under `tests/fixtures/weatherapi/`, custom `LicenseRef-Proprietary-API-Data`
  annotation plus a `LICENSES/` notice file for the commercial vendors). One call per fetch cached under
  `weather/weatherapi-…`; `lang=en` pinned; `tz_id` repairs a provisional zone (so coordinates work); the
  daily extremes and sun times come from the response's `day`/`astro` blocks (12-hour clock strings joined to
  the day's local date); `uv` fills the model's UV field; the condition table covers all 53 published codes
  with a unit test. A 403 keeps the `Upstream` taxonomy because it carries quota/plan refusals (the 401 → exit
  6 split is the shared helper's). Live smoke printed real data with the free-tier credit line.
- 2026-10-01 — World Weather Online landed (`src/provider/worldweatheronline.rs`, `tests/provider_wwo.rs`, recorded
  response and 401 fixtures under `tests/fixtures/wwo/`, `LicenseRef-Proprietary-API-Data` annotation). One call
  per fetch with `format=json` (the default is XML) and `tp=3`; every scalar arrives as a JSON string and the
  descriptive fields are single-element arrays, so `text_number` unwraps them; `hourly[].time` is an unpadded
  local `HHMM` joined to the day's date; the daily extremes and `astronomy[0]` sun times are the response's own.
  Two quirks pinned by tests and recorded in `docs/providers.md`: `observation_time` is the **UTC** wall clock
  (the docs say local; two recordings at known instants show UTC), and a provisional zone is refused before any
  request because the payload carries no zone. A day whose slots do not cover all four parts is skipped.
- 2026-10-01 — Pirate Weather landed (`src/provider/pirateweather.rs`, `tests/provider_pirateweather.rs`,
  recorded response and 401 fixtures under `tests/fixtures/pirateweather/`). One call per fetch with
  `units=si&exclude=minutely,alerts&lang=en&extend=hourly` (the extend is required for the 7-day horizon:
  without it the hourly block covers 48 h and days 3–7 would have no samples); the key is a path segment and
  is redacted everywhere; `timezone` repairs a provisional zone; `-999` and absent fields become `None`; the
  icon table covers the default set plus `hail` and refines the precipitation families by the provider's own
  mm/h bands. Deviations from the plan's sketch: `precipAccumulation` (centimetres) is **not** consumed — the
  parts sum the hourly liquid-equivalent intensities, so the scaling trap never applies — and `is_day` prefers
  an explicit `-day`/`-night` icon, falling back to the local civil day. Live smoke printed real data.
