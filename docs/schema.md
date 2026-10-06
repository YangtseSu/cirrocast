<!--
SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Schemas — v1

Two documents have a versioned, consumer-facing shape: the `-f json` output and the configuration
file. Both carry a `schema_version` and both are additive within one version. The cache envelope has
a third, internal, version stamp (see the end of this file); it is not a contract, and it is
deliberately independent of the other two.

## Compatibility rule

| Change | Effect | Version |
|---|---|---|
| a new key appears | non-breaking; a consumer must ignore keys it does not know | unchanged |
| a key's value becomes nullable (or a nullable value becomes always-present) | non-breaking: the document already promises `null` for a value the provider did not report, so a consumer must handle `null` for every key | unchanged |
| a key is removed, renamed, retyped, or changes its unit | breaking | `schema_version` bumps |

The crate version, the JSON schema version and the config schema version move **independently**: a
release that adds a backend is not a schema change, and a schema change is not a crate release. A
`schema_version` bump is named in `CHANGELOG.md` and in the release notes.

## JSON output schema (v2)

Written by `--format json` (`src/render/json.rs`, `SCHEMA_VERSION = 2`). The same contract as a
JSON Schema (Draft 2020-12) is [`docs/schema/json-v2.json`](schema/json-v2.json); step 28 adds the
frozen `json-v1.json` beside it and the test that validates live output against both. The rules
that make the document scriptable, in the order they matter:

* every key is **always present**; a value the provider did not report is `null`, never an omitted
  key, so `jq -r '.current.uv_index'` cannot fail with a missing path;
* all numbers are **canonical metric/SI** with the unit in the key name (`temp_c`, `wind_kmh`,
  `precip_mm`, `pressure_hpa`, `visibility_km`), so `--units`, `--color` and `--width` do not change
  this format by a single byte;
* timestamps are ISO 8601 with the location's own offset (`2026-10-01T22:00:00+08:00`), except
  `attribution.retrieved_at`, which is UTC (`…Z`);
* `days` is ordered oldest first and starts at the first location-local date whose four parts all
  have a sample (today whenever the backend's series covers it, otherwise the first complete day);
  `day.parts` always carries all four parts;
* `condition.text` is translated by `--lang`, `condition.code` is the canonical WMO 4677 number and
  is not;
* the credits the data licences require travel with the document in `attribution`.

### Key index

The table is the complete key set, and `tests/render_json.rs` reads it and fails if the rendered
document disagrees with it — a rename, a removal or an undocumented addition cannot ship unnoticed.
`[]` marks the element of an array; `Null` says whether the value may be `null` (every key is always
present).

The table lists the keys of a *report* document. A failed slot in a multi-location run renders a
different, three-key object instead — `query` (string: the location argument as typed), `error`
(object) with `error.code` (integer: the mapped process exit code) and `error.message` (string: the
message exactly as it would print on stderr) — so a consumer detecting a failure never has to
inspect the report keys. The top-level type is decided by the argument count, not the content: one
location is a plain object, several are an array of those objects in argument order (a failed slot
is an object with an `error` key rather than a report).

<!-- schema-key-index:begin -->
| Key | Type | Unit | Null | Meaning |
|---|---|---|---|---|
| `schema_version` | integer | — | no | schema version of this document; `2` |
| `location` | object | — | no | the place the report is for |
| `location.name` | string | — | no | display name |
| `location.admin1` | string | — | yes | first-level administrative division |
| `location.country` | string | — | no | country name |
| `location.country_code` | string | — | yes | ISO 3166-1 alpha-2 |
| `location.lat` | number | degrees | no | latitude, WGS 84 |
| `location.lon` | number | degrees | no | longitude, WGS 84 |
| `location.timezone` | string | — | no | IANA name the times are expressed in |
| `location.elevation_m` | number | m | yes | elevation above sea level |
| `location.source` | string | — | no | `geocoder`, `offline`, `osm`, `coordinates`, `ip` or `station` |
| `location.station` | string | — | yes | ICAO identifier; `null` for every non-station location |
| `current` | object | — | yes | current conditions; `null` for an observation-less backend |
| `current.time` | string | — | no | observation time at the location's offset |
| `current.condition` | object | — | no | condition now |
| `current.condition.code` | integer | — | no | WMO 4677 code, 0..=99 |
| `current.condition.text` | string | — | no | condition text in the report's language |
| `current.temp_c` | number | °C | no | air temperature |
| `current.feels_like_c` | number | °C | yes | apparent temperature |
| `current.humidity_pct` | integer | % | yes | relative humidity, 0..=100; `null` when the backend does not report one |
| `current.precip_mm` | number | mm | no | precipitation in the last hour |
| `current.pressure_hpa` | number | hPa | no | sea-level pressure |
| `current.visibility_km` | number | km | yes | horizontal visibility |
| `current.wind_kmh` | number | km/h | no | wind speed |
| `current.wind_dir_deg` | integer | degrees | yes | direction the wind blows *from*, clockwise from north; `null` for a variable or calm wind |
| `current.wind_gust_kmh` | number | km/h | yes | gust speed |
| `current.cloud_cover_pct` | integer | % | yes | total cloud cover, 0..=100; `null` when the backend does not report one |
| `current.uv_index` | number | index | yes | UV index |
| `current.is_day` | boolean | — | no | whether the location is in daylight |
| `days` | array | — | no | forecast days, oldest first |
| `days[].date` | string | — | no | location-local calendar date (`YYYY-MM-DD`) |
| `days[].sunrise` | string | — | yes | local sunrise (`HH:MM`) |
| `days[].sunset` | string | — | yes | local sunset (`HH:MM`) |
| `days[].min_c` | number | °C | no | daily minimum temperature |
| `days[].max_c` | number | °C | no | daily maximum temperature |
| `days[].parts` | object | — | no | the four parts of the day |
| `days[].parts.morning` | object | — | no | 06:00–12:00 local |
| `days[].parts.morning.condition` | object | — | no | most significant condition in the part |
| `days[].parts.morning.condition.code` | integer | — | no | WMO 4677 code |
| `days[].parts.morning.condition.text` | string | — | no | condition text in the report's language |
| `days[].parts.morning.temp_c` | number | °C | no | representative temperature |
| `days[].parts.morning.feels_like_c` | number | °C | yes | apparent temperature |
| `days[].parts.morning.precip_mm` | number | mm | no | precipitation total for the part |
| `days[].parts.morning.precip_prob_pct` | integer | % | yes | precipitation probability |
| `days[].parts.morning.humidity_pct` | integer | % | yes | relative humidity |
| `days[].parts.morning.visibility_km` | number | km | yes | horizontal visibility |
| `days[].parts.morning.wind_kmh` | number | km/h | no | wind speed |
| `days[].parts.morning.wind_dir_deg` | integer | degrees | yes | direction the wind blows *from* |
| `days[].parts.noon` | object | — | no | 12:00–18:00 local |
| `days[].parts.noon.condition` | object | — | no | most significant condition in the part |
| `days[].parts.noon.condition.code` | integer | — | no | WMO 4677 code |
| `days[].parts.noon.condition.text` | string | — | no | condition text in the report's language |
| `days[].parts.noon.temp_c` | number | °C | no | representative temperature |
| `days[].parts.noon.feels_like_c` | number | °C | yes | apparent temperature |
| `days[].parts.noon.precip_mm` | number | mm | no | precipitation total for the part |
| `days[].parts.noon.precip_prob_pct` | integer | % | yes | precipitation probability |
| `days[].parts.noon.humidity_pct` | integer | % | yes | relative humidity |
| `days[].parts.noon.visibility_km` | number | km | yes | horizontal visibility |
| `days[].parts.noon.wind_kmh` | number | km/h | no | wind speed |
| `days[].parts.noon.wind_dir_deg` | integer | degrees | yes | direction the wind blows *from* |
| `days[].parts.evening` | object | — | no | 18:00–24:00 local |
| `days[].parts.evening.condition` | object | — | no | most significant condition in the part |
| `days[].parts.evening.condition.code` | integer | — | no | WMO 4677 code |
| `days[].parts.evening.condition.text` | string | — | no | condition text in the report's language |
| `days[].parts.evening.temp_c` | number | °C | no | representative temperature |
| `days[].parts.evening.feels_like_c` | number | °C | yes | apparent temperature |
| `days[].parts.evening.precip_mm` | number | mm | no | precipitation total for the part |
| `days[].parts.evening.precip_prob_pct` | integer | % | yes | precipitation probability |
| `days[].parts.evening.humidity_pct` | integer | % | yes | relative humidity |
| `days[].parts.evening.visibility_km` | number | km | yes | horizontal visibility |
| `days[].parts.evening.wind_kmh` | number | km/h | no | wind speed |
| `days[].parts.evening.wind_dir_deg` | integer | degrees | yes | direction the wind blows *from* |
| `days[].parts.night` | object | — | no | 00:00–06:00 local |
| `days[].parts.night.condition` | object | — | no | most significant condition in the part |
| `days[].parts.night.condition.code` | integer | — | no | WMO 4677 code |
| `days[].parts.night.condition.text` | string | — | no | condition text in the report's language |
| `days[].parts.night.temp_c` | number | °C | no | representative temperature |
| `days[].parts.night.feels_like_c` | number | °C | yes | apparent temperature |
| `days[].parts.night.precip_mm` | number | mm | no | precipitation total for the part |
| `days[].parts.night.precip_prob_pct` | integer | % | yes | precipitation probability |
| `days[].parts.night.humidity_pct` | integer | % | yes | relative humidity |
| `days[].parts.night.visibility_km` | number | km | yes | horizontal visibility |
| `days[].parts.night.wind_kmh` | number | km/h | no | wind speed |
| `days[].parts.night.wind_dir_deg` | integer | degrees | yes | direction the wind blows *from* |
| `mode` | string | — | no | `forecast` or `archive`: whether `days`/`current` are a forecast or a historical answer (`--date`, `--history`) |
| `air` | object | — | yes | air quality; `null` when the run did not ask (`--aqi`) or the best-effort fetch degraded |
| `air.time` | string | — | no | observation time at the location's offset |
| `air.source` | string | — | no | the air-quality source, e.g. `open-meteo` |
| `air.aqi_us` | integer | index | yes | US AQI, exactly as the source reports it |
| `air.aqi_european` | integer | index | yes | European AQI, exactly as the source reports it |
| `air.category` | object | — | no | the category of each index, derived from the raw number |
| `air.category.us` | string | — | yes | US category, e.g. `good`, `unhealthy-sensitive` |
| `air.category.european` | string | — | yes | European category, e.g. `fair`, `extremely-poor` |
| `air.pm2_5` | number | μg/m³ | yes | fine particulate matter (PM2.5) |
| `air.pm10` | number | μg/m³ | yes | coarse particulate matter (PM10) |
| `air.o3` | number | μg/m³ | yes | ground-level ozone |
| `air.no2` | number | μg/m³ | yes | nitrogen dioxide |
| `air.so2` | number | μg/m³ | yes | sulphur dioxide |
| `air.co` | number | μg/m³ | yes | carbon monoxide |
| `air.pollen` | object | — | yes | pollen forecast; `null` outside the source's pollen domain |
| `air.pollen.alder` | number | grains/m³ | yes | alder pollen; `null` when the source did not report it (a measured `0` stays `0.0`) |
| `air.pollen.birch` | number | grains/m³ | yes | birch pollen; `null` when the source did not report it |
| `air.pollen.grass` | number | grains/m³ | yes | grass pollen; `null` when the source did not report it |
| `air.pollen.mugwort` | number | grains/m³ | yes | mugwort pollen; `null` when the source did not report it |
| `air.pollen.olive` | number | grains/m³ | yes | olive pollen; `null` when the source did not report it |
| `air.pollen.ragweed` | number | grains/m³ | yes | ragweed pollen; `null` when the source did not report it |
| `air.units` | object | — | no | the units the air numbers are in |
| `air.units.pollutants` | string | — | no | `μg/m³` |
| `air.units.pollen` | string | — | no | `grains/m³` |
| `marine` | object | — | yes | waves, swell and sea-surface temperature; `null` unless the run asked (`--marine`) and the best-effort fetch succeeded |
| `marine.time` | string | — | no | observation time, at the location's offset |
| `marine.source` | string | — | no | the marine source id, e.g. `open-meteo-marine` |
| `marine.wave_height_m` | number | m | yes | significant wave height |
| `marine.wave_direction_deg` | integer | ° | yes | direction the waves travel from, clockwise from north |
| `marine.wave_period_s` | number | s | yes | peak wave period |
| `marine.swell_wave_height_m` | number | m | yes | swell wave height |
| `marine.sea_surface_temp_c` | number | °C | yes | sea-surface temperature |
| `marine.sampled` | object | — | no | the sea cell the answer was sampled at |
| `marine.sampled.lat` | number | ° | no | latitude of the sampled cell |
| `marine.sampled.lon` | number | ° | no | longitude of the sampled cell |
| `marine.sampled.distance_km` | number | km | no | great-circle distance from the requested point |
| `marine.sampled.far` | boolean | — | no | whether that distance exceeds the model's far-cell threshold (`25 km`), so a consumer need not know the constant |
| `marine.days` | array | — | no | daily wave summary, oldest first |
| `marine.days[].date` | string | — | no | location-local calendar date (`YYYY-MM-DD`) |
| `marine.days[].wave_height_max_m` | number | m | yes | highest significant wave height that day |
| `marine.days[].wave_period_max_s` | number | s | yes | longest wave period that day |
| `marine.days[].wave_direction_dominant_deg` | integer | ° | yes | dominant wave direction that day |
| `astro` | object | — | yes | moon and sun, computed locally; `null` unless the run asked (`--moon` or `--format moon`) |
| `astro.computed_at` | string | — | no | when the block was computed, UTC |
| `astro.moon` | object | — | no | the Moon |
| `astro.moon.phase` | string | — | no | phase name in the report's language, e.g. `Waxing Crescent` |
| `astro.moon.phase_key` | string | — | no | stable slug: `new`, `waxing-crescent`, `first-quarter`, `waxing-gibbous`, `full`, `waning-gibbous`, `last-quarter`, `waning-crescent` |
| `astro.moon.illuminated_fraction` | number | 0..1 | no | illuminated fraction of the disc, geocentric |
| `astro.moon.age_days` | number | days | no | days since the preceding New Moon |
| `astro.moon.moonrise` | string | — | yes | moonrise at the location's offset; `null` when the event does not happen on the local day |
| `astro.moon.moonset` | string | — | yes | moonset at the location's offset; `null` when the event does not happen on the local day |
| `astro.moon.next` | array | — | no | the next four phase instants after the run's clock, chronological |
| `astro.moon.next[].phase` | string | — | no | phase name in the report's language |
| `astro.moon.next[].phase_key` | string | — | no | the same slug vocabulary as `astro.moon.phase_key` |
| `astro.moon.next[].at` | string | — | no | the instant, at the location's offset |
| `astro.sun` | object | — | no | the Sun |
| `astro.sun.sunrise` | string | — | yes | sunrise at the location's offset; `null` when the Sun does not rise |
| `astro.sun.sunset` | string | — | yes | sunset at the location's offset; `null` when the Sun does not set |
| `astro.sun.daylight_secs` | integer | seconds | yes | daylight span; `86400` on a polar day, `0` on a polar night, `null` when unknown |
| `astro.sun.polar` | string | — | yes | `day` or `night` inside the polar circles, else `null` |
| `astro.sun.source` | string | — | no | `provider` when the backend sent the times, `local` when they were computed here |
| `normals` | object | — | yes | the month's climate normal from the station nearest the location; `null` unless the run asked (`--normals` or `--format normals`) and the best-effort fetch succeeded |
| `normals.station` | string | — | no | the station's identifier, e.g. `CHM00054511` |
| `normals.station_name` | string | — | no | the station's name as the upstream catalog spells it, e.g. `BEIJING, CH` |
| `normals.distance_km` | number | km | no | great-circle distance from the requested point to the station |
| `normals.period` | string | — | no | the reference period the values are averaged over, as configured, e.g. `1991-2020` |
| `normals.month` | integer | — | no | the calendar month the values are for, `1`–`12` |
| `normals.temp_mean_c` | number | °C | no | mean of the month's mean daily temperatures over the period |
| `normals.temp_max_c` | number | °C | no | mean of the month's mean daily maxima |
| `normals.temp_min_c` | number | °C | no | mean of the month's mean daily minima |
| `normals.precip_mm` | number | mm | no | mean of the month's precipitation totals |
| `normals.years` | integer | years | no | how many years of the period contributed; never below 20, because a thinner record yields no block |
| `capabilities` | object | — | yes | what the answering backend offers; `null` for an unknown backend |
| `capabilities.current` | boolean | — | no | current conditions available |
| `capabilities.hourly` | boolean | — | no | hourly data available |
| `capabilities.daily` | boolean | — | no | daily data available |
| `capabilities.alerts` | boolean | — | no | weather alerts available |
| `capabilities.max_days` | integer | days | no | longest forecast the backend serves; `0` = observations only |
| `capabilities.history_days` | integer | days | no | how far back the backend can answer; `0` = forecast only, and `max_days: 0` with a positive value is an archive-only backend |
| `capabilities.marine` | boolean | — | no | whether the backend serves marine data (a supplementary source, never a forecast chain entry) |
| `capabilities.requires_key` | boolean | — | no | whether the backend needs an API key |
| `capabilities.key_env` | string | — | yes | the environment variable that supplies the key |
| `capabilities.locations` | object | — | no | location forms the backend accepts |
| `capabilities.locations.city` | boolean | — | no | resolved place names |
| `capabilities.locations.station` | boolean | — | no | METAR station identifiers |
| `capabilities.locations.lat_lon` | boolean | — | no | raw coordinates |
| `attribution` | object | — | no | provenance and the required credits |
| `attribution.provider` | string | — | no | registry id of the backend, e.g. `open-meteo` |
| `attribution.url` | string | — | no | endpoint the answer came from, without query string or key |
| `attribution.notice` | string | — | yes | data-licence credit; `null` when the backend has none |
| `attribution.location_notice` | string | — | yes | place-data credit; `null` when the source asks for none |
| `attribution.retrieved_at` | string | — | no | when the data was fetched or read from the cache, UTC |
| `alerts` | array | — | no | severe-weather warnings in force, strongest first |
| `alerts[].id` | string | — | no | the source's own identifier |
| `alerts[].source` | string | — | no | `nws`, `meteoalarm`, `qweather`, `hko`, `wmoswic`, `fpas` or `visualcrossing` |
| `alerts[].event` | string | — | no | event name, e.g. `Tornado Warning` |
| `alerts[].severity` | string | — | no | `unknown`, `minor`, `moderate`, `severe` or `extreme` |
| `alerts[].urgency` | string | — | no | `unknown`, `past`, `future`, `expected` or `immediate` |
| `alerts[].certainty` | string | — | no | `unknown`, `unobserved`, `possible`, `unlikely`, `likely` or `observed` |
| `alerts[].onset` | string | — | yes | when the event starts, at its own offset |
| `alerts[].expires` | string | — | yes | when the alert stops being live (`ends` when present, else CAP `expires`), at its own offset |
| `alerts[].areas` | array | — | no | affected areas, de-duplicated across `info` blocks |
| `alerts[].headline` | string | — | no | one-line summary |
| `alerts[].description` | string | — | yes | full description |
| `alerts[].instruction` | string | — | yes | what the reader is told to do |
| `alerts[].sender` | string | — | yes | issuing agency |
| `alert_credits` | array | — | no | the alert sources' required credit lines; empty when none apply |
<!-- schema-key-index:end -->

### Worked example

`cirrocast Beijing --days 1 --lang en-US -f json` on 2026-10-01, one of the ten fuzzy `Beijing`
candidates chosen by the documented ranking (the choice and its note go to stderr, so the document
stays pipeable):

```json
{
  "schema_version": 2,
  "location": {
    "name": "Beijing",
    "admin1": "Beijing Municipality",
    "country": "China",
    "country_code": "CN",
    "lat": 39.9075,
    "lon": 116.39723,
    "timezone": "Asia/Shanghai",
    "elevation_m": 49.0,
    "source": "geocoder",
    "station": null
  },
  "current": {
    "time": "2026-10-01T22:00:00+08:00",
    "condition": {
      "code": 0,
      "text": "Clear sky"
    },
    "temp_c": 17.3,
    "feels_like_c": 14.2,
    "humidity_pct": 24,
    "precip_mm": 0.0,
    "pressure_hpa": 1022.2,
    "visibility_km": 17.48,
    "wind_kmh": 3.4,
    "wind_dir_deg": 245,
    "wind_gust_kmh": 13.3,
    "cloud_cover_pct": 0,
    "uv_index": 0.0,
    "is_day": false
  },
  "days": [
    {
      "date": "2026-10-01",
      "sunrise": "06:10",
      "sunset": "17:57",
      "min_c": 11.1,
      "max_c": 22.9,
      "parts": {
        "morning": {
          "condition": {
            "code": 0,
            "text": "Clear sky"
          },
          "temp_c": 16.8,
          "feels_like_c": 13.1,
          "precip_mm": 0.0,
          "precip_prob_pct": 0,
          "humidity_pct": 16,
          "visibility_km": 17.38,
          "wind_kmh": 3.2,
          "wind_dir_deg": 273
        },
        "noon": {
          "condition": {
            "code": 0,
            "text": "Clear sky"
          },
          "temp_c": 22.9,
          "feels_like_c": 18.4,
          "precip_mm": 0.0,
          "precip_prob_pct": 0,
          "humidity_pct": 9,
          "visibility_km": 17.44,
          "wind_kmh": 7.8,
          "wind_dir_deg": 312
        },
        "evening": {
          "condition": {
            "code": 0,
            "text": "Clear sky"
          },
          "temp_c": 18.2,
          "feels_like_c": 14.5,
          "precip_mm": 0.0,
          "precip_prob_pct": 0,
          "humidity_pct": 21,
          "visibility_km": 17.48,
          "wind_kmh": 6.4,
          "wind_dir_deg": 209
        },
        "night": {
          "condition": {
            "code": 0,
            "text": "Clear sky"
          },
          "temp_c": 12.7,
          "feels_like_c": 8.9,
          "precip_mm": 0.0,
          "precip_prob_pct": 0,
          "humidity_pct": 29,
          "visibility_km": 17.34,
          "wind_kmh": 6.7,
          "wind_dir_deg": 301
        }
      }
    }
  ],
  "air": null,
  "astro": null,
  "alerts": [],
  "capabilities": {
    "current": true,
    "hourly": true,
    "daily": true,
    "alerts": false,
    "max_days": 16,
    "requires_key": false,
    "key_env": null,
    "locations": {
      "city": true,
      "station": false,
      "lat_lon": true
    }
  },
  "attribution": {
    "provider": "open-meteo",
    "url": "https://api.open-meteo.com/v1/forecast",
    "notice": "Open-Meteo.com (CC BY 4.0)",
    "location_notice": "Location data based on GeoNames (CC-BY-4.0) via Open-Meteo — https://open-meteo.com/",
    "retrieved_at": "2026-10-01T14:09:33Z"
  },
  "alert_credits": []
}
```

## Config schema (v2)

`$XDG_CONFIG_HOME/cirrocast/config.toml` (`~/.config/cirrocast/config.toml`), parsed by
`src/config/mod.rs` (`CURRENT_SCHEMA_VERSION = 2`), documented key by key in the README. Precedence,
highest first: **command line flag → `CIRROCAST_*` environment variable → this file → built-in
default**.

Migration hooks:

* a document without `schema_version` is read as the current version (the key was added with the
  first schema, and a hand-written file may leave it out);
* version `1` migrates to `2` by stamping the version: the tables the bump added (`[locations]`,
  `[templates]`) are optional, and absent means empty;
* version `0` is refused with the `cirrocast config init --force` hint; a version **above** the
  supported one is refused with an error naming both, rather than guessed at;
* a key this build does not know is **ignored on load** (so a document written by a newer release
  keeps working) but rejected by `config validate`, which exists to catch typos;
* `Config::migrate` is the single place a future version rewrite belongs, and it must come with a
  migration test from the previous version;
* adding a key with a built-in default is additive and does **not** bump the version: an older
  document simply does not carry it. Changing the meaning, the type or the validity of an existing
  key does, and the bump is named in the changelog.

### Worked example

The document `cirrocast config init` writes — every key with its default; `config set` rewrites it
in canonical form without the comments:

```toml
# cirrocast configuration.
#
# Every key is optional: a missing key falls back to the built-in default shown
# here. `cirrocast config show` prints the effective values, `config get <KEY>`
# reads one and `config set <KEY> <VALUE>` edits this file in place (which
# rewrites it in canonical form, dropping comments). Values given on the command
# line, or through the matching `CIRROCAST_*` variable, win over this file.

schema_version = 2

[defaults]
provider = "open-meteo"  # id, comma separated chain, or "auto" (the keyless chain)
format = "art-table"     # art-table | one-line | plain | json | dumb | alerts | aqi | moon | normals,
                         # or a one-line preset: full | minimal | short | default | uv | sun,
                         # or a [templates] key
units = "metric"         # metric | us | uk
days = 3                 # 0..=14; each provider clamps to its own maximum
language = "auto"        # "auto" or a BCP-47 tag such as "en-US", "zh-CN"
normals = false          # fetch the climate-normals comparison on every run (--normals forces it)

[location]
default = ""             # "Beijing", ":Beijing", "@39.9,116.4", "~Tsinghua", or "@home" for an
                         # alias below; empty = ask for the IP location
pick = "auto"            # auto (ask on a terminal when a name has several candidates) | never

[locations]
# @NAME aliases for the location argument. Values are any location argument, including another
# alias; chains are expanded with cycle detection.
# home = "@39.9,116.4"
# work = ":Shanghai"

[templates]
# Named one-line templates for `--format <NAME>` and `--template @NAME`. A value is a literal
# %-token template; an unknown token is an error, not printed literally.
# compact = "%c%t"

[geo]
strategy = "auto"        # auto (bundled GeoNames table first, network on a miss) | bundled | network
data = "auto"            # which city table answers: auto (user table when present) | bundled | user
update = "off"           # off | check: a once-a-day note when the table is older than the interval
update_interval_days = 90
update_url = ""          # source for `cirrocast location update-data`; empty = the official GeoNames dump

[units]
# Per-quantity overrides on top of `defaults.units`. Remove the `#` to pin one
# quantity; an absent key follows the unit system.
# temp = "c"        # c | f
# wind = "kmh"      # kmh | mph | mps | knots
# pressure = "hpa"  # hpa | inhg | mmhg
# distance = "km"   # km | mi
# precip = "mm"     # mm | in

[network]
timeout_secs = 15        # 1..=300
retries = 3              # 0..=10
proxy = ""               # e.g. "http://127.0.0.1:8080"; empty = connect directly
nominatim_url = ""       # Nominatim base URL for `~name` searches; empty = the public OpenStreetMap service
offline = "off"          # off | weather (cache-only forecast) | geo (bundled names, live weather) | all

[cache]
enabled = true
weather_ttl_secs = 600       # 10 minutes
ip_ttl_secs = 86400          # 24 hours; a larger value is capped there (ipapi.co's terms)
geocode_ttl_secs = 2592000   # 30 days

[render]
color = "auto"           # auto | always | never
width = 0                # 0 = detect from the terminal, or 40..=500 columns

[alerts]
enabled = true                # fetch warnings automatically when a source covers the location
severity_threshold = "minor"  # unknown | minor | moderate | severe | extreme
sources = ["auto"]            # ["auto"] (coverage-selected) or the wired ids: nws, meteoalarm,
                              # qweather, hko, wmoswic, fpas ("visualcrossing" is reserved for
                              # step 23 and rejected until then)
fpas_url = ""                 # FOSS Public Alert Server base URL; empty = https://alerts.kde.org
cache_ttl_secs = 300          # 5 minutes

[air]
index = "us"             # us | european: the AQI scale that drives the panel colour and %q

[normals]
period = "1991-2020"     # the reference window the normal is averaged over: two four-digit years
max_distance_km = 60     # farthest NOAA NCEI station that still answers, 1..=500

[status]
placeholder = "n/a"      # `cirrocast status` prints this when it has no reading to show

[providers.metar]
station = ""             # default ICAO identifier, e.g. "ZBAA"

[providers.qweather]
host = ""                # API host from https://console.qweather.com/setting,
                         # e.g. "https://<account-id>.re.qweatherapi.com"
```

API keys are **not** part of this schema: they live in `keys.toml` (mode `0600`) or in the
`CIRROCAST_<PROVIDER>_KEY` environment variables, so `config.toml` is safe to paste into a bug
report.

## Cache envelope (internal, v1)

Cache files carry `cache_schema_version` (`CACHE_SCHEMA_VERSION = 1` in `src/cache.rs`) beside the
cached body, its key and its TTL. The stamp is **not** a consumer contract: a mismatch is a cache
miss, never an error, and changing the envelope invalidates cached answers rather than a
configuration or the JSON output. It is independent of both versions above precisely so that a
config migration does not throw away cache entries and a cache change does not force one.
