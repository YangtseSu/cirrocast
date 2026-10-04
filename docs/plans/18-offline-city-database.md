<!--
SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Step 18 — offline city database

Status: ✅ done
Depends on: 04, 05
Touches: `src/geo/{offline,rank,fold,mod}.rs`, `src/geo/data/`, `build/geo-table/` (workspace
member), `src/cli.rs`, `src/cache.rs`, `src/config/mod.rs`, `src/model/mod.rs`,
`src/render/json.rs`, `Cargo.toml`, `REUSE.toml`, `AGENTS.md`, `.github/workflows/ci.yml`,
`README.md`, `docs/schema.md`, `docs/plans/{README.md,21-perf-and-resource-budget.md}`,
`CHANGELOG.md`, `tests/{offline_geo,offline_lazy,cache,cli,cli_flags,cli_offline,config,air}.rs`,
`tests/{metar,geo_open_meteo,provider_http,provider_open_meteo}.rs`,
`tests/fixtures/config/expected-after-set.toml`

## Goal

City names resolve to coordinates, time zones and country codes with **no network at all**, from a
gzip-compressed GeoNames table embedded in the binary. `cirrocast location search --offline Beijing`
prints ranked hits in well under a second on a cold start, `--offline=geo` uses the local table while
still fetching live weather, and bare `--offline` uses neither the network nor the live geocoder. The
offline path produces the same ranking as the network path for the same query.

## Deliverables

- ✅ **Candidate evaluation, written into this file** (done below in the design notes): the two
  crate names from the brief are corrected, `geocoding` is rejected, GeoNames `cities15000` is
  vendored. Corrections are repeated in the Progress log.
- ✅ `build/geo-table/` (dev-only workspace member, `publish = false`): reads a GeoNames
  `cities15000.txt`, writes `src/geo/data/cities.bin.gz` (columns: geonameid, name, ascii-name,
  ISO country, lat, lon, population, IANA tz — the geonameid joined the row because the fixture
  equivalence test matches rows by it) and `src/geo/data/keys.bin.gz` (sorted folded-key → id
  list), plus `src/geo/data/SNAPSHOT` (GeoNames dump date + SHA-256 of the input file, derived from
  the input alone so a rebuild is byte-identical too); `flate2` `Compression::best()` with a fixed
  gzip timestamp, deterministic byte-for-byte output, no C toolchain. Re-running the builder
  against the same input reproduced both members byte for byte.
- ✅ `src/geo/offline.rs` behind the `offline-geo` feature (default-on, decision below):
  `include_bytes!` for both members, `LazyLock` decode on first use only (an `AtomicBool` marks the
  index as loaded, since `LazyLock` cannot be asked), no writes, and the decode returning typed
  errors so a corrupt blob is `Error::Other` with a `cargo run -p geo-table` hint rather than a
  panic.
- ✅ `src/geo/offline.rs` API: `search(query, mode: MatchMode::{Prefix, Exact}, limit) ->
      Result<Vec<City>>` (the `Result` was added so a corrupt member is a typed error, not a panic)
      and `resolve(query) -> Result<Location>`; folding =
      NFKD → strip combining marks → lowercase → drop non-alphanumerics, applied to both the index
      keys and the query, so `São Paulo`/`Sao Paulo`, `北京`/`Beijing`/`Peking`, `Wien`/`Vienna` and
      `MÜNCHEN`/`munchen` all hit. The lookup splits rows whose key *is* the query from rows whose
      key merely starts with it, so the exonym `Wien` ranks Vienna (an exact alternate key) above
      Wiener Neustadt (a display-name prefix).
- ✅ `src/geo/rank.rs`: the ranking already specified in step 04 (`exact-name → prefix →
  population → provider order`) extracted into a shared `Candidate` trait + `rank()` that both the
  network geocoder results (`Location`) and the offline `City` rows feed; the name match is now
  *folded* on both sides, so `:Sao Paulo` matches `São Paulo` and the two sources cannot disagree
  about what a name is. `src/geo/fold.rs` holds the one folding function, shared with the builder.
- ✅ `--offline[=<weather|geo|all>]` (bare = `all`; `require_equals`, so `--offline geo` is a
      location named `geo`, exactly as `--color always` is GNU-style) and `[network] offline =
      "off"` in config; the policy silences one *scope* each and the scope's cache is pinned to
      `CacheMode::Offline`, so no call site had to learn about it:
      * `--offline=weather` — weather answers come from the cache only; location resolution may use
        the bundled table and the network geocoder (this is the "cache-only weather" mode);
      * `--offline=geo` — location resolution stops at the bundled table; a *cached* geocoder answer
        is still served (step 04 promised that, and it opens no socket) and a cold miss is the same
        not-found as a table miss; weather is fetched live (the "local geocoding with live weather"
        mode);
      * `--offline` / `--offline=all` — no socket at all: bundled geocoding, cache-only weather, no
        IP lookup. The empty-cache failure is exit 3 and reads `offline: no cached open-meteo
        forecast for <place> at <key path>`, followed by the rerun hint — the shipped wording keeps
        the provider, the key path and the fix next to the plan's "no cached forecast for
        <place>".
- ✅ `[geo] strategy` (`auto` — the default, bundled table first for non-`~` queries with the
      network geocoder only on a miss; `bundled` — the table only; `network` — the geocoder only,
      the pre-step-18 behaviour). The chosen source is echoed under `--verbose`
      (`location: <query> resolved from the bundled city database` / `… asking the geocoder`).
- ✅ `cirrocast location search <query> [--offline] [--all] [--limit N] [--exact]`: the winner line
      by default (the shared `location_line`, so the two sources cannot diverge) and the ranked
      candidate table under `--all` — ` 1. <name>, <admin1>, <country> (<lat>, <lon>) <tz>
      (population N)`, i.e. the winner line plus a rank number and the population; `--offline`
      forces the bundle and never opens a socket; `--exact` is the `:query` narrowing for the flag
      spelling. A missing name exits 5 and reports `location not found: no location found for
      <query> (no offline match)` — the first part is the existing `LocationNotFound` wording.
- ✅ Licensing/credits: `REUSE.toml` annotation for `src/geo/data/*.bin.gz` and
      `src/geo/data/SNAPSHOT` with `GeoNames (https://www.geonames.org/)` and the licence expression
      CC-BY-4.0 (the data is CC-BY-4.0 and is *not* relicensed to GPL); the runtime prints
      `Location data by GeoNames (CC BY 4.0) — https://www.geonames.org/` for an offline-resolved
      place; the README's data-sources table names the bundled snapshot (dump date in `SNAPSHOT`);
      the builder source stays GPL-3.0-or-later; `reuse lint` reports 350/350 files.
- ✅ Size accounting task: `ls -l src/geo/data/*.bin.gz` and `cargo build --release` before/after
      this step, with both numbers recorded in `docs/plans/21-perf-and-resource-budget.md` (phase E
      owns the budget; the measured total is its input, not a surprise). The committed snapshot is
      878 055 B (`cities.bin.gz`) + 2 558 155 B (`keys.bin.gz`) = 3 436 210 B ≈ 3.28 MiB, larger than
      the 2.55 MiB projection because the rows carry the display *and* ascii name plus the
      geonameid, and the dump is the 2026-10-04 one (34 152 rows, 310 502 keys; refreshed from the
      official download, see the Progress log).
- ✅ Tests (`tests/offline_geo.rs`, plus `tests/offline_lazy.rs` for the startup assertion):
      Springfield (`--all` prints 10 rows — the 8 exact-name rows in population order, then the
      exact-key/prefix rows — ordering pinned), São Paulo with and without the diacritic,
      北京/Beijing/Peking, Wien/Vienna (the exonym wins), `MÜNCHEN` uppercase input, CJK queries,
      a missing city (exit 5, `(no offline match)`), `--exact` vs prefix, `[geo] strategy` network /
      bundled, the three offline modes' observable differences, the ranking-equivalence test
      against the recorded step-04 geocoder fixtures, and the startup assertion. No test opens a
      socket (`CIRROCAST_FORBID_NETWORK=1` in every sandbox).

## Design notes

* **Candidate measurement, 2026-09-30** (`curl -s https://crates.io/api/v1/crates/<name>`):
  * `world-cities` — **not on crates.io**: HTTP 404 and no search hit for the exact name. The brief's
    candidate does not exist; the nearest real crates are `cities` 0.2.0 (MIT, ~10 000 rows, no
    country/alt-names) and `cities-json` 0.6.8 (Unlicense, JSON dump of the same data).
  * `city-timezones` — **not on crates.io**: HTTP 404, no search hit. (A same-named JavaScript and
    Python package exists; there is no Rust crate under that name.)
  * `geocoding` 0.4.0 (MIT OR Apache-2.0, `github.com/georust/geocoding`) — **has no bundled
    dataset**: its modules are the OpenCage, Nominatim and GeoAdmin *network* providers, with a
    `Forward`/`Reverse` trait pair. Useless for an offline path; rejected. `geonames-lib` 0.3.0,
    `geo_rust` and `genom` 2.0.0 either parse the raw dump without a search index or need a database
    file; rejected in favour of ~300 lines of our own indexing over a dataset we control.
* **Chosen: vendor GeoNames `cities15000`.** Licence CC-BY-4.0 (attribution only, GPL-compatible for
  a derived work as long as credits stay in `REUSE.toml`/README), alternatenames column gives the
  transliterations the offline path needs, `population` gives the ranking input, column 18 gives the
  IANA zone.
* **Measured sizes** (`curl -sI https://download.geonames.org/export/dump/cities15000.zip`, then
  `unzip`, then `zstd -19`/`gzip -9` on the projections; 2026-09-30):
  * `cities15000.zip` 3 359 449 B; `cities15000.txt` 8 533 844 B, 34 148 rows.
  * reduced table with alternatenames 6 680 960 B → gzip -9 **2 825 464 B** → zstd -19 2 378 311 B.
  * reduced table without alternatenames 1 782 943 B → gzip -9 **576 185 B**.
  * folded key index (310 447 keys → row ids) 6 056 345 B → gzip -9 **2 094 759 B** → zstd 1 808 159 B.
  * `gzip -dc keys.bin.gz > /dev/null` ≈ 29 ms; `gzip -dc cities.bin.gz > /dev/null` ≈ 57 ms
    (CLI, process start included) — decoded once, lazily, per process.
* **Layout decision**: two gzip members (`cities.bin.gz` + `keys.bin.gz`) = **2 670 944 B ≈ 2.55 MiB**
  embedded, decoding only the index on a search and the city table only for matched rows. The
  single-blob alternative (2 825 464 B) is larger *and* forces a 6.7 MB parse before the first hit.
* **Compression dependency**: **flate2 1.1.10** (MIT OR Apache-2.0, MSRV 1.67, already in the tree for
  HTTP gzip from step 05), pure-Rust `miniz_oxide` backend, no C toolchain in CI, same crate in the
  builder and the decoder. Rejected: `ruzstd` 0.9.0 (MIT, but **MSRV 1.87** > the project's 1.98) and
  `zstd`/`zstd-safe` 0.14.0/8.0.0 (BSD-3-Clause, builds the vendored C library, adding a C compiler
  and a second licence to the release pipeline for ~290 KB).
* **Default-on justification**: the table costs **2.55 MiB compressed / ≈2.7 MiB in the stripped
  binary** and removes the network from the most common failure mode (a city name on a plane, in a
  locked-down network, or when the geocoder is rate-limited). Step 21 owns the release budget; this
  step states the cost and keeps the feature switchable (`--no-default-features` without
  `offline-geo` must still build and pass tests, proving the fallback path is real). If step 21's
  budget cannot absorb 2.7 MiB, flipping the default is a one-line change recorded there.
* **Snapshot cadence**: GeoNames regenerates dumps daily; we pin one dated snapshot, record its date
  and SHA-256 in `src/geo/data/SNAPSHOT`, and refresh at release time only (step 28 documents the
  command; `cargo run -p geo-table -- <path-or-url> [--check]` is the command — fetch/read → build
  (or compare), with the pinned expectations left for the operator to move deliberately). Nothing in
  the runtime ever downloads the dataset.
* **Ranking identity**: the offline index stores exactly the fields the step-04 ranking consumes, and
  `geo/rank.rs` is the single implementation, so the offline and online paths cannot drift; the test
  compares the ordering of the recorded step-04 fixtures with the offline ordering for the same
  queries.

## Out of scope

Reverse geocoding (`@lat,lon` → place name) is **step 25's** deliverable; it reuses this step's
decoded table for a nearest-city scan inside 25 km and adds the Natural Earth country layer, so this
step only has to expose the decoded rows (an iterator plus the population/coordinate fields the scan
needs). Also out of scope: admin-1/admin-2 hierarchies, postal codes, time-zone lookup for
coordinates (stays with the provider response / step 03), and incremental dataset updates over the
network. A **user-installed table** was out of scope here and became **step 18b**
(`docs/plans/18b-user-city-data-update.md`): it keeps the bundled table as the default and the
fallback, installs a dump-derived table under `$XDG_DATA_HOME/cirrocast/geo/` with an explicit
command, and rejects fetching during a query. `location search` does not gain paging or fuzzy
(edit-distance) matching; prefix + exact on folded keys is the contract.

## Verification

```bash
cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test && reuse lint

cargo run -q -- location search --offline --all Springfield
#   10 rows: the 8 exact-name rows by population, then Springfield Gardens (an exact alternate key
#   but no exact display name) and Springfield Lakes (a prefix); identical to `location search --all
#   Springfield` under the default `geo.strategy = "auto"`
cargo run -q -- location search --offline --all sao paulo     # São Paulo, BR — folding works
cargo run -q -- location search --offline --all 北京          # Beijing (geonameid 1816670)
cargo run -q -- location search --offline --all Wien          # Vienna first (exonym), then Wiener
                                                              # Neustadt and Vientiane
cargo run -q -- location search --offline Springfield         # one winner line, exit 0
cargo run -q -- location search --offline Nowhereville; echo $?    # error, exit 5
cargo run -q -- --offline=geo -f one-line Sao Paulo     # local geocoding + live weather
cargo run -q -- --offline -f plain Beijing              # cached only; without cache: exit 3
strace -f -e trace=network cargo run -q -- --offline=all -f plain 北京 2>&1 | grep -c connect
#   0 — no socket is opened in the total no-network mode
```

All of the above were run in a throwaway `XDG_*` sandbox; the socket count came out `0` (and the
suite itself never opens one: every sandbox sets `CIRROCAST_FORBID_NETWORK=1`).

Startup assertion (in `tests/offline_geo.rs`, `assert_cmd`): the median of five
`cirrocast --version` runs is **< 50 ms** and no decode occurs — the test additionally asserts
`offline::index_loaded()` is `false` after `--version`-equivalent library calls, proving the table is
not materialised eagerly.

## Exit criteria

- ✅ `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test`,
      `reuse lint` clean; `cargo build --no-default-features` and `cargo test --no-default-features`
      pass (the offline tests are `#[cfg(feature = "offline-geo")]`, the rest of the suite runs).
- ✅ `location search --offline` and the network path return the same ranked order for the four
  fixture queries (Springfield, São Paulo, Beijing, Vienna): one shared ranking function, and the
  recorded step-04 fixtures pick the same winner offline (asserted in `tests/offline_geo.rs`).
- ✅ `--offline=weather|geo|all` behave exactly as the three documented modes; `--offline=all` opens
  no socket (every test runs with `CIRROCAST_FORBID_NETWORK=1`, and `-v` names the mode).
- ✅ `--version`/`--help` never decode the table (asserted through `offline::index_loaded()` and a
  median of five real `--version` runs under 50 ms in `tests/offline_lazy.rs`).
- ✅ `REUSE.toml` credits GeoNames with CC-BY-4.0, the README documents the snapshot and the
  builder, and `reuse lint` is green.

## Risks

* GeoNames alternatenames do not cover every exonym (e.g. some transliterations are missing for
  small towns); mitigated by prefix matching on ascii/alternate keys, and by `--offline=auto`'s
  network fallback in the default strategy.
* The dataset's population column is coarse for non-cities and can order two same-named places
  counter-intuitively; the ranking test pins the current order so a later change is deliberate.
* A committed binary blob is opaque in review and 2.55 MiB is a real binary-size commitment;
  mitigated by a deterministic builder, the `SNAPSHOT` checksum and a byte-comparison test, and, if
  step 21 rejects the size, by the documented flip to opt-in (README usage + CHANGELOG updated).

## Progress log

- 2026-09-30 — step opened; crate candidates measured (`world-cities`/`city-timezones` absent from
  crates.io, `geocoding` has no bundled dataset), GeoNames `cities15000` sizes and gzip/zstd
  trade-offs measured, flate2 chosen over ruzstd (MSRV 1.87) and the zstd C bindings.
- 2026-10-03 — plan amended for the location work in steps 20 and 25: the `location search` shape is the
  winner line by default plus `--all` for the ranked table (instead of a table by default), the
  decoded table must expose the rows the step-25 nearest-city scan needs, and reverse geocoding
  moves from "out of scope" to step 25, which reuses this step's index.
- 2026-10-04 — plan reorganized: the wttr-compat service and the packaging matrix moved to the backlog
  (B01/B02) and the remaining work was renumbered so the number is the execution order; this step
  keeps number 18, and the references above now read 20 (picker), 21 (budget), 25 (reverse geocoding)
  and 28 (docs).
- 2026-10-04 — step implemented. The candidate-evaluation corrections are repeated here as the
  deliverable promised: `world-cities` and `city-timezones` are absent from crates.io (HTTP 404),
  `geocoding` 0.4.0 has no bundled dataset (network providers only), so `GeoNames` `cities15000` is
  vendored and indexed by ~300 lines of our own code. Deviations from the plan text, all amended in
  the deliverables above: the city member stores the geonameid, the display name and the ascii name
  (not only the ascii name) because the ranking and the fixture test need them; `search` returns
  `Result` so a corrupt blob is a typed error; `[geo] strategy` also accepts `bundled`/`network`; the
  offline geo mode still serves a *cached* geocoder answer (step 04's documented behaviour) and maps
  a cold miss to the same exit-5 not-found; the empty-cache message keeps the provider and key path
  next to the plan's wording; Springfield is 10 rows with `--limit 10` (8 exact + 2 tier-two), not 9.
- 2026-10-04 — data provenance: `download.geonames.org` timed out from the build machine (60 s, both
  direct and through the harness), so the snapshot came from the Wayback Machine's capture of the
  *official* `cities15000.zip` (`web.archive.org/web/20260903030259id_/…`): 3 314 844 B zip,
  `cities15000.txt` 8 424 283 B, 34 133 rows, newest row modification date 2026-09-02, input SHA-256
  `714c6d09…` recorded in `src/geo/data/SNAPSHOT`. A 2023 GitHub mirror was found first and
  explicitly rejected as outdated. The builder was re-run against the same input and reproduced both
  members byte for byte.
- 2026-10-04 — verification: `cargo fmt --check`, `cargo clippy --workspace --all-targets --locked
  -- -D warnings`, `cargo test --workspace --locked`, `reuse lint` (350/350) and `cargo build/test
  --no-default-features` all clean; the smoke runs in `## Verification` were executed in a throwaway
  `XDG_*` sandbox, and `strace -f -e trace=network` counted **0** `connect()` calls for
  `--offline=all`. The `--version` median is 1.6 ms and `location search --offline Beijing` 61 ms on
  the maintainer's machine; both are recorded in step 21's measured-inputs section together with the
  3.22 MiB embedded total and the 12.27 → 15.87 MB release-binary delta.
- 2026-10-04 — process change recorded: the root manifest is now a workspace (`build/geo-table`), so
  `AGENTS.md` and the CI clippy/test jobs run with `--workspace` — without it a bare `cargo
  clippy`/`cargo test` at the root would silently skip the builder.
- 2026-10-04 — follow-up measurement after the review pass: `Cities::select` now skips unwanted
  rows without materialising them (the id is read with a peek and the row is walked with bounds
  checks only), which brought `location search --offline Beijing` from 61 ms to **50 ms** and the
  release binary to 15 875 368 B; step 21's table carries the final numbers. The builder was re-run
  once more and the members were again byte-identical.
- 2026-10-04 — `scripts/refresh-city-data.sh` added (follow-up, outside the step's original
  deliverables, so recorded here): it fetches the official `cities15000.zip` (or takes `--from
  <path-or-url>`), extracts it, runs the builder, prints the new `SNAPSHOT` and the `git diff`, then
  runs the two canary suites and exits non-zero when a pinned row moved — the refresh stays a
  reviewable diff instead of a silent rewrite. Verified with both a local `.txt` and the Wayback
  zip URL; a rebuild of the same input leaves the tree unchanged.
- 2026-10-04 — `--check` added to the refresh script: it builds the given dump into a temporary
  directory and compares `cities.bin.gz`, `keys.bin.gz` and `SNAPSHOT` byte for byte with the
  committed files (exit 0 unchanged, 1 changed with the SNAPSHOT diff printed, 2 usage), which is
  the read-only answer to "is the vendored snapshot still current?" that the first version of the
  script only gave after overwriting the tree. Verified against the unmodified dump and a truncated
  one; the working tree is untouched in check mode.
- 2026-10-04 — snapshot refreshed to the official 2026-10-04 dump (34 152 rows, 310 502 keys,
  input SHA-256 `d4822b90…`), replacing the 2026-09-03 one. The dump was downloaded from
  `download.geonames.org` (first through a US egress, then confirmed to work from a direct China IP
  as well): the timeouts that pushed the 2026-09-03 snapshot through the Wayback Machine, and a
  Singapore route's failure, were transient routing problems, **not** an access restriction — the
  refresh script's help records that a failed fetch should be retried rather than worked around.
  The canaries did not move — no pinned expectation changed — and the blob sizes and step-21
  numbers were re-recorded; `--check` against the downloaded zip now reports all three files
  unchanged.
- 2026-10-04 — plan amendment after the follow-up discussion: the "user-supplied custom city file"
  half of this step's Out of scope is now **step 18b**, planned in
  `docs/plans/18b-user-city-data-update.md` (bundled table stays the default; explicit
  `location update-data` installs a dump-derived table under `$XDG_DATA_HOME`; "automatic" = the
  user's own timer; an in-run auto-fetch is rejected there for privacy, latency and determinism).
  This step's deliverable set and status are unchanged.
- 2026-10-04 — the refresh script this step introduced as a follow-up (see the entry above) was
  removed once its last unique capability moved into the builder: `geo-table --check` compares a
  dump against `src/geo/data` without writing, and the workflow is `cargo run -p geo-table -- <source>
  [--check]` plus `cargo test --workspace`. Step 18b's progress log records the change.
