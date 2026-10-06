<!--
SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Step 26 — climate normals

Status: ⬜ not-started
Depends on: `03-canonical-model-and-units.md` (model + units), `06-open-meteo-provider.md` (report assembly), `08-cli-surface-and-formats.md` (flags, formats)
Touches: `src/normals/{mod,ncei}.rs`, `src/model/mod.rs`, `src/render/{art_table,plain,json}.rs`, `src/cli.rs`, `src/config/mod.rs`, `src/cache.rs`, `tests/normals.rs`, `tests/fixtures/normals/`, `docs/providers.md`, `README.md`, `docs/plans/README.md`, `CHANGELOG.md`

## Goal

`cirrocast --normals Beijing` answers the question a forecast alone cannot: how today compares with
the climate. A monthly normal (1991–2020 mean of the mean daily temperature, the mean high, the mean
low and the monthly precipitation) is computed from NOAA NCEI's Global Summary of the Month for the
station nearest the location, cached for 30 days, and rendered as one comparison line under the
table (`vs normal 1991–2020: high 31.2 °C (+1.4) · low 24.0 °C (−0.6) · precip 178 mm`) or as a
`Normals:` block in `plain` and a `normals` object in `json`. The server is keyless and worldwide.

## Deliverables

- ✅ Endpoint shape, measured 2026-10-06 direct (recorded here because it is not obvious):
  * nearest station — `GET https://www.ncei.noaa.gov/access/services/search/v1/data?dataset=global-summary-of-the-month&bbox=<maxLat>,<minLon>,<minLat>,<maxLon>&limit=5`
    (the bounding box is **NW corner then SE corner**; the documented-looking `SW,NE` order returns
    HTTP 500). The answer carries `results[]`, one entry per station file, each with its file `name`
    (`CHM00054511.csv`), its point as `centroid: [lon, lat]` (also in `location.coordinates` and
    `boundingPoints[].coordinates`) and a nested `stations[]` array whose `dataTypes[]` records name
    the covered datatypes; the nearest entry **by great-circle distance** wins and its
    `stations[].id` is the station to query. Measured 2026-10-06: `results[]` is **not**
    distance-ordered (the recorded Beijing box lists a station 119.7 km away first and the 12.4 km
    one last), so the client measures every returned entry; a box with no station answers `200` with
    `results: []` (recorded); `limit=5` keeps a dense two-degree box at 24 KB and the 60 km box the
    adapter derives at 67 KB.
  * values — `GET https://www.ncei.noaa.gov/access/services/data/v1?dataset=global-summary-of-the-month&stations=<ID>&startDate=<YYYY-01-01>&endDate=<YYYY-12-31>&format=json&units=metric&dataTypes=TAVG,TMAX,TMIN,PRCP`
    → rows `{DATE: "YYYY-MM", STATION, TAVG, TMAX, TMIN, PRCP}` with the values as JSON **strings**
    in °C and mm; the `dataTypes` projection cuts the 30-year window from 92 KB to 27 KB (both
    measured 2026-10-06) and a station can answer with `[]`; a station's name and coordinates come
    from the **search** response, not the data rows (they carry `STATION` and numbers only). The
    recordings live in `tests/fixtures/normals/` — exact request URLs in its `README.md`, the
    `LicenseRef-US-Government-Public-Domain` annotation in `REUSE.toml`.
- ✅ `src/normals/{mod,ncei}.rs`: `pub fn normals(loc: &Location, month: u8, env: &Env<'_>) ->
  Result<Option<Normals>>` — two requests, both through the shared client and cache
  (`normals/<station>-<period>-<MM>.json` for the summaries and
  `normals/search-<lat.2dp>-<lon.2dp>-<radius>km.json` for the station pick, TTL 30 days,
  `--no-cache`/`--refresh`/`--offline` honoured; an offline miss degrades to `Ok(None)` with a `-v`
  note, like the gates below); the configured window is fetched once per station and month, then the
  row for the requested month is averaged (mean of `TAVG`, `TMAX`, `TMIN`, sum-then-mean of `PRCP`
  over the rows carrying all four values); a station with fewer than
  20 usable years, no station inside `max_distance_km`, or no row for the month yields `Ok(None)`
  plus one `--verbose` line naming the reason — never an error that changes the exit code. `[normals] period`
  (default `"1991-2020"`, the WMO normal period; any two four-digit years are accepted) and
  `[normals] max_distance_km` (default 60) are config keys with `CIRROCAST_NORMALS_PERIOD` /
  `CIRROCAST_NORMALS_MAX_DISTANCE_KM`.
- ✅ `src/model/normals.rs` (re-exported from `src/model/mod.rs` as `model::Normals`, like every
  other panel type): `pub struct Normals { pub station: String, pub station_name: String,
  pub distance_km: f64, pub period: String, pub month: u8, pub temp_mean_c: f32,
  pub temp_max_c: f32, pub temp_min_c: f32, pub precip_mm: f32, pub years: u16 }` and
  `Report.normals: Option<Normals>` (additive in `json`, `schema_version` unchanged by the contract's
  additive rule); `Attribution` gains nothing — the credit is source-level and travels through the
  existing footer text.
- ⬜ `src/cli.rs`: `--normals` (fetch and render; implies the extra two requests) and
  `[defaults] normals = false` so a script that never asked pays nothing; `--format normals` prints
  the block alone (mirroring `--format aqi`), and `--normals` composes with every other format.
- ⬜ Renderers: `art_table` appends the comparison line to its footer (high/low against the daily
  `temp_max_c`/`temp_min_c`, precipitation against the requested days' total, deltas signed and
  coloured with the existing temperature ramp; `color = never` keeps the ± signs); `plain` prints a
  `Normals: 1991–2020 · Beijing (54511) 12 km · high 31.2 °C (+1.4) · low 24.0 °C (−0.6) · precip
  178 mm (−12%) · 30 years` line; `json` carries the typed object; `one_line` gets no token (the
  wttr.in token table stays untouched). Every number goes through the single conversion point, so
  `--units us` shows °F/inches.
- ⬜ Credit: the footer line includes `Climate normals computed from NOAA NCEI Global Summary of the
  Month (public domain)`, and `docs/providers.md` gains a normals row (endpoint, no auth, licence
  `LicenseRef-PublicDomain-USGov` or `CC0-1.0` as the page states, cache ceiling, `verified` date);
  README's sources section lists it beside Open-Meteo.
- ⬜ Tests (`tests/normals.rs`, offline): a station fixture (search + data) for a full 30-year month
  → the exact means and `years = 30`; a 12-year station → `Ok(None)` with the verbose reason; a
  point with no station in range; a month present in the data but not in the window; the second run
  served from cache; `--offline` with and without a cached entry; the renderer line and `json`
  object snapshots; the `NW,SE` bbox order asserted from the recorded request (a regression to
  `SW,NE` must fail).

## Design notes

* **Normals are computed here, not fetched as such.** GSOM publishes monthly summaries, not
  "normal" products; the 1991–2020 mean of the same calendar month is the standard definition of a
  climate normal, and the number of contributing years is printed so a thin record is visible. The
  credit says "computed from", matching the project's modified-data disclosure habit.
* **The two-request flow is a real cost, hence the 30-day cache and the opt-in flag.** The search
  response is 24–67 KB (it embeds per-datatype coverage; measured 2026-10-06), the data response
  92 KB for 30 years without the `dataTypes` projection and 27 KB with it; both are cached
  per station+month, and `--normals` off means zero requests.
* **No station-name table is bundled.** The search endpoint answers "which station is near me"
  directly, which avoids shipping GHCN metadata (≈10 MB) for a single line of output.
* **60 km is generous on purpose.** GSOM stations are sparse outside the US and Europe; a distant
  station still beats no normal, and the distance is printed so the user can discount it. A future
  tightening is a config change, not a code change.
* **Screened from the breezy audit**: NCEI's GSOM is what its `ncei` normals module uses (bbox →
  stations → data, Gaussian weighting); this step takes the two-step API and the public-domain
  licence and drops the weighting (nearest station, distance printed).

## Out of scope

Per-day/per-hour normals, seasonal or annual aggregates, paleoclimate and reanalysis baselines,
normals from non-NOAA sources (a second norms provider can hang off the same model field), and any
caching of the multi-year row set beyond the single station+month entry.

## Verification

```sh
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && reuse lint

cargo run -q -- --normals --lat 39.9042 --lon 116.4074 -f plain
#   weather block plus `Normals: 1991–2020 · … · high … (+x.x) · low … (−x.x) · precip … (…) · 30 years`
cargo run -q -- --normals -f normals Beijing
cargo run -q -- --normals -f json Beijing | jq '.normals | {station_name, distance_km, period, years}'
cargo run -q -- --normals --offline @0,-140 -v      # no station in range: verbose reason, exit 0
cargo run -q -- --normals --units us Beijing -f plain | grep -o '°F'
```

## Exit criteria

- ⬜ `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`, `reuse lint` clean.
- ⬜ The computed means for the pinned station/month fixture match the values averaged by hand in the
      test, and `years` equals the contributing row count.
- ⬜ A run without `--normals` makes no NCEI request; a second `--normals` run is a cache hit; an
      offline run with no entry degrades with a verbose note and exit 0.
- ⬜ The bbox order regression test fails when the corners are swapped.
- ⬜ Rendering respects `--units`, `--width` and `--color`, and `json` output stays additive.

## Risks

* NCEI endpoints are undocumented beyond the access-services page and the bbox order is a trap
  (measured); the regression test pins it and a failure is a one-line fix.
* Station coverage is uneven (sparse in parts of Africa and South America); the `Ok(None)` path plus
  the verbose reason is a first-class outcome, never an error.
* The 1991–2020 window will age (2031 moves to 1991–2030); the period is a config default, not a
  constant in the decoder.
* Large search responses are cached whole; `cache stat` will show the namespace's size and step 21's
  budget owns any trimming.

## Progress log

- 2026-10-03 — step opened. The NCEI access-services flow was probed through the local proxy and the
  exact parameter shapes (bbox corner order, `format=json&units=metric`, the `DATE: YYYY-MM` row
  shape, `TAVG/TMAX/TMIN/PRCP` presence, and the station metadata living in the search response)
  are recorded above; the idea and the two-step structure come from the breezy-weather audit
  (`ncei` normals module), minus its Gaussian weighting.
- 2026-10-04 — renumbered from 29 to 26 by the plan reorganization; dependencies (03, 06, 08) unchanged.
- 2026-10-06 — deliverable 1 closed. The flow was measured live directly (not through a proxy) and
  the recordings landed under `tests/fixtures/normals/`: five payloads with their exact request
  URLs and the values the tests will pin in the directory's `README.md`, and the public-domain
  `LicenseRef` annotation in `REUSE.toml`. What the measurement corrected in the text above: the
  search `limit` is pinned to 5; `results[]` is **not** distance-ordered (the recorded three-station
  box lists the 12.4 km station last), so the client measures every entry; `centroid` arrives as a
  `[lon, lat]` array; a station-free box answers `200` with `results: []`; and the
  `dataTypes=TAVG,TMAX,TMIN,PRCP` projection cuts the values response from 92 KB to 27 KB. Two
  stations were recorded on purpose: the 24-year holey Beijing record (October: 23 rows, 22 complete
  — `2020-10` carries only `PRCP`) and the complete 30-year Madison record, so the averaging and the
  completeness rule have a gappy and a full case; the gate payloads are trimmed from the recordings
  inside the tests rather than stored as extra fixtures.
- 2026-10-06 — deliverable 3 closed. The type lives in its own file (`src/model/normals.rs`),
  re-exported from the model root, exactly like the air, astro and marine panel types, so
  `model::Normals` is the interface the step names. The `json` object landed with it, because a
  model field with no producer is invisible: the machine-checked key index in `docs/schema.md`
  gained the ten `normals.*` rows, `tests/fixtures/report/beijing-normals.json` (the recorded
  `CHM00054511` September mean, self-consistent with the fixture's 2026-09-30 day) joined the
  fixture union the key-index test walks, and the four inline JSON snapshots moved with the added
  `"normals": null` key. Every `Report` literal in `src/` gained `normals: None`; the tests build
  reports from fixtures, so none needed it.
- 2026-10-06 — deliverable 2 closed, together with the `[normals]` configuration table it depends
  on (`period` validated as `YYYY-YYYY` with the earlier year first, `max_distance_km` a `u16` in
  `1..=500`, both with their `CIRROCAST_*` override). Two divergences from the plan text, recorded
  as the plan-driven workflow asks: the summaries cache key is
  `normals/<station>-<period>-<MM>.json`, not `<station>-<YYYY-MM>` — the configured window is part
  of what the cached body means, so a period change must miss by key instead of serving the old
  window until the TTL runs out — and the station pick got its own long-TTL entry,
  `normals/search-<lat.2dp>-<lon.2dp>-<radius>km.json`, which the plan left unspecified; the radius
  is part of the question, so it is part of the key. The contract's cache-layout bullet, the README
  key table, `docs/schema.md`, the shipped `DEFAULT_DOCUMENT` and the `expected-after-set.toml`
  canonical fixture moved with them, and the namespace count in `cache stat` is seven now. A
  *usable year* is a row carrying all four values (`TAVG`, `TMAX`, `TMIN`, `PRCP`): the recorded
  Beijing station has a `2020-10` row with a precipitation total and no temperatures, and per-field
  denominators would make the printed `years` mean two things at once. An offline miss degrades to
  `Ok(None)` with the cache's own message on the `-v` stream, which is what makes
  `--normals --offline` a note and exit 0 rather than a failure; the pure helpers (bbox corner
  order, the completeness rule, the `YYYY-MM` parse, the tolerant value decode) carry unit tests in
  the module, because the fixture-driven suite is deliverable 7.
