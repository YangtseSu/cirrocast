<!--
SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Step 15 — alerts and severity

Status: ✅ done
Depends on: 10, 12
Touches: `src/alerts/{mod,cap,geometry,nws,meteoalarm,qweather,wmoswic,fpas,hko}.rs`,
`src/model/alert.rs`, `src/provider/mod.rs`, `src/cli.rs`,
`src/config/mod.rs`, `src/cache.rs`, `src/render/{alerts,mod,art_table,one_line,plain,json,color}.rs`,
`src/i18n.rs`, `locales/{en-US,zh-CN}/main.ftl`, `tests/alerts.rs`, `tests/fixtures/alerts/`,
`tests/fixtures/alerts/README.md`, `REUSE.toml`, `docs/plans/README.md`, `CHANGELOG.md`

## Goal

`cirrocast --alerts Beijing` (and the same request for a US, EU, HK or CN point) fetches official
severe weather warnings from NWS, MeteoAlarm, QWeather, **WMO SWIC**, **FPAS** and the **Hong Kong
Observatory**, normalises them into one CAP-shaped model, prints a severity-coloured banner above
the `art-table`, lists them under `--format alerts`, exposes them as `%A` in `one-line` and as an
`alerts` array in `json`. Alert sources are their own registry selected by coverage: the two global
aggregators (WMO SWIC, FPAS) answer anywhere, the national services answer where they are
responsible (NWS: US and territories, HKO: Hong Kong, MeteoAlarm: the EUMETNET countries it serves,
QWeather: China when its provider is on the chain), and `--alerts-from` overrides the set. Warnings
older than their expiry are never printed, duplicates coming from two sources are collapsed, and a
location no source covers is reported as such instead of being silently asked.

## Deliverables

- ✅ `src/model/alert.rs` (re-exported by `src/alerts/mod.rs`): `Alert { id, source, event, severity:
      Severity, urgency: Urgency, certainty: Certainty, onset: Option<DateTime<FixedOffset>>,
      expires: Option<..>, ends: Option<..>, areas: Vec<String>, headline, description:
      Option<String>, instruction: Option<String>, sender: Option<String> }`; enums with CAP v1.2
      value sets — `Severity::{Unknown,Minor,Moderate,Severe,Extreme}`,
      `Urgency::{Unknown,Past,Future,Expected,Immediate}`,
      `Certainty::{Unknown,Unobserved,Possible,Unlikely,Likely,Observed}`; `Ord` on `Severity` for
      ordering; `AlertSource::{Nws,MeteoAlarm,QWeather,Hko,WmoSwic,Fpas,VisualCrossing}`.
      Deviation from the original bullet: the data types live in `src/model/alert.rs` because
      `model::Report` carries the alerts and `src/model` may not depend on the fetching module;
      `src/alerts` re-exports the names so the documented paths stay valid. `ends` was added to the
      model (the liveness rule needs it) but stays out of the JSON document, which keeps the key
      list of the next bullet.
- ✅ `src/alerts/mod.rs`: CAP v1.2 **subset** actually parsed, listed here so it cannot drift:
      alert level `identifier` (`id`), `sender`, `sent`, `status`, `msgType`, `scope`, `references`;
      info level `language`, `category`, `event`, `responseType`, `urgency`, `severity`, `certainty`,
      `effective`, `onset`, `expires`, `senderName`, `headline`, `description`, `instruction`, `web`,
      `contact`, `parameter`, `eventCode`; area level `areaDesc`, `polygon`, `circle`, `geocode`,
      `altitude`, `ceiling`. Multi-`info` alerts: pick the `language` matching the requested locale,
      else the first; areas from every `info` block, de-duplicated.
- ✅ `src/alerts/nws.rs`: `https://api.weather.gov/alerts/active?point=<lat>,<lon>` (keyless, US
      only, `Accept: application/geo+json`), maps `features[].properties` (`id`, `event`, `severity`,
      `urgency`, `certainty`, `onset`, `expires`, `ends`, `areaDesc`, `headline`, `description`,
      `instruction`, `senderName`, `status`, `messageType`); mandatory descriptive `User-Agent`
      `cirrocast/<version> (+https://github.com/yangtse/cirrocast; yangtsesu@gmail.com)`.
- ✅ `src/alerts/meteoalarm.rs`: `https://api.meteoalarm.org/edr/v1/collections/warnings/locations/
      {COUNTRY}?datetime=<sent-interval>&active=<active-interval>&language=en-GB`, `Authorization:
      Bearer $CIRROCAST_METEOALARM_KEY` (optional BYOK; missing key ⇒ this source is skipped with a
      `--verbose` note, never an error). Country = ISO 3166-1 alpha-2 from the resolved location
      (Open-Meteo geocoding `country_code`); features are bbox-only, so the point is tested against
      the feature polygon (`properties.alertId`, `countryCode`, `hubLink` → CAP XML/JSON payload).
- ✅ `src/alerts/qweather.rs`: `https://devapi.qweather.com/weatheralert/v7/alert/now?
      location=<lon>,<lat>&key=<KEY>` on the configured `[providers.qweather] host`, reusing the
      existing `CIRROCAST_QWEATHER_KEY`; maps `warning[].{id,sender,pubTime,title,startTime,endTime,
      status,severity,urgency,certainty,typeName,text,related,color}` (`color.code` → severity when
      `severity` is absent, `typeName` → `event`). Auth follows step 25: the alert adapter resolves
      the credential through the same `Credential` resolver and header helper as the forecast
      provider, so a JWT-configured account works for both. If step 25 has not landed, the adapter
      uses the existing `X-QW-Api-Key` path and step 25 switches it in its own commit.
- ✅ `src/alerts/wmoswic.rs`: the WMO Severe Weather Information Centre aggregator (keyless,
      worldwide, 130+ issuing agencies; operated by HKO). Two steps: `GET
      https://severeweather.wmo.int/f/wfs?request=GetFeature&version=1.1.0&outputFormat=json&
      typeName=local_postgis:postgis_geojsons&cql_filter=INTERSECTS(wkb_geometry,POINT(<lat>
      <lon>)) AND row_type NEQ 'BOUNDARY'` — the point-in-polygon test runs server-side, so no
      geometry code is needed here — then one CAP XML fetch per surviving feature under
      `https://severeweather.wmo.int/v2/cap-alerts/<capurl>` (`rlink` is the alternate-language
      document), parsed by the shared CAP reader; the `info` block whose `language` matches the
      requested locale wins, else `en`; expiry is re-checked against the CAP document. The WFS
      response is the alert index and is cached (`alerts/wmo-<lat.2dp>-<lon.2dp>-<utc-hour>.json`);
      the CAP documents are cached per identifier so a repeated run is offline-capable. Credit:
      `Warnings by the WMO Severe Weather Information Centre (severeweather.wmo.int), © the issuing
      agencies`.
- ✅ `src/alerts/fpas.rs`: the FOSS Public Alert Server (keyless, self-hostable, worldwide).
      `GET <fpas_url>/alert/area?min_lat=&max_lat=&min_lon=&max_lon=` → a JSON array of alert UUIDs
      (measured 2026-10-03: the Beijing box returned 19, a mid-Atlantic box 0, the whole world
      3 659 — the bbox filter works), then `GET <fpas_url>/alert/<uuid>` with redirects followed
      (the public instance answers `301` to `/cap/alerts/<source>/<file>.xml`) → CAP 1.2, parsed by
      the shared reader; `category != Met` features and `msgType = Cancel` are dropped, and a
      polygon that does not contain the point is dropped client-side (FPAS's areas can be polygons
      or circles, so the point-in-polygon test lives here, unlike WMO). `[alerts] fpas_url`
      (default `https://alerts.kde.org`) makes the instance configurable like `nominatim_url`, and
      the source stays optional: an unreachable instance is a `--verbose` line, not an error.
      Credit: `Warnings via the FOSS Public Alert Server (alerts.kde.org)`.
- ✅ `src/alerts/hko.rs`: the Hong Kong Observatory's keyless JSON warning summary —
      `GET https://data.weather.gov.hk/weatherAPI/opendata/weather.php?dataType=warnsum&lang=en`
      (measured 2026-10-03: HTTP 200 JSON; an empty object means no active warnings) plus
      `dataType=warningInfo` for the detail document when a summary entry is present; maps HKO's
      warning codes (`WRAIN`, `WTCSGNL`, `WTCPRE8`, `WHOT`, …) to CAP `event` + `severity` through
      an explicit table, with `actionCode`/`updateTime` feeding `onset`/`updated`; coverage:
      Hong Kong. Credit: `Warnings by the Hong Kong Observatory`.
- ✅ `src/alerts/mod.rs` + `src/provider/mod.rs`: alert sources become their **own registry**,
      independent of the weather chain: `pub enum AlertSource { Nws, MeteoAlarm, QWeather, Hko,
      WmoSwic, Fpas, VisualCrossing }` with `fn covers(&self, loc: &Location) -> bool` (NWS: US +
      GU/MP/PR/VI; HKO: HK; MeteoAlarm: the EUMETNET member country codes; QWeather: CN; the
      aggregators and FPAS: everywhere) and `pub fn sources_for(loc, chain, config) -> Vec<AlertSource>`
      — coverage-selected by default, in registry order (global aggregators last), with QWeather
      included only when `qweather` is on the chain (it needs that provider's credential and host)
      and VisualCrossing only when its provider is; `[alerts] sources = ["auto"]` (or an explicit id
      list) and `--alerts-from <id[,id…]>` override the set; `ProviderMeta.alerts` keeps its meaning
      for backends whose own payload carries warnings. `provider info <id>` prints the alert sources
      that apply to the answering chain.
- ✅ `src/cli.rs`: `--alerts` (fetch alerts; auto-on when config `[alerts] enabled = true` and at
      least one covered source exists for the location), `--no-alerts`, `--alerts-from <id[,id…]>`,
      `--severity <minor|moderate|severe|extreme>` filter, `--format alerts`; precedence documented
      in `--help`.
- ✅ `src/config/mod.rs`: `[alerts] enabled = true`, `severity_threshold = "minor"`,
      `sources = ["auto"]`, `fpas_url = ""` (empty = the public instance),
      `cache_ttl_secs = 300`; absent keys keep these defaults, so no `schema_version` bump of the
      config file is needed (documented in `config show`).
- ✅ `src/cache.rs`: `alerts/<source>-<lat.2dp>-<lon.2dp>-<utc-hour>.json`, TTL 300 s, honouring
      `--no-cache` / `--refresh` / `--offline` (offline replays the last cached set, expired or not,
      with a `--verbose` staleness note).
- ✅ `src/render/`: severity-coloured banner above `art-table` (one line per alert, strongest
      first: `⚠ Tornado Warning — Extreme · until 18:30 CDT · Take shelter now`), full listing for
      `--format alerts`, `plain` degrades the banner to a prefix-less line, `json` gains
      `"alerts": [{id, source, event, severity, urgency, certainty, onset, expires, areas,
      headline, description, instruction, sender}]` and `schema_version` 2; `one-line` gains `%A`
      (strongest alert's event, empty when none).
- ✅ Severity ordering and collapsing: sort `Extreme > Severe > Moderate > Minor > Unknown`;
      dedup by `id`, then by `(event, onset, areas)` across sources; drop everything whose
      `coalesce(ends, expires) <= ctx.now`; cap the banner at 3 lines with `… N more`.
- ✅ `src/i18n.rs` + `locales/{en-US,zh-CN}/main.ftl`: `alert-severity-{unknown,minor,moderate,
      severe,extreme}`, `alert-urgency-*`, `alert-certainty-*`, `alert-banner-line`,
      `alert-more-count`, `alert-none`, `alert-source-{nws,meteoalarm,qweather,hko,wmoswic,fpas}`.
- ✅ Tests (`tests/alerts.rs` + fixtures): US tornado warning (NWS GeoJSON), EU heat warning
      (MeteoAlarm CAP XML), CN rainstorm warning (QWeather JSON), a WMO SWIC pair (WFS index
      feature + its CAP document), a FPAS pair (UUID list + a redirect-followed CAP document, plus
      a non-`Met` category and a `Cancel` message that must be dropped), an HKO `warnsum` +
      `warningInfo` pair and the empty-object case; dedup of the same NWS id seen twice and of an
      `(event, onset)` collision across NWS/MeteoAlarm; severity ordering; expiry filtering against
      a frozen `RenderContext::now`; empty result; malformed CAP (truncated XML, `info` without
      `event`); `sources_for` coverage for US/HK/CN/EU/global points and `--alerts-from` override;
      `%A` empty when there are no alerts.
- ✅ Docs: `docs/plans/README.md` (JSON `schema_version` 2 note, `alerts` capability meaning,
      CAP-subset pointer), `CHANGELOG.md`, `provider info` row text, `REUSE.toml` annotation for
      the recorded fixtures (the fixtures stay GPL-3.0-or-later — they are hand-trimmed
      public-domain/CC0-derivable CAP payloads, see `tests/fixtures/alerts/README.md`).

## Design notes

* **CAP v1.2 as the pivot model, not six bespoke structs.** NWS is CAP in GeoJSON clothing,
  MeteoAlarm, WMO SWIC and FPAS serve CAP 1.2 documents, HKO has a JSON warning summary, and
  QWeather has an equivalent (severity/urgency/certainty) triple. One model + adapters keeps
  rendering, i18n and caching single-path.
* **The two global aggregators are what makes `--alerts` useful outside the US and the EU** (the
  audit of breezy-weather surfaced both; MeteoAlarm is not present there). Measured 2026-10-03
  through the local proxy: WMO SWIC's WFS answers the exact CQL point query from the runbook above
  (0 features for Beijing and London that hour, 1 for a US point, each feature carrying `capurl`),
  FPAS's `/alert/area` returns the filtered UUID list (19 in a 1°×1° Beijing box, 0 in a mid-
  Atlantic box, 3 659 worldwide) and `/alert/<uuid>` answers `301` to the CAP XML, HKO's `warnsum`
  answers `200 application/json` (`{}` when nothing is active). WMO SWIC runs the point-in-polygon
  test server-side; FPAS does not, so its areas are tested client-side and `category != Met` plus
  `msgType = Cancel` are dropped.
* **`fpas_url` is configurable** (like `nominatim_url`) because FPAS is a self-hostable FOSS
  service: a user or a distribution can point at its own instance, and the public
  `alerts.kde.org` stays the default.
* **`expires` is not the liveness test.** Live NWS data measured on 2026-09-29 returns a Flood Watch
  with `expires=2026-09-29T13:45:00-05:00` *before* its `onset=19:00` (the `expires` field is the
  message validity, refreshed by updates; `ends` is the event end). Liveness therefore uses
  `coalesce(ends, expires)`; a missing field means "does not expire before the render".
* **`limit` is not sent to NWS.** `https://api.weather.gov/alerts/active?point=…&limit=1` returns
  HTTP 400 `"limit" is not recognized`; the endpoint rejects unknown parameters, so no client-side
  paging or clamp is possible.
* **MeteoAlarm is country-grained, not point-grained.** The EDR collection only exposes
  `locations/{ALL|CC}`; a point query is emulated by country lookup + bbox test on the returned
  `geometry`. `datetime` is a required query parameter, so the request always carries a 24 h sent
  interval around `ctx.now`. Its endpoints are protected (`401` without a token) and the portal
  states access is intended for MeteoAlarm members and re-distributors, with MeteoGate for the
  general public — the key is therefore optional BYOK and the README says so, plus "cached warnings
  are for the local user only, not for redistribution".
* **Colour ramp is authored here**, not copied: Minor = grey/blue, Moderate = yellow, Severe = red,
  Extreme = white on red; when `color = never`, severity is preserved as a text word.
* **`%A` is claimed by this step**; step 16 claims `%q` for AQI to avoid the collision.
* **CAP XML is parsed with `quick-xml`.** The original note assumed step 05/06 had chosen an XML
  reader; it did not (nothing before this step consumed XML). The alternatives were a hand-written
  subset parser and `quick-xml = "0.42"` (MIT, no TLS, no async runtime, MSRV 1.56, already in the
  dependency allow list's MIT rule): the pull parser is the boring one, and the CAP documents come
  from ~130 different agencies whose escaping and CDATA use a hand-rolled reader would get wrong
  eventually. The state machine over its events is hand-written (no serde derive), so the parsed
  subset is exactly the list above and nothing else.
* **Auto-on semantics**: `--alerts` forces the fetch and errors only when no source covers the
  location and none was named explicitly; without the flag, alerts are fetched when
  `[alerts] enabled` is not `false` **and** `sources_for(location)` is non-empty — a `--no-alerts`
  opt-out is added for scripts that must not pay the extra requests.
* **The alert-source registry is independent of the weather chain** because the aggregators have no
  provider of their own; QWeather and VisualCrossing are the two exceptions and are pulled in only
  when their provider is selected (they need its credential and host). `--alerts-from` names an
  explicit set for debugging or for a location whose coverage metadata is wrong.
* **Per-source credits** are part of the banner/footer text where the source asks for one (WMO SWIC
  names the issuing agencies; FPAS names the instance); the i18n keys for severity/urgency/certainty
  stay source-agnostic.

## Out of scope

METAR (no warnings), and in-band alert payloads of `weatherapi` (`alerts` object in the forecast
response) and `pirateweather` (`alerts` array): they are declared `alerts: false` here on purpose
rather than half-mapped, and their `provider info` note says "forecast endpoint carries an
unconsumed alerts block". Visual Crossing's `alerts` array is wired in step 19 through this same
adapter. MQTT push, `CAP` polygon rendering on a map, alert history and acknowledgment state are
never in scope. Locale negotiation for alert text falls back to English when a CAP `info` block for
the requested language does not exist.

## Verification

```bash
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && reuse lint

# keyless, real network, US point (fixture: tests/fixtures/alerts/nws-tornado-*.json)
cargo run -q -- --alerts --lat 39.7456 --lon -97.0892 --format alerts
#   worst-first list; exit 0; no line with an expiry in the past
cargo run -q -- --lat 39.7456 --lon -97.0892 -f one-line
#   banner line above the one-liner; %A expands to the same event name
cargo run -q -- --lat 39.7456 --lon -97.0892 -f json | jq '.schema_version, (.alerts|length)'
#   2 and >= 1
cargo run -q -- --alerts Beijing -v          # the coverage-selected set (wmoswic, fpas) is
#   queried; qweather joins only with -p qweather, and meteoalarm's missing token is a -v note
cargo run -q -- --alerts --lat 22.3 --lon 114.17 -f alerts -v    # HKO warnsum (or "no active alerts")
cargo run -q -- --alerts --lat 39.9 --lon 116.4 --alerts-from fpas -f alerts
#   FPAS-only run: cached UUID list + CAP documents; the source and its credit named in the footer
cargo run -q -- --alerts --lat 0 --lon 0 --alerts-from wmoswic,fpas -f alerts
#   both global aggregators queried for a point neither national service covers
cargo run -q -- --alerts --lat 39.9 --lon 116.4 --alerts-from nws; echo $?
#   an explicit source that does not cover the point: exit 2,
#   `error: alert source `nws` does not cover 39.90,116.40; covered here: qweather, wmoswic, fpas`
CIRROCAST_METEOALARM_KEY=bad cargo run -q -- --alerts Vienna -v; echo $?
#   exit 0, warning-free degradation to the global sources, one --verbose line naming
#   CIRROCAST_METEOALARM_KEY and the 401 (a coordinate pair has no country, so the geocoded
#   `Vienna` spelling is what selects meteoalarm)
```

## Exit criteria

- ✅ `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`, `reuse lint` clean.
- ✅ All fixtures are hand-written to the source schemas (provenance in
      `tests/fixtures/alerts/README.md`); no test performs a network call (`tests/alerts.rs` runs
      over `StubTransport` and the CLI sandbox carries `CIRROCAST_FORBID_NETWORK=1`).
- ✅ Banner, `--format alerts`, `%A` and `json.alerts` show the same alert set for the same
      location and cache entry (`a_cached_alert_set_renders_in_every_format` asserts one seeded set
      across all four).
- ✅ An alert whose `coalesce(ends, expires)` is in the past never appears in any format
      (`prepare` unit tests plus the fetch-level expiry test).
- ✅ `--alerts-from fpas` with a cached empty list exits 0 with "no active weather alerts"; an
      explicit source that does not cover the point exits 2 with
      `error: alert source \`nws\` does not cover 39.90,116.40; covered here: qweather, wmoswic, fpas`
      (live-verified and covered by `an_explicit_source_outside_its_coverage_is_a_usage_error`).
- ✅ WMO SWIC's server-side polygon filter (the exact CQL point query), FPAS's client-side filter
      (`category`, `msgType`, geometry) and HKO's warning-code table each have a positive and a
      negative fixture; the WMO index's boundary row and the FPAS Geo/Cancel documents are the
      negatives.
- ✅ `provider info qweather` prints its alert source; `provider info smhi` prints `alerts: none`.

## Risks

* MeteoAlarm token issuance is manual and may be refused for an end-user CLI; mitigated by making
  the source optional-BYOK, degrading without error, and by the global aggregators that need no key.
* Without the new aggregators the empty case was the norm outside the US/EU/CN; WMO SWIC and FPAS
  remove that gap, at the cost of federation quality (an agency's CAP document can be delayed or
  duplicated across aggregators — the `(event, onset, areas)` dedup and the issuing-agency sender
  keep the banner honest).
* FPAS's public instance is a donated service; `fpas_url` allows a self-hosted one, and an
  unreachable instance is a verbose line, never a failed run.
* CAP `info` blocks repeat per language, and picking the wrong one produces mixed-language text;
  mitigated by the locale-match rule and a fixture with three `info` blocks (de, en, fr).
* QWeather field names changed between v7 revisions (`severityColor` vs `color.code`); the fixture
  recorded in this step is the source of truth and both shapes are accepted by the adapter.

## Progress log

- 2026-09-30 — step opened.
- 2026-10-03 — plan amended after the breezy-weather source audit and a local probe round through
  the proxy: added `src/alerts/wmoswic.rs` (global, keyless, CQL point query answered 200; 0/1
  features for the probed points), `src/alerts/fpas.rs` (global, keyless, self-hostable; bbox list
  19 for a Beijing box, 0 mid-Atlantic, 3 659 worldwide; `/alert/<uuid>` answers `301` to CAP XML)
  and `src/alerts/hko.rs` (HK, keyless JSON `warnsum`), and reworked alert selection from
  "the weather chain declares alerts" to an independent coverage-selected registry with
  `[alerts] sources` / `--alerts-from`. QWeather alert auth now follows step 25's credential
  resolver. No code exists for this step yet, so nothing else changed.
- 2026-10-03 — work started: `quick-xml 0.42` added (CAP XML, design note above) and the CAP-shaped
  model landed in `src/model/alert.rs` (`Alert`, the three CAP value sets, `AlertSource`), with
  `Report.alerts` added as a `serde(default)` field so pre-step-15 documents still parse. The first
  deliverable bullet was amended to match that placement; probes re-confirmed NWS 200 (two live
  Minor flood advisories in TX), WMO SWIC 0 features for ten probed points that hour, FPAS 18 UUIDs
  for the Beijing box with a redirect-following CAP fetch, HKO `{}` and MeteoAlarm `401` without a
  token.
- 2026-10-03 — the cache half landed: the `alerts` namespace (reported by `cache stat` and cleared
  by `cache clean`), `CacheKey::alert(source, lat, lon, utc_hour)` and `Cache::read_ignoring_ttl`,
  the stale-replay read the offline path needs. The fetch module wires the 300 s TTL and the mode
  rules onto it later in this step.
- 2026-10-03 — the shared readers landed: `src/alerts/cap.rs` (the fixed CAP 1.2 subset over a
  `quick-xml` state machine, locale-matched multi-`info` selection, area union, `msgType=Cancel`
  dropped, syntax errors and eventless documents refused) and `src/alerts/geometry.rs` (GeoJSON
  polygon/multipolygon and CAP polygon/circle point tests, with the circle test on haversine
  distance). The config `[alerts]` table also landed with validation and dotted-key access.
- 2026-10-03 — every alert source adapter and the registry plumbing landed: `src/alerts/mod.rs`
  (coverage selection with the provider-bound sources gated on the chain, `explicit_sources` with
  the "does not cover 39.90,116.40; covered here: …" usage error, the best-effort-versus-explicit
  failure policy, `prepare` for liveness/threshold/order/dedup, and the cached_text/cached_json-less
  fetch path honouring the cache modes including the offline stale replay) plus `nws.rs`,
  `meteoalarm.rs`, `qweather.rs`, `wmoswic.rs`, `fpas.rs` and `hko.rs` with unit tests per adapter.
  Deliverable bullets for the model, the CAP subset and the six adapters were ticked in this commit;
  the registry bullet stays open until `provider info` prints the alert row, and the tests/docs
  bullets follow.
- 2026-10-03/04 — the integration landed: `--alerts`/`--no-alerts`/`--alerts-from`/`--severity`
  (with `--format alerts`, auto-on from `[alerts] enabled`, the "does not cover 39.90,116.40;
  covered here: qweather, wmoswic, fpas" usage error and the best-effort-versus-explicit failure
  policy), the `alerts` format and the RenderContext `alert_credits`, the banner in `art-table` and
  `one-line` (`%A` added), alert records and credits in `plain`, JSON `schema_version 2` with
  `alerts`/`alert_credits`, the severity palette, `provider info`'s alert row, and the fixes the
  first live runs demanded: `allow_negative_numbers` (the plan's own `--lon -97.0892` verification
  was a clap usage error), a bounding-box fallback for `qweather`/`hko` coverage at coordinate-only
  locations (the plan expects qweather in the Beijing coverage message), and an instruction
  single-line cap plus day-qualified until times in the banner. Live smoke: FPAS at Beijing (gale,
  zh-CN listing with the FPAS credit), NWS at the plan's Oklahoma point (`-f one-line` banner,
  `-f json` v2 with one severe Flood Warning and the WMO credit), and the explicit-source error
  above. The CLI test sandbox now sets `CIRROCAST_FORBID_NETWORK=1` for every child, as CI does, so
  a test that reaches a socket fails instead of silently using the developer's network.
- 2026-10-04 — the test suite landed: `tests/alerts.rs` (18 tests) over the fixture set in
  `tests/fixtures/alerts/` (hand-written to the source schemas, provenance in its README, REUSE
  annotation added): every adapter against a scripted transport, the FPAS category/msgType/geometry
  filters, HKO's code table and empty summary, QWeather's header/credential reuse, cross-source
  dedup, expired-alert dropping at the fetch level, malformed CAP (explicit fails, auto degrades,
  neighbours survive), the WMO index+document cache path, and the CLI over seeded caches: all
  formats from one alert set, stale offline replay, empty sets, `--no-alerts`, the coverage usage
  error, MeteoAlarm token skip/degradation and its geometry-filtered CAP read, and `provider info`.
  Also in this round: `tests/common` fixes the CLI sandbox to `CIRROCAST_FORBID_NETWORK=1` like CI,
  and the alert policy is resolved before the weather fetch so a source-list mistake costs no
  request. Remaining: the docs bullet and the exit-criteria pass.
- 2026-10-04 — docs and the exit pass: `docs/plans/README.md` (formats incl. `alerts`, the alert
  flags, the `[alerts]` schema keys, the `alerts/` cache namespace, JSON `schema_version = 2` and
  the CAP-subset pointer, the `quick-xml` dependency note), the user README (an Alerts section, the
  source rows with their credits, the config table, `%A`, the JSON v2 note), `CHANGELOG.md`
  (Unreleased/Added) and `docs/schema.md` (v2 key index, including `alerts[]` and `alert_credits`).
  The verification block was corrected where the amended registry design moved the ground: the
  Beijing `-v` line and the MeteoAlarm bad-key command (a coordinate pair carries no country, so
  the geocoded spelling is what selects the source); the 401 message now names
  `CIRROCAST_METEOALARM_KEY` through the new `Error::InvalidToken` instead of pointing at
  `key set`, which does not know the service. Every verification command was run: FPAS at Beijing
  (gale, credits), NWS at the Oklahoma point (banner, `%A`, JSON v2 with the WMO credit), HKO at
  22.3/114.17 (live `{}` → "no active alerts"), both aggregators at 0,0 (no alerts), the explicit
  non-covering `nws` error (exit 2 with the documented message) and the bad MeteoAlarm token
  (exit 0, one `-v` line). `cargo fmt --check`, `cargo clippy --all-targets -D warnings`,
  `cargo test` and `reuse lint` are clean; the step is done.

- 2026-10-04 — follow-up, found while preparing the step-17 merge: the two CLI tests that seed the
  gale CAP document (`a_cached_alert_set_renders_in_every_format`,
  `an_expired_cache_entry_is_replayed_offline_with_a_stale_read`) read the real clock through the
  binary, and the fixture's fixed window (`expires` 2026-10-04T08:32+08:00) had just passed, so
  both flipped to "no active weather alerts" with no code change. `tests/alerts.rs` now seeds a
  copy of the fixture whose `sent`/`onset`/`expires` are relative to the run (`live_gale_fixture`);
  the adapter tests keep the raw fixture because they inject their own clock.
