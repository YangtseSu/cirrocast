<!--
SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Upstream providers — verified reference

Every external service `cirrocast` talks to, with what it accepts, what it returns, whether it needs a
key, what its free tier allows, and what its licence obliges us to do. This is the written half of the
provider registry: `src/provider/mod.rs` carries the machine-readable subset (`provider list` /
`provider info` print it), and this document carries the detail a registry row cannot hold — request
parameter names, response fields we consume, the exact quota wording, the traps, and the sources.

The file is the one `docs/plans/28-docs-and-guides.md` promises; it was authored in step 10 because the
registry re-verification of that step needs a written record of what was checked.

## How to read this document

* **Everything is dated.** A claim is true for the day it was fetched. `provider info` prints the same
  `verified` date the registry row carries; when a provider changes its terms or limits, the date moves.
* **Quotas carry a quote and a URL.** Free-tier numbers are copied verbatim from the provider's own
  pages. Where a provider publishes contradictory numbers, both are recorded and the contradiction is
  named instead of resolved by preference — `docs/plans/10-additional-providers.md` says which value
  the code uses.
* **Plan-dependent numbers are marked as such.** QWeather's and WWO's quotas depend on the account;
  PirateWeather's paid tiers are not public. Those are marked "plan-dependent".
* **`[INFERENCE]` marks a conclusion we drew** rather than a sentence a provider published.
* **Unverified items are listed, not omitted.** A number nobody could confirm does not silently become
  a number.
* **Client notes** at the end of each section state what our implementation must do about the facts
  above; they are the seed of the corresponding `src/provider/<id>.rs`.
* **Config keys and location rules live elsewhere.** This document names a config key only where a
  provider's behaviour depends on it (`providers.qweather.host`, `providers.metar.station`,
  `defaults.provider`); [`docs/configuration.md`](configuration.md) owns every key, its type and its
  default, and [`docs/location.md`](location.md) owns the user-facing location syntax and the
  service ranking. The service endpoints and their terms below are the provider half of the latter.
* **Network class** (step 24) is a separate axis from the licence: `free` means the backend is
  keyless or needs a key the user can self-host, its endpoint is documented and public, and no
  proprietary service sits in the request path (a self-hosted Bright Sky instance reading DWD open
  data is `free`); `nonfree` means a commercial service reached with the user's own key
  (OpenWeatherMap, WeatherAPI, WWO, PirateWeather, Visual Crossing, QWeather) or a portal-issued
  token for a commercial redistribution service (MeteoAlarm). It is metadata, not a filter:
  `provider list` prints it in the `NET` column and `provider info` prints a `network:` row, so a
  distribution can document what a default install talks to without a `--free-only` switch that
  would be a dead flag for most users.

## At a glance

### Backends

| id | Key | Free tier (verified 2026-09-30; `metar` and `qweather` re-verified 2026-10-01; the four step-23 backends and the two step-24 backends re-verified 2026-10-06) | Coverage | Granularity | Horizon | Status |
|---|---|---|---|---|---|---|
| `open-meteo` | none | 10 000 calls/day, 5 000/hour, 600/minute; non-commercial | global | hourly | 16 days | implemented (step 06) |
| `met-no` | none | 20 requests/second per application; a descriptive User-Agent with contact information is mandatory (`403` otherwise) | global | hourly for the first ~52 h, 6-hourly beyond | 9 days | implemented (step 23) |
| `open-meteo-archive` | none | shared with Open-Meteo (10 000 calls/day, non-commercial) | global | hourly and daily reanalysis | 1940-01-01 onward (historical only) | implemented (step 23) |
| `open-meteo-marine` | none | shared with Open-Meteo (10 000 calls/day, non-commercial) | coastal waters of the global wave models | hourly; `current` plus a daily wave summary requested | 8 forecast days | implemented (step 23) |
| `brightsky` | none | no key and no documented quota on the public instance; a self-hosted instance qualifies | Germany (DWD station network; DWD forecasts elsewhere at low density) | hourly | 10 days (measured) | implemented (step 24) |
| `nws` | none | no key; rate limits unpublished; requests identified by a descriptive User-Agent with contact information | the US and its territories (US, PR, VI, GU, AS, MP) | hourly (156 periods) plus 7 day/night periods | 6 whole local days hourly; 7 days daily (`max_days: 7`) | implemented (step 24) |
| `smhi` | none | no published quota; fair-use rules | Nordics and adjacent seas (SNOW1gv1 polygon) | 1 h near-term, 6 h / 12 h later | ≈10 days | implemented |
| `metar` | none | 100 requests/minute | worldwide stations | per observation (≈hourly) | observations only | implemented (step 11) |
| `visualcrossing` | `CIRROCAST_VISUALCROSSING_KEY` | 1 000 records/day on the free plan (`queryCost` ≈ 1 + 24×hours + days) | global | hourly | 15 days | implemented (step 23) |
| `openweathermap` | `CIRROCAST_OPENWEATHERMAP_KEY` | 60 calls/minute, 1 000 000 calls/month | global | 3-hourly | 5 days (40 slots) | implemented |
| `weatherapi` | `CIRROCAST_WEATHERAPI_KEY` | 100 000 calls/month; 3-day forecast (paid: 14) | global | hourly | 3 days free | implemented |
| `worldweatheronline` | `CIRROCAST_WORLDWEATHERONLINE_KEY` | 100 requests/day (free terms; a second page says 500/month) | global | 3-hourly (`tp=3`) | 5 days per FAQ, 14 per endpoint | implemented |
| `pirateweather` | `CIRROCAST_PIRATEWEATHER_KEY` | 10 000 calls/month (≈$2/month → 20 000) | global | hourly + 7 daily | 48 h hourly (`extend` 168 h), 7 days daily | implemented |
| `qweather` | `CIRROCAST_QWEATHER_KEY` | first 50 000 requests/month at ¥0; QPM 3 000 | global | hourly (up to 240 h) | 10 days (v1) | implemented |

`--provider auto` expands by coverage rather than from a fixed list: entries whose `covers` names
the location's `country_code` come first, then entries whose bounding box contains the point, then
every global keyless forecast entry — each tier in registry order. A US point starts at `nws`
(`nws, open-meteo, met-no`), a German point at `brightsky`, a Swedish point at `smhi` (its bounding
box) and a Norwegian point at `met-no` (national) with `smhi` behind it; a point with no country code
falls back to the global tier alone (`open-meteo, met-no`); `-v` prints the chain it chose.

The built-in default is **not** `auto`: it is the single keyless backend `open-meteo`, the chain
`defaults.provider` names ([`docs/configuration.md`](configuration.md)), so a bare run talks to one
provider and `auto` is opt-in. The `auto` expansion never selects `metar` — it is observation-only
(`max_days: 0`) and serves a station, not a forecast — and neither does it select
`open-meteo-archive` (history only) or `open-meteo-marine` (supplementary, `--marine`, never a chain
entry). A station run is the exception: `--station <ICAO>` against the configured chain (or `auto`)
prepends `metar`, so the configured fallbacks still apply. `tests/provider_auto.rs` pins each tier.

### Obligations that reach the rendered output

| id | Licence | Credit required | Cache ceiling | Extra duty |
|---|---|---|---|---|
| `open-meteo` | CC BY 4.0 | `<a href="https://open-meteo.com/">Weather data by Open-Meteo.com</a>`; a link next to displayed data | none published | geocoding adds "Location data based on GeoNames" |
| `met-no` | CC BY 4.0 | `Data from MET Norway (CC BY 4.0) — https://www.met.no/` | re-request only after the response's `Expires`, with `If-Modified-Since` and `304` | no `Yr` in the product name or UI; a descriptive User-Agent with contact information is mandatory (`403` otherwise) |
| `open-meteo-archive` | CC BY 4.0 (ERA5/Copernicus reanalysis) | `Open-Meteo.com (CC BY 4.0, ERA5/Copernicus) — https://open-meteo.com/` | none published | name the ERA5/Copernicus reanalysis, not only Open-Meteo (in the credit line) |
| `open-meteo-marine` | CC BY 4.0 (Copernicus Marine Service, DWD ICON Wave) | `Marine data: Open-Meteo.com (CC BY 4.0) — https://open-meteo.com/ (Copernicus Marine Service, DWD ICON Wave)` | none published | attribute the Copernicus Marine Service and DWD ICON Wave products (in the credit line) |
| `brightsky` | CC BY 4.0 (DWD open data) | `Bright Sky (DWD open data, CC BY 4.0) — https://brightsky.dev/` | none published | the DWD's own Terms of Use apply to the data; no published quota |
| `nws` | US Government work (public domain) | no mandated string; works "consisting predominantly of the material appearing in NWS Web pages" must carry the 17 U.S.C. § 403 notice | none published | NWS name/logo are trademarks; cached warnings are for the local user only, not for redistribution |
| `smhi` | CC BY 4.0 SE | name SMHI as the source and state modifications; `Källa: SMHI` is the conventional rendering, not SMHI's own wording | none published; caching encouraged | — |
| `metar` | US Government work (public domain) | no mandated string; NWS asks that derived works not claim NWS endorsement and that predominantly-NWS works carry the 17 U.S.C. § 403 notice | none published | NWS name/logo are trademarks |
| `visualcrossing` | proprietary (Visual Crossing per-account terms) | `Visual Crossing Weather — https://www.visualcrossing.com/` | per-account terms: local display only, no redistribution of cached data | the key is BYOK and the account's terms bind the user; alerts carry the payload's own `alerts[]` |
| `openweathermap` | ODbL 1.0 | "Weather data provided by OpenWeather" + link to https://openweathermap.org/ + the OpenWeather logo, visible where the data appears | none published (10-minute model refresh) | share-alike only if we ever publish an adapted database |
| `weatherapi` | proprietary (Zoomash Ltd) | free keys: credit WeatherAPI.com by name or logo; the docs suggest `Powered by <a href="https://www.weatherapi.com/">WeatherAPI.com</a>` | current 60 min, forecast 24 h | mandatory end-user disclaimer; no resale; one key per app |
| `worldweatheronline` | proprietary (Zoomash Ltd) | free keys: "Weather Data by WorldWeatherOnline.com" | current 60 min, forecast 24 h | mandatory end-user disclaimer; no resale |
| `pirateweather` | proprietary (PirateWeatherAPI) | none documented | `Cache-Control: max-age=900` is sent | no multi-account quota circumvention; warranty disclaimer |
| `qweather` | proprietary (QWeather Developers License) | name QWeather + https://www.qweather.com; recommended "Weather service by QWeather" | real-time 10–30 min, hourly 30–60 min, daily 1–6 h (guidance) | GeoAPI data must not be bulk-cached or indexed; weather warnings and air quality must reproduce the response's `metadata.attributions` in full (rendered — see `### qweather`) |

Where a credit lands is part of the output contract, not a detail of each renderer: `plain` and
`json` carry it in the document, `art-table` in a footer, and both `one-line` and the `status` probe
on stderr (one line has no room for it). The rules, and the breaking-change policy for everything a
consumer parses, are in [`docs/ecosystem.md`](ecosystem.md).

### Status-code behaviour (the chain contract, applied per upstream)

`fetch_chain` falls through to the next backend only for transport/upstream failures; everything else
is the user's answer. Concretely, for the backends above:

| Upstream answer | Mapped to | Effect |
|---|---|---|
| `401` that means "bad or missing credential" | `Error::InvalidKey` (exit 6) | chain stops; message names `cirrocast key set <id>` |
| `403` (quota, plan, permission or host mismatch) | `Error::Upstream` (exit 3) | chain continues; the body text is the actionable part, and a backend may refine it in its own decoder (WeatherAPI's `403` stays `Upstream`; only `401` becomes exit 6) |
| `429` | `Error::Upstream` (exit 3) | chain continues; `Retry-After` honoured when present and clamped (PirateWeather, ipwho.is publish it; WeatherAPI, ipapi.co and Open-Meteo do not) |
| `5xx`, timeouts, connection failures | `Error::Upstream` / `Error::Network` (exit 3) | chain continues |
| `400` (bad parameter or rejected location) | `Error::Upstream` (exit 3) | chain continues; no provider maps it to `Error::Usage` — the answer is a provider refusal, not a command-line mistake |
| `404` "no such location" / out-of-area | `Error::Upstream` (exit 3) | `auto` falls through to the next backend; SMHI answers `404` (the docs claim `400`), QWeather answers `400` |
| `200` with an error envelope (`{"error":true,…}`, `{"success":false,…}`, `{"data":{"error":[…]}}`) | `Error::Upstream(reason)` | ipwho.is, ipapi.co and WorldWeatherOnline do this; Open-Meteo sends its envelope with HTTP `400`, and no Open-Meteo struct carries an error field |

## Backends

### `open-meteo`

Keyless default. One request per fetch; hourly data is aggregated into the four day parts in the
location's time zone by `src/provider/open_meteo.rs`.

**Endpoints** (verified 2026-09-30)

| Purpose | Method | URL | Parameters we send |
|---|---|---|---|
| Forecast | GET | `https://api.open-meteo.com/v1/forecast` | `latitude`, `longitude`, `current`, `hourly`, `daily`, `timezone=auto`, `forecast_days`, `temperature_unit=celsius`, `wind_speed_unit=kmh`, `precipitation_unit=mm` |

Documented parameters not sent today: `past_days` (0–92), `models` (plural — the singular `model=` is
**silently ignored**, verified: `model=ecmwf_ifs025` returned the default grid while `models=bogus`
returned HTTP 400), `cell_selection`, `minutely_15`, `start_date`/`end_date`, `format` (`json`/`csv`/`xlsx`),
`apikey` (commercial only, with the `customer-api.open-meteo.com` host).

**Response fields consumed.** `ForecastResponse` reads `latitude`, `longitude`, `elevation`,
`utc_offset_seconds`, `timezone`, `timezone_abbreviation`, `current`, `hourly` and `daily`. `current`
(`CurrentBlock`) reads `time`, `temperature_2m`, `relative_humidity_2m`, `apparent_temperature`,
`is_day`, `precipitation`, `weather_code`, `cloud_cover`, `pressure_msl`, `surface_pressure` (kept,
not rendered), `wind_speed_10m`, `wind_direction_10m`, `wind_gusts_10m`, `visibility` (metres) and
`uv_index`. `hourly` and `daily` are column objects (`{ "time": [...], "<variable>": [...] }`):
`HourlyBlock` reads `time`, `temperature_2m`, `apparent_temperature`, `precipitation_probability`,
`precipitation`, `weather_code`, `wind_speed_10m`, `wind_direction_10m`, `relative_humidity_2m` and
`visibility`; `DailyBlock` reads `time`, `weather_code` (unused), `temperature_2m_max`,
`temperature_2m_min`, `sunrise` and `sunset`. The `*_units` objects and `current.interval` are not
deserialised — the request fixes the units.

**Auth.** None. `apikey` is documented as "Only required to commercial use to access reserved API
resources for customers. The server URL requires the prefix `customer-`."
(<https://open-meteo.com/en/docs>)

**Limits.** "Less than 10'000 API calls per day, 5'000 per hour and 600 per minute."
(<https://open-meteo.com/en/terms>). The pricing table also prints `300.000 calls / month`, but the
same page states "A usage statistics portal is under development. Until it is available, monthly limits
are not enforced." — treat 300 k as advisory. Call accounting is fractional: a request covering more
than 10 variables or more than two weeks "is considered multiple API calls". Free tier is
non-commercial ("You may only use the free API services for non-commercial purposes."), and
Open-Meteo "reserve[s] the right to block applications and IP addresses that misuse our service
without prior notice." (<https://open-meteo.com/en/pricing>, <https://open-meteo.com/en/terms>)

**Coverage and granularity.** Global, 17 model families behind `best_match`. Hourly is the native
output; daily is "a simple 24 hour aggregation from hourly values"; the horizon is 16 days
(`forecast_days`, default 7) and 15-minutely data exists for some models/regions. Returned coordinates
are the model grid cell and "might be a few kilometres away from the requested coordinate".

**Attribution and licence.** CC BY 4.0; the licence page asks for a link next to any displayed data:
`<a href="https://open-meteo.com/">Weather data by Open-Meteo.com</a>`
(<https://open-meteo.com/en/licence>). The geocoding page adds "Location data based on GeoNames".
`Data: Open-Meteo.com (CC BY 4.0) — https://open-meteo.com/` is the line the renderers print today:
CC BY 4.0 asks for credit plus a link to the material, so the licence id and the service link are
both part of the shipped string (the `Data:` label itself is a localized catalog string).

**Client notes.** Ask for metric units explicitly (the single conversion point is `src/render/`);
`timezone=auto` because `daily` requires a timezone; treat a missing `visibility`/`precipitation_probability`
hour as `None`, never as zero; the error envelope is `{"error": true, "reason": "…"}` with HTTP 400.

**Unverified.** The € amounts of the commercial plans (the pricing page renders them client-side); an
observed HTTP 429 (would need 600 req/min); whether `apikey` works on `customer-` hosts.

### `smhi`

Keyless, regional. **The endpoint in the original step 10 plan is gone**: `category=pmp3g` returns 404
and was decommissioned 2026-03-31. The live service is **SNOW1gv1** at a different path with a
different response shape (verified 2026-09-30).

**Endpoints** (verified 2026-09-30)

| Purpose | Method | URL | Parameters |
|---|---|---|---|
| Point forecast | GET | `https://opendata-download-metfcst.smhi.se/api/category/snow1g/version/1/geotype/point/lon/<lon>/lat/<lat>/data.json` | path `lon` **before** `lat`; optional `timeseries=<n>` (truncate), `parameters=a,b` |
| Parameter catalogue | GET | `…/api/category/snow1g/version/1/parameter.json` | — |
| Run timestamps | GET | `…/createdtime.json`, `…/times.json` | — |
| Valid-area polygon | GET | `…/geotype/polygon.json` | — |

Old → new path mapping: `pmp3g` → `snow1g`, `version/2` → `version/1`,
`geopoint/lat/<lat>/lon/<lon>` → `geotype/point/lon/<lon>/lat/<lat>`. Docs moved from
`opendata.smhi.se/apidocs/metfcst/` (404) to <https://opendata.smhi.se/metfcst/snow1gv1>.

**Response fields consumed.** Top level: `createdTime`, `referenceTime`, `geometry.coordinates`,
`timeSeries[]`; per step `time` (interval end), `intervalParametersStartTime` (interval start) and
`data` — a **JSON object keyed by parameter name**, not the old array of
`{name, level, levelType, unit, values[]}`. Metadata for each name lives in `parameter.json`.

Parameters a weather client needs (old name → new):

| Old | New `name` | `shortName` | `levelType`/`level` | Unit |
|---|---|---|---|---|
| `t` | `air_temperature` | `2t` | `hl`/2 | `Cel` |
| `ws` | `wind_speed` | `ws` | `hl`/10 | `m/s` |
| `wd` | `wind_from_direction` | `wd` | `hl`/10 | `degree` |
| `wsymb2` | `symbol_code` | `Wsymb2` | `hl`/0 | — |
| `r1` | `precipitation_amount_mean_deterministic` | `avg_tprate` | `sfc`/0 | `kg/m2` |
| `pmean` | `precipitation_amount_mean` | `tpratemean` | `hl`/0 | `kg/m2` |
| `vis` | `visibility_in_air` | `vis` | `hl`/2 | `km` |
| `tcc_mean` | `cloud_area_fraction` | `tcc` | `entireAtmosphere`/0 | **`oktas`** (0–8; ×12.5 = %) |

The other 16 fields in every step include `wind_speed_of_gust`, `relative_humidity`,
`air_pressure_at_mean_sea_level`, `thunderstorm_probability`, `probability_of_frozen_precipitation`,
`low/medium/high_type_cloud_area_fraction`, `cloud_base_altitude`, `cloud_top_altitude`,
`precipitation_amount_min/max/median`, `probability_of_precipitation`, `precipitation_frozen_part`,
`predominant_precipitation_type_at_surface`.

**Auth.** None; no User-Agent or contact requirement is published for this API (unlike the sibling
`metobs` service). Fair use, verbatim: "Lasta inte ner SMHIs tjänster i onödan … Undvik även att hämta
samma data fler gånger." and SMHI reserves the right to block IPs on abuse; scheduled access is
explicitly allowed ("Ja, du kan schemalägga (exempelvis crontab) ett jobb …"), parallel connections
only "om varje anslutning motsvarar en slutanvändare", and the forecast API updates hourly
(<https://www.smhi.se/data/om-smhis-data/villkor-for-anvandning>,
<https://www.smhi.se/data/om-smhis-data/fragor-och-svar>).

**Coverage and granularity.** The valid area is the `geotype/polygon.json` ring: roughly
lon −18.1…44.1, lat 49.9…74.9, irregular (Sweden, adjacent seas, neighbouring countries, a Norwegian
lobe). Out-of-area requests returned **HTTP 404 with an empty body** in every probe, although the docs
claim 400 "FIELD POINT OUT OF BOUNDS" — the client must treat both as "outside coverage" and fall
through in `auto`. Coordinates snap to the nearest ~2.5 km grid point. A run carried **82 steps ≈ 10
days**, with the step widening 1 h → 6 h (from day 3) → 12 h (from day 7); the horizon varies per run
("maximum approximately 10 days").

**`Wsymb2` (1–27).** The canonical table is client-rendered and unreachable by non-JS fetches; the
code→name mapping below is SMHI's public symbol explainer, whose image filenames carry the code. Code 5
is absent from that page. Swedish names, verbatim:
1 Klart, 2 Mest klart, 3 Halvklart, 4 Mycket moln, 6 Mulet, 7 Dimma, 8 Lätt regnskur, 9 Regnskur,
10 Kraftig regnskur, 11 Åskskur, 12 Lätt by av regn och snö, 13 By av regn och snö, 14 Kraftig by av
regn och snö, 15 Lätt snöby, 16 Snöby, 17 Kraftig snöby, 18 Lätt regn, 19 Regn, 20 Kraftigt regn,
21 Regn med åska, 22 Lätt snöblandat regn, 23 Snöblandat regn, 24 Kraftigt snöblandat regn,
25 Lätt snöfall, 26 Snöfall, 27 Ymnigt snöfall. Intensity thresholds are water-equivalent: light
≤ 0.5 mm/h, moderate 0.5–4 mm/h, heavy > 4 mm/h; "Lätt snöfall" corresponds to less than 1 cm of snow
(<https://www.smhi.se/kunskapsbanken/meteorologi/vaderprognoser/vad-betyder-smhis-vadersymboler>).

**Attribution and licence.** "Med våra öppna data följer licensvillkoren Creative commons Erkännande
4.0 SE." — CC BY 4.0 SE, commercial use permitted; the credit requirement is "Du ska ange SMHI som
källa och även ange om du har ändrat i licensmaterialet." The literal string "Källa: SMHI" is not
SMHI's own wording, only the conventional rendering of that requirement
(<https://www.smhi.se/data/om-smhis-data/villkor-for-anvandning>).

**Client notes.** Read `data` by key, never by index or order; the `parameters` filter needs **literal
commas** (`%2C` silently returns only the first parameter) — the backend therefore sends no filter at
all; map `9999` (and `precipitation_frozen_part: -9`) to `None`; precipitation accumulates over the
interval, so divide by the interval width for a rate; `cloud_area_fraction` is oktas; `symbol_code`
arrives as an integer; the grid-snapped `geometry.coordinates` are surfaced in the `-v` raw summary
rather than replacing the requested coordinates.

**Implemented 2026-10-01** (`src/provider/smhi.rs`, `max_days: 10`, credit line `SMHI (CC BY 4.0 SE)`).
Two limitations come from the payload, not from the client, and both are deliberate:

* **The series starts at the current hour**, so the location-local today is complete only when the
  fetch happens before 06:00 local (its night hours are already past). A `DayPart` cannot represent
  "no data", so the backend emits the first days whose four parts are all covered — usually starting
  tomorrow — instead of inventing values for windows SMHI never served. The current-conditions block
  still answers "what is it like now".
* **The payload carries no time zone and no daylight flag.** A location whose zone is still the
  provisional `UTC` (raw coordinates, an untagged OSM place) is refused with a usage error naming the
  fix rather than being aggregated in the wrong zone; `is_day` is derived from the local civil day
  (06:00–18:00) until step 17 computes real sun times. Note the ordering: the request goes out before
  that check, so an out-of-coverage point still fails as `Upstream` and lets a chain fall through.

Out-of-coverage answers (HTTP 404 with an HTML body; the docs claim 400) become
`upstream: out of coverage: 39.90,116.40 is outside the SMHI valid area`.

**Unverified.** The canonical `Wsymb2` table text (client-rendered docs); an English symbol table (none
published); gzip/`Cache-Control`/`ETag` headers (no header inspection available); a published numeric
rate limit (none found); the multipoint grid endpoint (every probe returned 406).

### `metar` (aviationweather.gov)

Keyless, station-based observations (step 11), and the `auto` coverage expansion never selects it
because it serves a station, not a forecast (`max_days: 0`). A station run reaches its station two
ways: `--station <ICAO>` (or the `providers.metar.station` config key) names the identifier
directly, and `@lat,lon` maps to the nearest row of the embedded station table — the chosen station
and its distance are printed under `-v`. The
decoder reads the raw report rather than the JSON fields, the embedded station table answers the
common identifiers without a request, and `stationinfo` extends that to any station at one cached
request per 30 days.

**Endpoints** (verified 2026-10-01, all GET, no key)

| Purpose | URL | Parameters |
|---|---|---|
| Observations | `https://aviationweather.gov/api/data/metar` | `ids` (ICAO, comma-separated; also `@WA` state prefixes), `bbox`, `format` (`raw` default; `json` for us), `taf=true` to embed the station's TAF, `hours` (default 1.5), `date` |
| Forecasts | `https://aviationweather.gov/api/data/taf` | `ids`, `bbox`, `format`, `metar=true`, `time` (`valid`/`issue`), `date` |
| Station metadata | `https://aviationweather.gov/api/data/stationinfo` | `ids`, `bbox`, `format` |

**Response fields consumed.** Two `Deserialize` structs, no more. `MetarReport` (the
`metar?format=json` object) reads `icaoId`, `obsTime` and `rawOb` — the values come from the decoded
raw report, not the JSON fields — plus `temp`, `dewp`, `wdir`, `wspd`, `wgst`, `visib`, `altim` and
`wxString`, each an `Option<serde_json::Value>` used only for the `-vv` cross-check because upstream
types drift (`wdir` is a number or the string `"VRB"`, `visib` a number of statute miles or `"10+"`).
`receiptTime`, `reportTime`, `slp`, `clouds`, `lat`, `lon`, `elev`, `name` and `fltCat` are not
deserialised. `StationInfo` (`stationinfo?format=json`) reads `icaoId`, `site`, `lat`, `lon`, `elev`,
`state` and `country`. The TAF is fetched with `format=raw` and consumed as the raw text; it is not
deserialised, so `issueTime`/`validTimeFrom`/`fcsts[]` are not read.

**Auth.** None; the docs ask for a custom user agent ("Set a custom user agent to prevent automated
filtering inadvertently blocking valid traffic.").

**Limits.** "All requests are rate limited to 100 requests per minute."; "Most endpoints return a
maximum of 400 entries"; the database keeps 30 days; CORS is not permitted; undocumented query
parameters are rejected; bulk consumers are pointed at the `/data/cache/*.gz` files instead of large
queries (<https://aviationweather.gov/data/api/>).

**Coverage and granularity.** Worldwide METAR/TAF/station info; METARs update at most hourly and `hours`
is the only look-back window; TAFs are multi-period (typically 24–30 h).

**Attribution and licence.** US Government work, public domain, with NWS conditions: no claim of
ownership, no implied endorsement, and third parties producing works "consisting predominantly of the
material appearing in NWS Web pages" must carry the 17 U.S.C. § 403 notice; the NWS name and logo are
trademarks (<https://www.weather.gov/disclaimer>).

**Implemented 2026-10-01** (`src/provider/metar.rs`, `max_days: 0` — observation only). Two calls per
fetch at most (the observation, plus `stationinfo` for a station outside the embedded table, plus the
TAF under `-v`), each cached under its own `weather/metar-<ICAO>-{current,taf}.json` /
`station/<ICAO>.json` key. The report prints the registry credit
`aviationweather.gov (NOAA/NWS, public domain)`; no string is mandated, and the payload's own fields
are used only for the `-vv` cross-check because the decoder reads the raw report.

**Client notes.** `metar_id` does not exist (the identifier is `icaoId`); `wmoId`/`id`/`cover`/`rawTaf`
are live but undocumented — parse defensively; `metar?ids=ZZZZ` answers 204 while `taf?ids=ZZZZ`
answers 200 with `[]`; a 504 can be transient where the same request later returns 400; non-ICAO ids
are 400, unknown products 404.

**Unverified.** The 400 body shape (documented as `{"status":"error","error":"…"}` but never captured
live); an observed 429/403; CORS headers.

### `met-no` (MET Norway)

Keyless, global. One `compact` request per fetch; the payload has no time zone and no daily block, so
the four day parts are aggregated in the location's zone and the day's extremes come from its own
samples. **Implemented 2026-10-06 (step 23)** (`src/provider/met_no.rs`, `max_days: 9`).

**Endpoints** (verified 2026-10-06)

| Purpose | Method | URL | Parameters we send |
|---|---|---|---|
| Compact forecast | GET | `https://api.met.no/weatherapi/locationforecast/2.0/compact` | `lat`, `lon` (at most four decimals, truncated), `altitude` only when the location carries an elevation |

**Response fields consumed.** `CompactResponse` reads `properties` only: `meta.updated_at`,
`meta.units.wind_speed` and `timeseries[]`. Each `Entry` reads `time` (RFC 3339, UTC) and `data`:
`instant.details` (`air_pressure_at_sea_level`, `air_temperature`, `cloud_area_fraction`,
`relative_humidity`, `wind_from_direction`, `wind_speed`) plus the forward-looking `next_1_hours`,
`next_6_hours` and `next_12_hours` blocks, of which a period's `summary.symbol_code` and
`details.precipitation_amount` are read. `next_12_hours` is kept only as a last-resort fallback, and a
trailing row with no period at all is dropped rather than zero-filled.

**Auth.** None; a descriptive `User-Agent` carrying contact information is mandatory, and generic
agents (`okhttp`, `Dalvik`, `fhttp`, `Java`) are refused with `403`, which is why the shared client
sends the project's identifying agent.

**Limits.** "20 requests/second per application"; coordinates are served at four decimals; re-requests
must wait for the response's `Expires` and use `If-Modified-Since` (`304`) — the terms require the
conditional handshake, so the cache stores `Expires`/`Last-Modified` per entry and a `304` serves the
stored body instead of transferring it again.

**Coverage and granularity.** Global; 9-day horizon, with hourly rows for roughly the first 52 h and
6-hourly rows beyond (each row is read from the finest `next_*` block it carries). The recorded fixture
carries 52 one-hour rows and 78 six-hour rows spanning ~9.9 days.

**Attribution and licence.** CC BY 4.0 ("Data from MET Norway"); the terms additionally forbid `Yr` in
a product name or UI regardless of credit. The registry row's credit line is
`Data from MET Norway (CC BY 4.0) — https://www.met.no/`.

**Traps the implementation handles.** Coordinates are truncated (not rounded) to four decimals so the
requested point never drifts into a neighbouring cell; the `_day`/`_night`/`_polartwilight` suffix is
stripped before the symbol lookup; `wind_speed` is converted from `m/s` only when
`meta.units.wind_speed` says so; a location with a provisional `UTC` zone aggregates in that zone (the
payload carries no zone to repair it with).

**`symbol_code` (41 base codes).** Strip the suffix and look the base up: `clearsky`→0, `fair`→1,
`partlycloudy`→2, `cloudy`→3, `fog`→45; `lightrainshowers`/`rainshowers`/`heavyrainshowers`→80/81/82
and their `andthunder` forms→95; `lightsleetshowers`/`sleetshowers`→68 and `heavysleetshowers`→69
(with thunder→95); `lightsnowshowers`/`snowshowers`→85 and `heavysnowshowers`→86 (with thunder→95);
`lightrain`/`rain`/`heavyrain`→61/63/65 (with thunder→95); `lightsleet`/`sleet`→68 and `heavysleet`→69
(with thunder→95); `lightsnow`/`snow`/`heavysnow`→71/73/75 (with thunder→95).
`lightssleetshowersandthunder` and `lightssnowshowersandthunder` reproduce MET's own double-`s`
spelling. Sleet has no WMO umbrella of its own, hence WMO 68/69; an unknown base becomes WMO 3 and
names itself under `-v`.

**Unverified.** The `403` body a banned agent receives (the status is documented, the body was not
captured); whether the 20 req/s ceiling is enforced per application or per source address.

### `open-meteo-archive`

Keyless reanalysis. The same service family as `open-meteo` on a different host and pointing
backwards: one request per fetch over an explicit `start_date`/`end_date` window, decoded by the
shared `open_meteo::report_for`. **Implemented 2026-10-06 (step 23)**
(`src/provider/open_meteo_archive.rs`, `max_days: 0`, `history_days: 30000`).

**Endpoints** (verified 2026-10-06)

| Purpose | Method | URL | Parameters we send |
|---|---|---|---|
| Reanalysis | GET | `https://archive-api.open-meteo.com/v1/archive` | `latitude`, `longitude`, `start_date`, `end_date`, `hourly` (the forecast list without `precipitation_probability`), `daily`, `timezone=auto`, `temperature_unit=celsius`, `wind_speed_unit=kmh`, `precipitation_unit=mm` |

**Response fields consumed.** The shared Open-Meteo `ForecastResponse`/`HourlyBlock`/`DailyBlock`
structs — only the id, URL and cache key differ. There is **no `current` block**: the endpoint answers
none, so the report's current conditions are `None`. `visibility` is still requested because the shared
hourly decoder requires the array to exist, and upstream's `null`s become `None`;
`precipitation_probability` is omitted because the reanalysis has no such variable. The daily block is
`weather_code,temperature_2m_max,temperature_2m_min,sunrise,sunset`.

**Auth.** None.

**Limits.** The Open-Meteo free tier (`< 10 000 calls/day`, non-commercial), shared with the forecast
API.

**Coverage and granularity.** Global ERA5 / ERA5-Land / IFS reanalysis from **1940-01-01** onward;
hourly, with daily aggregates. `history_days: 30000` is the registry's marker for "the archive is the
product"; `max_days: 0` says there is no forecast horizon.

**Traps the implementation handles.** The window is mandatory (`--date <YYYY-MM-DD>` or
`--history <N>d`) and `--days` on this entry is a usage error (exit 2), because `max_days: 0` means
"no forecast to shorten"; a window starting before 1940 is refused. **ERA5 lags the present by about
five days**, so a window whose end falls inside `today − 5` is delegated to the forecast host through
`open_meteo::fetch_window` (the seam the two backends share, keyed by the window's end); a window
reaching today is refused rather than keyed like a forecast, which would collide in the cache.

**Attribution and licence.** CC BY 4.0 over the Copernicus/ERA5 reanalysis. Registry credit line:
`Open-Meteo.com (CC BY 4.0, ERA5/Copernicus) — https://open-meteo.com/`.

**Unverified.** The exact latency boundary (the ~5-day figure is ERA5's published assimilation lag,
not a constant the API documents); whether an out-of-range date is a `400` or a `200` with an error
envelope.

### `open-meteo-marine`

Keyless, supplementary. Never a forecast chain entry: the registry row carries `marine: true`,
`select` refuses `--provider open-meteo-marine` as a usage error (exit 2), and the panel is requested
with `--marine` beside the weather answer. **Implemented 2026-10-06 (step 23)**
(`src/provider/open_meteo_marine.rs`, `max_days: 8`).

**Endpoints** (verified 2026-10-06)

| Purpose | Method | URL | Parameters we send |
|---|---|---|---|
| Marine | GET | `https://marine-api.open-meteo.com/v1/marine` | `latitude`, `longitude`, `current`, `daily`, `cell_selection=sea`, `timezone=auto` |

`current` is `wave_height,wave_direction,wave_period,swell_wave_height,sea_surface_temperature`;
`daily` is `wave_height_max,wave_period_max,wave_direction_dominant`. No `hourly` and no
`forecast_days` are sent — the panel is current conditions plus the daily wave summary, so the API's
default span is what it should be.

**Response fields consumed.** `MarineResponse` reads `latitude`, `longitude`, `timezone`, `current`
and `daily`. `current` reads `time` (local wall clock), `wave_height`, `wave_direction`, `wave_period`,
`swell_wave_height` and `sea_surface_temperature`; `daily` reads the parallel
`time`/`wave_height_max`/`wave_period_max`/`wave_direction_dominant` arrays, a short array leaving a
value absent instead of panicking.

**Auth.** None; the Open-Meteo free tier applies.

**Coverage and granularity.** The nearest **sea** cell of the global wave models
(`cell_selection=sea`), 8 forecast days. The response's own coordinates are the sampled cell, so a
land point gets the nearest open water, which can be hundreds of kilometres away.

**Traps the implementation handles.** A `current` block is **required**: a response without one, or
with every reading `null`, is an upstream error naming the place rather than an all-`None` panel. The
great-circle distance between the requested point and the sampled cell travels with the reading, and a
`-v` note names a cell more than **25 km** away (`model::FAR_CELL_KM`). `timezone=auto` makes the API
answer in the location's own zone and echo its name; `current.time` is resolved in that zone, not in a
provisional `UTC`.

**Attribution and licence.** CC BY 4.0 with the Copernicus Marine Service and DWD ICON Wave products
behind it. Registry credit line:
`Marine data: Open-Meteo.com (CC BY 4.0) — https://open-meteo.com/ (Copernicus Marine Service, DWD ICON Wave)`.

**Unverified.** Which wave model `best_match` selects for a given cell; whether the default span
without `forecast_days` really reaches the registry's 8 days (the step-23 probe confirmed
`forecast_days=8` is accepted and returns 192 hourly slots, but the request sends no such parameter).

### `nws` (api.weather.gov)

Keyless, the US and its territories. Fetching is a **two-step** walk, like a station lookup: the
coordinate resolves to an office and a grid cell first, and only then do that cell's hourly and daily
resources answer. `auto` ranks it first for a US point, and a coordinate outside the coverage falls
through with a reason the user can read.

**Endpoints** (verified 2026-10-06, all GET, no key, `Accept: application/geo+json`)

| Purpose | Method | URL | Parameters we send |
|---|---|---|---|
| Point → grid mapping | GET | `https://api.weather.gov/points/{lat:.4},{lon:.4}` | path only; the `Accept` media type |
| Hourly forecast | GET | `https://api.weather.gov/gridpoints/{gridId}/{x},{y}/forecast/hourly` | path only; the `Accept` media type |
| Daily (day/night) forecast | GET | `https://api.weather.gov/gridpoints/{gridId}/{x},{y}/forecast` | path only; the `Accept` media type |

The mapping is cached in its own **`grid/`** namespace (`grid/nws-<lat.3dp>-<lon.3dp>.json`, TTL **30
days**), because grids move far more slowly than a forecast; the two forecast resources live under the
configured weather TTL. A repeated run therefore pays for the two forecast requests, not the mapping
again.

**Response fields consumed.** `PointsResponse` reads `properties.{gridId, gridX, gridY, timeZone,
relativeLocation.properties.{city, state}}` — the zone repairs a provisional `UTC` before the day
parts are anchored, and the grid and place are what `-v` names. `HourlyResponse` and `DailyResponse`
each read `properties.periods[]`; one shared `Period` reads `startTime`, `endTime`, `temperature`,
`temperatureUnit`, `probabilityOfPrecipitation`, `relativeHumidity` (each a `{unitCode, value}` object
with a nullable `value`), `windSpeed`, `windDirection`, `shortForecast` and `icon`. The daily periods
repeat every field but `relativeHumidity`, so the fields the hourly series alone carries are optional
here.

**Auth.** None; a descriptive `User-Agent` carrying contact information is mandatory (step 15 fixed
the string for the alert adapter, and this backend reuses the same constant). `403`/`429` stay
`Error::Upstream`/`Network` so a chain continues.

**Limits.** No key and **no published rate limit**; the registry row records the policy the client
applies: "no key; rate limits unpublished; requests identified by User-Agent; the point → grid mapping
is cached 30 days".

**Coverage and granularity.** The US and its territories — `covers: countries(["US", "PR", "VI",
"GU", "AS", "MP"])`. The recorded `forecast/hourly` series carries **156 periods** covering six whole
local days plus the trailing partial one, and the daily resource carries **14 periods = 7 day/night
pairs**; `max_days: 7` is that measured daily horizon. The four canonical day parts are aggregated in
the location's zone, from NWS's own local periods.

**Attribution and licence.** `api.weather.gov` serves US government work in the public domain, under
the same NWS conditions the `metar` section records: no claim of ownership, no implied endorsement, and
works "consisting predominantly of the material appearing in NWS Web pages" must carry the 17 U.S.C.
§ 403 notice; the NWS name and logo are trademarks. No string is mandated, so the registry credit line
is `api.weather.gov (NOAA/NWS, public domain)`.

**Implemented 2026-10-06 (step 24)** (`src/provider/nws.rs`, `max_days: 7`, `current: false`), with the
traps the decoder handles:

* **`temperatureUnit` is read per period**, never assumed: a `F` value is converted to °C at decode
  time, so the model and the cache stay metric; any other spelling (`C`) is already canonical.
* **`windSpeed` is a string and can be a range** (`"10 mph"`, `"5 to 10 mph"`): the **upper** bound is
  kept and one `-v` note names the range it collapsed; a string with no number (calm) is `0`.
* **`windDirection` is a cardinal** (`SSW`), mapped back to the sector centre through the shared
  `model::units::compass_degrees` table — the same inverse `metar` uses.
* **The percentage fields are objects**: `probabilityOfPrecipitation` and `relativeHumidity` are
  `{unitCode, value}` with a nullable `value`; a `null` stays absent, never `0`.
* **The icon URL is consulted before the text.** The `land`/`marine` and `day`/`night` path segments
  are stripped, the most severe of the icon's condition codes wins, and `shortForecast` is the
  fallback. The icon table carries every code of the vendor's published list (35 entries — the 34 of
  `/icons` plus the docs' `freezing_rain` alias for `fzra`) and the text table 28 substrings, matched
  longest-first (`"rain and snow"` before `"rain"`, `"snow showers"` before `"snow"`, `"blowing
  dust"` before `"dust"`) so composition wins; an unknown pair becomes WMO 3 (overcast) plus one
  `-v` line. The obscurations keep the model's own codes (`haze`→5, `smoke`→4, `dust`→6,
  `blowing dust`→7) rather than collapsing into `fog`; the list's one combined icon, `fog`
  (`Fog/mist`), takes the more significant of the two.
* **The daily high/low is anchored on the period's `endTime`**, not its start: NWS's `Tonight` period
  runs into the next day, and the canonical day's `Night` part is those small hours, so the low
  belongs to the date the period ends on. A date the daily block does not reach falls back to the
  hourly samples' own extremes rather than dropping the day.
* **`properties.timeZone` repairs a provisional zone** (a coordinate or OSM place starts as `UTC`)
  before the day parts are anchored.
* **The forecast payload carries no pressure**, so the canonical `Current` block cannot be filled
  honestly and the row declares **`current: false`**; the renderers omit the block rather than invent
  one.
* **An out-of-coverage point answers `404`**, kept as `Error::Upstream` (exit 3) with a message naming
  the point (`… is outside the NWS coverage`), so an `auto` chain falls through readably.
* **Alerts are step 15's**, not this provider's: the `alerts: true` registry row describes the NWS
  warnings reached through the alert registry, and this backend fills `Report.alerts` with an empty
  vector.

**Unverified.** The published rate limit (none found); the `403`/`429` bodies; whether the 156-period
series length varies by office.

### `brightsky` (Bright Sky)

Keyless, Germany. One `/weather` request per fetch; the hourly rows become the four canonical day parts
in the location's zone, and the `sources[]` block names the DWD station behind them.

**Endpoints** (verified 2026-10-06)

| Purpose | Method | URL | Parameters we send |
|---|---|---|---|
| Hourly forecast | GET | `https://api.brightsky.dev/weather` | `lat`, `lon` (four decimals), `date` (location-local today), `last_date`, `tz=UTC` |

`units` is **not** sent: the API's default group is `dwd` (°C, km/h, hPa, mm), which is already the
canonical set, whereas `units=si` would return Kelvins, m/s and Pascals for the provider to convert
back; the field names are the same in either mode. `date`/`last_date`/`tz=UTC` make the request span
UTC and the rows are re-anchored in the location's zone at decode time.

**Response fields consumed.** `WeatherResponse` reads `weather[]` and `sources[]`. `Record` reads
`timestamp`, `temperature`, `wind_speed`, `wind_direction`, `wind_gust_speed`, `relative_humidity`,
`precipitation`, `condition`, `icon`, `visibility`, `cloud_cover` and `pressure_msl` — every
meteorological value optional, because upstream reports `null` for a reading it has no model for.
`Source` reads `id`, `dwd_station_id`, `wmo_station_id`, `station_name`, `observation_type`,
`distance` and `last_record` for the `-v` block. Unknown fields are ignored on purpose (Bright Sky adds
parameters to the payload without notice).

**Auth.** None; no key. The public instance is free to use, and the project is open source.

**Limits.** No key and **no documented quota** on the public instance; the registry row records that
"the DWD's Terms of Use apply to the data". **The horizon is a measured 10 whole days**: every
2026-10-06 probe that asked past it clamped to the forecast feed's `sources[].last_record`
(`2026-10-16T04:00Z` at Berlin's `BERLIN-ALEX.`, Munich's `MUENCHEN STADT` and Bergen's `BERGEN`
alike), i.e. `today + 10`, so `max_days: 10` fits under that boundary.

**Coverage and granularity.** Germany — `covers: countries(["DE"])`. The DWD station network answers
hourly; the registry `coverage` string notes that DWD forecasts also reach the rest of the world at low
density.

**Attribution and licence.** DWD open data under **CC BY 4.0**, resold by Bright Sky. The registry
credit line is `Bright Sky (DWD open data, CC BY 4.0) — https://brightsky.dev/`, and the DWD's own
Terms of Use apply to the data.

**Implemented 2026-10-06 (step 24)** (`src/provider/brightsky.rs`, `max_days: 10`), every fact measured
on the day the provider landed:

* **`last_date` is an inclusive *timestamp*, not a date.** `date=2026-10-06&last_date=2026-10-06`
  returned 1 row, `last_date=2026-10-07` 25 rows and `2026-10-08` 49: the answer always ends at
  `last_date` 00:00Z. Asking for `today + days - 1` therefore serves one whole day short, so the
  request uses `last_date = today + days` — the instant at the start of the day *after* the last
  requested one — which yields exactly `days` whole days; the trailing boundary row is the incomplete
  next day and `covered_days` drops it.
* **`units` stays at the API's default `dwd`** (°C, km/h, hPa, mm) rather than `si` (Kelvins, m/s,
  Pa).
* **`precipitation: null` drops the row**, it never becomes 0 mm: the canonical day part requires the
  value, so a hole must not read as a measurement. `relative_humidity`, `visibility`, `cloud_cover`,
  gust and wind direction are optional in the model and keep their own nullness — the Berlin recording
  reports `relative_humidity: null` on every row, which is exactly this case.
* **`condition` and `icon` are two small closed sets** — `CONDITIONS` (7 values) and `ICONS` (12) —
  written out and exhaustiveness-tested. `condition` is the precipitation/obscuration state and wins
  whenever it is not `dry`; for `dry` the sky state comes from `icon`; an unknown value becomes WMO 3
  plus a `-v` line. `wind` is not a sky state and WMO has no wind-only code, so it maps to overcast.
* **The `sources[]` block is station metadata**, carried into `Attribution.raw` so `-v` names the DWD
  station (`#<id> <name> (DWD …, WMO …, <observation_type>, <distance>)`).
* **The current block requires pressure**: `current_of` picks the most recent row whose instant has
  already passed (else the first) and returns `None` when that row lacks temperature, wind,
  precipitation, pressure or a resolvable condition — a zero would be a lie.

**Self-hosting.** Bright Sky is open source over DWD open data, so a self-hosted instance keeps the
`free` network class; the public instance (`https://api.brightsky.dev/weather`) answered `200` for this
recording.

**Unverified.** The forecast feed's exact stop date beyond the 10-day probe; whether the public
instance enforces a rate limit (none published); the `sources[]` shape at a German point far from any
DWD station.

### `visualcrossing`

BYOK. One timeline request per fetch; the payload carries the current conditions, the daily blocks with
their hours, and its own `alerts[]`. **Implemented 2026-10-06 (step 23)**
(`src/provider/visualcrossing.rs`, `max_days: 15`, key `CIRROCAST_VISUALCROSSING_KEY`).

**Endpoints** (verified 2026-10-06, GET, base
`https://weather.visualcrossing.com/VisualCrossingWebServices/rest/services/timeline`)

| Purpose | Method | URL | Parameters we send |
|---|---|---|---|
| Timeline | GET | `…/timeline/<lat>,<lon>/next<days>days` | `unitGroup=metric`, `include=current,days,hours`, `key` (secret) |
| Timeline, current only | GET | `…/timeline/<lat>,<lon>/today` | `unitGroup=metric`, `include=current`, `key` (secret) |
| Timeline, explicit window | GET | `…/timeline/<lat>,<lon>/<date1>/<date2>` | `unitGroup=metric`, `include=current,days,hours`, `key` (secret) |

The third row is the fallback: when the server rejects `next<days>days` with a `400`, the backend
retries once with an explicit `date1`/`date2` range computed from the location's local today (the same
span). Coordinates go out at four decimals.

**Response fields consumed.** `TimelineResponse` reads `resolvedAddress`, `timezone`,
`currentConditions`, `days[]` and `alerts[]`. `CurrentBlock` reads `datetime`, `datetimeEpoch`, `temp`,
`feelslike`, `humidity`, `precip`, `windspeed`, `windgust`, `winddir`, `pressure`, `cloudcover`,
`visibility`, `solarradiation` (the `is_day` signal: zero at night), `uvindex` and `icon`. `DayBlock`
reads `datetime`, `tempmax`, `tempmin`, `sunrise`/`sunriseEpoch`, `sunset`/`sunsetEpoch` and `hours[]`;
`HourBlock` reads `datetime`, `datetimeEpoch`, `temp`, `feelslike`, `precip`, `precipprob`, `windspeed`,
`winddir`, `humidity`, `visibility` and `icon`. `AlertBlock` reads `id`, `event`, `headline`,
`description`, `onset`/`onsetEpoch`, `ends`/`endsEpoch`, `expires`/`expiresEpoch`, `severity`,
`urgency` and `certainty`.

**Auth.** The key travels in the `key` query parameter and is redacted everywhere; the registry row has
`requires_key: true` and `key_env: CIRROCAST_VISUALCROSSING_KEY`.

**Limits.** Free plan **1 000 records/day**, and the payload's own `queryCost` is the accounting:
≈ `1 + 24×hours + days`, so the default 3-day `include=current,days,hours` query costs about **76
records** (the hand-authored fixture carries `"queryCost": 76`). Asking only for the days requested
and nothing longer is what keeps a 15-day hourly query (≈ 360 records) affordable.

**Coverage and granularity.** Global; hourly timeline up to **15 days**. `timezone` names the zone the
timestamps are in and repairs a provisional location zone, exactly as for Open-Meteo.

**Traps the implementation handles.** `datetimeEpoch` is authoritative — the local `datetime` strings
are a bare `HH:MM:SS` for hourly records (the date lives in the parent day) and are only a fallback,
joined to that parent date before being resolved. The fixed `icon` vocabulary maps to WMO:
`clear-day`/`clear-night`→0, `partly-cloudy-day`/`partly-cloudy-night`→2, `cloudy`/`wind`→3, `fog`→45,
`rain`→63, `showers-day`/`showers-night`→81, `snow`→73, `sleet`→68, `freezing-rain`→66, `hail`→96,
`thunderstorm`/`thunder-rain`→95; an icon outside the list becomes WMO 3 and names itself under `-v`.

**Alerts.** The payload's own `alerts[]` becomes `AlertSource::VisualCrossing` and joins the alert
layer; `--alerts-from visualcrossing` needs `--provider visualcrossing` on the chain, because the
warnings travel inside that provider's payload and there is no endpoint to call — naming the source
without the provider is a usage error. An alert whose timestamps cannot be read is skipped with a `-v`
note, never a hard failure; `severity`/`urgency`/`certainty` are parsed through the model's CAP
parsers, so a vendor spelling this build does not know degrades to `Unknown`.

**Attribution and licence.** Per-account Visual Crossing terms — local display only, no redistribution
of cached data, the key is BYOK. Registry credit line:
`Visual Crossing Weather — https://www.visualcrossing.com/`.

**Unverified.** The live response and the live `400` body: no Visual Crossing key exists in this
repository, so `tests/fixtures/visualcrossing/timeline.json` (and the adjacent `error_400.json`) is
**hand-authored from the published timeline schema** rather than recorded from a session. The
published alert object documents only `event`, `headline`, `description`, `onset` and `ends`; the
decoder reads the CAP triple defensively.

### `openweathermap`

BYOK. Two calls per fetch (current + 3-hourly forecast).

**Endpoints** (verified 2026-09-30, GET)

| Purpose | URL | Parameters |
|---|---|---|
| Current | `https://api.openweathermap.org/data/2.5/weather` | `lat`, `lon`, `appid`, `units=metric`, `lang`, `mode` |
| Forecast | `https://api.openweathermap.org/data/2.5/forecast` | `lat`, `lon`, `appid`, `units=metric`, `lang`, `cnt`, `mode` |

`q=`/`id=`/`zip=` lookups still work but are deprecated ("API requests by city name, zip-codes and city
id have been deprecated"); the replacement is the separate Geocoding API. `exclude` exists only on One
Call, not on 2.5.

**Response fields consumed.** `CurrentResponse` reads `dt`, `timezone` (an offset in seconds,
**not** an IANA name), `name`, `weather` (a `Vec<Weather>`, of which `id` and `icon` are read),
`main` (`temp`, `feels_like`, `humidity`, `pressure`), `wind` (`speed`, `deg`, `gust`), `clouds.all`,
`visibility` (metres, capped at 10 km), `rain` and `snow` (their `1h` key). `sys.sunrise`/`sunset`
are **not** deserialised. `ForecastResponse` reads `city` (`timezone`, `name` only — not the
sunrise/sunset pair) and `list`, whose `Slot` reads `dt`, `main`, `weather.id`, `wind`, `clouds`,
`visibility`, `rain`/`snow` (their `3h` key), `pop` and `sys.pod`.

**Auth.** `appid` query parameter, account-scoped limits ("API call limits are applied at the account
level, not per API key or per product"), key activation "up to 2 hours after your successful
registration" (a fresh key returns 401 until then), and the free host is only `api.openweathermap.org`
— paid plans use a different host sent by email (<https://openweathermap.org/appid>).

**Limits.** "60 calls/minute 1,000,000 calls/month" on the Free plan; the docs recommend "no more than
once in 10 minutes for each location"; over-limit requests answer 429 and the account may be suspended
"for a couple of hours to several days randomly"
(<https://openweathermap.org/full-price>, <https://openweathermap.org/faq>). The free plan includes
current weather and the 5-day/3-hour forecast; 16/30-day daily, 4-day hourly and One Call are paid.

**Coverage and granularity.** Global, blended (stations, satellites, radar, GFS, ECMWF, OWHL). Current
weather is typically a station observation (`"base": "stations"`); the forecast is a 3-hour grid of 40
slots anchored to local midnight, with `dt` in UTC and `list[].dt_txt` in UTC.

**Attribution and licence.** ODbL 1.0 with mandatory visible attribution on every self-service plan:
"'Weather data provided by OpenWeather' / Hyperlink to our website https://openweathermap.org/ /
OpenWeather logo", and "Attribution placed only in hidden documentation or deep legal pages is not
sufficient." The pricing page's recommended line is "Weather data © OpenWeather". Share-alike applies
only if we publish an adapted database (<https://openweathermap.org/full-price>,
<https://openweathermap.org/faq>). No caching ceiling is published.

**Implemented 2026-10-01** (`src/provider/openweathermap.rs`). Two calls per fetch, each cached under
its own `weather/openweathermap-{current,forecast}-…` key. Deviations from the plan's sketch, all
recorded here first: `units=metric` serves wind in **m/s**, so the provider converts to km/h; the day
extremes are computed from the slots (`main.temp_min/max` are not daily extremes); a partial
location-local today is skipped like SMHI's; `weather[0].icon`'s trailing letter supplies `is_day`;
and a **provisional (UTC) zone is refused before any request**, because the payload carries only a
UTC offset and the aggregation needs a real zone. The credit line printed is
`OpenWeather (ODbL 1.0) — https://openweathermap.org/`.

**Client notes.** `rain`/`snow` blocks are absent (not zero) when nothing falls, and the key differs
between the endpoints (`rain.1h` vs `rain.3h`); `units=metric` must be explicit (the default is
Kelvin) and does not affect precipitation (always mm/h); `main.temp_min/max` are **not** daily extremes;
`weather[0]` is the primary condition and `weather[0].main` is not translated — map from `id`;
`511` (freezing rain) and `616` (rain and snow) carry snow icons upstream; `pop` is 0–1; `lang`
translates only the city name and description.

**Unverified.** A gzip/`Accept-Encoding` policy (no mention in any OWM page); a caching/retention rule
(none published); the behaviour of `cnt` above 40; the 401 body (not captured); live per-coordinate
behaviour (the public sample host serves canned data).

### `weatherapi`

BYOK. One call per fetch; the free plan is the constraint.

**Endpoints** (verified 2026-09-30, GET, base `https://api.weatherapi.com/v1`)

| Purpose | URL | Parameters |
|---|---|---|
| Forecast | `/forecast.json` | `key`, `q`, `days` (1–14; free plan 3), `lang`, `aqi`, `alerts`, `hour`, `dt`/`unixdt` |
| Current only | `/current.json` | `key`, `q`, `aqi`, `lang` |
| City search | `/search.json` | `key`, `q` |
| Astronomy | `/astronomy.json` | `key`, `q`, `dt` |

`q` accepts `lat,lon`, a city name, US zip, UK postcode, Canadian postal code, `metar:<ICAO>`,
`iata:<code>`, `auto:ip`, an IP address, or `id:<search-id>`.

**Response fields consumed.** `Forecast` reads `location`, `current` and `forecast`. `LocationBlock`:
`name`, `region`, `country`, `tz_id`, `lat`, `lon`, `localtime_epoch`. `CurrentBlock`:
`last_updated_epoch`, `temp_c`, `feelslike_c`, `humidity`, `pressure_mb`, `wind_kph`, `wind_degree`,
`gust_kph`, `vis_km`, `uv`, `precip_mm`, `is_day`, `cloud` and `condition.code`. `ForecastDays`:
`forecastday[]`, whose `ForecastDay` reads `date`, `day` (`maxtemp_c`, `mintemp_c`, `condition`, `uv`),
`astro` (`sunrise`, `sunset` only — not the moon fields) and `hour[]`
(`time_epoch`, `temp_c`, `feelslike_c`, `humidity`, `chance_of_rain`, `precip_mm`, `wind_kph`,
`wind_degree`, `gust_kph`, `vis_km`, `condition.code`). The response's `alerts` block is **not**
deserialised; warnings come from the separate alert registry.

**Auth.** `key` query parameter; sign-up at <https://www.weatherapi.com/signup.aspx>; the key stays the
same across plan changes; a compromised key is rotated "within 4 business hours of notification".

**Limits.** Free: 100 000 calls/month ("Calls per month | 100K | 3 Million | …"), 3-day forecast, 1-day
history, `Limited` air quality and alerts; the quota "is reset at midnight on 1st of each month UTC"
and over-quota access simply stops for the month. Paid plans get a 14-day forecast. The terms promise a
"per-minute burst limit … published at weatherapi.com/pricing.aspx", but **no number is published on
any first-party page** — do not hard-code one. HTTP 429 is undocumented (the published error table has
only 400/401/403) (<https://www.weatherapi.com/pricing.aspx>, <https://www.weatherapi.com/terms.aspx>).

**Coverage and granularity.** Global, "1 to 11 km" points; sources include ECMWF, WMO, NASA, NOAA GFS2
and JMA; history is archived forecast data from 2010, not observations. `forecastday[].hour[]` carries
exactly 24 entries per day even across DST; timestamps are local-time strings **without offsets** —
`location.tz_id` is the IANA zone and `*_epoch` fields are the ones to compute with. Hour strings are
unpadded (`"2023-01-13 6:30"`).

**Attribution and licence.** The docs ask free users to link back and provide ready-made snippets
(`Powered by <a href="https://www.weatherapi.com/">WeatherAPI.com</a>`); the API terms turn it into an
obligation: "If you are a Free API user then for all uses of the data, you will credit WeatherAPI.com
by name or brand logo as the source of the data." Commercial use is allowed on Free, reselling is not,
one key serves one application. Caching: "current conditions data — maximum 60 minutes; forecast data —
maximum 24 hours". A mandatory end-user disclaimer applies to anything shown to users, quoted in full
in the terms (<https://www.weatherapi.com/terms.aspx>).

**Implemented 2026-10-01** (`src/provider/weatherapi.rs`). One call per fetch; `lang=en` is pinned
(our Fluent catalogs own the wording); `tz_id` repairs a provisional zone, so coordinates work where
`openweathermap` refuses them; the daily extremes and sun times come from the response's own `day`
and `astro` blocks (the 12-hour clock strings are joined to the day's local date); `uv` fills the
model's UV field; and the full 60-code table is unit-tested, obscurations included (`haze`→5,
`smoke`→4, `dust`→6, `mist`→10; only the storm forms and smog take the nearest described family).
Credit line:
`WeatherAPI.com (free-tier attribution) — https://www.weatherapi.com/`. The free tier's 3-day
horizon is the registry's `max_days`; a `403` keeps the `Upstream` taxonomy (exit 3) because it
carries quota and plan refusals, while a `401` becomes exit 6.

**Client notes.** Condition codes are **60 codes over 1000–1282** (not 1000–1087); the canonical list is
<https://www.weatherapi.com/docs/weather_conditions.json>, which the vendor explicitly blesses for
vendoring ("Please download the list and use it offline"). Day/night variants differ only in the icon
text/assets; store `is_day` next to `condition.code`. Astro times are 12-hour strings (`"04:31 PM"`,
`[INFERENCE]` `%I:%M %p`) and can be `"No moonrise"`/`"No moonset"`; a post-midnight moonset belongs to
the previous date's block. Errors are `{"error":{"code":…,"message":…}}`: 401/1002 missing key,
400/1003 missing `q`, 400/1006 no location, 401/2006 invalid key, 403/2007 quota, 403/2008 disabled,
403/2009 plan. `precip_mm` can be 0.0 during light rain.

**Unverified.** The `days`-beyond-plan error code (none published; every live probe returned 401 with
no body); the per-minute burst limit; whether `lang=zh` works (the docs table and the conditions file
disagree on `zh`/`zh_cn`); which free features are "Limited"; `forecastday[0].hour[]` length on the
current day.

### `worldweatheronline`

BYOK. The free tier is small and the documentation contradicts itself; the code follows the
conservative reading.

**Endpoints** (verified 2026-09-30)

| Purpose | Method | URL | Parameters |
|---|---|---|---|
| Local weather | GET | `https://api.worldweatheronline.com/premium/v1/weather.ashx` | `key`, `q` (name, `lat,lon`, IP, postcode), `format=json`, `num_of_days`, `tp`, `cc`, `fx`, `mca`, `includelocation`, `showlocaltime`, `lang`, `alerts`, `aqi`, `extra` |
| Bulk | POST | same path | up to 10 locations; each counts against the quota |
| Location search | GET | `…/premium/v1/search.ashx` | `key`, `query` |

`api.worldweatheronline.com/free/v1/…` is **not documented anywhere current** and returned HTTP 403 in
every probe; every official example uses `premium/v1`, and the pricing page says "All plans share the
same core API suite". The free key therefore goes on the `premium/v1` path.

**Response fields consumed.** `Envelope` reads `data`; `Data` reads `current_condition`,
`weather` and `error` (`ErrorBlock.msg`). `CurrentBlock` (the single `current_condition[0]`) reads
`observation_time`, `temp_C`, `FeelsLikeC`, `humidity`, `pressure`, `windspeedKmph`, `winddirDegree`,
`visibility`, `precipMM`, `cloudcover`, `uvIndex` and `weatherCode` — every scalar through a
string-or-number `text_number` helper. `DayBlock` reads `date`, `maxtempC`, `mintempC`,
`astronomy` (`AstroBlock.sunrise`/`sunset` only) and `hourly[]`, whose `HourBlock` reads `time`,
`tempC`, `FeelsLikeC`, `humidity`, `chanceofrain`, `precipMM`, `windspeedKmph`, `winddirDegree`,
`visibility` and `weatherCode`. `request`, `nearest_area`, `weatherDesc`, `uvIndex` on the day,
`moonrise`/`moonset`/`moon_phase` and `windgustKmph` are not deserialised.

**Auth.** `key` query parameter, no header scheme; sign-up with email verification; the terms require
keeping the key out of public repositories and forbid sharing it.

**Limits.** Contradictory, recorded as such: the pricing page and the free terms say **100 requests/day**
("We request our free weather API users to not exceed 100 requests per day."), while the docs index
callout says 500 requests/month. Forecast length is likewise inconsistent: the endpoint reference
allows `num_of_days` 1–14 (`0` = current only), the FAQ says the Free API gives "up to 5 days" and
Premium "up to 15 days", and the pricing matrix ticks 3/5/7/10/14-day forecasts for Free. Free uptime
is 95% with no SLA. `tp` ∈ {1,3,6,12,24}, default 3. Commercial use on the free tier is disputed across
the provider's own pages (pricing says no; the FAQ and the T&C say yes).

**Coverage and granularity.** Global ("approximately 3 million cities"); sources ECMWF, WMO, GTS,
satellites, NCEP GFS, JMA; data updated "every three-four hours"; all times local to the location,
`hourly[].time` being an unpadded local `HHMM` string (`"0"`, `"700"`, `"2100"`).

**Attribution and licence.** Proprietary; free-tier credit is mandatory: "the only mandatory credit is
to write **Weather Data by WorldWeatherOnline.com**" (non-website surfaces), with a link and title
required on websites. Clause 1C adds a mandatory end-user disclaimer ("informational purposes only …
Forecasts are probabilistic … not be used as the sole basis for decisions involving personal safety,
aviation, marine navigation, emergency planning, or other safety-critical activities"). Caching limits:
current conditions 60 minutes, forecast 24 hours. No resale, no bulk copying
(<https://www.worldweatheronline.com/weather-api/api/free-api-terms.aspx>,
<https://www.worldweatheronline.com/weather-api/api/api-t-and-c.aspx>).

**Implemented 2026-10-01** (`src/provider/worldweatheronline.rs`). One call per fetch with `format=json`
and `tp=3`; the string-valued scalars and single-element arrays are unwrapped by `text_number`; the
daily extremes and `astronomy[0]` sun times come from the response; a day whose slots do not cover
all four parts is skipped. Two findings recorded here: **`observation_time` is the UTC wall clock**,
not the local time the docs claim (two recordings at known instants both matched UTC — the provider
joins it to the fetch instant's UTC date and converts), and the payload carries **no time zone**, so
a provisional (UTC) location is refused before any request like OpenWeatherMap's. Credit line:
`WorldWeatherOnline.com (free-tier attribution) — https://www.worldweatheronline.com/`.

**Client notes.** `format=json` is mandatory — the documented default is `xml`; every scalar is a JSON
**string** (`"temp_C": "18"`); single-element arrays wrap descriptions and areas
(`weatherDesc[0].value`); astronomy is nested under `weather[].astronomy`; `weatherCode` is a closed
60-code set with separate day/night icons
(<https://www.worldweatheronline.com/feed/wwoConditionCodes.txt>), each obscuration mapping onto the
model's own code (125/128/131/134/137/140→5/6/7/7/7/7, 143→10, 146→4, 149/152/155→5, 158/161→6).
Error mapping observed live: 400
missing parameter, 401 bad key on `premium/v1`, 403 wrong/deprecated path tier, 404 unknown path. The
classic `data.error[].msg` envelope is not documented on any current page.

**Unverified.** The error body shape; per-tier `tp`/`num_of_days` maxima; whether a free key is
restricted to a path; whether the old `v1`/`v2`/`v3` paths exist; the monthly-vs-daily quota conflict;
the 429 behaviour; alert coverage by country; whether the historical archive is on the free tier.

### `pirateweather`

BYOK. Dark Sky-shaped payloads; the key is a path segment.

**Endpoints** (verified 2026-09-30)

| Purpose | Method | URL | Parameters |
|---|---|---|---|
| Forecast | GET | `https://api.pirateweather.net/forecast/<apikey>/<lat>,<lon>` | `units` (default `us`; we send `si`), `exclude`, `extend=hourly`, `lang`, `version`, `include`, `aqiunits`, `icon` |
| Historical | GET | `https://timemachine.pirateweather.net/forecast/<apikey>/<lat>,<lon>,<time>` | as above plus the mandatory time |

There is **no `tz` parameter**; the response's `timezone` name and `offset` (hours, sometimes a float)
carry the local calendar. The key may alternatively travel in an `apikey` header with a dummy path
segment.

**Response fields consumed.** `Forecast` reads `timezone`, `offset`, `currently`, `hourly`, `daily`
and `flags`; `latitude`, `longitude`, `elevation`, `minutely` and `alerts` are not deserialised.
`Series` is a `{ "data": [...] }` block; `Block` (one `currently`, `hourly.data[]` or
`daily.data[]` entry, all fields optional) reads `time` (UNIX UTC seconds), `icon`, `temperature`,
`apparent_temperature`, `precip_intensity`, `precip_probability`, `humidity`, `cloud_cover`,
`pressure`, `wind_speed`, `wind_gust`, `wind_bearing`, `visibility`, `uv_index`, and for daily
entries `temperature_high`, `temperature_low`, `sunrise_time` and `sunset_time`. `Flags` reads `units`
and `sources` (`summary`, `precipType`, `dewPoint`, `moonPhase` and `precipAccumulation` are not
deserialised). Under `units=si`, `precip_intensity` is mm/h of liquid water and
`humidity`/`cloud_cover`/`precip_probability` are 0–1;
`visibility` is capped at 16 km.

**Auth.** Key in the path (or header); sign-up through the Apiable portal, and "it can take up to 20
minutes for the change to propagate to the gateway"; an invalid key returns a Kong-generated 401 in
`text/html`, before any coordinate validation.

**Limits.** Free tier: **10 000 calls/month**; a "$2 monthly donation lets you raise your API limit
from 10,000 calls/month to 20,000"; a per-key rate limit of "1 to 4/ per second (depending on the
plan)"; quota exhaustion answers 429. The response headers report usage
(`Ratelimit-Limit`/`-Remaining`/`-Reset`, `X-Forecast-API-Calls`). The only use restriction in the
terms is that users "do not attempt to circumvent the call limit of the API (i.e. by making multiple
accounts)" (<https://docs.pirateweather.net/en/latest/>, <https://docs.pirateweather.net/en/latest/API/>).

**Coverage and granularity.** Global; GFS/GEFS backbone with HRRR/NBM/RTMA-RU/URMA over North America,
ECCC models, DWD MOSMIX, ECMWF IFS/AIFS, RAQDPS and FMI SILAM for air quality, ERA5 for history.
Blending is per-element first-non-null selection, so one response mixes sources; stale runs are
excluded (NBM after 2 days, GFS/GEFS/ECMWF after 5). Minutely covers 60 minutes (15-minute accuracy
inside the HRRR domain), hourly 48 h (`extend=hourly` → 168 h), daily 7 days with 4 a.m. summary
windows.

**Attribution and licence.** **No mandatory credit is documented** — the docs, the Terms and the
changelog contain no branding clause, and "Powered by Pirate Weather" is the project's own Home
Assistant constant, not a licence term. The terms do carry a warranty disclaimer ("should not be used
for life or property critical applications") and a $100 liability cap. The service sends
`Cache-Control: max-age=900, must-revalidate` (<https://pirate-weather.apiable.io/terms>,
<https://docs.pirateweather.net/en/latest/DataSources/>).

**Implemented 2026-10-01** (`src/provider/pirateweather.rs`). One call per fetch with
`units=si&exclude=minutely,alerts&lang=en&extend=hourly`; the key is a path segment and is redacted
everywhere; `timezone` repairs a provisional zone; `-999` and absent fields become `None`; the icon
table covers the default set plus `hail` and refines rain/snow/sleet by the provider's own mm/h
bands (0.4 / 2.5, a three-way collapse of its four bands). Two decisions worth recording:
`precipAccumulation` (centimetres under `si`) is **not** consumed — the day parts sum the hourly
liquid-equivalent intensities, so the scaling trap never applies — and `is_day` prefers an explicit
`-day`/`-night` icon, falling back to the local civil day for the icons without a variant. Credit
line: `Pirate Weather — https://pirateweather.net/` (the terms mandate no attribution; the line
names the service and its home page rather than inventing a mandated string).

**Client notes.** Do not send `tz`; localise with `timezone` + `offset * 3600`. `flags.sources` is a
candidate list, not per-value provenance (`sourceIDX` with `version=2` is). `-999` appears in place of
missing values and fields may be absent; `icon` needs a fallback branch (including `none`); requesting
a future time more than an hour ahead is a 400; the project's own intensity bands are
0.02/0.4/2.5/10 mm/h — the Dark Sky 0.4/3.4 figures are **not** PirateWeather's.

**Unverified.** The paid-tier catalogue and prices (the portal is a JS shell; its JSON endpoints
return 401); which plan gets which per-second limit; the production `Cache-Control` value (read from
source, not from a live response); a live 429 body; any attribution requirement (absence of
documentation is not proof of absence).

### `qweather`

BYOK. **The city-based v7 APIs are deprecated (EOL 2027), so this backend speaks the v1 endpoints**
(`/weather/v1/{current,hourly,daily}/{lat}/{lon}`), verified live on 2026-10-01 against the account
host the console issues.

**Endpoints** (verified 2026-10-01, all GET, HTTPS only, host from the account)

| Purpose | URL template | Parameters |
|---|---|---|
| Current | `https://<host>/weather/v1/current/<lat>/<lon>` | `lang` |
| Hourly | `https://<host>/weather/v1/hourly/<lat>/<lon>` | `hours` (1–240; probed: `241` → `400`), `lang` |
| Daily | `https://<host>/weather/v1/daily/<lat>/<lon>` | `days` (1–10; probed: `11` → `400`), `lang` |

The host is **per account** (`<account-id>.re.qweatherapi.com` in the verified account; the console
shows it at <https://console.qweather.com/>) and is part of the authentication ("even if a developer's credentials are leaked, an attacker cannot request data
without knowing the API Host"). The legacy shared domains (`api.qweather.com`, `devapi.qweather.com`,
`geoapi.qweather.com`) answer `403` with `"title": "Invalid Host"` — probed with a valid key on
2026-10-01 — so `<host>` has no default and comes from `providers.qweather.host` (the
`[providers.qweather]` table of `config.toml`).

**Response shape.** Every value is a **measure object** (`{"value": 10.96, "unit": "°C"}`), the
timestamps are **UTC instants** (`2026-10-01T00:00Z`, minutes form — valid ISO 8601, not strict
RFC 3339), and the payload carries **no time zone and no observation time**. Our `CurrentResponse`
and `HourlyResponse` read:

* `metadata` (`tag`, `attributions[]`), `condition.code` (a string), and `temperature`, `feelsLike`,
  `windGust`, `pressure` and `visibility` (each a `Measure` with its unit checked against the
  canonical one), `humidity` (0–1), `wind` (`Wind.speed` a `Measure`, `Wind.direction.degree`),
  `precipitation` (`Precipitation.amount` a `Measure`, `.probability` a percent on hourly entries),
  `cloudCover` (0–1) and `uvIndex`.
* `hours[]` adds `forecastTime`; the `days[]` block — with its `astro` and
  `daytime`/`nighttime` sub-blocks — is **not deserialised**, because its split does not map onto the
  four canonical parts and the daily extremes are derivable from the hourly series.

**Auth.** Two modes, both against the same per-account host:

* **API KEY** — `X-QW-Api-Key: <key>` header (the `key=` query form also works; never both). The
  API KEY *signature* flow is retired, and API-KEY volume will be limited from 2027 (the docs give
  both 2027-01-01 and 2027-02-01).
* **JWT** (the vendor's recommendation) — `Authorization: Bearer <token>`, an Ed25519 (EdDSA) token
  minted in-process per fetch. The console steps: `openssl genpkey -algorithm ED25519 -out key.pem`,
  upload the **public** half under *Console → Project → Add Credential → JSON Web Token*, and keep
  the **credential id** the console issues plus the **developer id** (`iss`, ten characters starting
  with `Q`) and the **project id** (`sub`). Header `{"alg":"EdDSA","kid":"<credential id>"}`, payload
  `{"iss","sub","iat","exp"}`; `exp − iat` must not exceed 24 h (86 400 s) and `typ`, `aud` and `nbf`
  are reserved and must not be sent. This client mints a **15-minute** token — backdated 30 s to
  absorb clock skew — for every fetch and persists nothing; a long-running consumer therefore never
  presents a token older than its own request. The private key stays in `keys.toml` (mode `0600`)
  except inside the signer, and the minted token is registered as a request secret, so no `-v` line,
  error message or cache envelope can carry it. `cirrocast key set qweather --jwt --key-file <PATH|->`
  `--credential-id <ID> --developer-id <ID> --project-id <ID>` stores it (the PEM is validated
  before anything is written, and read from a file or stdin, never from argv); the quartet
  `CIRROCAST_QWEATHER_JWT_{CREDENTIAL_ID,DEVELOPER_ID,PROJECT_ID,PRIVATE_KEY}` configures the same
  four items without touching the file.

Resolution for `qweather`, first complete set wins: the environment quartet → the `[jwt.qweather]`
table of `keys.toml` → `CIRROCAST_QWEATHER_KEY` → the `[keys] qweather` entry. A **partial** JWT set
(some but not all of the quartet, or a table missing a field) is a configuration error (exit 4)
naming the missing items — never a silent fall-back to the API key. `key list` shows the identifiers
(`qweather  jwt (kid …, iss …, sub …)`) and the masked API key beside them when both are stored,
never the PEM; `key rm qweather` removes both forms. The API host requirement is unchanged: it is
part of the authentication, and a JWT-configured run without `providers.qweather.host` fails the
same way an API-key one does.

**Limits.** Pay-as-you-go with the **first 50 000 requests/month at ¥0** (no "Standard" free plan);
QPM 3 000 for pay-as-you-go, 50 000+ for Premium. Non-2xx responses are not billed, but sustained
invalid traffic can suspend the account. Recommended cache ages: real-time 10–30 min, hourly
30–60 min, daily 1–6 h.

**Coverage and granularity.** Global ("200+ countries or regions … over 500,000 cities"); hourly up
to 240 hours and daily up to 10 days on v1 (both probed). The v1 endpoints are metric-only: `unit=i`
is ignored (probed).

**Attribution and licence.** The terms page (`https://dev.qweather.com/docs/terms/attribution/`) is
explicit: name "QWeather" plus a link to `https://www.qweather.com` for any use of the service, and
**for weather warnings and for air quality you must display all of the response's attribution lines
in full and unmodified** (the v7 wording was `refer.sources`; v1 spells it `metadata.attributions`).
The alert source carries those lines on every decoded alert (`Alert::credit`) and
`alerts::credits` prints them after the registry's own line, so the issuing agency and the standing
disclaimer reach every renderer — the footer, `plain`, the alert listing, and `-f json` as
`alerts[].credit`. GeoAPI data must not be bulk-cached or indexed — this backend does not call
GeoAPI.

**Errors.** RFC 7807 `application/problem+json` with `error.status/type/title/detail/invalidParams`;
a bad key is `401` (`#unauthorized`), a bad parameter or location `400`
(`#invalid-parameter`). `403` covers no-credit, overdue, invalid host and permission refusals.

**The v1 host compresses every response.** `Content-Encoding: gzip` on both endpoints, probed
2026-10-06 over HTTP/1.1 and HTTP/2 and with `Accept-Encoding: identity` sent explicitly — the
header is ignored, so a client must decode gzip or misread every body (a stale cache hides it until
the entry expires). `src/http.rs` decodes after its capped read, under the same size ceiling.

**The alert API is v1, and it is not a separate subscription.** `GET
/weatheralert/v1/current/{lat}/{lon}` answers `200` on the account host used 2026-10-07 (JWT
authentication), with the same `metadata` envelope the forecast endpoints use and an `alerts` array
that is empty when nothing is in force (`metadata.zeroResult`). The v7 spelling this source first
spoke — `/weatheralert/v7/alert/now?location=<lon>,<lat>` — is retired: it answers `404` with an
**empty** body (no RFC 7807 envelope) while the v1 path works, which is what the earlier
"separate subscription" reading was built on. The v1 payload spells CAP's severity, urgency and
certainty value sets verbatim and carries `messageType.code` (`alert`/`update`/`cancel`).

**Implemented 2026-10-01** (`src/provider/qweather.rs`, `max_days: 10`). Two calls per fetch
(current + hourly), each cached under `weather/qweather-{current,hourly}-…`; every measure's unit is
checked against the canonical one (`°C`, `m/s`, `mm`, `hPa`, `m`) and a mismatch is an upstream error
rather than a silent conversion; the day extremes come from the samples. Findings pinned by tests:

* **The hourly series is anchored to the next UTC midnight**, not to the current hour (probed at
  16:40Z and 23:51Z, both starting at the following `00:00Z`), so the location-local today is usually
  incomplete and is skipped — the first fully covered local day is emitted, the same rule SMHI,
  OpenWeatherMap and WWO use.
* **The current block carries no observation time**, so `observed_at` is the fetch instant.
* **The payload has no time zone**, so a provisional (UTC) location is refused with a usage error
  (the daily boundaries would reveal the offset, but an offset is not a zone).
* The `days[]` block is **not consumed**: its `daytime`/`nighttime` split does not map onto the four
  canonical parts, and the extremes are derivable from the hourly series.

Credit line: `QWeather — https://www.qweather.com/`. A missing host with a key present is
`Error::Config` (exit 4) with `set providers.qweather.host (see
https://console.qweather.com/, or cirrocast provider info qweather)`; with no credential
either, the chain reports the missing credential first (exit 6), naming both `key set` forms.

**JWT mode implemented 2026-10-06** (step 27). The token shape, the claim set, the 24 h ceiling and
the console flow come from the vendor documentation (read 2026-10-03) and are pinned by a byte-exact
test vector computed independently of this implementation; the request path is exercised through
`StubTransport` in both modes, and the cache-key invariance, the redaction and the exit codes have
their own tests. **Not verified against the live service in this repository**: no credential was
used, so the first live call is the opt-in smoke in `tests/live.rs` (`live_qweather_jwt`, enabled
with `CIRROCAST_LIVE_TESTS=1` plus the quartet and `CIRROCAST_QWEATHER_HOST`). The vendor's token
validator (Console → JWT Validation) is the tool to reach for if a live token is refused.

**Unverified.** The Developers License text (403) and every rendered `dev.qweather.com` page (403 —
the v1 facts above come from live responses and the public docs repository); the exact QPM scope
(per project vs per account); whether `days[]`'s `daytime`/`nighttime` windows are fixed local
windows (observed 07:00–19:00 local, which is not the solar day); whether the live JWT endpoint
accepts the 15-minute lifetime and the 30 s backdating as documented (the vector is only as good as
the documentation).

## Climate normals (NOAA NCEI)

`--normals` is the one comparison rather than a reading: the month's climate normal is **computed
here** from NOAA NCEI's Global Summary of the Month (GSOM), the mean of the requested calendar
month over the configured reference period for the station nearest the location. Two keyless
requests, both cached for 30 days; the credit
`Climate normals computed from NOAA NCEI Global Summary of the Month (public domain)` travels with
the block (`art-table`'s footer, `plain`'s document, the standalone view).

### At a glance

| Service | Endpoint | Auth | Licence / credit | Policy | Cache ceiling | Verified |
|---|---|---|---|---|---|---|
| NCEI station search | `GET www.ncei.noaa.gov/access/services/search/v1/data?dataset=global-summary-of-the-month&bbox=<maxLat>,<minLon>,<minLat>,<maxLon>&limit=5` | none | US government work, public domain (`LicenseRef-US-Government-Public-Domain`) | no key; HTTPS only (the service enforces HSTS); no request quota published on the access-services documentation page (checked 2026-10-06); `limit=5` keeps the answer at 24–67 KB measured | 30 days (`normals/search-<lat.2dp>-<lon.2dp>-<radius>km.json`) | 2026-10-06 |
| NCEI monthly summaries | `GET www.ncei.noaa.gov/access/services/data/v1?dataset=global-summary-of-the-month&stations=<ID>&startDate=<first>-01-01&endDate=<last>-12-31&format=json&units=metric&dataTypes=TAVG,TMAX,TMIN,PRCP` | none | the same | the same; 92 KB for 30 years without the `dataTypes` projection, 27 KB with it (measured) | 30 days (`normals/<station>-<period>-<MM>.json`) | 2026-10-06 |

### The two-step flow (measured 2026-10-06)

1. **Which station answers?** The search endpoint takes a bounding box whose corners are
   **north-west first**, then south-east: `bbox=<maxLat>,<minLon>,<minLat>,<maxLon>`. The
   documented-looking `SW,NE` order answers **`HTTP 500`**, and the client's regression test pins
   the order. The box is the configured radius converted at 111 km per degree of latitude with the
   longitude half-width divided by `cos(lat)` (clamped at a pole and the antimeridian). The answer
   carries one `results[]` entry per station file, each with its file `name`
   (`CHM00054511.csv`), its point as `centroid: [lon, lat]` (also in `location.coordinates` and
   `boundingPoints[].coordinates`) and a nested `stations[]` list whose `dataTypes[]` records name
   the covered datatypes. Two traps were measured: the entries are **not** ordered by distance (a
   recorded two-degree Beijing box lists a station 119.7 km away first and the 12.4 km one last),
   so every entry is measured; and a box with no station answers `200` with `results: []`, which
   is an answer, not an error.
2. **What does that station say?** The data endpoint answers one row per `YYYY-MM` of the period —
   `{DATE, STATION, TAVG, TMAX, TMIN, PRCP}` with the values as JSON **strings** in °C and mm. The
   `dataTypes` projection is what keeps the response reasonable (92 KB → 27 KB for 30 years
   measured); a station may answer `[]` when the file holds nothing for the window. The client
   averages the requested month over the rows carrying **all four** values — a year with a
   precipitation total and no temperatures is a real GSOM shape (the recorded Beijing station has
   one in `2020-10`) and must not make the printed `years` count mean two things at once — and
   refuses a month with fewer than 20 such years.

Request timings measured from this network: the search took 1.9–10.3 s and the values request
1.8–16.7 s (NCEI is slower than the weather APIs), which is what the 30-day TTL and the opt-in flag
are for: a run without `--normals` makes no NCEI request at all. Recorded fixtures and their exact
request text are in `tests/fixtures/normals/`.

## Location and IP services

The user-facing half of these services — which one a query reaches, how candidates are ranked and
how `--ip` picks a locator — is in [`docs/location.md`](location.md); this section records the
upstream endpoints, their terms and their measured behaviour. Their selection keys and environment
variables are in [`docs/configuration.md`](configuration.md).

### At a glance

| Service | Endpoint | Auth | Licence / credit | Policy | Cache ceiling | Verified |
|---|---|---|---|---|---|---|
| Open-Meteo geocoding | `GET geocoding-api.open-meteo.com/v1/search` | none | GeoNames data, CC BY 4.0, via Open-Meteo | free tier shared with the forecast API: non-commercial, 10 000/day, 5 000/hour, 600/minute | 30 days (`cache.geocode_ttl_secs`) | 2026-10-06 |
| GeoNames search | `GET secure.geonames.org/searchJSON` | **BYOK**: `CIRROCAST_GEONAMES_USER`, else `keys.toml [keys] geonames` (`key set geonames`) | GeoNames CC BY 4.0: `Location data by GeoNames (CC BY 4.0) — https://www.geonames.org/` | free registration, no card; per-account daily credit quota | 30 days (`cache.geocode_ttl_secs`) | 2026-10-06 |
| Nominatim `/search` | `GET <base>/search` | none; a descriptive `User-Agent` is mandatory | ODbL: `Location data © OpenStreetMap contributors (ODbL)` | "absolute maximum of 1 request per second", results must be cached, no autocomplete, no bulk geocoding | 30 days (`nominatim::TTL`), shared throttle state | 2026-10-06 |
| Nominatim `/reverse` | `GET <base>/reverse` | none; the same `User-Agent` | ODbL, the same credit | the same one-request-per-second rule and caching duty | 30 days, same namespace and throttle | 2026-10-06 |
| ipwho.is | `GET ipwho.is/` | none | none required | 1 000 requests/day per client IP; `429` + `Retry-After` beyond it | ≤ 24 h (`cache.ip_ttl_secs`) | 2026-10-06 |
| ipapi.co | `GET ipapi.co/json/` | none | none required (DB-IP/IP2Location attribution is ipapi.co's own) | up to 1 000 lookups/day, "not meant for use in production" | **at most 24 h** by its terms — the cap `cache.ip_ttl_secs` honours | 2026-10-06 |
| IP.SB | `GET api.ip.sb/geoip` | none | none required | keyless public endpoint; no published quota | ≤ 24 h, the same cap | 2026-10-06 |

### Bundled location data (no network, no service)

| Data set | Source | Licence | Committed size | Verified |
|---|---|---|---|---|
| City table (`cities.bin.gz`, `keys.bin.gz`, `SNAPSHOT`) | GeoNames `cities15000` dump | CC BY 4.0 | 3.4 MiB compressed | 2026-10-04 (dump date in `SNAPSHOT`) |
| Country layer (`countries.bin.gz`, `COUNTRIES`) | Natural Earth 1:50m `admin_0_countries`, quantised to 1e-3° | public domain (CC0-1.0); no credit required | 407 KiB compressed | 2026-10-06 (input sha256 in `COUNTRIES`) |

Both are refreshed and checked with `cargo run -p geo-table -- …` (see `AGENTS.md` § Releasing);
`--check` compares the committed bytes against the pinned source without writing anything.

### Open-Meteo geocoding (keyless)

`GET https://geocoding-api.open-meteo.com/v1/search` with `name` (required), `count` (default 10, max
100), `language`, `format`, `countryCode`; `GET …/v1/get?id=<geoname-id>` resolves a single id. The
`results[]` entries carry `id`, `name`, `latitude`, `longitude`, `elevation`, `feature_code`,
`country_code`, `admin1`, `timezone`, `population`, `country`. No key; the free-tier budget is shared
with the forecast API (10 000/day, 5 000/hour, 600/minute) and commercial use is not permitted on it.
Empty or single-character searches return HTTP 200 **without a `results` key**; empty fields are
omitted, not nulled. Data is GeoNames under CC BY 4.0 — the credit "Location data based on GeoNames"
plus a link to Open-Meteo is what `geo::attribution_line` prints
(<https://open-meteo.com/en/docs/geocoding-api>, <https://open-meteo.com/en/licence>).

### Nominatim (keyless, `~query`)

`GET https://nominatim.openstreetmap.org/search?format=jsonv2&q=…&limit=…&addressdetails=1&extratags=1`
with `accept-language: en`. Policy, verbatim: "No heavy uses (an absolute maximum of 1 request per
second)", results "must be cached on your side", no autocomplete and no bulk geocoding, a valid
User-Agent or Referer identifying the application is mandatory, and the service must be switchable
without a software update — which is what `network.nominatim_url` / `CIRROCAST_NOMINATIM_URL` are for.
Data is ODbL: "Clearly display attribution as suitable for your medium", the accepted string being
"© OpenStreetMap contributors" with a link to the licence; every response also carries its own
`licence` field. `lat`/`lon` arrive as JSON strings; `place_id` is not stable across servers
(<https://operations.osmfoundation.org/policies/nominatim/>,
<https://wiki.osmfoundation.org/wiki/Licence/Attribution_Guidelines>).

**Verified through a proxy.** The public instance is unreachable directly from the verification
network (connect timeouts) but answers normally through a local HTTP proxy (`[network] proxy`,
measured 2026-10-06); the recordings under `tests/fixtures/geo/` were made that way.

**`/reverse` (step 25).** `GET <base>/reverse?format=jsonv2&lat=…&lon=…&zoom=10&addressdetails=1`
with the same `accept-language: en` and `User-Agent`, the same one-request-per-second throttle and
the same cached namespace as `/search` (30 days). The answer is a single jsonv2 object with the same
fields as a search hit, so it is mapped by the same code; a point the service cannot name comes back
as `{"error":"Unable to geocode"}` or a `404`, which `reverse` turns into "no name here" — a
legitimate answer for the open ocean — rather than a failure. Coordinates are rounded to five
decimals in both the request and the cache key, so two spellings of one point share an entry. The
recorded answer for a city-centre coordinate is the *suburb* the point falls in
(`39.9042,116.4074` → `Donghuamen Subdistrict`), and a point nothing names answers `200` with
`{"error":"Unable to geocode"}` — both measured 2026-10-06.

### GeoNames `searchJSON` (BYOK, step 25)

`GET https://secure.geonames.org/searchJSON` with `q` (required), `fuzzy` (0..1; this client sends
0.8, the step's threshold), `maxRows`, `style=FULL` and `username`. The `secure.` host is the one
whose certificate matches the name: `api.geonames.org` fails TLS validation with "no alternative
certificate subject name matches target hostname", measured 2026-10-06. Registration is free and
needs no card; the account name is a **credential**, never a bundled value — `CIRROCAST_GEONAMES_USER`
or `keys.toml [keys] geonames`, written by `cirrocast key set geonames` from stdin and masked by
`key list`. Measured refusals, both live on 2026-10-06: **no `username`, or an unregistered one** →
HTTP 401 with `{"status":{"value":10,"message":…}}`, mapped to `Error::MissingKey` (exit 6) naming
the credential rather than to the shared helper's invalid-key path; **the daily credit quota** →
HTTP 200 with `{"status":{"value":18,"message":"the daily limit of 20000 credits for demo has been
exceeded. …"}}` (18/19/20 are the documented limits), mapped to `Error::Upstream` (exit 3) quoting
the service's sentence, never a silently empty result. `lat` and `lng` arrive as JSON **strings**,
`population` as a number whose `0` means unknown, and `style=FULL` adds `timezone.timeZoneId`,
`adminName1` and `countryName`. A row that cannot become a location — the `0, 0` sentinel, a
country code that is not two ASCII letters, no usable zone — is dropped rather than failing the
query, because this source is one of several a name query merges. Credit: `Location data by
GeoNames (CC BY 4.0) — https://www.geonames.org/`
(<https://www.geonames.org/export/web-services.html>, <https://www.geonames.org/about.html>).

### IP.SB (keyless, last-resort IP locator)

`GET https://api.ip.sb/geoip` — the caller's own address, no key, worldwide. Measured 2026-10-06:
HTTP 200 flat JSON with `city`, `region`, `country`, `country_code`, `latitude`, `longitude`,
`timezone` (plus `ip`, `asn`, `isp`, `organization`, …), city-level. The per-address path
(`/geoip/<ip>`) answers **403 Forbidden** on the free endpoint and a `?ip=` parameter is ignored
(the answer stays the caller's), so only the caller-address form is used. An address the service
cannot place comes back as `0, 0` with empty names — a sentinel, so it is refused as upstream data
and the chain moves on rather than querying the weather for the Gulf of Guinea. No credit line is
required; the `--ip` disclosure names the service that answered. The local proxy's TLS handshake to
this host fails while a direct connection succeeds (measured 2026-10-03), which is part of why it
sits last in the chain (<https://ip.sb/>, <https://api.ip.sb/geoip>).

### ipwho.is (keyless, primary IP locator)

`GET https://ipwho.is/` (caller's IP) or `https://ipwho.is/<ip>`; optional `fields`, `output`, `lang`.
Free tier: "1,000 requests per day per client IP address", no key, HTTPS included, commercial use
allowed. Over-limit is `429` **with** a `Retry-After` header ("Access will be restored automatically
after 24 hours"). Application errors ride on HTTP 200 as `{"success":false,"message":…}` — a 2xx status
is not success — while a non-IP path segment is a 404. The terms grant no redistribution and require no
credit line; the privacy disclosure naming the service is our own practice. First of the three
services the `auto` chain tries (then ipapi.co, then IP.SB; step 25)
(<https://ipwhois.io/documentation>, <https://ipwhois.io/terms>).

### ipapi.co (keyless, fallback IP locator)

`GET https://ipapi.co/json/` (or `https://ipapi.co/<ip>/json/`). Free tier: "up-to 1000 IP lookups in a
day (approximately 30K/month)", explicitly "not meant for use in production"; repeat queries of the
same IP count again. Errors can ride on HTTP 200 as `{"error":true,"reason":…}`; over-quota is 429 with
a `RateLimited` reason and no documented `Retry-After`. `country` holds the **two-letter code** while
`country_name` holds the display name. The licence condition that shapes our config: "cache, store, or
retain any data … beyond the minimum time necessary for immediate use, which shall not exceed 24
hours", which is why `cache.ip_ttl_secs` is capped at 86 400. No credit line is imposed on API clients;
the upstream DB-IP/IP2Location attribution belongs to ipapi.co's own footer
(<https://ipapi.co/terms>, <https://ipapi.co/api/>).

## Re-verification log

* **2026-10-07** — the QWeather **alert** endpoint re-probed with the account's JWT credential,
  after a Chinese run's `404` note was traced to the path rather than to a subscription. `GET
  {host}/weatheralert/v1/current/<lat>/<lon>` answers `200`, gzip-encoded, with
  `{"metadata":{"tag":…,"zeroResult":true},"alerts":[]}` for a quiet point; the v7 spelling
  (`/weatheralert/v7/alert/now?location=…`) answers `404` with an empty body — exactly what the
  2026-10-06 entry below recorded, and the reason its "separate subscription" reading is corrected
  here. A scan of forty points (thirty-five mainland cities plus Hong Kong, Taipei, Tokyo, Manila
  and New Orleans) found live alerts at Shenzhen (森林火险, `moderate`, `urgency: null`) and Hong Kong
  (火災危險, `severe`, `senderName` 香港天文台, with `metadata.attributions` naming the issuing agency
  and a standing disclaimer). The payload spells CAP's severity/urgency/certainty value sets
  verbatim and carries `messageType.code` (`alert`/`update`/`cancel`); `src/alerts/qweather.rs` now
  speaks the v1 endpoint, with `qweather-v1` as its cache-key discriminator. The attribution terms
  page (`dev.qweather.com/docs/terms/attribution/`) was read the same day: it requires the
  response's attribution lines shown **in full and unmodified** for weather warnings *and* air
  quality, so `Alert::credit` now carries `metadata.attributions` into `alerts::credits` and
  `alerts[].credit`.
* **2026-10-06** — the whole document re-checked against the shipped binary (step 28), no upstream
  fetch needed. **Registry**: `provider list` and `provider info <id>` for all fourteen ids agree
  with the at-a-glance table and the sections — the six `CIRROCAST_*_KEY` names, the `NET` classes,
  every `max_days`, and the alert attribution (`nws` prints `its own payload` because its row is
  `alerts: true`; `visualcrossing` prints `its own payload, visualcrossing`; `qweather` prints
  `qweather`). **Correction**: the built-in default chain is the single backend `open-meteo`
  (`-v` prints `provider: open-meteo (from the config or the built-in default)`; `config show` has
  `[defaults] provider = "open-meteo"`), not `auto` as the document claimed — `auto` is opt-in, and
  with no explicit `--provider`, `--station <ICAO>` prepends `metar` to the configured chain.
  **Selection**: `--provider` accepts the fourteen ids plus `auto` (case-insensitive, `-`/`_`
  tolerated) and refuses `open-meteo-marine` with the `--marine` hint (exit 2); `--alerts-from`
  accepts `nws, meteoalarm, qweather, hko, wmoswic, fpas, visualcrossing` and refuses
  `visualcrossing` without `--provider visualcrossing` (exit 2). **Flags**: every flag the document
  names (`--provider`, `--alerts-from`, `--station`, `--marine`, `--date`, `--history`, `--days`,
  `--normals`) exists as described, and `--provider open-meteo-archive` with `--days` (or with no
  window) is the promised usage error (exit 2). **QWeather JWT**: `key set qweather --jwt
  --key-file … --credential-id … --developer-id … --project-id …` stores it, `key list` prints
  `qweather  jwt (kid …, iss …, sub …)` beside the masked API key, and `key rm qweather` removes both
  forms. **metar**: `--station KJFK` and `--provider metar @40.64,-73.78` both resolve through the
  embedded station table. **Normals**: the 20-year minimum and the two 30-day cache TTLs match
  `src/normals/ncei.rs`.
* **2026-10-06** — the step-26 NCEI access services checked directly against the live endpoints.
  **Search**: the `bbox` corner order is `maxLat,minLon,minLat,maxLon` (an `SW,NE` attempt answered
  `HTTP 500`); a two-degree Beijing box returned three station files in an order that is **not**
  distance (119.7 km, 89.4 km, then the 12.4 km `CHM00054511` last), each carrying `centroid` as a
  `[lon, lat]` array and a `stations[]` list with per-datatype coverage records; a station-free
  Pacific box answered `200` with `results: []` in 246 bytes. **Values**: `format=json&units=metric`
  answers `DATE`-keyed rows whose values are JSON strings; the `dataTypes=TAVG,TMAX,TMIN,PRCP`
  projection cut the 30-year window from 91 616 B to 26 877 B at Beijing and 35 864 B at Madison;
  the recorded stations have real holes (Beijing's `2020-10` carries `PRCP` and no temperatures),
  which is why a year counts only when all four values are present. The access-services
  documentation page was read the same day: it documents the data endpoint, states the service is
  HTTPS-only (HSTS) and publishes no request quota.
* **2026-10-06** — the step-25 location services checked against their live pages and probes.
  **GeoNames**: the endpoint, the parameter set, the string-typed coordinates and the `style=FULL`
  fields from the published documentation; two refusals measured live — no `username` and an
  unregistered one both answer `401` with `value: 10`, the `demo` account answers `200` with
  `value: 18` and the 20 000-credit sentence; `api.geonames.org` fails TLS certificate validation
  while `secure.geonames.org` serves the same API, which is why the client pins the latter.
  **IP.SB**: `api.ip.sb/geoip` answered `200` with the documented flat JSON from this network,
  `/geoip/<ip>` answered `403` and `?ip=` was ignored. **Nominatim** (search and
  `/reverse`): verified through a local HTTP proxy (the public instance times out when dialled
  directly): the request shape, the mapped fields, the lower-cased country code, the suburb-level
  answer `zoom=10` gives for a city centre, and the `200` + `{"error":"Unable to geocode"}` refusal
  were all observed; the `nominatim_reverse_*.json` fixtures are recordings made with the client's
  own headers.
  **Natural Earth**: the pinned `v5.1.2` GeoJSON (byte-identical to `master` on this date) measured
  at 242 features, 99 613 points and 416 965 bytes compressed at 1e-3°, inside the step's 1 MiB
  budget; the four `-99` shapes resolve through `ISO_A2_EH` (`TW`, `XK`) and the explicit
  `DISPUTED_CODES` table (`CY`, `SO`). The registry rows and the tables above carry
  `verified: 2026-10-06`.

* **2026-10-06** — the two step-24 backends checked against `src/provider/mod.rs` and their modules,
  and their registry rows carry `verified: 2026-10-06`. NWS: the two-step `points` → `gridpoints`
  walk, the `grid/` namespace with its 30-day TTL, the mandatory descriptive `User-Agent`, the
  measured 156 hourly periods and 14 daily periods (`max_days: 7`), `temperatureUnit` read per
  period, the `windSpeed` range collapsed to its upper bound, the cardinal `windDirection`, the
  `{unitCode, value}` percentage objects, the 30-entry icon and 26-entry text tables, `current:
  false` (no pressure in the payload) and the out-of-coverage `404` naming the point. Bright Sky: the
  inclusive-`last_date` timestamp (1/25/49 rows for `last_date` = today/+1/+2, so the request uses
  `today + days`), the measured 10-day horizon (`sources[].last_record` clamped every probe to
  `2026-10-16T04:00Z`), the `dwd` default unit group (°C, km/h, hPa, mm) rather than `si`,
  `precipitation: null` dropping the row, the `CONDITIONS`/`ICONS` tables and the self-hosting path;
  the public instance answered `200`.
* **2026-10-06** — the four step-23 backends (now implemented, so their sections above replace the
  former "planned backends" table) re-verified against `src/provider/mod.rs` and their modules:
  met.no's 9-day horizon with ~52 h of hourly rows then 6-hourly (52 one-hour and 78 six-hour rows in
  the recorded fixture), the 41 base symbols including MET's double-`s` spellings and WMO 68/69 for
  sleet, the `Expires`/`If-Modified-Since` handshake, the `403` User-Agent policy and 20 req/s;
  Open-Meteo archive from 1940-01-01, keyless ERA5/ERA5-Land/IFS with `max_days: 0` and `history_days:
  30000`, the ~5-day ERA5 latency delegation to the forecast host; Open-Meteo marine 8 days,
  `cell_selection=sea`, the 25 km far-cell note and the required `current` block; Visual Crossing 15
  days on the free plan's 1 000 records/day (`queryCost` ≈ 1 + 24×hours + days, the default 3-day
  query ≈ 76), the `next<days>days`→`date1`/`date2` fallback, the 16-entry icon table and the payload
  `alerts[]` bound to `--provider visualcrossing`. All four registry rows carry `verified: 2026-10-06`.
* **2026-10-01** — QWeather re-verified for **v1** (the v7 endpoints are deprecated): live probes against the
  account host confirmed the paths, the measure-object shape, the UTC-instants-without-a-zone rule, the
  `hours` ≤ 240 and `days` ≤ 10 ceilings, the metric-only behaviour, the RFC 7807 errors and the `403
  Invalid Host` answer of the legacy shared domains. The section above records the findings; the backend
  implements them.
* **2026-09-30** — first full pass: all eight v1 backends and the four location/IP services checked
  against live documentation; registry rows in `src/provider/mod.rs` corrected and stamped
  `verified: 2026-09-30`. Notable corrections: SMHI's endpoint moved from `pmp3g` to `snow1g/version/1`
  (the old one is decommissioned and 404s); QWeather's city v7 APIs are deprecated with an EOL in 2027
  and the free allowance is 50 000 requests/month rather than a "Standard" plan; WeatherAPI's free tier
  is 100 000 calls/month with a 3-day forecast (not 1 000 000); WWO's `free/v1` path is undocumented
  and 403s while free keys work on `premium/v1`; PirateWeather has no `tz` parameter and no documented
  attribution obligation; OpenWeatherMap's data licence is ODbL with mandatory visible attribution.
  Pages that refused automated reads: `dev.qweather.com` and `www.qweather.com` (403 — facts taken from
  the `qwd/dev-site` sources that render them), `www.qweather.com/terms/developers-eula` (403),
  `opendata.smhi.se` documentation pages (client-rendered Docusaurus), and the Nominatim public
  instance (timeouts from the verification network).
