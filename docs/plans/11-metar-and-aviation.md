<!--
SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Step 11 — METAR and aviation weather

Status: not-started
Depends on: 05 (http-cache-and-ip-location), 10 (additional-providers)
Touches: `src/provider/metar.rs` (new), `src/provider/metar/` (new decoder + station table), `src/provider/mod.rs`,
`src/geo/tz.rs` (new), `src/cli.rs`, `src/config/mod.rs`, `Cargo.toml`, `tests/metar.rs` (new),
`tests/fixtures/metar/**` (new), `tests/fixtures/stationinfo/**` (new), `REUSE.toml`, `README.md`

## Goal

Add the second keyless backend: a station-based aviation provider that turns one ICAO identifier into a
real observation with no API key — `cirrocast --station KJFK` prints a current condition block (art-table),
a full field set (json/one-line) or a pipe-friendly line (plain), all decoded from the upstream METAR
report. It advertises `current: true, hourly: false, daily: false, max_days: 0`, resolves the station to a
`Location` (name, latitude, longitude, elevation, timezone) through the embedded 50-station table plus a
30-day-cached stationinfo lookup, and never pretends to have a forecast: the renderers state the
observation age instead, and `--verbose` adds the raw METAR/TAF text with an explicit note.

## Deliverables

- [ ] `src/provider/metar.rs`: `MetarProvider` implementing the binding `Provider` trait (`fetch(&self, …)`,
  synchronous, all I/O through `Env::http`/`Env::cache`), a `ProviderId::Metar` variant, and the registry
  row in `src/provider/mod.rs` with `Capabilities { current: true, hourly: false, daily: false, alerts: false,
  max_days: 0, requires_key: false, key_env: None, location_kinds: Station | LatLon }`.
- [ ] Upstream calls, all keyless, `Accept: application/json` where applicable and the shared `http.rs` UA:
  current observation `https://aviationweather.gov/api/data/metar?ids=<ICAO>&format=json`, raw text
  `…/metar?ids=<ICAO>&format=raw`, TAF `…/taf?ids=<ICAO>&format=raw`, station metadata
  `…/stationinfo?ids=<ICAO>&format=json`. One request per station per resource; obs-time results go through
  `cache.rs` under `weather/metar-<ICAO>-current.json` (TTL `cache.weather_ttl_secs`).
- [ ] `src/provider/metar/station_table.rs`: an embedded, ICAO-sorted `static STATIONS: &[Station]` of ~50
  frequently used stations (`icao`, `name`, `lat`, `lon`, `elev_m`, `tz: &'static str`), binary-searched by
  ICAO. Values are transcribed from the stationinfo rows recorded in `tests/fixtures/stationinfo/`; provenance
  (NOAA/NWS public-domain station metadata) is stated in the module header comment.
- [ ] `src/geo/tz.rs`: `pub fn lookup(lat: f64, lon: f64) -> Option<chrono_tz::Tz>` — the single
  coordinate → IANA timezone helper, backed by a new dependency `tzf-rs` (small, offline, embedded index; no
  network and no data files at runtime). If step 04 or 05 already ships an equivalent helper, extend that
  module instead of adding a second one.
- [ ] Station → `Location` resolution order, each step logged under `-v`: explicit `--station` / config
  `[providers.metar] station` → embedded table (no I/O) → `cache/station/<ICAO>.json` (30-day TTL) → live
  stationinfo. `Location` is built from `site` (name), `lat`, `lon`, `elev` (metres), and the timezone from
  the table entry or `geo::tz::lookup`; when neither yields a timezone the location is UTC and `--verbose`
  says so.
- [ ] `--station <ICAO>` in `src/cli.rs`: value parser accepting `^[A-Za-z][A-Za-z0-9]{3}$`, upper-cased
  before use, any other length/alphabet rejected as a clap usage error (exit 2) naming the offending value and
  the accepted form (`--station EGLL`). `--station` without `-p` selects `metar`; `--station` together with a
  location argument, or with an explicit `-p` chain that contains no station-capable provider, is a usage
  error naming both flags.
- [ ] `src/provider/metar/decode.rs`: `pub(crate) fn decode_metar(raw: &str) -> Result<Decoded>` — pure, no
  I/O — plus the token tables it needs. Documented rules: wind `ddd ff KT`, `ddd ffGfm KT`, `VRB ff KT`,
  `00000KT` (calm), `ddd ff KT` with a following `MPS` variant converted to km/h; visibility in metres
  (`9999`, `0800`, `CAVOK` ⇒ ≥ 10 km) or statute miles (`10SM`, `P6SM`, `M1/4SM`); RVR groups (`R28/1200`,
  `R06L/2000FT`) parsed and kept in the raw/decoded detail only; present-weather groups (`-SHRA`, `BR`, `FG`,
  `FZRA`, `+TSRA`, `VCSH`, `RE…`) mapped to WMO 4677 codes (BR → 10 mist, FG → 45, `-SHRA` → 80, `+TSRA` →
  95/96 by intensity); cloud layers `FEW/SCT/BKN/OVC/VV` with heights in hundreds of feet → metres, plus the
  no-cloud tokens `NSC`/`NCD`/`CLR`; `T`/`M` signed temperature and dewpoint (`M05/M02`); altimeter in `Q`
  (hPa, used as-is) or `A` (inHg × 33.8639 → hPa); trend/remark groups (`AUTO`, `COR`, `NOSIG`, `BECMG`,
  `TEMPO`, `RMK …`) never decoded, kept verbatim in `Attribution.raw`.
- [ ] Decoded fields → `Current` in canonical metric (`temp_c`, `dewpoint_c`, `wind_kmh` = knots × 1.852,
  `wind_gust_kmh`, `wind_dir_deg` with `VRB` ⇒ `None`, `visibility_km`, `pressure_hpa`, `condition`,
  `humidity_pct` derived with the Magnus formula from temperature/dewpoint and marked derived in the doc
  comment, `observed_at` from the upstream `obsTime` epoch). Fields that stay `None`/empty for every METAR
  report: `precip_mm` (unless the `RMK` group carries `P####`), `uv_index`, `apparent_temp_c`,
  `sunrise`/`sunset`, `days` (`vec![]`), `attribution.raw` = raw METAR + raw TAF when fetched.
- [ ] Capability-driven rendering, no renderer special-cases: art-table prints the current block plus an
  `observed 12:20Z · 12 min ago` line and a `no forecast: METAR is an observation` footer; `plain` prints the single
  line; `one-line` fills every current token and leaves forecast tokens empty; `json` emits the full `current`
  object, `"days": []` and a `capabilities` object so consumers see `daily: false`; `-v` appends the raw METAR, the
  raw TAF (when present) and the no-forecast note pointing at a forecast provider.
- [ ] `--verbose` TAF handling: fetched only when verbose is on, printed verbatim (multi-line, indentation
  preserved), never decoded — this step adds no trend interpretation.
- [ ] Error paths with the binding taxonomy: unknown/unsupported station → `Error::LocationNotFound`
  (exit 5) naming the ICAO and pointing at `cirrocast location search`; empty METAR/TAF payload or an empty
  JSON array for a station that exists → `Error::Upstream` (exit 3) naming the station and the upstream URL;
  transport failure or non-2xx → `Error::Upstream`/`Network` after the `http.rs` retry policy so a
  `--provider metar,open-meteo` chain can fall through; malformed JSON → `Error::Upstream` with the decode
  error in the cause chain, never a panic.
- [ ] Fixtures: six recorded METAR reports plus their stationinfo rows, covering calm wind, gusting wind,
  `VRB` direction (upstream sends `"wdir": "VRB"` as a string), snow, a thunderstorm, and a report with no
  visibility group at all; candidate stations `KSEA`, `KORD`, `ENGM`, `CYYZ`, `KDFW`, `KSMF`, the exact
  report pinned inside the fixture. Plus one empty-array and one truncated-JSON file for the step 12 sweep;
  all stored under `tests/fixtures/metar/<ICAO>/` and `tests/fixtures/stationinfo/<ICAO>.json`.
- [ ] Fixture licensing: NOAA/NWS station metadata and reports are US-government public domain, so those two
  directories get exact-path `REUSE.toml` annotations carrying a public-domain `LicenseRef` (notice text in
  `LICENSES/`); the blanket `tests/fixtures/**` GPL override from step 01 no longer covers them.
- [ ] `tests/metar.rs`: decoder unit tests per fixture (asserting the decoded values byte-for-byte against
  the expectations table in the test file), a station-table lookup test, and CLI-level tests using a
  pre-seeded stationinfo cache file so nothing touches the network; live checks are `#[ignore]`d.
- [ ] `provider list` / `provider info metar` text: keyless, station-based, current-only (`max_days = 0` ⇒
  `--days` is ignored with a single warning), data source `aviationweather.gov` with the attribution line
  shown by `-f json` (`attribution.name`) and in `--verbose`.
- [ ] `[providers.metar] station = "KJFK"` honoured as the default location when the command line carries no
  location, `--station` and `--lat/--lon` (the station's coordinates) are mutually consistent, and a station
  configured while another provider is default only affects `metar`.
- [ ] `README.md`: a backends-table row for `metar` (keyless, station-based, current-only, TAF via
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
* Historical METAR archives, station time-series and `--days` support for this backend: step 19 adds
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

- [ ] `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`, `reuse lint` all clean.
- [ ] `cargo test --test metar` passes with the six fixtures; no test performs a live request.
- [ ] `cargo run -- --station kjfk -f plain` (station normalised to `KJFK`) prints one line with a decoded
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
