<!--
SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Step 23 — more providers

Status: ⬜ not-started
Depends on: 10, 15, 16
Touches: `src/provider/{mod,met_no,visualcrossing,open_meteo_archive,open_meteo_marine}.rs`,
`src/provider/open_meteo.rs`, `src/model/{mod,condition.rs}`, `src/render/{art_table,plain,json}.rs`,
`src/config/keys.rs`, `src/cli.rs`, `src/http.rs`, `src/cache.rs`, `src/i18n.rs`,
`locales/{en-US,zh-CN}/main.ftl`, `tests/{provider_met_no,provider_visualcrossing,archive,marine}.rs`,
`tests/fixtures/{met-no,visualcrossing,archive,marine}/`, `REUSE.toml`, `README.md`,
`docs/plans/README.md`, `CHANGELOG.md`

## Goal

Four new `--provider` keys beyond the ten of phases A–C: `met-no` (keyless, global, 9 days),
`visualcrossing` (BYOK, 15 days, supplies its own alerts), `open-meteo-archive` (historical only) and
`open-meteo-marine` (waves/swell/sea-surface behind `--marine`). `auto` becomes the interim fixed
list `open-meteo,met-no,smhi` (+`metar` with `--station`; step 24 replaces it with the
coverage-ranked expansion), the README backend matrix and `keys.rs` list every new source, and
scraped HTML sites stay explicitly refused.

## Deliverables

- ⬜ `src/provider/mod.rs`: `ProviderId::{MetNo, VisualCrossing, OpenMeteoArchive, OpenMeteoMarine}`
      (+ `as_str`/`FromStr`/`all()`/metadata rows, after `open-meteo`, before the BYOK block);
      `ProviderMeta`/`Capabilities` gain `history_days: u16` (0 = no archive; `max_days: 0` already
      means "no forecast horizon") and `marine: bool`.
- ⬜ `src/provider/met_no.rs`: `https://api.met.no/weatherapi/locationforecast/2.0/compact?lat=<lat>
      &lon=<lon>` (coordinates truncated to 4 decimals, optional `&altitude=`), keyless, global, **9
      days** hourly; parses `properties.meta.{updated_at,units}` and `properties.timeseries[].data`
      (`instant.details.*`, `next_1_hours`, `next_6_hours`, `next_12_hours`); day-part aggregation
      (step 03 contract) uses `next_1_hours` for the first 24 h and `next_6_hours` beyond.
- ⬜ `src/http.rs` + `src/cache.rs`: `Expires`/`Last-Modified` driven caching — store both headers
      per entry, re-request only after `Expires`, send `If-Modified-Since: <last-modified>` and treat
      `304` as a cache hit (required by the MET Terms of Service for `met-no`).
- ⬜ `src/provider/met_no.rs` **symbol_code → WMO table, written out** (suffixes `_day`, `_night`,
      `_polartwilight` stripped before lookup; unknown code ⇒ WMO 3 + one `--verbose` line naming
      it): `clearsky`→0, `fair`→1, `partlycloudy`→2, `cloudy`→3, `fog`→45, `lightrainshowers`→80,
      `rainshowers`→81, `heavyrainshowers`→82, `lightrain`→61, `rain`→63, `heavyrain`→65,
      `lightsleetshowers`/`sleetshowers`/`lightsleet`/`sleet`→68, `heavysleetshowers`/`heavysleet`→69,
      `lightsnowshowers`/`snowshowers`→85, `heavysnowshowers`→86, `lightsnow`→71, `snow`→73,
      `heavysnow`→75, every `*andthunder` form→95; the canonical set in `src/model/condition.rs`
      gains **68/69** ("light/heavy rain and snow", i.e. sleet), which the step-03 subset
      (Open-Meteo's list) lacks — art, `condition-68`, `condition-69`, i18n and the classification
      helpers change in the same commit.
- ⬜ `src/provider/visualcrossing.rs`: `https://weather.visualcrossing.com/
      VisualCrossingWebServices/rest/services/timeline/<lat>,<lon>/next<days>days?unitGroup=metric&
      include=current,days,hours&key=<KEY>` — BYOK `CIRROCAST_VISUALCROSSING_KEY`, **15 days**,
      global; `icon` → WMO (`clear-day/clear-night`→0, `partly-cloudy-day/night`→2, `cloudy`→3,
      `wind`→3, `fog`→45, `rain`→63, `showers-day/night`→81, `snow`→73, `sleet`→68,
      `freezing-rain`→66, `hail`→96, `thunderstorm`/`thunder-rain`→95, unknown→3 + `--verbose`);
      `alerts[]` feeds step 15's layer as `AlertSource::VisualCrossing` (`event`, `headline`,
      `description`, `onset`, `expires`, `severity`, `certainty`, `urgency`); `next<days>days` falls
      back to explicit `date1/date2` if the server rejects the keyword.
- ⬜ `src/provider/open_meteo_archive.rs`: `https://archive-api.open-meteo.com/v1/archive?
      latitude=&longitude=&start_date=<YYYY-MM-DD>&end_date=<YYYY-MM-DD>&daily=…&hourly=…&
      timezone=auto` — keyless, ERA5/ERA5-Land/IFS from **1940-01-01**, `history_days = 30000`,
      `Capabilities { current: false, hourly: true, daily: true, alerts: false, max_days: 0,
      history_days: 30000 }`; a date inside ERA5's ~5-day latency window falls through to
      `open-meteo` with `past_days`.
- ⬜ `src/provider/open_meteo_marine.rs`: `https://marine-api.open-meteo.com/v1/marine?
      latitude=&longitude=&current=wave_height,wave_direction,wave_period,swell_wave_height,
      sea_surface_temperature&daily=wave_height_max,wave_period_max,wave_direction_dominant&
      cell_selection=sea&timezone=auto` — keyless, **8 forecast days**, coastal; carried as
      `Report.marine: Option<Marine>` and merged by a `--marine` supplementary fetch, so
      `--provider open-meteo-marine` alone is a usage error (exit 2); a cell more than 25 km away is
      named under `--verbose`; `src/cli.rs` gains `--date <YYYY-MM-DD>` and `--history <N>d` (both
      need a chain entry with `history_days > 0`, else usage error), `--marine` and `--days` clamped
      per provider against `max_days` with the warn-once behaviour.
- ⬜ `auto` chain `open-meteo,met-no,smhi` (+`metar` with `--station`) as the **interim** fixed
      list — step 24 replaces it with the coverage-ranked expansion once national backends carry
      `covers` metadata — and update the selection paragraph, provider list and CLI block of
      `docs/plans/README.md`, the README backend matrix (key/env var, coverage, days, alerts,
      attribution per row) and `src/config/keys.rs` with `CIRROCAST_METEOALARM_KEY` and
      `CIRROCAST_VISUALCROSSING_KEY`; `provider info` rows for the new ids and the re-checked old
      ones gain coverage, days, key env var, alert sources, rate limit and the exact attribution
      string each backend requires.
- ⬜ Fixtures + tests: recorded `met-no` compact JSON (Oslo plus a `polartwilight` symbol),
      `visualcrossing` timeline JSON with an alert object, archive JSON for a fixed date and marine
      JSON for a coastal point; a symbol-table test over all 46 met.no base codes including the
      `lightssleetshowersandthunder` double-`s` spelling; a `304` cache test; `--date 1940-01-01` and
      a date inside the ERA5 latency window; a land point for `--marine`; the capability gates
      (`--days` on a history-only entry, `--history` on a forecast-only entry) both exit 2.
- ⬜ Re-verify every `ProviderMeta` limit against the live docs (endpoint alive, `max_days`,
      `history_days`, key requirement, coverage) and record each correction in the Progress log; the
      refusal note for HTML-only sources (e.g. gismeteo) and `wttr.in` as a data source lands in
      `README.md` and `provider info`.

## Design notes

* **met.no, measured 2026-09-30** (`curl -sI -A 'cirrocast-planning/0.1'
  '…/locationforecast/2.0/compact?lat=59.91&lon=10.75'`): HTTP/2 200 with `expires` and
  `last-modified`, body carrying `properties.meta.units` and 87 hourly rows exposing
  `next_1/6/12_hours`. Terms: a descriptive `User-Agent` with contact information is mandatory
  (generic UAs get `403`; `okhttp`, `Dalvik`, `fhttp`, `Java` are banned), ≤ 20 req/s, coordinates
  ≤ 4 decimals, `If-Modified-Since` on refresh; data is CC BY 4.0, attribution "Data from MET Norway",
  naming restriction (no "Yr" in our name or UI). Quotas: met.no 20 req/s per application; Visual
  Crossing free plan **1 000 records/day** (`queryCost` ≈ `1 + 24×hours + days`, so the default
  3-day `include=current,days,hours` query costs ~76 records); Open-Meteo free tier **< 10 000
  calls/day, non-commercial**, CC-BY-4.0 with a link next to displayed data. Exceeding a quota yields
  `Error::Upstream` naming the provider, never a silently empty report.
* **Historical-only degradation**: `max_days: 0` + `history_days > 0` is the machine-readable form of
  "archive only"; the CLI refuses `--days` on such an entry, renderers print a dated archive header
  (`2026-09-14 · archive`), `one-line` prepends the date and `json` marks `"mode": "archive"`;
  `--history 7d` on the default chain resolves to `open-meteo` with `past_days=7`.
* **Marine degradation**: a land point returns the nearest sea cell, so the panel names the sampled
  coordinate when it differs by more than 25 km and `json.marine` is `null` (never zero-filled)
  when the response has no `current` block.
* **Licences** (all compatible with shipping data inside a GPL-3.0-or-later program with README
  credits): MET Norway CC BY 4.0; Visual Crossing per-account terms (local display only, key BYOK, no
  redistribution of cached data); Open-Meteo CC BY 4.0 with ERA5/Copernicus and CAMS attribution
  lines; Copernicus Marine Service licence plus DWD ICON Wave for the marine product. No new crate,
  so the `cargo deny` surface is unchanged.
* **Scraped sources are refused**: (1) legal — gismeteo-class sites publish no data licence and their
  terms forbid extraction; (2) robustness — an HTML parser breaks silently on a layout change, which
  for a weather tool means confidently wrong output; (3) supportability — no documented API, no
  `User-Agent` policy to honour, no contact, no status page. The same test (documented API, stated
  licence, identity/attribution requirements) admits every backend above, so the rule is uniform.
* **Admission criteria, sharpened by the breezy-weather audit (2026-10-03)** — a candidate backend
  must additionally: load an endpoint that is documented and public (no undocumented dashboard or
  NinJo/`llj`-style internal routes); require no credential shipped inside this project (BYOK from
  the user, or none at all — an app-minted JWT from a bundled secret, as Météo-France's client does,
  is refused); not act as a reskin of a third party's API (the audit's Bangladesh source is an
  aggregator, not the agency); not be reachable through a reverse-engineered private API (the
  audit's Xiaomi/`china` module ships a hard-coded app key and signature and is the canonical
  rejection); and, for a "free" key, need no credit card or phone number. Coverage is judged against
  what the model can express: a backend whose payload cannot fill the four day parts waits for a
  model change rather than half-filling the table (step 24 records the daily-only national sources
  this excludes).

## Out of scope

Open-Meteo's single-runs/previous-runs/climate APIs, marine *historical* data, sea ice and
current-based routing, MET Norway's Nowcast and MetAlerts (step 15's alert registry covers alerting,
including the global WMO SWIC and FPAS aggregators), Visual Crossing's `forecastDataset`, and
provider flags beyond `--marine`/`--date`/`--history`.

## Verification

```bash
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && reuse lint

cargo run -q -- provider list                    # 12 rows, KEY column shows the env vars
cargo run -q -- -p met-no -f one-line --lat 59.91 --lon 10.75
#   within 2 °C of `-p open-meteo`, exit 0, User-Agent with contact info (checked with -vv)
cargo run -q -- -p visualcrossing --lat 38.97 --lon -77.35; echo $?
#   exit 6 plus "run `cirrocast key set visualcrossing`" when no key is configured
CIRROCAST_VISUALCROSSING_KEY=demo cargo run -q -- --alerts -f alerts --lat 38.97 --lon -77.35
#   alerts listed with source "visualcrossing"
cargo run -q -- -p open-meteo-archive --date 2026-09-14 --lat 52.52 --lon 13.41 -f plain
#   dated archive block; the same provider with --days 3 exits 2
cargo run -q -- --marine --lat 54.54 --lon 10.23 # marine panel appended to art-table
cargo run -q -- -p open-meteo-marine --lat 54.54 --lon 10.23; echo $?   # exit 2, supplementary only
```

## Exit criteria

- ⬜ `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`, `reuse lint` clean.
- ⬜ All 46 met.no base symbols and their `_day`/`_night`/`_polartwilight` variants map to the WMO
      codes above, including the double-`s` spellings, and `Expires`/`If-Modified-Since` caching
      demonstrably avoids a second body transfer (304 fixture).
- ⬜ `auto` resolves to `open-meteo,met-no,smhi`, every entry answers for a Stockholm point, and
      `--station ESSA` prepends `metar`; the README matrix, the `docs/plans/README.md` contract text
      and `keys.rs` list all twelve ids and both new env vars, with no stale capacity claim left.
- ⬜ Every registry row is re-verified against live docs with corrections in the Progress log.

## Risks

* **SMHI's documented forecast endpoint is dead today** (Progress log): `auto` still lists `smhi` as
  specified, but the adapter must be re-pointed at the product the portal now publishes; if no
  keyless SMHI forecast endpoint exists, the chain ships as `open-meteo,met-no` and the deviation is
  recorded in the Progress log and CHANGELOG rather than linking a 404.
* met.no's `403` policy means a bad `User-Agent` (proxied builds, distro packagers) breaks the
  backend, and Visual Crossing record accounting is easy to underestimate (15-day hourly ≈ 360
  records, a third of the free daily quota); mitigated by one documented UA constant carrying
  contact information, by requesting only the days asked for, and by documenting `queryCost`.

## Progress log

- 2026-09-30 — step opened; registry re-checks run with `curl -sI`/`curl -s` and a
  `cirrocast-planning/0.1` User-Agent. `open-meteo` `max_days: 16` confirmed (docs: 0–16), no
  correction. `smhi`: every `pmp3g` path probed on `opendata-download-metfcst.smhi.se` (`/api.json`,
  `/api/category/pmp3g.json`, `/…/version/2.json`, `/…/geotype/point/lon/16/lat/58/data.json`,
  `/…/geopoint/lat/59.33/lon/18.07/data.json`) answers **404** while `snow1g` on the same host answers
  200, and the registry's `docs_url` (`https://opendata.smhi.se/apidocs/metfcst/index.html`) is 404
  too — correction: that `docs_url` moves to the current portal (`https://opendata.smhi.se/`,
  `https://opendata.smhi.se/sitemap.xml`) and the forecast endpoint is re-derived before the adapter
  is written. New rows measured: met-no 200 with `expires`/`last-modified`, 9-day horizon;
  visualcrossing 15 days, free plan 1 000 records/day; open-meteo-marine accepts `forecast_days=8`
  (192 hourly slots, so `max_days: 8`, not the docs default 7); open-meteo-archive covers 1940-01-01
  onward with ERA5's ~5-day latency.
- 2026-10-03 — plan amended after the breezy-weather audit: `auto` is explicitly the *interim* fixed
  list (step 24 makes it coverage-aware), the source-admission criteria gained the audit's rules
  (documented public endpoint, user-supplied credentials only, no reskins, no reverse-engineered
  APIs, no card/phone for a free key), and the alert note now points at step 15's independent alert
  registry instead of MeteoAlarm alone.
- 2026-10-04 — renumbered from 19 to 23 by the plan reorganization; the interim `auto` list is still replaced by step 24, whose dependency on this file is unchanged.
