<!--
SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Step 24 — keyless national backends and coverage-aware `auto`

Status: ✅ done
Depends on: `06-open-meteo-provider.md`, `10-additional-providers.md` (registry, chain, shared helper), `23-more-providers.md` (`met-no` proves the keyless national pattern and the `Expires` cache)
Touches: `src/provider/{mod,nws,brightsky}.rs`, `src/cache.rs`, `src/cli.rs`, `src/config/mod.rs`, `tests/{provider_nws,provider_brightsky,provider_auto}.rs`, `tests/fixtures/{nws,brightsky}/`, `docs/providers.md`, `README.md`, `docs/plans/{README,23-more-providers}.md`, `CHANGELOG.md`

## Goal

Two more official, keyless backends join the registry — **NWS** (`api.weather.gov`, US and
territories, hourly + 7-day periods, its alerts already covered by step 15) and **Bright Sky**
(DWD open data for Germany, hourly, self-hostable) — and `--provider auto` stops being a fixed list:
it expands per resolved location from registry metadata (country → bounding box → global), so a US
point starts at `nws`, a German point at `brightsky`, a Swedish point at `smhi`, a Norwegian point at
`met-no`, and everything else at `open-meteo`, always followed by the global keyless entries.
`provider list`/`provider info` also state each backend's network class (free vs non-free service),
so a distribution can document what a default install talks to.

## Deliverables

- ✅ `src/provider/mod.rs`: `ProviderId::{Nws, BrightSky}` (+ `as_str`/`FromStr`/`all()`/metadata
  rows); `ProviderMeta` gains `covers: Coverage` (the machine-readable form beside the existing
  free-text `coverage` string) and `network: NetworkClass`;
  `pub enum Coverage { Countries(&'static [&'static str]), Bbox([f64; 4]), Global }` and
  `pub enum NetworkClass { Free, NonFree }` with the definition recorded in `docs/providers.md`
  (free = keyless or self-hosted key, documented public API, no proprietary service in the path;
  non-free = a commercial BYOK service such as OpenWeatherMap, WeatherAPI, WWO, PirateWeather,
  Visual Crossing, QWeather, MeteoAlarm's token portal); `Capabilities` of both new rows:
  `Nws { hourly: true, daily: true, max_days: 7, alerts: true, requires_key: false,
  location_kinds: LatLon }`, `BrightSky { hourly: true, daily: true, max_days: <measured>,
  alerts: true, requires_key: false, location_kinds: LatLon }`.
- ✅ `src/provider/nws.rs` (two-step, like a station lookup): `GET
  https://api.weather.gov/points/{lat:.4},{lon:.4}` → `properties.{gridId, gridX, gridY, timeZone,
  relativeLocation.properties.{city, state}}`; the mapping is cached in a new **`grid/`** namespace
  (`grid/nws-<lat.3dp>-<lon.3dp>.json`, TTL 30 days) so a repeated run costs one request; then
  `GET https://api.weather.gov/gridpoints/{gridId}/{gridX},{gridY}/forecast/hourly` for the series
  and `…/forecast` for the daily high/low periods, both under `CacheKey::weather_part` and the
  existing TTL. `properties.timeZone` repairs a provisional (coordinates/OSM) zone. Mandatory
  descriptive `User-Agent` with contact information (step 15 already fixes the string for the alert
  adapter; this step reuses the same constant); `403`/`429` stay `Error::Upstream`/`Network` so the
  chain continues.
- ✅ `src/provider/nws.rs` decoding: hourly `properties.periods[]` → the four day parts through
  `provider::dayparts`; `temperature` is Fahrenheit for US points (`temperatureUnit` is read, not
  assumed — `°F` is converted to °C at decode time, the model stays metric), `windSpeed` is a string
  (`"10 mph"`, `"5 to 10 mph"` → the upper bound plus one `--verbose` note) and °-cardinal
  `windDirection` maps through the compass table shared with `metar`; `probabilityOfPrecipitation`
  and `relativeHumidity` are objects (`{value: Option<u8>}`); `shortForecast`/`icon` map to WMO
  through an explicit table (rain/thunder/snow/sleet/fog/cloud families, `*_day`/`*_night`
  variants), unknown text → WMO 3 plus one `--verbose` line, exhaustiveness-tested over the
  recorded fixtures. Daily periods give `temp_min_c`/`temp_max_c`; `max_days: 7`.
- ✅ `src/provider/brightsky.rs`: `GET https://api.brightsky.dev/weather?lat=&lon=&date=&tz=UTC`
  (DWD data resold by Bright Sky; `units` left at its SI default), consuming
  `weather[].{timestamp, temperature, wind_speed_10, wind_direction_10, wind_gust_speed_10,
  relative_humidity, precipitation_10, condition, icon, visibility, cloud_cover}` and
  `sources[]` for the station metadata carried into `Attribution.raw`; the condition/icon table is
  written out and exhaustiveness-tested, and the forecast horizon is the measured one (see the
  probe requirement below), not a guessed constant. **First commit of this provider is the probe**:
  `api.brightsky.dev` was unreachable from the recording network on 2026-10-03 (connection timeouts
  through the proxy, three attempts), so the step records the live response shape, the horizon and
  the terms on the day it lands; if the public instance stays unreachable, the provider ships
  against a recorded self-hosted instance (Bright Sky is open source and reads DWD open data)
  and the deviation — including the instance URL knob `[providers.brightsky] url` — goes into
  this file's Progress log.
- ✅ `src/provider/mod.rs`: `pub fn auto_chain(loc: &Location) -> Vec<ProviderId>` — entries whose
  `covers` contains the location's `country_code` (exact ISO code first, then a containing
  `Bbox[]`) come first, ordered by registry order, then every `Coverage::Global` keyless entry in
  registry order; a location with no `country_code` falls back to the global tier only; `metar` is
  never included and a station still prepends `metar` (step 06/11 semantics unchanged). Selection
  and fall-through rules do not change: only `Error::Upstream`/`Network` continues down the chain.
  `-v` prints the expansion (`auto: nws, open-meteo, met-no, smhi (US point)`).
- ✅ `src/cache.rs` + `docs/plans/README.md`: the new `grid/` namespace joins the cache-layout
  paragraph and `cache stat`/`cache clean` (which enumerate namespaces); its TTL (30 days) is
  documented beside the geocode TTL.
- ✅ `src/cli.rs`: `provider list` gains a `NET` column (`free`/`nonfree`) and `provider info <id>`
  prints `coverage` and `network` rows from the registry; no new flags (backends stay selectable
  with `--provider`).
- ✅ Fixtures and tests (offline): recorded NWS `points` + `forecast/hourly` + `forecast` payloads
  (a US point), a Fahrenheit/wind-string/missing-humidity edge fixture, `grid/` mapping reuse across
  two runs, an out-of-coverage `points` 404 naming the point; recorded Bright Sky payloads with a
  station block and a null-precipitation row; `provider_auto.rs` asserting the expansion for US, DE,
  SE, NO, HK and a coordinate with no country code, plus that every keyless registered entry appears
  in exactly one tier and that `metar` never does.
- ✅ Docs: `README.md` provider matrix rows (id, key env var, free/paid, coverage, days, credit);
  `docs/providers.md` rows with endpoint, auth (none), User-Agent policy, quota/terms notes, licence
  and `verified` dates, plus the network-class definition; `CHANGELOG.md`.
- ✅ Re-verify the registry rows this step touches (NWS horizon/UA policy, Bright Sky instance and
  terms) against the live pages and record each correction in the Progress log.

## Design notes

* **Why these two and not the rest of the breezy audit (2026-10-03).** Everything below was
  screened against the same bar step 23 set (documented API, stated licence, honest attribution,
  machine-readable payload that fits the canonical model) plus one new one: a provider must be able
  to fill all four day parts from real data. Screened out: **HKO** and **JMA** (official and
  keyless, but their public forecast payloads are *daily*; the canonical model requires four
  aggregated parts per day, and Open-Meteo already covers HK/JP — a daily-only day model would be a
  model change, not a provider); **FMI** (WFS/GML XML); **KNMI** (`api.app.knmi.cloud` addresses
  locations by numeric grid ids, needing a mapping table of its own); **DMI**, **Ilmateenistus**,
  **PAGASA** (undocumented website endpoints); **Météo-France** (client-minted JWT from a secret
  shipped inside the app — the opposite of the BYOK rule); **BMD** (third-party aggregator, not the
  agency); **AccuWeather** and the rest of the commercial APIs (already covered or out of scope);
  **Xiaomi/Caiyun** (`china`) — reverse-engineered, hard-coded app key and signature, forbidden by
  the project's no-reverse-engineering rule. HKO's keyless JSON alerts are adopted by step 15
  instead, where the daily-payload problem does not exist.
* **`auto` ranks by coverage, not by a hardcoded list.** A fixed list either excludes a national
  backend or wastes requests on it everywhere; registry metadata (country, bbox, global) makes the
  expansion deterministic, testable and visible under `-v`, and keeps `open-meteo` as the universal
  last resort. Step 23 ships the interim fixed list (`open-meteo,met-no,smhi`) and this step
  replaces it; step 23's file is amended in the same commit that lands the ranking.
* **A `grid/` namespace rather than a station namespace.** The NWS point→grid mapping is neither a
  station nor a forecast; giving it its own namespace keeps `cache stat` honest and lets
  `cache clean` treat it like other long-TTL metadata.
* **Network class is metadata, not a filter.** A `--free-only` switch would be a dead flag for most
  users; the field plus documentation is what distributions need, and `AGENTS.md` refuses dead
  flags.
* **Fahrenheit never leaves the provider.** `temperatureUnit` is read per millisecond-field; the
  conversion to °C happens in the decoder, so the single-conversion-point rule (`AGENTS.md` §5)
  holds and the cache stays unit-free.

## Out of scope

Daily-only national sources (needs a model change; see the screening note), radar/satellite imagery,
minutely nowcasts, provincial/regional mesonets, and any provider requiring a bundled credential.

## Verification

```sh
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && reuse lint

cargo run -q -- -v -p nws --lat 39.7456 --lon -97.0892 -f one-line      # real US data, -v shows the grid mapping
cargo run -q -- -p nws --lat 48.85 --lon 2.35 -f plain                  # out of coverage: exit 3, point named
cargo run -q -- -v --lat 39.7456 --lon -97.0892 -f plain                # auto: nws, open-meteo, … (US point)
cargo run -q -- -v --lat 52.52 --lon 13.41 -f plain                     # auto: brightsky first when reachable
cargo run -q -- provider list | grep -E 'nws|brightsky'                 # KEY=none, NET=free, MODE rows
cargo run -q -- provider info nws                                       # coverage, network, credit, verified date
cargo run -q -- --offline --lat 39.7456 --lon -97.0892 -p nws           # cached grid mapping + weather replay
```

## Exit criteria

- ✅ `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`, `reuse lint` clean.
- ✅ NWS renders real hourly data for a US point without a key; a non-US point exits 3 naming it, and
      the chain falls through to `open-meteo`.
- ✅ Bright Sky's response shape, horizon and terms are recorded (live or self-hosted) before its
      mapping table is frozen; the deviation, if any, is in the Progress log.
- ✅ `auto` expansions for US/DE/SE/NO/other and for a country-less coordinate match the coverage
      table exactly and are pinned by tests; `-v` prints them.
- ✅ `provider list` shows `NET` for every row and `provider info` prints coverage/network; the
      README matrix and `docs/providers.md` carry the same values.
- ✅ `cache stat` reports the `grid/` namespace and `cache clean` deletes it.

## Risks

* Bright Sky's public instance may remain unreachable or change its terms; the self-hosting path and
  the recorded-probe requirement are the mitigation, and dropping the provider is a recorded
  deviation rather than a guessed implementation.
* NWS is US-only and asks for an identifying `User-Agent`; the alert adapter already fixed the
  string, and `403` handling keeps the chain alive for everyone else.
* Coverage metadata can drift (NWS territories, bbox edges); the `verified` date per row and the
  `provider_auto` tests make a change deliberate.
* The `grid/` namespace grows `cache stat`'s report and needs docs; the contract paragraph is
  updated in the same commit.

## Progress log

- 2026-10-03 — step opened after the breezy-weather audit; `api.weather.gov`, the HKO open-data API
  and `api.brightsky.dev` were probed through the local proxy (`api.weather.gov` and HKO answered
  200 JSON; Bright Sky timed out, hence the probe-first requirement). The coverage-ranking idea,
  the `NetworkClass` metadata and the screening list come from the same audit.
- 2026-10-03 — NWS shape pinned by probe: `points/39.7456,-97.0892` → `TOP 32 81`,
  `America/Chicago`; the returned `forecastHourly` URL answered 200 with **156** hourly periods,
  `temperatureUnit: "F"`, `windSpeed: "5 mph"`, cardinal `windDirection`,
  `probabilityOfPrecipitation`/`relativeHumidity` as `{unitCode, value}` objects — exactly the
  parsing traps listed in the deliverables. `api.brightsky.dev` remained unreachable (45 s
  timeout), so its mapping stays behind the record-before-freeze rule.
- 2026-10-04 — renumbered from 28 to 24 by the plan reorganization; it still replaces step 23's interim `auto` list and amends `23-more-providers.md` in the same commit.

- 2026-10-06 — step executed. `ProviderMeta` gained the machine-readable `covers: Coverage`
  (`countries` + `bbox` + `global`, so a row can be both a national service and worldwide — MET
  Norway is) and `network: NetworkClass` (`free`/`nonfree`); `ProviderId::all()` is 14 rows.
  `auto` is now `auto_chain(Option<&Location>)` + `select_for(spec, loc)`: tier 0 = a country the
  backend serves, tier 1 = a containing bounding box, tier 2 = global, registry order inside each
  tier; the CLI re-ranks **per slot**, so a multi-location run gets the right chain for each place,
  and `-v` prints it (`provider: auto for Berlin (52.52, 13.41, DE): brightsky, open-meteo,
  met-no`). Station-only, archive-only and supplementary rows never enter. `CacheKey::grid` and the
  `grid/` namespace (30-day TTL) hold the NWS coordinate → grid mapping, and `cache stat`/`clean`
  report and remove it; `provider list` gained the `NET` column and `provider info` the
  `network:`/`history days:`/`marine:` rows. `docs/providers.md` defines the network class.
- 2026-10-06 — `nws` landed: two-step fetch with the point → grid mapping cached for 30 days,
  `temperatureUnit` read per period, `windSpeed` ranges collapsed to their upper bound with one `-v`
  note, cardinal `windDirection` through the shared compass table (which gained its inverse,
  `model::units::compass_degrees`, instead of a second table), `{unitCode, value}` objects with
  nullable values, icon/text → WMO tables, `max_days: 7`, an out-of-coverage `404` rewritten to
  `Error::Upstream` naming the point, and `current: false` because the payload carries no pressure
  (the draft row implied a current block; the code is honest about the payload instead).
  Measurements: `points/39.7456,-97.0892` → `TOP 32 81`, the hourly series is 156 periods covering
  six whole local days, the daily block is 7 day/night pairs.
- 2026-10-06 — `brightsky` landed: `weather?lat=&lon=&date=&last_date=&tz=UTC` with the measured
  fact that `last_date` is an **inclusive timestamp**, not a date — the plan's `today + days - 1`
  served exactly one day short (1/25/49 rows for `last_date` = date/+1/+2), so the request sends
  `today + days`; the horizon measured 10 whole days (`last_record` clamps to today + 10 at Berlin,
  Munich and Bergen alike) and `max_days: 10`. The API's default unit group is `dwd` (°C, km/h,
  hPa, mm) and **not** `si` (Kelvins, m/s, Pa), which is why the request leaves the units alone;
  `precipitation: null` drops the row instead of inventing 0 mm, and the `sources[]` station block
  travels in the `-v` summary. The public instance answered (`200`, ~0.9 s), so the self-host
  deviation path (`[providers.brightsky] url`) was not needed. The draft row's `alerts: true` was
  dropped: the `/weather` payload carries no warnings and no alert source is bound to `brightsky`.
  `covers: countries(["DE"])`, `network: Free`.
- 2026-10-06 — deviations recorded against the plan's letter: (1) `met-no` ranks at tier 0 for
  Norway (`national_and_global(["NO"])`) rather than only at the global tier, so a Norwegian point
  starts at its national service as the goal's example says; SMHI's box also reaches southern
  Norway, so Oslo expands to `met-no, smhi, open-meteo`. (2) `nws` and `brightsky` declare
  `location_kinds.city` as well as `lat_lon` — a resolved place supplies coordinates, and a
  LatLon-only row can never enter a city-capable `auto` chain. (3) The `grid/` namespace joins the
  five entry namespaces (the contract paragraph in `docs/plans/README.md` was updated in the same
  commit).
- 2026-10-06 — live verification: `-p nws --lat 39.7456 --lon -97.0892` answers real data
  (`+28 °C`, `16 km/h SW`, `46%` humidity) with the NOAA credit; a Paris point exits 3 with
  `no NWS forecast grid for point 48.8500,2.3500; the point is outside the US and its territories`;
  `-p auto Norman` prints `nws, open-meteo, met-no` and takes NWS; `-p auto Berlin` prints
  `brightsky, open-meteo, met-no` and takes Bright Sky (`+14 °C`, `10 km/h SW`); `provider list`
  shows 14 rows with `NET` (`free` for the keyless entries, `nonfree` for the commercial BYOK ones).
