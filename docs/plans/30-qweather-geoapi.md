<!--
SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Step 30 — QWeather GeoAPI as a location source

Status: ⬜ not-started
Depends on: `04-geocoding-and-location-syntax.md` (the `Geocoder` trait, ranking and attribution lines), `05-http-cache-and-ip-location.md` (the shared client and cache), `18-offline-city-database.md` (the bundled table this source supplements), `20-location-candidate-selection.md` (the picker and the merged candidate list), `25-location-sources-2.md` (the multi-source chain, the merge and the GeoNames BYOK precedent), `27-qweather-jwt-auth.md` (the credential and host)
Touches: `src/geo/{mod,qweather,chain,merge,rank}.rs`, `src/model/mod.rs`, `src/config/mod.rs`, `src/cli.rs`, `tests/{geo_qweather,geo_chain}.rs`, `tests/fixtures/qweather/`, `docs/providers.md`, `docs/location.md`, `docs/configuration.md`, `README.md`, `docs/plans/README.md`, `CHANGELOG.md`

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

- ⬜ **Live probe, recorded in the design notes below** (the account host + credential, before any
      code): `GET {host}/geo/v2/city/lookup?location=<name>&number=<n>&lang=<lang>`; confirm the
      response envelope (`code` + `location[]`), the row fields
      (`id`, `lat`/`lon` as **strings**, `adm2`, `adm1`, `country` as a localized name, `tz`,
      `utcOffset`, `type`, `rank`), whether any ISO country code appears anywhere in the payload,
      what `number` returns for a `新乡`-style query (district rows? duplicated names?), and how the
      service spells a Latin query's names (`lang=en` vs `lang=zh`). Correct this file where the
      service differs from its docs.
- ⬜ `src/geo/qweather.rs`: a `Geocoder` over the shared `HttpClient` **without a cache entry** (the
      storage rule below forbids keeping GeoAPI rows on disk; `-v` says the request is uncached), the
      credential and host resolved exactly
      as `src/alerts/qweather.rs` resolves them (API key or JWT through `QWeatherAuth`, a missing
      host is the documented configuration error naming the console, a missing credential under an
      explicit selection is `Error::MissingKey` naming `cirrocast key set qweather` and
      `CIRROCAST_QWEATHER_KEY` — the GeoNames precedent).
- ⬜ The row mapping: `name` from `adm2`/the query? — decide from the probe and record it; `admin1`
      from `adm1`; `country` from `country`; `lat`/`lon` parsed from the strings with the same
      range check every other source uses; `tz` parsed through `chrono_tz` with an unknown zone
      **dropping the row** (the model has no provisional zone for a geocoded place, and inventing
      UTC would make the forecast render in the wrong day); `elevation_m`/`population` stay `None`
      (`rank` is QWeather's own ordering, not a population — it orders the rows *within this
      source* and is never stored as one).
- ⬜ The `lang` rule: the script of the query decides it (`lang=zh` for a CJK query, `lang=en`
      otherwise), the same rule `src/geo/open_meteo.rs` already applies to GeoNames' per-language
      alternate names — so a Latin query does not come back with a Chinese `adm1` that the merge
      cannot fold against the other sources' rows.
- ⬜ Chain wiring: `GeoSource::QWeather` in `src/geo/chain.rs`, `"qweather"` appended to
      `SEARCH_SETTINGS` (and therefore `GEO_SEARCHES`, the config validator and the `--help`
      epilogue), `docs/configuration.md` and `docs/location.md` updated, and **`auto` left
      untouched**: the enum's `parse` arm for `auto` keeps its three keyless sources.
- ⬜ Provenance and credit: a `LocationSource` variant for a GeoAPI answer (its own `as_str`, the
      `json` renderer's `source` value and the `attribution_line` text naming QWeather with
      `https://www.qweather.com`, the provider registry's licence line reused), plus `named_by` so
      a named coordinate keeps the credit. The new `location.source` value extends the list in
      `docs/schema.md` and `docs/schema/json-v2.json` — additive, so `schema_version` stays.
- ⬜ The storage rule, honoured in code and documented: **GeoAPI data must not be cached, extracted
      or bulk-stored in any form.** The terms are explicit that most of the underlying providers
      permit real-time use and forbid *any* storage, so this source writes **no cache entry at all**
      (not even a short-lived one), builds no index, table or cross-query store, and says so in its
      module docs, in `docs/providers.md` and in the `-v` narration. The shared cache is bypassed for
      this source alone; a repeat query asks again.
- ⬜ Tests: decode tests over recorded payloads (a multi-row answer, a row with an unknown `tz`, a
      row with an out-of-range coordinate, an empty `location[]`), a chain test (explicit selection
      answers through the stub transport, `auto` never selects it, a missing credential errors with
      the documented message), a merge test proving a `country_code: None` candidate is kept and
      deduped against a keyless source's row for the same place, and a `#[ignore]`d live probe.
- ⬜ Fixtures under `tests/fixtures/qweather/` (already annotated as recorded QWeather API data in
      `REUSE.toml`), with the request URLs recorded in the fixture README.
- ⬜ Docs: `docs/providers.md` gains a `### qweather GeoAPI` block under the location services
      (endpoint, parameters, the missing country code and its consequences, the caching ban, the
      cost of one request per name query), the README location section names the source, the
      `CHANGELOG.md` entry, and the plan index marks this step.

## Design notes

* **Why a fourth source when three exist.** The bundled table stops at GeoNames' `cities15000`
  (towns above ~15 000 people), Open-Meteo redistributes the same rows, GeoNames' search is fuzzy
  but still GeoNames, and Nominatim is OSM's view. None of them can answer "the place QWeather calls
  新乡县" — the LocationID whose point their forecast endpoints serve. For a QWeather user that
  alignment is the difference between the district and its parent city.
* **The missing ISO country code is the reason `auto` stays keyless.** The response has `country` as
  a localized name and no code. `Location.country_code` drives `auto` provider selection and the
  alert registry's coverage rule; a `None` there degrades both to bounding boxes. Keeping the
  source out of `auto` means the default path never pays that price, and a user who selects it
  explicitly gets the better Chinese name resolution knowingly.
* **Ordering inside the source, ranking across sources.** The merge's first-wins rule keeps this
  source's own order (QWeather's `rank`), and the shared step-04 keys (population, then name) order
  the merged list; with `population: None` a GeoAPI row sorts by name against Open-Meteo's
  population-bearing rows. Recorded so the behaviour is a decision rather than a surprise.
* **Cost and quota.** One request per name query on the same free tier as the forecast (first
  50 000 requests/month at ¥0) and **no cache entry**: the storage rule below makes every query a
  fresh request, which is the price of using this source at all.
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
#   → the QWeather row, its adm1/adm2, the zone, and the QWeather credit line
CIRROCAST_GEO_SEARCH=qweather cargo run -q -- location search 新乡县 --all   # the ranked table
CIRROCAST_GEO_SEARCH=qweather cargo run -q -- location search Beijing -f json | jq '.location.source'
#   → "qweather" as the location source, coordinates from the GeoAPI row
cargo run -q -- location search Beijing --geo-search qweather        # without a credential: exit 6,
#   naming `cirrocast key set qweather` and `CIRROCAST_QWEATHER_KEY`
cargo run -q -- location search Beijing -v                          # `auto`: unchanged, no GeoAPI call
cargo test --workspace --locked && cargo clippy --workspace --all-targets --locked -- -D warnings
reuse lint
```

## Exit criteria

- ⬜ A Chinese district-level query that the bundled table cannot answer resolves through GeoAPI to a
      point inside the district, with the zone and the administrative chain printed, verified live
      and pasted into the progress log.
- ⬜ `auto` performs no GeoAPI request under any configuration, proven by a chain test and by a
      `--verbose` run that shows the three keyless sources only.
- ⬜ A GeoAPI-resolved location renders like any other and its `source` field and credit line name
      QWeather, while **no GeoAPI row is ever written to disk**: the run's cache directory gains no
      entry for the query, proven by a test that inspects the cache after a stubbed GeoAPI answer.
- ⬜ `docs/providers.md`, `docs/location.md`, `docs/configuration.md`, the README and the CHANGELOG
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
