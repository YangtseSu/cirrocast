<!--
SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
SPDX-License-Identifier: GPL-3.0-or-later
-->

# B04 — weather (life) indices (backlog)

Status: ⏸ backlog — deferred on 2026-10-07: no scheduled phase asks for it, the panel is text-heavy
and daily-cadence, and the cheapest half of it (the locally computable indices) is worth a step of
its own. Scheduling it means renumbering it into the A-series tail first (`docs/plans/README.md`).
Depends on: 03 (canonical model and units), 16 (the air panel, the "supplementary fetch" pattern and the credit mechanism), 17 (the locally computed astro block), 27 (the QWeather credential and host), 24 (the coverage gate a national keyless source needs)
Touches: `src/model/{indices,mod}.rs` (new), `src/indices/{mod,qweather}.rs` (new), `src/render/{indices,mod,art_table,plain,one_line,json}.rs`, `src/template.rs`, `src/cli.rs`, `src/config/mod.rs`, `src/cache.rs`, `src/i18n.rs`, `locales/{en-US,zh-CN}/main.ftl`, `tests/{indices,templates}.rs`, `tests/fixtures/qweather/`, `docs/providers.md`, `docs/schema.md`, `README.md`, `CHANGELOG.md`

## Goal

`cirrocast --indices Beijing` answers "what does today's weather mean for what I am about to do":
dressing, car wash, fishing, UV, sports, allergy, comfort, flu — as the levels their publishers
issue, not as numbers this client invented. The panel is daily, text-heavy and supplementary, built
exactly like the air panel: a best-effort fetch after the forecast, never a chain entry, never an
exit code, with the source's own credit line in the footer and in `json`.

Half of the feature needs no new upstream at all: the repo already fetches everything an apparent
temperature, a heat index, a humidex, a WBGT and a pollen band need. The step ships those first, then
the upstream indices behind the provider that is already wired.

## Source survey (2026-10-07, provider docs/OpenAPI and live JSON)

| Source | Index types | Coverage | Auth | Free tier | Licence | Repo status |
|---|---|---|---|---|---|---|
| **QWeather** `/v7/indices/{1d,3d}?type=&location=` | `type` 1 sports, 2 car wash, 3 dressing, 4 fishing, 5 UV; 6 travel, 7 allergy, 8 comfort, 9 flu, 10 air-pollution dispersion, 11 air-conditioner, 12 sunglasses, 13 make-up, 14 drying, 15 traffic, 16 SPF | 1–5 global; 6–16 China | BYOK key or Ed25519 JWT + the per-account host | 50 000 req/month at ¥0 | proprietary; name QWeather + link | the backend, credential, host, cache and error taxonomy exist — indices are a new call on the same host |
| **AccuWeather** `/indices/v1/daily/{1,5,10,15}day/{locationKey}` | ~50 IDs (running, fishing, arthritis, asthma, cold, flu, migraine, drying, car wash, dog walking, air quality, grass pollen, UV) | global | BYOK apikey | trial only for indices: **paid from the Standard tier** | proprietary; caching/redistribution restricted | new backend |
| **DWD open data** `/climate_environment/health/alerts/{biowetter,uvi,gt,s31fg}.json` | biowetter (wellbeing, heart/circulation, rheumatic, asthma, thermal load), UV by city, thermal danger, pollen danger (8 species, 3 days) | **Germany only** | keyless | free, no quota | CC BY 4.0 | new backend, cheap: four JSON files |
| **KMA life-weather indices** (data.go.kr 15085288/15085289) | UV, air stagnation, pollen risk, food poisoning, cold risk, asthma/COPD | **South Korea only** | service key | free after approval | Korean public-data terms | new backend |
| **Taiwan CWA** F-D0047-091/093 | comfort index (`CI`), apparent temperature, UV | **Taiwan only** | API key | free | Taiwan open-data terms | new backend |
| **JMA / MoE Japan** | WBGT heat-stress; 熱中症警戒アラート XML | **Japan only** | keyless | free | Japanese government terms | new backend |
| **ECCC Canada** `citypageweather-realtime` | humidex, wind chill, UV index | **Canada only** | keyless | free | Open Government Licence – Canada | new backend |
| **Google Pollen API** `pollen/v1/forecast:lookup` | tree/grass/weed pollen index | global | GCP key + billing | pay-as-you-go | Maps Platform terms; cache ≤ 30 days | new backend |
| OpenWeatherMap / Open-Meteo / Visual Crossing / WeatherAPI / WWO / WeatherKit / Tomorrow.io / Meteosource | **no life indices**: UV, feels-like, and (Visual Crossing) `heatindex`/`windchill`/`wbgt`, (WeatherAPI Pro+) pollen, (Tomorrow.io) UV concern | — | — | — | — | UV and feels-like are already modelled; Visual Crossing's extra fields are a payload extension |
| Meteomatics / meteoblue | pollen and air-quality packages | global | OAuth/key | trial | commercial | reject for now |

Sources: QWeather's docs and OpenAPI (`dev.qweather.com/docs/api/indices`, the `indices-type` page),
`developer.accuweather.com` (indices API and pricing), the DWD open-data directory
(`opendata.dwd.de/climate_environment/health/alerts/`), `data.go.kr` (KMA 15085288/15085289),
`opendata.cwa.gov.tw`, `jma.go.jp` / MoE, `api.weather.gc.ca`, `developers.google.com/maps/documentation/pollen`,
plus the vendors' own API docs for the "no indices" rows. Full list in the creation log entry.

## Deliverables

- ⬜ **Live probe first**: the QWeather `/v7/indices/1d?type=1,3&location=…` call on the account host (the repo speaks v1 there — verify the v7 product is served at all, what `daily[]` carries, and the level/category vocabulary), plus a DWD sample and the QWeather "coverage per type" claim. Correct this file where the service differs from its docs.
- ⬜ **The local set, no new request**: apparent temperature filled for the backends that leave `feels_like_c` empty (SMHI, met-no, Bright Sky, NWS, METAR), heat index / humidex / wind chill / WBGT from temperature, humidity, wind and (where fetched) shortwave radiation, and a pollen-risk band derived from the six species `Report::air` already carries. Each derived value is documented as *computed here*, not as a published index.
- ⬜ `src/model/indices.rs`: `LifeIndex { kind: IndexKind, level: Option<u8>, category: String, text: Option<String>, date, source }` with `IndexKind` a typed slug (never a bare localised string as the key), and `Report.indices: Option<Vec<LifeIndex>>` as a `serde(default)` field so older documents parse.
- ⬜ `src/indices/qweather.rs`: the v7 indices call on the configured host with the step-27 credential (key or JWT), `type` list from the run's requested kinds, cached under its own `indices/<source>-…` key (daily cadence — the weather TTL does not apply), errors mapped like every other QWeather call.
- ⬜ Surface: `--indices` flag, `--format indices` standalone view, an `art-table` panel beside the air one, `plain` lines, a `%i` token (`src/template.rs` row + `tests/templates.rs` row), and a typed `json` array with `docs/schema.md` + `docs/schema/json-v2.json` updated (additive, `schema_version` stays).
- ⬜ Credits: the per-source line the air panel already uses (QWeather name + link; government credit for a DWD/KMA/CWA/ECCC source), in the footer and in `json`.
- ⬜ Tests: decode tests per source over recorded fixtures, the "no indices for this point" case (a coverage miss is not an error), the panel at 59/60/80/120 columns, the token-count assertion, and an `#[ignore]`d live probe.
- ⬜ Docs: `docs/providers.md` gains the indices block (endpoint, types, coverage per type, cadence, credit), the README documents the flag and the format, `CHANGELOG.md` gains the entry.

## Design notes

* **Local first, upstream second.** The computed set costs no request, no credential and no licence, and it is the only version of this feature a default keyless install gets. Ship it as its own commit before the QWeather source.
* **QWeather is the upstream that is already paid for.** Its credential, host, error taxonomy and cache plumbing exist; types 1–5 are global, and 6–16 are the reason a Chinese reader would want the panel at all. The documented path is v7 — the generation QWeather is retiring (its city-based weather APIs carry EOL 2027); whether indices migrate to v1 is unverified, so the probe comes first and the module must not assume a v1 path exists.
* **A keyless national alternative exists but is narrow.** DWD's four JSON files give real biowetter/pollen/UV/thermal indices with no credential, behind the same country-coverage gate `brightsky` uses. It is a second provider, not a prerequisite.
* **Never invent a category.** Levels and category words come from the publisher; the client localises the *labels* it owns and prints the publisher's text as-is, exactly as the alert panel prints descriptions.
* **No pre-fetched index tables.** Visual Crossing forbids redistributing cached data and Google caps pollen caching at 30 days; the design is one cached response per query per source, never a bundled table.
* **Admission rule.** Documented public APIs only (step 23's rule): no reverse-engineered app feeds.

## Out of scope

* The long tail of national keyless index sources (DWD, KMA, CWA, ECCC, JMA) as a group: each is its own backend with its own coverage gate, and one of them belongs with the first upstream step only if the probe shows QWeather cannot serve the types that matter.
* Paid-only sources (AccuWeather indices, Google Pollen, Meteomatics, meteoblue, Tomorrow.io).
* Hourly indices, multi-day horizons beyond the publisher's own (QWeather's `3d`), and any client-side model of a "risk score" that no publisher issues.
* Pollen *alerts*: that is step 15's registry, not this panel.

## Verification

```bash
cargo run -q -- --indices Beijing                 # the panel, local set + the QWeather source
cargo run -q -- --indices Beijing -f json | jq '.indices[] | {kind, level, category, source}'
cargo run -q -- --indices Berlin -p open-meteo    # the local set only, "upstream indices unavailable"
cargo run -q -- -f indices Beijing                # the standalone view
cargo run -q -- --indices Beijing --indices-from qweather   # explicit source, missing key → exit 6
cargo test --workspace --locked && cargo clippy --workspace --all-targets --locked -- -D warnings
reuse lint
```

## Exit criteria

- ⬜ A keyless install prints the computed indices (apparent temperature, heat index/humidex/WBGT, pollen band) for any point, with no new request and no new licence.
- ⬜ With a QWeather credential, the panel adds the publisher's indices for the requested types, in the reader's language, with the source's credit line — verified live and pasted into the progress log.
- ⬜ A point the selected source does not cover renders "not available" (exit 0), never an invented category.
- ⬜ `docs/providers.md`, `docs/schema.md`, the README and the CHANGELOG describe the panel, the sources and their credits; `reuse lint` is clean.

## Risks

* **Text-heavy output.** A sixteen-type panel does not fit a terminal; the flag must take a type list (`--indices dressing,uv`) and default to a short curated set.
* **A retiring v7 product.** If QWeather moves or drops `/v7/indices`, the source is dead on arrival; the probe and the "unavailable, exit 0" path are the mitigation, and the local set keeps the feature alive.
* **Computed vs published confusion.** The two must be visibly distinct in the output (a "computed" marker or a separate section), or a reader will treat a client-side humidex as an official warning level.

## Progress log

- 2026-10-07 — backlog item created at the user's request, together with a source survey. The survey was run against the providers' official documentation and live responses on 2026-10-07: QWeather's indices docs and `indices-type` page (types 1–16 and their per-type coverage), AccuWeather's indices API and pricing, the DWD open-data health alerts directory, KMA's data.go.kr datasets, Taiwan CWA, JMA/MoE, ECCC, Google's Pollen API, and the "no indices" rows for OpenWeatherMap, Open-Meteo, Visual Crossing, WeatherAPI, WWO, WeatherKit, Tomorrow.io and Meteosource. No code exists for this item.
