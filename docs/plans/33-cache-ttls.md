<!--
SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Step 33 — cache TTLs by data kind

Status: ✅ done
Depends on: `05-http-cache-and-ip-location.md` (the cache and its TTL plumbing), `10-additional-providers.md` (the two backends whose fetch answers from more than one request), `23-more-providers.md` (the supplementary-fetch pattern the split entries follow)
Touches: `src/config/mod.rs`, `src/provider/{mod,qweather,openweathermap}.rs`, `tests/{config,provider_qweather,provider_owm}.rs`, `tests/fixtures/config/expected-after-set.toml`, `docs/{configuration,providers}.md`, `CHANGELOG.md`, `docs/plans/README.md`

## Goal

The cache stops treating every weather answer the same: a "now" answer (the current block) keeps
the 10-minute `cache.weather_ttl_secs`, while a forecast series that has its own cache entry — the
hourly or daily part of a fetch answered by more than one request — gets the new
`cache.forecast_ttl_secs` (30 minutes). A repeated run therefore reuses the series instead of paying
for it again, which is what QWeather's best-practices page asks for ("缓存应该是弹性的" — don't use
one policy for everything) and what its recommended table spells out: 10–30 minutes for real-time
data against 30–60 for the hourly series.

## Deliverables

- ✅ `cache.forecast_ttl_secs` (u32, default 1800) in `CacheConfig`, validated `> 0`, registered in
  `KEY_TABLE` with its `config get`/`set` arms and the `[cache]` block of the generated document, and
  documented in `docs/configuration.md` with the reasoning (the provider's own cadence guidance).
- ✅ `provider::weather_ttl(env)` and `provider::forecast_ttl(env)` beside `local_today`, so the two
  kinds are named where they are chosen rather than spelled as raw seconds at each call site.
- ✅ The two backends whose fetch answers from more than one request use the split:
  `src/provider/qweather.rs` (current → `weather_ttl`, hourly → `forecast_ttl`) and
  `src/provider/openweathermap.rs` (current → `weather_ttl`, forecast → `forecast_ttl`). The
  single-request backends are untouched: their one body is the whole answer, and its TTL stays
  `weather_ttl_secs`.
- ✅ Tests: `tests/provider_qweather.rs` and `tests/provider_owm.rs` each advance the injected clock
  by 20 minutes and prove the second fetch costs **one** request instead of two (the series is served
  from the cache, the current block is not), beside the existing test that proves a repeat within the
  short TTL costs none; `tests/config.rs` probes the new key and the canonical-document fixture
  carries it.
- ✅ Docs: `docs/configuration.md`'s `[cache]` table, `docs/providers.md`'s best-practices audit
  (now saying both TTLs sit inside the published recommendations), and the CHANGELOG entry.

## Design notes

* **Why a second key and not a bigger `weather_ttl_secs`.** Raising the one TTL to 30 minutes would
  make every run serve a stale *current* block — the thing a reader re-runs the program to see. The
  two kinds have different cadences, so they get different numbers, which is exactly the elasticity
  the page asks for.
* **Why a configuration key and not a constant.** The other TTLs (`weather`, `ip`, `geocode`, and
  `alerts.cache_ttl_secs`) are all user-tunable, and a user with a metered plan may want a longer
  series TTL than 30 minutes; the key costs one row in a table that already exists.
* **Why only the split-fetch backends.** A backend that answers from one request has one body and one
  entry; splitting its TTL would mean splitting its cache entry, which is a different design with no
  evidence behind it. The helper names make the distinction available the day a third backend splits.
* **No new dependency, no request change.** The change is which number a cache entry is written with;
  the wire traffic is identical for a cold run and lower for a warm one.

## Out of scope

* Per-provider TTL overrides and a TTL table keyed by product (the page's other rows — indices,
  minutely, air quality — belong to the steps that add those features).
* Backoff jitter (the page's anti-collision advice for fleets of devices) and any change to
  `alerts.cache_ttl_secs` (5 minutes, inside the page's 5–20 minute recommendation) or to the IP and
  geocode TTLs, whose ceilings come from other services' terms.
* Caching anything the GeoAPI terms forbid: step 30's source writes no entry at all.

## Verification

```bash
cargo run -q -- -vv Beijing                  # the second run inside 10 minutes: both parts cached
sleep 700 && cargo run -q -- -vv Beijing     # > 10 minutes: only `current` is fetched again
cargo run -q -- config get cache.forecast_ttl_secs     # → 1800
cargo run -q -- config set cache.forecast_ttl_secs 3600 && cargo run -q -- config get cache.forecast_ttl_secs
cargo test --workspace --locked && cargo clippy --workspace --all-targets --locked -- -D warnings
reuse lint
```

## Exit criteria

- ✅ A repeated run after the short TTL but inside the series TTL fetches only the "now" part, proven
  by the two provider tests with an advanced clock.
- ✅ `cache.forecast_ttl_secs` round-trips through `config get`/`set`, is validated `> 0`, and appears
  in the generated configuration document.
- ✅ `docs/configuration.md` and the providers audit explain the two TTLs and their source (the
  provider's published cadence), and `reuse lint` is clean.

## Risks

* **A stale series.** 30 minutes is inside the published 30–60 minute recommendation for the hourly
  series, and the day-keyed cache entry still rolls over at local midnight; a user who wants fresher
  series data can lower the key (or set it to `weather_ttl_secs`'s value, restoring the old
  behaviour).
* **Two numbers to explain.** The configuration table and the audit paragraph carry the reasoning, so
  the next person does not have to guess which one applies where.

## Progress log

- 2026-10-07 — step created and finished from the best-practices audit (the user's question about
  `https://dev.qweather.com/docs/best-practices/`): the cache page's elasticity section and its
  per-datatype table showed the single 10-minute TTL was more conservative than the provider's own
  guidance for series data. The key, the two helpers, the two backends, the tests, the docs and the
  generated configuration document landed together; `--help` is unaffected (no new flag), so the
  help/man snapshots and the line budget did not move.
