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

The file is the one `docs/plans/23-docs-and-guides.md` promises; it was authored in step 10 because the
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

## At a glance

### Backends

| id | Key | Free tier (verified 2026-09-30) | Coverage | Granularity | Horizon | Status |
|---|---|---|---|---|---|---|
| `open-meteo` | none | 10 000 calls/day, 5 000/hour, 600/minute; non-commercial | global | hourly | 16 days | implemented (step 06) |
| `smhi` | none | no published quota; fair-use rules | Nordics and adjacent seas (SNOW1gv1 polygon) | 1 h near-term, 6 h / 12 h later | ≈10 days | implemented |
| `metar` | none | 100 requests/minute | worldwide stations | per observation (≈hourly) | observations only | step 11 |
| `openweathermap` | `CIRROCAST_OPENWEATHERMAP_KEY` | 60 calls/minute, 1 000 000 calls/month | global | 3-hourly | 5 days (40 slots) | implemented |
| `weatherapi` | `CIRROCAST_WEATHERAPI_KEY` | 100 000 calls/month; 3-day forecast (paid: 14) | global | hourly | 3 days free | implemented |
| `worldweatheronline` | `CIRROCAST_WORLDWEATHERONLINE_KEY` | 100 requests/day (free terms; a second page says 500/month) | global | 3-hourly (`tp=3`) | 5 days per FAQ, 14 per endpoint | implemented |
| `pirateweather` | `CIRROCAST_PIRATEWEATHER_KEY` | 10 000 calls/month (≈$2/month → 20 000) | global | hourly + 7 daily | 48 h hourly (`extend` 168 h), 7 days daily | implemented |
| `qweather` | `CIRROCAST_QWEATHER_KEY` | first 50 000 requests/month at ¥0; QPM 3 000 | global | hourly (up to 240 h) | 10 days (v1) | implemented |

### Obligations that reach the rendered output

| id | Licence | Credit required | Cache ceiling | Extra duty |
|---|---|---|---|---|
| `open-meteo` | CC BY 4.0 | `<a href="https://open-meteo.com/">Weather data by Open-Meteo.com</a>`; a link next to displayed data | none published | geocoding adds "Location data based on GeoNames" |
| `smhi` | CC BY 4.0 SE | name SMHI as the source and state modifications; `Källa: SMHI` is the conventional rendering, not SMHI's own wording | none published; caching encouraged | — |
| `metar` | US Government work (public domain) | no mandated string; NWS asks that derived works not claim NWS endorsement and that predominantly-NWS works carry the 17 U.S.C. § 403 notice | none published | NWS name/logo are trademarks |
| `openweathermap` | ODbL 1.0 | "Weather data provided by OpenWeather" + link to https://openweathermap.org/ + the OpenWeather logo, visible where the data appears | none published (10-minute model refresh) | share-alike only if we ever publish an adapted database |
| `weatherapi` | proprietary (Zoomash Ltd) | free keys: credit WeatherAPI.com by name or logo; the docs suggest `Powered by <a href="https://www.weatherapi.com/">WeatherAPI.com</a>` | current 60 min, forecast 24 h | mandatory end-user disclaimer; no resale; one key per app |
| `worldweatheronline` | proprietary (Zoomash Ltd) | free keys: "Weather Data by WorldWeatherOnline.com" | current 60 min, forecast 24 h | mandatory end-user disclaimer; no resale |
| `pirateweather` | proprietary (PirateWeatherAPI) | none documented | `Cache-Control: max-age=900` is sent | no multi-account quota circumvention; warranty disclaimer |
| `qweather` | proprietary (QWeather Developers License) | name QWeather + https://www.qweather.com; recommended "Weather service by QWeather" | real-time 10–30 min, hourly 30–60 min, daily 1–6 h (guidance) | GeoAPI data must not be bulk-cached or indexed; weather warnings must reproduce `refer.sources` |

### Status-code behaviour (the chain contract, applied per upstream)

`fetch_chain` falls through to the next backend only for transport/upstream failures; everything else
is the user's answer. Concretely, for the backends above:

| Upstream answer | Mapped to | Effect |
|---|---|---|
| `401` / `403` that means "bad or missing credential" | `Error::MissingKey` / `InvalidKey` (exit 6) | chain stops; message names `cirrocast key set <id>` |
| `429` | `Error::Upstream` (exit 3) | chain continues; `Retry-After` honoured when present and clamped (PirateWeather, ipwho.is publish it; WeatherAPI, ipapi.co and Open-Meteo do not) |
| `5xx`, timeouts, connection failures | `Error::Upstream` / `Error::Network` (exit 3) | chain continues |
| `400` with an invalid-parameter body | `Error::Usage` (exit 2) where the fault is ours; `Error::Upstream` where the body is a provider refusal | per provider note |
| `404` "no such location" (QWeather, SMHI out-of-area) | `Error::Upstream` for SMHI (so `auto` falls through) and for QWeather's "no such location" | see each section |
| `200` with an error envelope (`{"error":true,…}`, `{"success":false,…}`) | `Error::Upstream(reason)` | Open-Meteo, ipwho.is, ipapi.co all do this |

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

**Response fields consumed.** Top level: `latitude`, `longitude`, `utc_offset_seconds`, `timezone`,
`elevation`, `current_units`, `current`, `hourly_units`, `hourly`, `daily_units`, `daily`. `hourly` and
`daily` are column objects (`{ "time": [...], "<variable>": [...] }`); `current` is a flat object whose
`interval` is 900 s. Units are echoed back per request in `*_units`.

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
`Data: Open-Meteo.com (CC BY 4.0)` is the line the renderers print today.

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

Keyless, station-based observations. Implemented in step 11, listed here because it shares the
attribution and error surface.

**Endpoints** (verified 2026-09-30, all GET, no key)

| Purpose | URL | Parameters |
|---|---|---|
| Observations | `https://aviationweather.gov/api/data/metar` | `ids` (ICAO, comma-separated; also `@WA` state prefixes), `bbox`, `format` (`raw` default; `json` for us), `taf=true` to embed the station's TAF, `hours` (default 1.5), `date` |
| Forecasts | `https://aviationweather.gov/api/data/taf` | `ids`, `bbox`, `format`, `metar=true`, `time` (`valid`/`issue`), `date` |
| Station metadata | `https://aviationweather.gov/api/data/stationinfo` | `ids`, `bbox`, `format` |

**Response fields consumed.** METAR JSON is a bare array; per object: `icaoId`, `receiptTime`,
`obsTime`, `reportTime`, `temp`, `dewp`, `wdir`, `wspd`, `wgst`, `visib`, `altim`, `slp`, `wxString`,
`clouds[{cover,base}]`, `rawOb`, `lat`, `lon`, `elev`, `name`, `fltCat`. Station info adds `iataId`,
`faaId`, `wmoId`, `site`, `state`, `country`. TAF objects carry `rawTAF`, `issueTime`, `validTimeFrom`,
`validTimeTo` and a `fcsts[]` change-group list.

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

**Client notes.** `metar_id` does not exist (the identifier is `icaoId`); `wmoId`/`id`/`cover`/`rawTaf`
are live but undocumented — parse defensively; `metar?ids=ZZZZ` answers 204 while `taf?ids=ZZZZ`
answers 200 with `[]`; a 504 can be transient where the same request later returns 400; non-ICAO ids
are 400, unknown products 404.

**Unverified.** The 400 body shape (documented as `{"status":"error","error":"…"}` but never captured
live); an observed 429/403; CORS headers.

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

**Response fields consumed.** Current: `main.temp/feels_like/humidity/pressure`, `weather[0].id`,
`wind.speed/deg/gust`, `clouds.all`, `visibility` (metres, "maximum value … is 10 km"), `rain.1h`,
`snow.1h`, `sys.sunrise/sunset`, `dt`, `timezone` (shift in seconds, **not** an IANA name), `name`.
Forecast: `list[].dt/pop/main.*/weather[0].id/wind.*/rain.3h/snow.3h/sys.pod` plus the `city` block
(`name`, `timezone`, `sunrise`, `sunset` — today's times only, not per day).

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

**Response fields consumed.** `location.{name,region,country,lat,lon,tz_id,localtime_epoch,localtime}`;
`current.{temp_c,feelslike_c,humidity,pressure_mb,wind_kph,wind_degree,wind_dir,gust_kph,vis_km,uv,
precip_mm,is_day,condition.code,last_updated_epoch}`;
`forecast.forecastday[].{date,date_epoch,day.{maxtemp_c,mintemp_c,avgtemp_c,maxwind_kph,totalprecip_mm,
avghumidity,uv,condition},astro.{sunrise,sunset,moonrise,moonset,moon_phase,moon_illumination},
hour[].{time,time_epoch,temp_c,feelslike_c,chance_of_rain,precip_mm,wind_kph,gust_kph,vis_km,uv,is_day,
condition.code}}`; `alerts.alert[]` with `alerts=yes`.

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
model's UV field; and the full 53-code table is unit-tested. Credit line:
`WeatherAPI.com (free-tier attribution) — https://www.weatherapi.com/`. The free tier's 3-day
horizon is the registry's `max_days`; a `403` keeps the `Upstream` taxonomy (exit 3) because it
carries quota and plan refusals, while a `401` becomes exit 6.

**Client notes.** Condition codes are **53 codes over 1000–1282** (not 1000–1087); the canonical list is
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

**Response fields consumed.** `data.{request,nearest_area,current_condition,weather,alerts}`;
`current_condition[0].{temp_C,FeelsLikeC,weatherCode,weatherDesc[0].value,humidity,windspeedKmph,
winddirDegree,pressure,precipMM,visibility,uvIndex,observation_time}`;
`weather[].{date,maxtempC,mintempC,uvIndex,astronomy.{sunrise,sunset,moonrise,moonset,moon_phase},
hourly[].{time,tempC,FeelsLikeC,weatherCode,weatherDesc[0].value,windspeedKmph,winddirDegree,precipMM,
humidity,pressure,cloudcover,chanceofrain,windgustKmph,visibility}}`.

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
49-code set with separate day/night icons
(<https://www.worldweatheronline.com/feed/wwoConditionCodes.txt>). Error mapping observed live: 400
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

**Response fields consumed.** Top level `latitude`, `longitude`, `timezone`, `offset`, `elevation`,
`currently`, `minutely`, `hourly`, `daily`, `alerts`, `flags`; per block `time` (UNIX UTC seconds),
`summary`, `icon`, `precipIntensity`, `precipProbability`, `precipType`, `temperature`,
`apparentTemperature`, `dewPoint`, `humidity`, `pressure`, `windSpeed`, `windGust`, `windBearing`,
`cloudCover`, `visibility`, `uvIndex`; daily adds `sunriseTime`, `sunsetTime`, `moonPhase`,
`precipAccumulation`, `temperatureHigh/Low`. Under `units=si`, `precipAccumulation` is **centimetres**
and `precipIntensity` is mm/h of liquid water; `humidity`/`cloudCover`/`precipProbability` are 0–1;
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

The host is **per account** (`<account-id>.re.qweatherapi.com` in the verified account) and is part of
the authentication ("even if a developer's credentials are leaked, an attacker cannot request data
without knowing the API Host"). The legacy shared domains (`api.qweather.com`, `devapi.qweather.com`,
`geoapi.qweather.com`) answer `403` with `"title": "Invalid Host"` — probed with a valid key on
2026-10-01 — so `<host>` has no default and comes from `[providers.qweather].host`.

**Response shape.** Every value is a **measure object** (`{"value": 10.96, "unit": "°C"}`), the
timestamps are **UTC instants** (`2026-10-01T00:00Z`, minutes form — valid ISO 8601, not strict
RFC 3339), and the payload carries **no time zone and no observation time**:

* `current`: `condition{text,code}`, `temperature`, `feelsLike`, `humidity` (0–1), `wind{direction{degree,compass},speed{value,unit:"m/s"},scale}`, `windGust`, `precipitation{amount{mm},intensity{mm/h},type}`, `pressure{hPa}`, `visibility{m}`, `dewPoint`, `cloudCover` (0–1), `uvIndex`, plus `metadata{tag,attributions[]}`.
* `hours[]`: the same fields plus `forecastTime`, `precipitation.probability` (percent) and per-hour `uvIndex`.
* `days[]`: `forecastStartTime`/`forecastEndTime` (UTC instants of the **location-local midnight** boundaries — `16:00Z` for Beijing, `04:00Z` for New York), `astro{...}` (sunrise/sunset as UTC instants), `temperatureMax/Min/Avg`, `uvIndexMax`, `daytime`/`nighttime` sub-blocks with their own condition/temperature/wind/precipitation.
* `metadata.attributions` lists the attribution page upstream asks to be shown.

**Auth.** `X-QW-Api-Key: <key>` header (the `key=` query form also works; never both). The API KEY
*signature* flow is retired, and API-KEY volume will be limited from 2027 (the docs give both
2027-01-01 and 2027-02-01).

**Limits.** Pay-as-you-go with the **first 50 000 requests/month at ¥0** (no "Standard" free plan);
QPM 3 000 for pay-as-you-go, 50 000+ for Premium. Non-2xx responses are not billed, but sustained
invalid traffic can suspend the account. Recommended cache ages: real-time 10–30 min, hourly
30–60 min, daily 1–6 h.

**Coverage and granularity.** Global ("200+ countries or regions … over 500,000 cities"); hourly up
to 240 hours and daily up to 10 days on v1 (both probed). The v1 endpoints are metric-only: `unit=i`
is ignored (probed).

**Attribution and licence.** Required regardless of plan: name "QWeather" plus the URL
`https://www.qweather.com`; no logo is required by the docs (the EULA text itself was 403-blocked, so
that reading is docs-only). Weather-warning data must reproduce `refer.sources` verbatim, and GeoAPI
data must not be bulk-cached or indexed — this backend does not call GeoAPI.

**Errors.** RFC 7807 `application/problem+json` with `error.status/type/title/detail/invalidParams`;
a bad key is `401` (`#unauthorized`), a bad parameter or location `400`
(`#invalid-parameter`). `403` covers no-credit, overdue, invalid host and permission refusals.

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

Credit line: `QWeather — https://www.qweather.com/`. A missing host is `Error::Config` (exit 4) with
`set providers.qweather.host (see cirrocast provider info qweather)`.

**Unverified.** The Developers License text (403) and every rendered `dev.qweather.com` page (403 —
the v1 facts above come from live responses and the public docs repository); the exact QPM scope
(per project vs per account); whether `days[]`'s `daytime`/`nighttime` windows are fixed local
windows (observed 07:00–19:00 local, which is not the solar day).

## Location and IP services

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

**Unverified.** The public instance was unreachable from the verification network (timeouts, no status),
so the live response shape is documented-only.

### ipwho.is (keyless, primary IP locator)

`GET https://ipwho.is/` (caller's IP) or `https://ipwho.is/<ip>`; optional `fields`, `output`, `lang`.
Free tier: "1,000 requests per day per client IP address", no key, HTTPS included, commercial use
allowed. Over-limit is `429` **with** a `Retry-After` header ("Access will be restored automatically
after 24 hours"). Application errors ride on HTTP 200 as `{"success":false,"message":…}` — a 2xx status
is not success — while a non-IP path segment is a 404. The terms grant no redistribution and require no
credit line; the privacy disclosure naming the service is our own practice
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

## Planned backends (step 19)

Not implemented; the facts below are the measurements recorded in `docs/plans/19-more-providers.md`
(2026-09-30) and are repeated here so this file stays the single provider index.

| id | Key | Free tier | Coverage | Horizon | Notes |
|---|---|---|---|---|---|
| `met-no` | none | 20 req/s per application | global | 9 days | requires an identifying `User-Agent` and `If-Modified-Since`/`304` handling per MET's terms; CC BY 4.0 |
| `visualcrossing` | BYOK | 1 000 records/day | global | 15 days | supplies its own alerts; per-account terms, local display only |
| `open-meteo-archive` | none | shared with Open-Meteo | global | historical | covers 1940-01-01 onward; ERA5 latency window applies |
| `open-meteo-marine` | none | shared with Open-Meteo | global coastal | 8 days | waves/swell/sea-surface; supplementary `--marine` fetch |

## Re-verification log

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
