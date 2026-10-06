<!--
SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Climate-normals fixtures

Responses of the NOAA NCEI Access Services API (`https://www.ncei.noaa.gov/access/services/`),
used by `tests/normals.rs` and the unit tests of `src/normals/ncei.rs`. Nothing here is fetched by a
test run.

The Global Summary of the Month (GSOM) is US Government material and therefore public domain; the
recordings keep the upstream's `LicenseRef-US-Government-Public-Domain` annotation in `REUSE.toml`,
and the rendered credit says the normals are computed from it.

## Recorded

Fetched **2026-10-06** with the exact request the adapter sends (`limit=5`; the box the adapter
derives from the point and the configured radius; `format=json&units=metric` with the
`dataTypes=TAVG,TMAX,TMIN,PRCP` projection on the values request), so a test can assert the URL text
verbatim. The bounding box is `maxLat,minLon,minLat,maxLon` — the **north-west corner first**; the
documented-looking `SW,NE` order answers HTTP 500.

| File | Request | What it pins |
|---|---|---|
| `search-beijing.json` | `GET …/search/v1/data?dataset=global-summary-of-the-month&bbox=41.0000,115.4000,39.0000,117.4000&limit=5` | Three stations in the box and the fact that `results[]` is **not** ordered by distance, measured from the recorded box's centre (40.0, 116.4): `CHM00054527` (119.7 km) first, `CHM00054405` (89.4 km) second, `CHM00054511` (12.4 km from that centre; 11.08 km from the report fixture's own 39.9042, 116.4074) **last** — the client must measure every entry. Each entry carries its file `name` (`CHM00054511.csv`), its `centroid` as `[lon, lat]`, and a nested `stations[]` list whose `dataTypes[]` records name the covered datatypes |
| `gsom-54511-1991-2020.json` | `GET …/data/v1?dataset=global-summary-of-the-month&stations=CHM00054511&startDate=1991-01-01&endDate=2020-12-31&format=json&units=metric&dataTypes=TAVG,TMAX,TMIN,PRCP` | 277 rows across 24 years with real gaps. October has 23 rows but only 22 carry all four values (`2020-10` has `PRCP` and no temperatures), so October's normal is the mean over the 22 complete rows: TAVG 14.1 °C, TMAX 19.3636 °C, TMIN 8.8045 °C, PRCP 29.1091 mm |
| `search-madison.json` | `GET …/search/v1/data?dataset=global-summary-of-the-month&bbox=43.6136,-90.1413,42.5326,-88.6611&limit=5` | The exact box the adapter derives for the default 60 km at 43.0731, −89.4012, with two candidates inside the radius (`USW00014837` at 8.8 km — the nearest, listed **first** here — and an `USC…` station at 26.1 km) and three beyond it, so the distance gate and the nearest-wins choice have material in one recording |
| `gsom-14837-1991-2020.json` | `GET …/data/v1?dataset=global-summary-of-the-month&stations=USW00014837&startDate=1991-01-01&endDate=2020-12-31&format=json&units=metric&dataTypes=TAVG,TMAX,TMIN,PRCP` | A complete record: 360 rows, every month of 1991–2020 with all four values. October's normal is the mean over 30 rows: TAVG 9.9033 °C, TMAX 15.4933 °C, TMIN 4.31 °C, PRCP 70.28 mm |
| `search-empty-pacific.json` | `GET …/search/v1/data?dataset=global-summary-of-the-month&bbox=0.5,-140.5,-0.5,-139.5&limit=5` | A box with no station: HTTP 200 and `results: []` — an empty answer is a valid answer, not an error |

The `gsom-*` recordings are the whole 30-year window for one station; the two `search-*` recordings
whose stations are queried are the answers to the same boxes the adapter builds, so a future change
to the box arithmetic shows up as a failing URL assertion rather than as a silently different
search.

## Derived in the tests

The gate cases are **trimmed from the recordings above inside the tests**, never hand-written
payloads: `tests/normals.rs` keeps only the years 1991–2002 of `gsom-14837-1991-2020.json` for the
"fewer than 20 usable years" gate (12 complete Octobers) and drops every June row of the same
recording for the "no row for the requested month" gate. Both trims are applied to the decoded
recording before it is scripted as the transport's answer, so the fixture stays verbatim.
