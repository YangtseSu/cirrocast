<!--
SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Step 11 — METAR and aviation weather

Status: ✅ done
Depends on: 05 (http-cache-and-ip-location), 10 (additional-providers)
Touches: `src/provider/metar.rs` (new), `src/provider/metar/` (new decoder + station table), `src/provider/mod.rs`,
`src/geo/tz.rs` (new), `src/cli.rs`, `src/model/mod.rs` (the station location), `src/cache.rs` (station keys
and namespace), `src/render/{art_table,json}.rs`, `src/i18n.rs`, `src/model/condition.rs`, `src/render/art.rs`,
`locales/*/main.ftl`, `Cargo.toml`/`Cargo.lock`, `tests/metar.rs` (new), `tests/live.rs`,
`tests/fixtures/metar/**` (new), `tests/fixtures/stationinfo/**` (new), `REUSE.toml`,
`LICENSES/LicenseRef-US-Government-Public-Domain.txt` (new), `docs/providers.md`, `README.md`,
`docs/plans/README.md`

## Goal

Add the second keyless backend: a station-based aviation provider that turns one ICAO identifier into a
real observation with no API key — `cirrocast --station KJFK` prints a current condition block (art-table),
a full field set (json/one-line) or a pipe-friendly line (plain), all decoded from the upstream METAR
report. It advertises `current: true, hourly: false, daily: false, max_days: 0`, resolves the station to a
`Location` (name, latitude, longitude, elevation, timezone) through the embedded 50-station table plus a
30-day-cached stationinfo lookup, and never pretends to have a forecast: the renderers state the
observation age instead, and `--verbose` adds the raw METAR/TAF text with an explicit note.

## Deliverables

- ✅ `src/provider/metar.rs`: `MetarProvider` implementing the binding `Provider` trait (`fetch(&self, …)`,
  synchronous, all I/O through `Env::http`/`Env::cache`), a `ProviderId::Metar` variant, and the registry
  row in `src/provider/mod.rs` with `Capabilities { current: true, hourly: false, daily: false, alerts: false,
  max_days: 0, requires_key: false, key_env: None, location_kinds: Station | LatLon }`.
- ✅ Upstream calls, all keyless, `Accept: application/json` where applicable and the shared `http.rs` UA:
  current observation `https://aviationweather.gov/api/data/metar?ids=<ICAO>&format=json`, raw text
  `…/metar?ids=<ICAO>&format=raw`, TAF `…/taf?ids=<ICAO>&format=raw`, station metadata
  `…/stationinfo?ids=<ICAO>&format=json`. One request per station per resource; obs-time results go through
  `cache.rs` under `weather/metar-<ICAO>-current.json` (TTL `cache.weather_ttl_secs`).
- ✅ `src/provider/metar/station_table.rs`: an embedded, ICAO-sorted `static STATIONS: &[Station]` of ~50
  frequently used stations (`icao`, `name`, `lat`, `lon`, `elev_m`, `tz: &'static str`), binary-searched by
  ICAO. Values are transcribed from the stationinfo rows recorded in `tests/fixtures/stationinfo/`; provenance
  (NOAA/NWS public-domain station metadata) is stated in the module header comment.
- ✅ `src/geo/tz.rs`: `pub fn lookup(lat: f64, lon: f64) -> Option<chrono_tz::Tz>` — the single
  coordinate → IANA timezone helper, backed by a new dependency `tzf-rs` (small, offline, embedded index; no
  network and no data files at runtime). If step 04 or 05 already ships an equivalent helper, extend that
  module instead of adding a second one.
- ✅ Station → `Location` resolution order, each step logged under `-v`: explicit `--station` / config
  `[providers.metar] station` → embedded table (no I/O) → `cache/station/<ICAO>.json` (30-day TTL) → live
  stationinfo. `Location` is built from `site` (name), `lat`, `lon`, `elev` (metres), and the timezone from
  the table entry or `geo::tz::lookup`; when neither yields a timezone the location is UTC and `--verbose`
  says so.
- ✅ `--station <ICAO>` in `src/cli.rs`: value parser accepting `^[A-Za-z][A-Za-z0-9]{3}$`, upper-cased
  before use, any other length/alphabet rejected as a clap usage error (exit 2) naming the offending value and
  the accepted form (`--station EGLL`). `--station` without `-p` selects `metar`; `--station` together with a
  location argument, or with an explicit `-p` chain that contains no station-capable provider, is a usage
  error naming both flags.
- ✅ `src/provider/metar/decode.rs`: `pub(crate) fn decode_metar(raw: &str) -> Result<Decoded>` — pure, no
  I/O — plus the token tables it needs. Documented rules: wind `ddd ff KT`, `ddd ffGfm KT`, `VRB ff KT`,
  `00000KT` (calm), `ddd ff KT` with a following `MPS` variant converted to km/h; visibility in metres
  (`9999`, `0800`, `CAVOK` ⇒ ≥ 10 km) or statute miles (`10SM`, `P6SM`, `M1/4SM`); RVR groups (`R28/1200`,
  `R06L/2000FT`) parsed and kept in the raw/decoded detail only; present-weather groups (`-SHRA`, `BR`, `FG`,
  `FZRA`, `+TSRA`, `VCSH`, `RE…`) mapped to WMO 4677 codes (BR → 10 mist, FG → 45, `-SHRA` → 80, `+TSRA` →
  95/96 by intensity); cloud layers `FEW/SCT/BKN/OVC/VV` with heights in hundreds of feet → metres, plus the
  no-cloud tokens `NSC`/`NCD`/`CLR`; `T`/`M` signed temperature and dewpoint (`M05/M02`); altimeter in `Q`
  (hPa, used as-is) or `A` (inHg × 33.8639 → hPa); trend/remark groups (`AUTO`, `COR`, `NOSIG`, `BECMG`,
  `TEMPO`, `RMK …`) never decoded, kept verbatim in `Attribution.raw`.
- ✅ Decoded fields → `Current` in canonical metric (`temp_c`, `dewpoint_c`, `wind_kmh` = knots × 1.852,
  `wind_gust_kmh`, `wind_dir_deg` with `VRB` ⇒ `None`, `visibility_km`, `pressure_hpa`, `condition`,
  `humidity_pct` derived with the Magnus formula from temperature/dewpoint and marked derived in the doc
  comment, `observed_at` from the upstream `obsTime` epoch). Fields that stay `None`/empty for every METAR
  report: `precip_mm` (unless the `RMK` group carries `P####`), `uv_index`, `apparent_temp_c`,
  `sunrise`/`sunset`, `days` (`vec![]`), `attribution.raw` = raw METAR + raw TAF when fetched.
- ✅ Capability-driven rendering, no renderer special-cases: art-table prints the current block plus an
  `observed 12:20Z · 12 min ago` line and a `no forecast: METAR is an observation` footer; `plain` prints the single
  line; `one-line` fills every current token and leaves forecast tokens empty; `json` emits the full `current`
  object, `"days": []` and a `capabilities` object so consumers see `daily: false`; `-v` appends the raw METAR, the
  raw TAF (when present) and the no-forecast note pointing at a forecast provider.
- ✅ `--verbose` TAF handling: fetched only when verbose is on, printed verbatim (multi-line, indentation
  preserved), never decoded — this step adds no trend interpretation.
- ✅ Error paths with the binding taxonomy: unknown/unsupported station → `Error::LocationNotFound`
  (exit 5) naming the ICAO and pointing at `cirrocast location search`; empty METAR/TAF payload or an empty
  JSON array for a station that exists → `Error::Upstream` (exit 3) naming the station and the upstream URL;
  transport failure or non-2xx → `Error::Upstream`/`Network` after the `http.rs` retry policy so a
  `--provider metar,open-meteo` chain can fall through; malformed JSON → `Error::Upstream` with the decode
  error in the cause chain, never a panic.
- ✅ Fixtures: six recorded METAR reports plus their stationinfo rows, covering calm wind, gusting wind,
  `VRB` direction (upstream sends `"wdir": "VRB"` as a string), snow, a thunderstorm, and a report with no
  visibility group at all; candidate stations `KSEA`, `KORD`, `ENGM`, `CYYZ`, `KDFW`, `KSMF`, the exact
  report pinned inside the fixture. Plus one empty-array and one truncated-JSON file for the step 12 sweep;
  all stored under `tests/fixtures/metar/<ICAO>/` and `tests/fixtures/stationinfo/<ICAO>.json`.
- ✅ Fixture licensing: NOAA/NWS station metadata and reports are US-government public domain, so those two
  directories get exact-path `REUSE.toml` annotations carrying a public-domain `LicenseRef` (notice text in
  `LICENSES/`); the blanket `tests/fixtures/**` GPL override from step 01 no longer covers them.
- ✅ `tests/metar.rs`: decoder unit tests per fixture (asserting the decoded values byte-for-byte against
  the expectations table in the test file), a station-table lookup test, and CLI-level tests using a
  pre-seeded stationinfo cache file so nothing touches the network; live checks are `#[ignore]`d.
- ✅ `provider list` / `provider info metar` text: keyless, station-based, current-only (`max_days = 0` ⇒
  `--days` is ignored with a single warning), data source `aviationweather.gov` with the attribution line
  shown by `-f json` (`attribution.name`) and in `--verbose`.
- ✅ `[providers.metar] station = "KJFK"` honoured as the default location when the command line carries no
  location, `--station` and `--lat/--lon` (the station's coordinates) are mutually consistent, and a station
  configured while another provider is default only affects `metar`.
- ✅ `README.md`: a backends-table row for `metar` (keyless, station-based, current-only, TAF via
  `--verbose`) and a short "aviation" usage snippet.

## Design notes

* Upstream JSON is loosely typed: `wdir` is an integer for a real direction and the string `"VRB"` for a
  variable one, `visib` is a *string* (`"10+"`, `"6+"` meaning "greater than"), and `wgst`/`wxString` are
  absent when there is no gust or no weather. The wire struct therefore uses `#[serde(untagged)]` enums plus
  `Option` for every optional member, and visibility is decoded from the raw report group (authoritative,
  metres or statute miles) with the JSON `visib` value used only as a cross-check.
* Visibility is displayed in kilometres (canonical) with the unit conversion left to the render layer;
  statute miles from the raw report are converted once, in the decoder, via × 1.609344.
* `tzf-rs` justification: METAR carries no timezone and stationinfo has no timezone field, yet the header must
  show the station-local observation time. The alternative (UTC everywhere, or a UTC-only header for every
  station not in the embedded table) is rejected because it would mislabel nearly every station; `tzf-rs` is
  offline, small, MIT-licensed and needs no runtime data file.
* Coverage is two-layer: the embedded table needs no I/O for the common stations, and stationinfo extends it at
  one cached request per station per 30 days.
* Rejected alternatives: decoding from the JSON fields only (loses metre-based visibility, RVR and remarks);
  scraping `aviationweather.gov` HTML (unstable); adding a per-provider CLI flag for the station
  (the binding contract says new providers add no flags — `--station` already exists in the surface).

## Out of scope

* Full TAF decoding (trend windows, `FM`/`TEMPO`/`PROB` groups, flight-category forecasting): no step file
  owns it; v1 keeps TAF as raw text behind `--verbose`.
* SIGMET/AIRMET/PIREP products and radar imagery: not owned by any step file; step 15 covers consumer-facing
  weather alerts only.
* IATA/FAA/WMO identifier lookup (3-letter IATA codes, 5-digit WMO numbers): only 4-letter ICAO is accepted.
* Historical METAR archives, station time-series and `--days` support for this backend: step 23 adds
  additional forecast backends; METAR stays current-only.
* The alerts field of `Report` (stays empty here) and alert rendering: step 15.

## Verification

```
# record/refresh the fixtures (manual, once)
curl -sS 'https://aviationweather.gov/api/data/metar?ids=KJFK&format=json' > tests/fixtures/metar/KJFK/current.json
curl -sS 'https://aviationweather.gov/api/data/metar?ids=KJFK&format=raw'  > tests/fixtures/metar/KJFK/raw.metar
curl -sS 'https://aviationweather.gov/api/data/stationinfo?ids=KJFK&format=json' > tests/fixtures/stationinfo/KJFK.json
# smoke runs
cargo run -- --station kjfk -f plain          # one line, station echoed as KJFK, temperature in °C
cargo run -- --station KJFK -f art-table      # current block + "observed … ago" + no-forecast footer
cargo run -- --station KJFK -f json | jq '.current.temp_c, .days, .capabilities.daily'   # number, [], false
cargo run -- --station KJFK --verbose         # raw METAR + raw TAF + "METAR has no forecast" note
cargo run -- --station ZZZZ                   # exit code 5, message names ZZZZ
cargo run -- --station 12                     # exit code 2, message shows the accepted form
```

## Exit criteria

- ✅ `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`, `reuse lint` all clean.
- ✅ `cargo test --test metar` passes with the six fixtures; no test performs a live request.
- ✅ `cargo run -- --station kjfk -f plain` (station normalised to `KJFK`) prints one line with a decoded
  temperature, wind and visibility, and `--station ZZZZ` exits 5 with a message naming `ZZZZ`.

## Risks

* Upstream field drift on `aviationweather.gov` (e.g. `visib` going numeric) would break the wire struct:
  mitigated by `#[serde(untagged)]` + `Option` everywhere, an upstream-type-independent decoder and the step 12
  robustness sweep.
* Rate limiting or a UA policy change upstream: the shared client sends a descriptive UA, results are cached,
  and retries are bounded by `network.retries`; a 429 surfaces as `Error::Upstream` so a chain can fall through.
* Stationinfo coverage gaps or missing `elev`: the embedded table covers the common stations, and a missing
  elevation leaves the field `None` rather than inventing a value.
* Decoder over-reach into remarks (`RMK AO2 SLP160 …`) could produce wrong values: remarks are never
  interpreted except the documented `P####` precipitation group, which is itself covered by a fixture.

## Progress log

- 2026-09-30 — step file written (status: not-started); upstream payload shape verified against
  `aviationweather.gov/api/data/{metar,stationinfo}` while drafting.
- 2026-10-01 — step implemented end to end. Where reality diverged from this file, the file was
  changed in the same commit and the reason is here:
  * **One request per observation, not two.** `metar?format=json` already carries `rawOb` — the raw
    report, byte for byte, which `tests/metar.rs` asserts against the `format=raw` fixture. Calling
    `format=raw` as well would spend a second request on the same bytes, so the provider fetches the
    JSON only; `format=raw` stays in the recording recipe above.
  * **The station table carries `state` and `country` too.** Without them a station answered from
    the table would render as `New York/JF Kennedy Intl` while the same station answered from
    `stationinfo` renders as `…, NY, US` — two spellings of one place. The two extra columns are
    transcribed from the same recorded rows; the `tz` column is `geo::tz::lookup`'s answer for the
    row's coordinates (computed when the row was added, pinned by a test).
  * **`--station` selects `metar` alone when the provider comes from the configuration or the
    default.** The chain is not silently extended with a coordinate backend: the user asked for an
    observation. An *explicit* `-p auto` gains `metar` in front, and any explicit chain must contain
    `metar` (wherever it sits) or the run is a usage error naming both flags.
  * **The station identifier travels in `Location`.** `LocationSource::Station` alone could not
    carry it (the display name is the site name), so `Location` gained `station: Option<String>`
    (`#[serde(default)]`) and the `json` document gained `location.station` — additive within
    `schema_version = 1`, like the new `capabilities` object that says `daily: false` for `metar`.
  * **Six WMO codes were added to the described set** (4 smoke, 5 haze, 6 dust, 7 sand, 10 mist,
    79 ice pellets) with art blocks and en-US/zh-CN text: the decoder maps METAR obscurations to
    them, and a report of `BR` would otherwise render as “Unknown”. `Condition::is_precipitation`
    now covers 71..=79 (ice pellets fall).
  * **The observation line's separator is charset-dependent** (`·` on a UTF-8 terminal, `|` on a
    `dumb` one), because the ASCII fold turns `·` into `.`, which does not read as a separator.
  * **An empty body is an answer, not a decode failure.** `stationinfo` answers an unknown station
    with `204 No Content` (not `[]`), so the provider runs the cache policy through its own
    `cached_json` instead of `fetch_json`: an empty metadata body means “no such station”
    (`LocationNotFound`, exit 5) and an empty observation body means “nothing to report”
    (`Upstream`, exit 3, naming the station and the URL).
  * **`--days` warns only when it was actually given.** With `metar` the default `days = 3` is
    dropped silently (the user did not ask for a forecast; the footer says so); an explicit
    `--days`/`CIRROCAST_DAYS` prints the one warning.
  * **`tzf-rs` is 1.3.7, not 2.x** — 2.x requires rustc 1.88 and the crate declares MSRV 1.85 —
    with `tzf-dist` pinned to `0.0.2026-c-fix1` in `Cargo.lock` (the `d-fix1` release changed the
    loading API without a semver bump, so `cargo update` needs `--precise`). The dependency comment
    in `Cargo.toml` says so.
  * **`cache stat` reports a fourth namespace** (`station`) and the 30-day metadata lives there, so
    `cache clean` can drop it without touching observations.
  * **`decode_metar`/`Decoded` are `pub`,** not `pub(crate)`: the fixture-driven expectation table
    lives in `tests/metar.rs`, which can only reach the public API.
  * **`provider::capabilities_of`/`display_name_of`** were added so the renderers shape the
    observation block from the registry row rather than from the `metar` id.
  * **The credit line is `attribution.notice`,** not `attribution.name`: the `json` schema fixed
    that key in step 08, and the deliverable above used a name from before it. `-f json` carries
    `"notice": "aviationweather.gov (NOAA/NWS, public domain)"` and `--verbose` prints the same
    line under `attribution:`.
- 2026-10-01 — the `tzf-rs` decision above was corrected on the user's instruction ("pin nothing; use
  the latest stable release"): the manifest now accepts any `tzf-rs` 2.x (resolved to 2.1.2, with
  `tzf-dist` 0.0.2026-d-fix1 and no `--precise` lock pin) and `rust-version` moved from 1.85 to 1.88,
  which is what 2.x requires and what the crate's let-chains already implied. The upgrade also dropped
  the `prost`/`anyhow`/`petgraph` build tree the 1.3.x line pulled in; `docs/plans/README.md` and the
  step 12 MSRV deliverable were updated with it. The earlier bullet is kept as the record of what was
  done at the time.
- 2026-10-01 — the MSRV correction above stopped at 1.88; on the same instruction it moved again to
  1.98, i.e. to the toolchain the project actually builds with (no floor below stable is held).
  `Cargo.toml` declares `rust-version = "1.98"`, `docs/plans/README.md` and step 12 carry the same
  number, and the new floor surfaced two clippy lints that are gated on it — `Duration::from_hours`
  is now used for the 30-day TTLs (`src/geo/nominatim.rs`, `src/provider/metar.rs` and their tests)
  instead of `from_secs(2_592_000)`-style arithmetic. One test assertion was made age-independent in
  the same pass: the observation line's unit (`min` vs `h`) depends on the wall clock, so the test
  asserts the line's shape rather than a number that drifts an hour after recording.
- 2026-10-01 — verification: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
  `cargo test` (197 unit + 22 integration binaries) and `reuse lint` clean; the live smoke runs of
  `--station kjfk/KJFK/ZBAA/EGLL` (plain, art-table, dumb, one-line, json, `-v`) observed against the
  live service, `--station ZZZZ` → exit 5, `--station 12` → exit 2, `@51.5,-0.45 -p metar` → EGLL
  (3 km, named under `-v`), and the three `#[ignore]`d live tests pass with
  `CIRROCAST_LIVE_TESTS=1`.
