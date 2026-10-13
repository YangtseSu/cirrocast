<!--
SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Step 30 — QWeather GeoAPI as a location source

Status: ✅ done
Depends on: `04-geocoding-and-location-syntax.md` (the `Geocoder` trait, ranking and attribution lines), `05-http-cache-and-ip-location.md` (the shared client and cache), `18-offline-city-database.md` (the bundled table this source supplements), `20-location-candidate-selection.md` (the picker and the merged candidate list), `25-location-sources-2.md` (the multi-source chain, the merge and the GeoNames BYOK precedent), `27-qweather-jwt-auth.md` (the credential and host)
Touches: `src/geo/{mod,qweather,chain}.rs` (new `qweather.rs`; `merge.rs` and `rank.rs` needed no change), `src/auth` (reused), `src/model/mod.rs`, `src/config/mod.rs`, `src/cli.rs`, `src/render/json.rs`, `tests/{geo_qweather,geo_chain,geo_merge}.rs`, `tests/fixtures/qweather/{README.md,geo/}`, `docs/{providers,location,configuration,schema}.md`, `docs/schema/json-v2.json`, `README.md`, `docs/plans/README.md`, `CHANGELOG.md`

## Goal

`[geo] search` gains a fourth source: `qweather`, QWeather's GeoAPI (`/geo/v2/city/lookup`), which
answers a name query with QWeather's own city identity — the LocationID whose coordinates their
forecast endpoints are built around, the administrative chain (`adm1`/`adm2`), a 1:1 IANA zone and a
`rank`. It is the only source that can resolve a Chinese district or township
(`location=新乡县`) to the same point QWeather serves weather for, and the only one that returns a
zone name outright.

It is **explicit-selection only** (`[geo] search = "qweather"`, or `CIRROCAST_GEO_SEARCH=qweather`
for one run — the same config key and environment override the other three sources use, no new
flag), never part of `auto`: it needs the qweather credential and host, and its rows carry no ISO
country code, so the keyless `auto` chain keeps producing the fully-populated `Location`s the rest
of the crate expects.

## Deliverables

- ✅ **Live probe, recorded in the design notes below** (the account host + credential, before any
      code): `GET {host}/geo/v2/city/lookup?location=<name>&number=<n>&lang=<lang>`; confirm the
      response envelope (`code` + `location[]`), the row fields
      (`id`, `lat`/`lon` as **strings**, `adm2`, `adm1`, `country` as a localized name, `tz`,
      `utcOffset`, `type`, `rank`), whether any ISO country code appears anywhere in the payload,
      what `number` returns for a `新乡`-style query (district rows? duplicated names?), and how the
      service spells a Latin query's names (`lang=en` vs `lang=zh`). Correct this file where the
      service differs from its docs. **Measured 2026-10-13**: the envelope and row fields held, with
      `rank` a string too, **no ISO country code anywhere**, and — the one real divergence — a query
      matching nothing answers **HTTP 400 `#no-such-location`**, not a `200` with an empty array.
      See the design notes and the progress log.
- ✅ `src/geo/qweather.rs`: a `Geocoder` over the shared `HttpClient` **without a cache entry** (the
      storage rule below forbids keeping GeoAPI rows on disk; `-v` says the request is uncached), the
      credential and host resolved exactly
      as `src/alerts/qweather.rs` resolves them (API key or JWT through `QWeatherAuth`, a missing
      host is the documented configuration error naming the console, a missing credential under an
      explicit selection is the exit-6 credential error naming both `cirrocast key set qweather`
      forms and `CIRROCAST_QWEATHER_KEY`). **Correction**: that error is `Error::MissingCredential`,
      not `MissingKey` — `qweather` has two authentication modes (step 27), and the shared
      `provider::missing_credential` helper is what the forecast, alert and air sources already
      report, so a second convention here would only tell a JWT user about an API key.
- ✅ The row mapping: `name` from the row's **`name`** field (the probe's answer; `adm2` is the
      second-level division, e.g. `新乡` for `新乡县`, and the model has no field for it);
      `admin1` from `adm1`; `country` from `country`; `lat`/`lon` parsed from the strings with the
      same range check every other source uses; `tz` parsed through `chrono_tz` with an unknown zone
      **dropping the row** (the model has no provisional zone for a geocoded place, and inventing
      UTC would make the forecast render in the wrong day); `elevation_m`/`population` stay `None`.
      **Corrected**: `rank` is not even decoded. It is QWeather's importance hint (a capital is 10, a
      town 33+) and not a population, and the response already arrives in the service's relevance
      order — which is what the `Geocoder` contract asks a source to return; the shared ranking
      orders the merged candidates for the query afterwards, so re-sorting by `rank` here would be
      computation with no effect on any output. Never stored as a population, as the plan required.
- ✅ The `lang` rule: the script of the query decides it (`lang=zh` for a CJK query, `lang=en`
      otherwise), the same rule `src/geo/open_meteo.rs` already applies to GeoNames' per-language
      alternate names — so a Latin query does not come back with a Chinese `adm1` that the merge
      cannot fold against the other sources' rows. The probe confirmed the service matches either
      spelling under either setting but spells its answer in the requested one (`新乡县` →
      `Xinxiang County` under `lang=en`).
- ✅ Chain wiring: `GeoSource::QWeather` in `src/geo/chain.rs`, `"qweather"` appended to
      `SEARCH_SETTINGS` (and therefore `GEO_SEARCHES` and the config validator),
      `docs/configuration.md` and `docs/location.md` updated, and **`auto` left untouched**: the
      enum's `parse` arm for `auto` keeps its three keyless sources. `--help` names no geocoding
      source ids, so the generated help and man page are unchanged.
- ✅ Provenance and credit: a `LocationSource::QWeather` variant for a GeoAPI answer (the `json`
      renderer's `source` value — the model's `Copy` enum needs no `as_str` — and the
      `attribution_line` text naming QWeather with `https://www.qweather.com`, the provider
      registry's licence line reused as `Location data by QWeather — https://www.qweather.com/`).
      `named_by` needed no new arm: only the coordinate-naming path (step 25) sets it, and its
      sources are the bundled tables and Nominatim. The new `location.source` value extends the list
      in `docs/schema.md` and `docs/schema/json-v2.json` — additive, so `schema_version` stays.
- ✅ The storage rule, honoured in code and documented: **GeoAPI data must not be cached, extracted
      or bulk-stored in any form.** The terms are explicit that most of the underlying providers
      permit real-time use and forbid *any* storage, so this source writes **no cache entry at all**
      (not even a short-lived one), builds no index, table or cross-query store, and says so in its
      module docs, in `docs/providers.md` and in the `-v` narration. The shared cache is bypassed for
      this source alone — the geocoder type holds no `Cache`, only the run clock the JWT form of the
      credential needs — and a repeat query asks again. `--offline=geo` refuses the source outright
      rather than opening a socket, since it has nothing cached to serve.
- ✅ Tests: decode tests over recorded payloads (`tests/geo_qweather.rs`: the district answer, the
      Latin answer, the measured `400` refusal as a no-hit, the `401` as the invalid-credential
      error, a missing host, an offline refusal) with the hand-written edge cases (a missing or empty
      `location[]`, an unknown `tz`, an out-of-range coordinate, a blank name, a non-numeric
      latitude) inline in `src/geo/qweather.rs`; a chain test (`tests/geo_chain.rs`: explicit
      selection answers through the stub transport, `auto` never selects it even with a credential
      configured, a missing credential errors with the documented message, and a search leaves the
      cache root empty so a repeat query goes out again); a merge test
      (`tests/geo_merge.rs`) — **corrected**: a `country_code: None` candidate is *kept* (a missing
      code is not the invalid `-99` spelling) and **never merges** with a keyless row for the same
      place, because `merge` treats a missing code as "no country to match on". That cannot show a
      place twice in practice: `[geo] search` selects one source, so a GeoAPI row is alone on the
      merged list; and a `#[ignore]`d live probe (`tests/geo_qweather.rs::live_qweather_geoapi`).
- ✅ Fixtures under `tests/fixtures/qweather/geo/` (already annotated as recorded QWeather API data
      in `REUSE.toml`), with the request URLs recorded in the fixture README: `xinxiangxian-zh.json`,
      `xinxiang-zh.json`, `beijing-en.json` and `no-such-location.json`, plus the shared
      `error_401.json`.
- ✅ Docs: `docs/providers.md` gained a `### QWeather GeoAPI city/lookup` block under the location
      services (endpoint, parameters, row fields, the missing country code and its consequences, the
      caching ban, the cost of one request per name query) and a re-verification log entry; the
      README location section names the source; the `CHANGELOG.md` entry landed under `[Unreleased]`;
      and the plan index marks this step.

## Design notes

* **Measured facts (2026-10-13, the account host, before any code).**
  `GET {host}/geo/v2/city/lookup?location=…&number=…&lang=…` with `X-QW-Api-Key`:
  * the success envelope is `{"code":"200","location":[…],"refer":{"sources":["QWeather"],
    "license":["QWeather Developers License"]}}`; the documented "status" is the HTTP status, not a
    second field;
  * a row is `{name, id, lat, lon, adm2, adm1, country, tz, utcOffset, isDst, type, rank, fxLink}`
    with `lat`, `lon` **and `rank`** as strings. `新乡县` (10 rows) answered with the district first
    (`101180313`, 35.19084, 113.80521, `adm2` 新乡, `adm1` 河南省, `country` 中国, `tz`
    Asia/Shanghai, `rank` `"15"`); `新乡` (10 rows) answered with 新乡 first and 新乡县 second, so
    the response order is relevance and not `rank` (`获嘉` is `33`, `辉县` `35`, both below the two
    `15`s); `beijing-en` (5 rows) spelled `Beijing`, `Haidian`, `Chaoyang`, `Shunyi`, `Huairou` with
    `country` `China`. A Chinese query under `lang=en` spells its answer `Xinxiang County`, and a
    Latin query under `lang=zh` spells it `北京` — the setting changes the spelling, never the match;
  * **no ISO 3166-1 code appears anywhere**: `country` is a localized name, `refer` names the vendor
    and `fxLink` only repeats the LocationID. `Location.country_code` is therefore `None` for a row
    from this source — the fact the whole explicit-only rule rests on;
  * a query matching nothing (`Zzzqqqnotaplace`) answers **HTTP 400** with
    `{"error":{"status":400,"type":"https://dev.qweather.com/docs/resource/error-code/#no-such-location",
    "title":"No Such Location","detail":"Cannot find the location of the query, please try another
    location."}}` — **not** the `200` with an empty `location[]` this file assumed. A one-character
    `location` is `400 #invalid-parameter` and an absent one `400 #missing-parameter`; both are
    client bugs and stay upstream failures. A wrong or absent key is **HTTP 401**
    `…#unauthorized`, the same spelling the forecast and alert endpoints send.
* **Why a fourth source when three exist.** The bundled table stops at GeoNames' `cities15000`
  (towns above ~15 000 people), Open-Meteo redistributes the same rows, GeoNames' search is fuzzy
  but still GeoNames, and Nominatim is OSM's view. None of them can answer "the place QWeather calls
  新乡县" — the point the LocationID names and their forecast endpoints serve. The client uses that
  *point*, not the LocationID itself (a forecast here is requested by coordinates), which is why the
  alignment is exact rather than approximate. For a QWeather user that is the difference between the
  district and its parent city. The live comparison is in the progress log: `auto` reaches the same
  town only through Nominatim, and arrives without a zone (provisional UTC).
* **The missing ISO country code is the reason `auto` stays keyless.** The response has `country` as
  a localized name and no code. `Location.country_code` drives `auto` provider selection and the
  alert registry's coverage rule; a `None` there degrades both to bounding boxes. Keeping the
  source out of `auto` means the default path never pays that price, and a user who selects it
  explicitly gets the better Chinese name resolution knowingly.
* **Ordering inside the source, ranking across sources.** The source returns the rows in the order
  the service sent them — the relevance order, which is what the `Geocoder` contract asks for —
  and does not decode `rank` at all. The chain then merges (first-wins on a duplicate) and the
  shared step-04 keys (population, then name) order the merged list; with `population: None` a
  GeoAPI row sorts by name against Open-Meteo's population-bearing rows. Recorded so the behaviour
  is a decision rather than a surprise: the within-source order is transient — `location search`
  ranks for the query afterwards — which is a second reason `rank` needs no arm of its own.
* **Cost and quota.** One request per name query on the same free tier as the forecast (first
  50 000 requests/month at ¥0), with the requested `number` **clamped to the documented `1..=20`** so
  a `--limit 50` cannot turn into a `400 #invalid-parameter` — and **no cache entry**: the storage
  rule makes every query a fresh request, which is the price of using this source at all. A silenced
  geo scope (`--offline=geo`) refuses the source instead of letting it open a socket, because it has
  nothing cached to serve.
* **The credential error is the shared one.** `qweather` accepts an API key or a JWT (step 27), so a
  missing credential under an explicit selection is `Error::MissingCredential` (exit 6) from the
  same `provider::missing_credential` helper the forecast, alert and air sources use. It names both
  `key set` forms and `CIRROCAST_QWEATHER_KEY`, which is strictly more useful than the
  `Error::MissingKey` this file first specified.
* **No new dependency.** `chrono-tz` already parses the zone names; the request goes through the
  shared client; `deny.toml` is untouched.

## Out of scope

* POI lookup (`/geo/v2/poi/lookup`) and POI range search: the product resolves places, not shops.
* The `adm` and `range` filters: a plain name query carries neither a parent division nor a country
  hint, and inventing one from the query text is guesswork. (`range` is also how a caller would get
  a country code back into the row — worth revisiting only if the missing code becomes a real
  problem.)
* Reverse geocoding through QWeather: `[geo] reverse` keeps its bundled-tables-then-Nominatim rule;
  a coordinate query to `/geo/v2/city/lookup` is a separate decision with its own credit and terms
  questions.
* The top-city API and the LocationID-as-input path: this source resolves names, and a user who has
  a LocationID can already write the coordinates.

## Verification

```bash
# with the account configured (host + credential):
CIRROCAST_GEO_SEARCH=qweather cargo run -q -- location search 新乡县 -v
#   → the QWeather row, its adm1, the zone, the `not cached` note and the QWeather credit line
CIRROCAST_GEO_SEARCH=qweather cargo run -q -- location search 新乡县 --all   # the ranked table
CIRROCAST_GEO_SEARCH=qweather cargo run -q -- 新乡县 -f json | jq '.location.source'
#   → "qweather" as the location source, coordinates from the GeoAPI row (a weather query, not
#     `location search`, which has no `-f`; and via `CIRROCAST_GEO_SEARCH`, since `--geo-search`
#     is not a flag — `[geo] search` has no flag by design)
CIRROCAST_GEO_SEARCH=qweather cargo run -q -- location search 新乡县            # without a credential
#   → exit 6, naming `cirrocast key set qweather` (both forms) and `CIRROCAST_QWEATHER_KEY`
cargo run -q -- location search 新乡县 -v                          # `auto`: unchanged, no GeoAPI call
cargo test --workspace --locked && cargo clippy --workspace --all-targets --locked -- -D warnings
reuse lint
```

## Exit criteria

- ✅ A Chinese district-level query that the bundled table cannot answer resolves through GeoAPI to a
      point inside the district, with the zone and the administrative chain printed, verified live
      and pasted into the progress log.
- ✅ `auto` performs no GeoAPI request under any configuration, proven by a chain test and by a
      `--verbose` run that shows the three keyless sources only.
- ✅ A GeoAPI-resolved location renders like any other and its `source` field and credit line name
      QWeather, while **no GeoAPI row is ever written to disk**: the run's cache directory gains no
      entry for the query, proven by a test that inspects the cache after a stubbed GeoAPI answer.
- ✅ `docs/providers.md`, `docs/location.md`, `docs/configuration.md`, the README and the CHANGELOG
      describe the source, its explicit-only rule and its terms; `reuse lint` is clean.

## Risks

* **Terms.** The cache page of the best-practices set
  (`https://dev.qweather.com/docs/best-practices/cache/`) is blunt: *"你不能缓存、提取、批量存储 GeoAPI
  中提供的所有数据 … 大多数服务商（几乎是所有）许可你实时的使用，但禁止你将这些数据进行任何形式的存储"* —
  most of the underlying providers forbid storing the data **in any form**, not merely bulk-caching
  it. Hence the no-entry rule above; the module docs plus `docs/providers.md` state it, so a later
  "let us cache the lookup for a day" or "let us pre-seed the offline table from GeoAPI" idea is
  visibly out of bounds.
* **Duplicate-looking candidates.** If `lang` is chosen badly, a GeoAPI row ("北京") will not fold
  against a keyless row ("Beijing") and the picker shows the same city twice. The script rule above
  is the mitigation; the merge test pins it.
* **Credential-bound geocoding** breaks the keyless-first principle by construction. That is why the
  source is explicit-only and why its failure mode is a usage error, not a silent fallback to a
  different place.

## Progress log

- 2026-10-07 — step created from the QWeather free-tier audit (the user's question about the GeoAPI
  half of the 50 000-request plan). Endpoint, parameters, row fields and the missing ISO country
  code read from the published docs (2026-10-07); the live probe is the first deliverable so the
  mapping decisions are made against the account, not the documentation.
- 2026-10-07 — the plan was corrected after reading QWeather's best-practices set: its cache page
  forbids storing GeoAPI data **in any form** for most of the underlying providers, which is
  stronger than the "no bulk caching or indexing" wording this file carried. The source therefore
  writes no cache entry at all (the deliverable, the design note, the exit criterion and the risk
  bullet above all changed), and every query costs one request — recorded so the design is not
  softened again later for convenience.
- 2026-10-13 — step executed end to end against the account host, live, before any code was written.
  The probe's findings are in `## Design notes` above and corrected five plan statements:
  (a) a query matching nothing answers **HTTP 400 `#no-such-location`**, not a `200` with an empty
  `location[]` — the geocoder maps that refusal back to an empty hit list, so the chain still
  reports a no-hit (the deliverable's "empty `location[]`" edge case survives as a hand-written
  decode test, since the service never sends it);
  (b) the row's `name` is the *place*, not `adm2` (which is the second-level division, 新乡 for
  新乡县, and has no model field) — decided from the probe rather than guessed;
  (c) the missing credential is `Error::MissingCredential`, not `MissingKey`, because `qweather`
  has two authentication modes;
  (d) `rank` is not decoded at all: the `Geocoder` contract asks for the service's own order, which
  is the relevance order the response already carries (`新乡` answers 新乡县 second, before rows with
  a lower `rank` number), and the shared ranking re-orders the merged list afterwards, so sorting by
  `rank` would be computation with no effect on any output;
  (e) the merge test proves the *documented* behaviour, not the planned one: a `country_code: None`
  row is kept (unlike the invalid `-99` spelling) and **never merges** with a keyless row for the
  same place, because `merge` reads a missing code as "no country to match on".
  Two implementation notes the plan did not anticipate: the requested `number` is clamped to the
  documented `1..=20` (so `--limit 50` cannot become a `400 #invalid-parameter`), and a silenced geo
  scope refuses the source rather than opening a socket — it has nothing cached to serve, because it
  never caches. The plan's verification block named two things that do not exist (`--geo-search`, and
  `-f json` on `location search`) and was corrected to the environment override plus a weather query.
- 2026-10-13 — verification, all live against the account host with `XDG_CONFIG_HOME` pointing at a
  scratch config (the host only) and `CIRROCAST_QWEATHER_KEY` in the environment:
  `CIRROCAST_GEO_SEARCH=qweather location search 新乡县 -v` printed
  `新乡县, 河南省, 中国 (35.19, 113.81) Asia/Shanghai`,
  `location: qweather: 10 candidates (not cached: GeoAPI data may not be stored)` and
  `Location data by QWeather — https://www.qweather.com/`;
  the same command without the variable (`auto`) printed
  `Xinxiang County, Henan, China (35.19, 113.80) <timezone resolved at fetch time>` after asking
  open-meteo, geonames and nominatim, at 113.80 rather than 113.81 and without a zone — the
  district-vs-parent-city and zone-vs-placeholder difference the step exists for;
  `新乡县 -f json` (a weather query) answered `"source": "qweather"`,
  `"timezone": "Asia/Shanghai"`, `"country_code": null` and a report whose attribution carried
  `"location_notice": "Location data by QWeather — https://www.qweather.com/"`;
  `--offline=geo location search 新乡县 -v` printed
  `qweather: offline, not asked (a GeoAPI answer is never stored)` and the documented
  `no location found for 新乡县 (no offline match)` (exit 5);
  without the credential it exited 6 with
  `missing credential for qweather: run `cirrocast key set qweather` (API key) or
  `cirrocast key set qweather --jwt` (JWT), or set CIRROCAST_QWEATHER_KEY in the environment`;
  the fixture live probe (`cargo test --test geo_qweather -- --ignored`) passed against the account;
  and after two searches the scratch cache root **did not exist** (`find` reported no such
  directory), which is the no-entry rule observed rather than asserted.
  `cargo fmt --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`,
  `cargo test --workspace --locked` (543 lib + every integration suite) and `reuse lint`
  (`539 / 539 files`) are clean, and `tests/geo_qweather.rs`, `tests/geo_chain.rs` and
  `tests/geo_merge.rs` pass under `CIRROCAST_FORBID_NETWORK=1`. The generated `--help` and man page
  needed no regeneration: `--help` names the geocoding sources nowhere, only the environment
  variables, which did not move.
