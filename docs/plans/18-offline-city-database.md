<!--
SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Step 18 — offline city database

Status: not-started
Depends on: 04, 05
Touches: `src/geo/offline.rs`, `src/geo/mod.rs`, `src/geo/rank.rs`, `src/cli.rs`,
`src/config/mod.rs`, `src/error.rs`, `src/geo/data/`, `build/geo-table/` (workspace member),
`Cargo.toml`, `REUSE.toml`, `README.md`, `tests/offline_geo.rs`, `tests/fixtures/geo/`,
`docs/plans/README.md`, `CHANGELOG.md`

## Goal

City names resolve to coordinates, time zones and country codes with **no network at all**, from a
gzip-compressed GeoNames table embedded in the binary. `cirrocast location search --offline Beijing`
prints ranked hits in well under a second on a cold start, `--offline=geo` uses the local table while
still fetching live weather, and bare `--offline` uses neither the network nor the live geocoder. The
offline path produces the same ranking as the network path for the same query.

## Deliverables

- [ ] **Candidate evaluation, written into this file** (done below in the design notes): the two
  crate names from the brief are corrected, `geocoding` is rejected, GeoNames `cities15000` is
  vendored. Corrections are repeated in the Progress log.
- [ ] `build/geo-table/` (dev-only workspace member, `publish = false`): reads a GeoNames
  `cities15000.txt`, writes `src/geo/data/cities.bin.gz` (columns: ascii-name, ISO country, lat,
  lon, population, IANA tz) and `src/geo/data/keys.bin.gz` (sorted folded-key → row-id list), plus
  `src/geo/data/SNAPSHOT` (GeoNames dump date + SHA-256 of the input file); `flate2`
  `Compression::best()`, deterministic byte-for-byte output, no C toolchain.
- [ ] `src/geo/offline.rs` behind the `offline-geo` feature (default-on, decision below):
  `include_bytes!` for both members, `LazyLock`/`OnceLock` decode on first use only,
  `insert_bytes!`-free build (the binary never writes), and the decode wrapped in
  `Error::Config`-free `Result` handling so a corrupt blob is a hard error with a
  `cargo run -p geo-table` hint rather than a panic.
- [ ] `src/geo/offline.rs` API: `search(query, mode: MatchMode::{Prefix, Exact}, limit) ->
      Vec<City>` and `resolve(query) -> Result<Location>`; folding =
      NFKD → strip combining marks → lowercase → drop non-alphanumerics, applied to both the index
      keys and the query, so `São Paulo`/`Sao Paulo`, `北京`/`Beijing`/`Peking`, `Wien`/`Vienna` and
      `MÜNCHEN`/`munchen` all hit.
- [ ] `src/geo/rank.rs`: the ranking already specified in step 04 (`exact-name → prefix →
      population → provider order`) extracted into a shared function that both the network geocoder
      results and the offline `City` rows feed; offline rows supply the same inputs (name, ascii
      name, population, country), so `location search` orders identically in both modes.
- [ ] `--offline[=<weather|geo|all>]` (bare = `all`, matching step 05's meaning) and
      `[network] offline = "off"` in config:
      * `--offline=weather` — weather answers come from the cache only; location resolution may use
        the bundled table and the network geocoder (this is the "cache-only weather" mode);
      * `--offline=geo` — location resolution uses the bundled table only (no Nominatim, no
        Open-Meteo geocoding, no IP lookup); weather is fetched live (the "local geocoding with live
        weather" mode);
      * `--offline` / `--offline=all` — no socket at all: bundled geocoding, cache-only weather, no
        IP lookup, `Error::Upstream`-free failure with
        `error: offline and no cached forecast for <place>` (exit 3) when the cache is empty.
- [ ] `auto` geo strategy (`[geo] strategy = "auto"`, the default): bundled table first for
      non-`~` queries; the network geocoder is consulted only when the table yields no hit or the
      query is `~`-prefixed, and the chosen source is echoed under `--verbose`.
- [ ] `cirrocast location search <query> [--offline] [--limit N] [--exact]`: table output with
      name, admin/country, population, coordinates and IANA zone; `--offline` forces the bundle and
      never opens a socket; a missing name exits 5 with
      `error: location not found: <query> (no offline match)`.
- [ ] Licensing/credits: `REUSE.toml` annotation for `src/geo/data/*.bin.gz` and `src/geo/data/SNAPSHOT`
      with a GeoNames copyright line (`GeoNames (https://www.geonames.org/)`) and the licence
      expression CC-BY-4.0 (the data is CC-BY-4.0 and is *not* relicensed to GPL);
      the README credits section gains "City data: GeoNames (CC BY 4.0), dump <date>"; the builder
      source stays GPL-3.0-or-later; `reuse lint` must stay green with the new annotation.
- [ ] Size accounting task: `ls -l src/geo/data/*.bin.gz` and `cargo build --release` before/after
      this step, with both numbers recorded in `docs/plans/22-perf-and-resource-budget.md` (phase E
      owns the budget; the measured 2.55 MiB is its input, not a surprise).
- [ ] Tests (`tests/offline_geo.rs`): Springfield (ambiguous, 9 rows — ordering pinned), São Paulo
      with and without the diacritic, 北京/Beijing/Peking, Wien/Vienna, `MÜNCHEN` lowercase input, a
      CJK query against the ASCII-only index, a missing city (exit 5), `--exact` vs prefix, the
      ranking-equivalence test against the recorded step-04 geocoder fixtures, and the startup
      assertion below.

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
  builder and the decoder. Rejected: `ruzstd` 0.9.0 (MIT, but **MSRV 1.87** > the project's 1.85) and
  `zstd`/`zstd-safe` 0.14.0/8.0.0 (BSD-3-Clause, builds the vendored C library, adding a C compiler
  and a second licence to the release pipeline for ~290 KB).
* **Default-on justification**: the table costs **2.55 MiB compressed / ≈2.7 MiB in the stripped
  binary** and removes the network from the most common failure mode (a city name on a plane, in a
  locked-down network, or when the geocoder is rate-limited). Step 22 owns the release budget; this
  step states the cost and keeps the feature switchable (`--no-default-features` without
  `offline-geo` must still build and pass tests, proving the fallback path is real). If step 22's
  budget cannot absorb 2.7 MiB, flipping the default is a one-line change recorded there.
* **Snapshot cadence**: GeoNames regenerates dumps daily; we pin one dated snapshot, record its date
  and SHA-256 in `src/geo/data/SNAPSHOT`, and refresh at release time only (step 23 documents the
  command). Nothing in the runtime ever downloads the dataset.
* **Ranking identity**: the offline index stores exactly the fields the step-04 ranking consumes, and
  `geo/rank.rs` is the single implementation, so the offline and online paths cannot drift; the test
  compares the ordering of the recorded step-04 fixtures with the offline ordering for the same
  queries.

## Out of scope

Reverse geocoding (`@lat,lon` needs no database), admin-1/admin-2 hierarchies, postal codes,
time-zone lookup for coordinates (stays with the provider response / step 03), a user-supplied
custom city file, and incremental dataset updates over the network. `location search` does not gain
paging or fuzzy (edit-distance) matching; prefix + exact on folded keys is the contract.

## Verification

```bash
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && reuse lint

cargo run -q -- location search --offline Springfield
#   9 rows, ranked by population descending, identical order to `location search Springfield`
cargo run -q -- location search --offline sao paulo     # São Paulo, SP, BR — folding works
cargo run -q -- location search --offline 北京          # Beijing — id 1816670
cargo run -q -- location search --offline Wien          # Wien/Vienna both listed
cargo run -q -- location search --offline Nowhereville; echo $?    # error, exit 5
cargo run -q -- --offline=geo -f one-line Sao Paulo     # local geocoding + live weather, exit 0
cargo run -q -- --offline -f plain Beijing              # cached only; without cache: exit 3
strace -f -e trace=network cargo run -q -- --offline=all -f plain 北京 2>&1 | grep -c connect
#   0 — no socket is opened in the total no-network mode
```

Startup assertion (in `tests/offline_geo.rs`, `assert_cmd`): the median of five
`cirrocast --version` runs is **< 50 ms** and no decode occurs — the test additionally asserts
`offline::index_loaded()` is `false` after `--version`-equivalent library calls, proving the table is
not materialised eagerly.

## Exit criteria

- [ ] `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`, `reuse lint` clean.
- [ ] `location search --offline` and the network path return the same ranked order for the four
  fixture queries (Springfield, São Paulo, Beijing, Vienna).
- [ ] `--offline=weather|geo|all` behave exactly as the three documented modes; `--offline=all` opens
  no socket (verified with `strace`).
- [ ] `cargo build --no-default-features` builds and `cargo test --no-default-features` passes (the
  non-offline path is not a stub); `--version`/`--help` never decode the table, median startup < 50 ms.
- [ ] `REUSE.toml` credits GeoNames with CC-BY-4.0, README credits the dump date, `reuse lint` green.

## Risks

* GeoNames alternatenames do not cover every exonym (e.g. some transliterations are missing for
  small towns); mitigated by prefix matching on ascii/alternate keys, and by `--offline=auto`'s
  network fallback in the default strategy.
* The dataset's population column is coarse for non-cities and can order two same-named places
  counter-intuitively; the ranking test pins the current order so a later change is deliberate.
* A committed binary blob is opaque in review and 2.55 MiB is a real binary-size commitment;
  mitigated by a deterministic builder, the `SNAPSHOT` checksum and a byte-comparison test, and, if
  step 22 rejects the size, by the documented flip to opt-in (README usage + CHANGELOG updated).

## Progress log

- 2026-09-30 — step opened; crate candidates measured (`world-cities`/`city-timezones` absent from
  crates.io, `geocoding` has no bundled dataset), GeoNames `cities15000` sizes and gzip/zstd
  trade-offs measured, flate2 chosen over ruzstd (MSRV 1.87) and the zstd C bindings.
