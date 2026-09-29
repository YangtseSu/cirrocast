<!--
SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Step 15 — alerts and severity

Status: ⬜ not-started
Depends on: 10, 12
Touches: `src/alerts/{mod,nws,meteoalarm,qweather}.rs`, `src/provider/mod.rs`, `src/cli.rs`,
`src/config/mod.rs`, `src/cache.rs`, `src/render/{mod,art_table,one_line,plain,json,color}.rs`,
`src/i18n.rs`, `locales/{en-US,zh-CN}/main.ftl`, `tests/alerts.rs`, `tests/fixtures/alerts/`,
`tests/fixtures/alerts/README.md`, `REUSE.toml`, `docs/plans/README.md`, `CHANGELOG.md`

## Goal

`cirrocast --alerts Beijing` (and the same request for a US or EU point) fetches official severe
weather warnings from NWS, MeteoAlarm and QWeather, normalises them into one CAP-shaped model,
prints a severity-coloured banner above the `art-table`, lists them under `--format alerts`, exposes
them as `%A` in `one-line` and as an `alerts` array in `json`. Warnings older than their expiry are
never printed, duplicates coming from two sources are collapsed, and a provider that has no alerts
(`open-meteo`, `smhi`) is reported as such instead of being silently asked.

## Deliverables

- ⬜ `src/alerts/mod.rs`: `Alert { id, source, event, severity: Severity, urgency: Urgency,
      certainty: Certainty, onset: Option<DateTime<FixedOffset>>, expires: Option<..>, areas:
      Vec<String>, headline, description: Option<String>, instruction: Option<String>, sender:
      Option<String> }`; enums with CAP v1.2 value sets — `Severity::{Unknown,Minor,Moderate,Severe,
      Extreme}`, `Urgency::{Unknown,Past,Future,Expected,Immediate}`,
      `Certainty::{Unknown,Unobserved,Possible,Unlikely,Likely,Observed}`; `Ord` on `Severity` for
      ordering; `AlertSource::{Nws,MeteoAlarm,QWeather,VisualCrossing}`.
- ⬜ `src/alerts/mod.rs`: CAP v1.2 **subset** actually parsed, listed here so it cannot drift:
      alert level `identifier` (`id`), `sender`, `sent`, `status`, `msgType`, `scope`, `references`;
      info level `language`, `category`, `event`, `responseType`, `urgency`, `severity`, `certainty`,
      `effective`, `onset`, `expires`, `senderName`, `headline`, `description`, `instruction`, `web`,
      `contact`, `parameter`, `eventCode`; area level `areaDesc`, `polygon`, `circle`, `geocode`,
      `altitude`, `ceiling`. Multi-`info` alerts: pick the `language` matching the requested locale,
      else the first; areas from every `info` block, de-duplicated.
- ⬜ `src/alerts/nws.rs`: `https://api.weather.gov/alerts/active?point=<lat>,<lon>` (keyless, US
      only, `Accept: application/geo+json`), maps `features[].properties` (`id`, `event`, `severity`,
      `urgency`, `certainty`, `onset`, `expires`, `ends`, `areaDesc`, `headline`, `description`,
      `instruction`, `senderName`, `status`, `messageType`); mandatory descriptive `User-Agent`
      `cirrocast/<version> (+https://github.com/yangtse/cirrocast; yangtsesu@gmail.com)`.
- ⬜ `src/alerts/meteoalarm.rs`: `https://api.meteoalarm.org/edr/v1/collections/warnings/locations/
      {COUNTRY}?datetime=<sent-interval>&active=<active-interval>&language=en-GB`, `Authorization:
      Bearer $CIRROCAST_METEOALARM_KEY` (optional BYOK; missing key ⇒ this source is skipped with a
      `--verbose` note, never an error). Country = ISO 3166-1 alpha-2 from the resolved location
      (Open-Meteo geocoding `country_code`); features are bbox-only, so the point is tested against
      the feature polygon (`properties.alertId`, `countryCode`, `hubLink` → CAP XML/JSON payload).
- ⬜ `src/alerts/qweather.rs`: `https://devapi.qweather.com/weatheralert/v7/alert/now?
      location=<lon>,<lat>&key=<KEY>` on the configured `[providers.qweather] host`, reusing the
      existing `CIRROCAST_QWEATHER_KEY`; maps `warning[].{id,sender,pubTime,title,startTime,endTime,
      status,severity,urgency,certainty,typeName,text,related,color}` (`color.code` → severity when
      `severity` is absent, `typeName` → `event`).
- ⬜ `src/provider/mod.rs`: add `alerts: bool` to `ProviderMeta` and to `Capabilities`; rows:
      `nws`-backed sources are *not* providers — `open-meteo` `false`, `smhi` `false`, `metar`
      `false`, `qweather` `true`, others `false` (see Out of scope), and a new
      `ProviderMeta.alert_sources: &'static [AlertSource]` so `provider info` can print
      "alerts: NWS, MeteoAlarm" for any location on the chain.
- ⬜ `src/cli.rs`: `--alerts` (auto-on when the selected chain offers alerts **and** config
      `[alerts] enabled = true`), `--no-alerts`, `--severity <minor|moderate|severe|extreme>` filter,
      `--format alerts`; precedence documented in `--help`.
- ⬜ `src/config/mod.rs`: `[alerts] enabled = true`, `severity_threshold = "minor"`,
      `cache_ttl_secs = 300`; absent keys keep these defaults, so no `schema_version` bump of the
      config file is needed (documented in `config show`).
- ⬜ `src/cache.rs`: `alerts/<source>-<lat.2dp>-<lon.2dp>-<utc-hour>.json`, TTL 300 s, honouring
      `--no-cache` / `--refresh` / `--offline` (offline replays the last cached set, expired or not,
      with a `--verbose` staleness note).
- ⬜ `src/render/`: severity-coloured banner above `art-table` (one line per alert, strongest
      first: `⚠ Tornado Warning — Extreme · until 18:30 CDT · Take shelter now`), full listing for
      `--format alerts`, `plain` degrades the banner to a prefix-less line, `json` gains
      `"alerts": [{id, source, event, severity, urgency, certainty, onset, expires, areas,
      headline, description, instruction, sender}]` and `schema_version` 2; `one-line` gains `%A`
      (strongest alert's event, empty when none).
- ⬜ Severity ordering and collapsing: sort `Extreme > Severe > Moderate > Minor > Unknown`;
      dedup by `id`, then by `(event, onset, areas)` across sources; drop everything whose
      `coalesce(ends, expires) <= ctx.now`; cap the banner at 3 lines with `… N more`.
- ⬜ `src/i18n.rs` + `locales/{en-US,zh-CN}/main.ftl`: `alert-severity-{unknown,minor,moderate,
      severe,extreme}`, `alert-urgency-*`, `alert-certainty-*`, `alert-banner-line`,
      `alert-more-count`, `alert-none`, `alert-source-{nws,meteoalarm,qweather}`.
- ⬜ Tests (`tests/alerts.rs` + fixtures): US tornado warning (NWS GeoJSON), EU heat warning
      (MeteoAlarm CAP XML), CN rainstorm warning (QWeather JSON); dedup of the same NWS id seen
      twice and of an `(event, onset)` collision across NWS/MeteoAlarm; severity ordering;
      expiry filtering against a frozen `RenderContext::now`; empty result; malformed CAP
      (truncated XML, `info` without `event`); `%A` empty when there are no alerts.
- ⬜ Docs: `docs/plans/README.md` (JSON `schema_version` 2 note, `alerts` capability meaning,
      CAP-subset pointer), `CHANGELOG.md`, `provider info` row text, `REUSE.toml` annotation for
      the recorded fixtures (the fixtures stay GPL-3.0-or-later — they are hand-trimmed
      public-domain/CC0-derivable CAP payloads, see `tests/fixtures/alerts/README.md`).

## Design notes

* **CAP v1.2 as the pivot model, not three bespoke structs.** NWS is CAP in GeoJSON clothing,
  MeteoAlarm serves CAP 1.2 documents, QWeather has an equivalent (severity/urgency/certainty)
  triple. One model + three adapters keeps rendering, i18n and caching single-path.
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
* **No new crates.** CAP XML is parsed with the XML reader already chosen in step 05/06 (recorded
  before `serde_json`-style structs are derived); if step 05 landed on `quick-xml`, the CAP reader
  uses it with a hand-written state machine over the 17 elements listed above (`quick-xml` 0.42.0,
  MIT, MSRV 1.56 — GPL-compatible).
* **Auto-on semantics**: `--alerts` forces the fetch and errors when nothing on the chain can serve
  it; without the flag, alerts are fetched only when at least one chain entry declares
  `alerts: true` and `[alerts] enabled` is not `false` — a `--no-alerts` opt-out is added for
  scripts that must not pay the extra request.

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
cargo run -q -- --alerts Beijing -v          # "qweather: alert source enabled" (no meteoalarm key)
cargo run -q -- --alerts -p smhi,open-meteo --lat 59.33 --lon 18.07; echo $?
#   usage error on stderr, exit 2: no provider on the chain offers alerts
CIRROCAST_METEOALARM_KEY=bad cargo run -q -- --alerts --lat 48.2 --lon 16.37 -v; echo $?
#   exit 0, warning-free degradation to NWS/no alerts, one --verbose line about the 401
```

## Exit criteria

- ⬜ `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`, `reuse lint` clean.
- ⬜ All fixtures are recorded files; no test performs a network call (`tests/alerts.rs` passes
      with the loopback blocked).
- ⬜ Banner, `--format alerts`, `%A` and `json.alerts` show the same alert set for the same
      location and cache entry.
- ⬜ An alert whose `coalesce(ends, expires)` is in the past never appears in any format.
- ⬜ `--alerts` with a chain that offers no alerts exits 2 with the exact message
      `error: none of the selected providers offers alerts`.
- ⬜ `provider info qweather` prints its alert source; `provider info smhi` prints `alerts: none`.

## Risks

* MeteoAlarm token issuance is manual and may be refused for an end-user CLI; mitigated by making
  the source optional-BYOK, degrading without error, and keeping NWS/QWeather keyless-or-existing.
* NWS is US-only and MeteoAlarm EU-only, so a global `--alerts` often returns nothing; the empty
  case is a first-class tested path with an explicit "no active alerts for <place>" line.
* CAP `info` blocks repeat per language, and picking the wrong one produces mixed-language text;
  mitigated by the locale-match rule and a fixture with three `info` blocks (de, en, fr).
* QWeather field names changed between v7 revisions (`severityColor` vs `color.code`); the fixture
  recorded in this step is the source of truth and both shapes are accepted by the adapter.

## Progress log

- 2026-09-30 — step opened.
