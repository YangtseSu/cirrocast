<!--
SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Step 10 — Additional providers

Status: not-started
Depends on: 05, 06, 08
Touches: src/provider/{mod,openweathermap,weatherapi,worldweatheronline,pirateweather,qweather,smhi}.rs, src/render/mod.rs,
src/cli.rs, src/http.rs, tests/provider_*.rs, tests/fixtures/{owm,weatherapi,wwo,pirateweather,qweather,smhi}/, README.md,
docs/plans/01-project-scaffold.md (registry corrections)

## Goal
Add six real backends behind the existing `Provider` trait — OpenWeatherMap, WeatherAPI, World Weather Online,
PirateWeather, QWeather and SMHI — each with its endpoint, auth, native-code → WMO mapping, day-part aggregation,
declared limits and attribution, behind one shared HTTP helper and a contract-faithful fallback chain.

## Deliverables
- [ ] `provider/mod.rs`: `HttpRequest { url, query: &[(&str, &str)], headers: &[(&str, &str)], cache_tag: &str,
      ttl_secs: u64, key_secret: Option<&str> }` and `http_json<T: DeserializeOwned>(env, req) -> Result<T>` —
      the one helper for UA + `Accept-Encoding`, the cache policy from `cache.rs` (TTL, offline/refresh/no-cache),
      retries with backoff, gzip and JSON decode, and the error taxonomy (401/403 → `MissingKey`/`InvalidKey`
      exit 6, 429 → `Upstream` with `Retry-After` honoured, 5xx/timeout → `Upstream`/`Network` exit 3, malformed
      body → `Upstream`), masking the secret in every log line (`***`).
- [ ] Fallback chain semantics as contracted: only `Error::Upstream`/`Error::Network` continue to the next entry,
      while `Usage`/`Config`/`Location`/`MissingKey`/`InvalidKey` abort immediately with their own exit code. An
      exhausted chain reports every attempt: `error: all providers failed: open-meteo (upstream timeout after
      15s); smhi (out of coverage: 39.90,116.40 outside the PMP3g domain)`.
- [ ] Capability-driven degradation, documented in `src/render/mod.rs` and tested: a provider with
      `daily == false` still renders `plain`/`one-line`/`json` (day sections collapse to current conditions) and
      `art-table` falls back to a single current-conditions block with a `--verbose` note — never an error.
- [ ] `openweathermap.rs`: current `https://api.openweathermap.org/data/2.5/weather?lat=&lon=&units=metric&appid=`
      and forecast `https://api.openweathermap.org/data/2.5/forecast?lat=&lon=&units=metric&appid=` (3-hourly,
      40 slots ≈ 5 days); auth is the `appid` query param from `CIRROCAST_OPENWEATHERMAP_KEY`; free tier
      60 calls/min, 1,000,000 calls/month, **2** calls per fetch (a registry note). Mapping: an explicit
      `weather[0].id` table by group (`2xx` thunder, `3xx` drizzle, `5xx` rain and showers, `6xx` snow and
      snow-showers, `7xx` atmosphere → 45, `800`–`804` clear→overcast), else Unknown, exhaustiveness-tested.
      Aggregation: 3-hourly `list` slots into local 00–06/06–12/12–18/18–24 windows, summing
      `rain.3h`+`snow.3h`, averaging `main.feels_like`. Limits: `max_days: 5`, `hourly: false` (3 h),
      `uv_index: None`, visibility current-only, `requires_key: true`; OWM's terms require attribution.
- [ ] `weatherapi.rs`: `GET https://api.weatherapi.com/v1/forecast.json` with `key`, `q` (`lat,lon` or name),
      `days` and `lang=en` (pinned: provider-side translation is deliberately unused, our Fluent catalogs own the
      wording); auth `key` from `CIRROCAST_WEATHERAPI_KEY`; free tier 1,000,000 calls/month, 3-day forecast, no
      air quality, 1 call per fetch. Mapping: explicit `condition.code` (1000–1087) table by family (clear/cloud
      1000–1009, fog 1030/1135/1147, drizzle and rain 1150–1201, sleet/snow/ice pellets/showers 1204–1264,
      thunder 1273–1282). Aggregation: `forecastday[].hour[]`, `day.mintemp_c/maxtemp_c`, `astro.sunrise/sunset`
      parsed from `%I:%M %p` to `HH:mm`, `day.uv`. Limits: `hourly: true`, `max_days: 3` on a free key (a paid key
      raises the cap; the step 08 clamp covers it), `requires_key: true`.
- [ ] `worldweatheronline.rs`: `GET https://api.worldweatheronline.com/premium/v1/weather.ashx` with `key`,
      `q=lat,lon`, `format=json`, `num_of_days=` and `tp=3`. The free tier serves only `weather.ashx` with `key=`
      query auth (no OAuth header, no `v3` endpoints) and caps `num_of_days` at 3; the daily quota is
      plan-specific and MUST be read off the live pricing page while implementing. `format=json` is mandatory —
      the default response is pipe-delimited text. Auth: `CIRROCAST_WORLDWEATHERONLINE_KEY`. Mapping: an explicit
      `weatherCode` table by family (sky, mist/fog, patchy/freezing/blowing, thunder, drizzle/sleet, rain, snow,
      showers, thunder-with-rain/snow), exhaustiveness-tested. Caveat: WWO wraps scalars in single-element arrays
      (`hourly[0].weatherDesc[0].value`), so a `serde` helper `one<T>` (scalar or array) lives here and is
      unit-tested, and `hourly[].time` is an unpadded local-time string. Limits: `max_days: 3`, `hourly: false`,
      `requires_key: true`, `location_kinds: LatLon`.
- [ ] `pirateweather.rs`: `GET https://api.pirateweather.net/forecast/<key>/<lat>,<lon>` with
      `units=si&exclude=minutely,alerts&lang=en` (Dark-Sky-compatible payload; the key is a **path** segment and
      is masked in logs); auth `CIRROCAST_PIRATEWEATHER_KEY`; free tier ≈ 10,000 calls/month (dev plan), 1 call
      per fetch — re-verify. Mapping: an explicit `currently.icon` string table (`clear-day`/`clear-night`,
      `partly-cloudy-day`/`-night`, `cloudy`, `wind`, `fog`, `rain`, `snow`, `sleet`) refined by
      `precipIntensity` in Dark Sky's mm/h bands (`<0.4` light, `0.4..=3.4` moderate, `>3.4` heavy) into that
      family's sibling. Aggregation: `hourly.data[]` (unix `time`, localised) in six-hour windows;
      `precipAccumulation` is centimetres under `units=si` and must become millimetres — pinned by a fixture.
      Limits: `daily: true` (`daily.data[]`), `hourly: true`, `max_days: 7`, `requires_key: true`,
      `location_kinds: LatLon`.
- [ ] `qweather.rs`: `GET https://<host>/v7/weather/now?location=<lon>,<lat>&unit=m` and
      `GET https://<host>/v7/weather/<nd>d?location=<lon>,<lat>&unit=m` with `<nd>` ∈ 3/7/10/15/30 (7 by
      default). `<host>` has no default and comes from config `[providers.qweather].host` (the console-issued
      host differs between the China and global sign-up flows); a missing host while qweather is selected is
      `Error::Config` (exit 4) with `set [providers.qweather].host (see cirrocast provider info qweather)`.
      Auth: `X-QW-Api-Key: <key>` header from `CIRROCAST_QWEATHER_KEY`; the legacy JWT flow is **not**
      implemented. Free tier: quotas and the maximum `<nd>` are plan/account-specific — read them in the console
      and record them in the registry. Mapping: an explicit `now.icon`/`iconDay`/`iconNight` table by family
      (clear, partly cloudy, overcast, rain, snow, fog/haze, extreme hot/cold, unknown), where `iconNight` selects
      night art. Aggregation: day parts from `/v7/weather/24h` when the plan allows, else synthesized from
      `daily[]` with `warning: qweather has no hourly data on this plan; day parts show daily values`. China-first
      naming: the `q=`/`CityLookup` name is kept in both scripts and the display name picked from it (`zh-CN` →
      Chinese, `en-US` → English/pinyin). Limits: `daily: true`, `max_days: 7` (free-plan dependent),
      `requires_key: true`, `location_kinds: City|LatLon`; responses are gzip-encoded (step 05's `ureq` `gzip`
      feature, `flate2` as contingency), proven by a `.json.gz` fixture.
- [ ] `smhi.rs`: `GET https://opendata-download-metfcst.smhi.se/api/category/pmp3g/version/2/geopoint/lat/<lat>/lon/<lon>/data.json`
      — keyless, no auth, open data (attribution "Källa: SMHI"; no published quota, so a descriptive UA and the
      cache do the throttling). Outside the PMP3g domain (Nordics and nearby waters) the service errors or returns
      an empty/short `timeSeries`, which MUST surface as `Error::Upstream` (so `auto` falls through) and never as
      a hard failure or a silently empty report.
- [ ] `smhi.rs` full `Wsymb2` (1–27) → WMO: `1`→0 clear, `2`→1 nearly clear, `3`→2 variable cloudiness, `4`→2
      halfclear, `5`→3 cloudy, `6`→3 overcast, `7`→45 fog, `8`→80 light rain showers, `9`→81 moderate rain
      showers, `10`→82 heavy rain showers, `11`→95 thunderstorm, `12`→66 light sleet showers, `13`→67 moderate
      sleet showers, `14`→67 heavy sleet showers, `15`→85 light snow showers, `16`→85 moderate snow showers,
      `17`→86 heavy snow showers, `18`→61 light rain, `19`→63 moderate rain, `20`→65 heavy rain, `21`→95 thunder,
      `22`→66 light sleet, `23`→67 moderate sleet, `24`→67 heavy sleet, `25`→71 light snowfall, `26`→73 moderate
      snowfall, `27`→75 heavy snowfall; sleet is deliberately lossy because WMO 4677 has no mixed-precipitation
      umbrella, so the `66/67` family carries it and the native value stays in `raw`.
- [ ] `smhi.rs` parsing: `parameters[]` is a flat array per time step and must be read by `name` (`t`, `ws`,
      `wd`, `wsymb2`, `r1` mm/h, `pmean`, `vis`, `tcc_mean`), never by index, with `r1` scaled by the step
      length. The horizon varies per run (≈ 2–10 days), so the provider emits as many days as the payload
      contains, `max_days: 10` is an upper bound and fewer days than requested is not an error; no UV parameter
      exists (`uv_index: None`). Limits: `current: true` (nearest step), `hourly: true`, `daily: true`,
      `alerts: false`, `requires_key: false`, `key_env: None`, `location_kinds: LatLon`.
- [ ] `provider info <ID>` prints id, display name, capabilities, auth mechanism and key variable, how to store
      the key (`cirrocast key set <id>`), attribution with URL and licence, rate limits, coverage, day-part
      granularity, docs URL and the registry row's `verified` date; `provider list` gains `key: yes|no` and
      `max days` columns, `--verbose` prints `attribution: <text> (<url>)` after every successful fetch (silenced
      by `-q`; OWM, WeatherAPI and PirateWeather require it by their terms), and the README gains a provider table
      (id, key env var, free-tier note, coverage, granularity) plus one paragraph on the chain and attribution.
- [ ] Registry re-verification: every declared limit in the provider registry (`src/provider/mod.rs`, rows from
      step 01/06) is re-read against the provider's live documentation here; each row gains
      `verified: <YYYY-MM-DD>` and any wrong number is corrected in the registry **and** in
      `docs/plans/01-project-scaffold.md` in the same commit, while quotas that cannot be pinned without an
      account (QWeather, WWO, PirateWeather) are recorded as "plan-dependent, read from the console/pricing page".
- [ ] Fixtures (recorded, never live): `tests/fixtures/owm/{current,forecast,forecast-partial-last-window}.json`,
      `weatherapi/{forecast,forecast-key-error}.json`, `wwo/{weather-ashx,weather-ashx-single-element-arrays}.json`,
      `pirateweather/{forecast,forecast-null-values}.json`, `qweather/{now,7d,7d-no-hourly,error-401}.json` +
      `7d.json.gz`, `smhi/{geopoint-2day,geopoint-out-of-coverage}.json`; keys are scrubbed and a test greps the
      fixture tree for key-shaped strings.
- [ ] `tests/provider_*.rs` (one per provider, offline): mapping exhaustiveness (anything unlisted → Unknown),
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
- [ ] `cargo fmt --check` / `cargo clippy --all-targets -- -D warnings` / `cargo test` / `reuse lint` all clean
- [ ] `provider list`/`provider info <ID>` cover all seven providers with capabilities, auth, key variable, attribution, limits and verification date
- [ ] SMHI renders real data for Stockholm without a key; an out-of-coverage request exits 3 and `-p auto` falls through (observed under `-v`)
- [ ] Each provider renders from its fixtures offline with correct condition text/art and no key bytes in any fixture or log
- [ ] 401/403 → exit 6, 429 → chain continues, 500/timeout → exit 3, missing QWeather host → exit 4 with the hint;
      `--days` beyond a provider maximum warns once and clamps
- [ ] A provider with `daily == false` still renders `plain`/`one-line`/`json`, and `art-table` degrades to a
      single current-conditions block
- [ ] Every registry limit carries a `verified` date matching the live documentation at that date

## Risks
- Free-tier limits drift and upstream schemas change: the registry carries a `verified` date per row, `--days`
  clamps instead of erroring when a maximum shrinks, fixtures pin the parsed shape, and live smoke tests are opt-in.
- Aggregation and configuration bugs (a `Europe/Stockholm` DST day, partial trailing days, a QWeather host copied
  from the wrong console) are the likeliest failures, mitigated by fixed-instant boundary fixtures and by refusing
  to guess a host; key leakage is prevented by masking secrets and scrubbing fixtures.

## Progress log
- 2026-09-30 — step file written; endpoints, mapping strategies, aggregation rules, the SMHI `Wsymb2` table and
  the registry re-verification duty recorded.
