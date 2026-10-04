<!--
SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Step 18b — user city-data updates

Status: ✅ done
Depends on: 18
Touches: `src/geo/table.rs` (new), `src/geo/update.rs` (new), `src/geo/offline.rs`, `src/geo/mod.rs`,
`src/cli.rs`, `src/config/mod.rs`, `src/paths.rs`, `build/geo-table/` (switches to the shared
encoder), `README.md`, `docs/plans/README.md`, `CHANGELOG.md`, `REUSE.toml`,
`tests/{offline_geo,offline_lazy,config,cli,cli_flags}.rs`,
`tests/fixtures/geo/cities-sample.txt`, `tests/fixtures/geo/cities-sample.zip`

## Goal

The bundled `GeoNames` table stays the default and the fallback, and a user may install a newer
one — built from a `cities15000` dump by the tool itself — into `$XDG_DATA_HOME/cirrocast/geo/` with
`cirrocast location update-data`. Name resolution prefers the user table when it is present and
valid, `-v` says which table answered, and the bundled table takes over again when the user table is
missing or corrupt. "Automatic" updates are the user's own scheduler running the same idempotent
command; the query path never fetches anything, so a run stays deterministic, offline-capable and
free of hidden requests.

## Deliverables

- ✅ `src/geo/table.rs`: the blob format extracted into one always-compiled module — magic and
  version, the row and key encoders, the decoders, and the byte reader — shared by
  `src/geo/offline.rs` (decode) and `build/geo-table` (encode, which drops its own copy). The
  builder and the runtime can no longer drift, and the encode→decode round trip becomes an
  in-crate unit test instead of a cross-crate one.
- ✅ `src/geo/update.rs`: `build_candidate(source, http, verbose)` resolves a `.txt`/`.zip` path or
  URL, fetches through the shared `HttpClient` (never a private socket; proxy, timeout, retries and
  the `CIRROCAST_FORBID_NETWORK` guard all apply), extracts with a **minimal ZIP reader** —
  End-of-Central-Directory, a `cities15000.txt` member (or the first `.txt`), deflate through
  `flate2`, CRC-32 checked; zip64, encrypted entries and unknown methods are typed errors — and
  encodes the blobs with `geo::table`, proving them by decoding them back. `install(candidate,
  paths)` writes `$XDG_DATA_HOME/cirrocast/geo/{cities.bin.gz,keys.bin.gz,SNAPSHOT}` atomically
  (`tmp` + rename, the previous table kept until the new one validates); `compare(candidate,
  paths)` is the `--check` half and writes nothing. No new dependency.
- ✅ Runtime source resolution: `[geo] data = "auto" | "bundled" | "user"` (default `auto`).
  `auto` uses the user table when it exists and decodes, else the bundled one — a user table that
  fails to decode is a one-line stderr warning (silenced by `-q`) plus the fallback; `bundled`
  ignores the user table (the pre-18b behaviour, and what CI pins); `user` refuses to run without a
  valid user table (`Error::Config` naming the file and `location update-data`). The lazy-decode
  guarantee stays: `--version`/`--help` and any run that never resolves a name touch neither table,
  and `tests/offline_lazy.rs` keeps asserting it.
- ✅ `cirrocast location update-data [--from <path|url>] [--check] [--timeout <SECS>]`: the manual
  update. Without `--from` it reads the official `cities15000.zip` (or `[geo] update_url`); `--check`
  builds the candidate and compares it with the active table without writing (exit 0 identical,
  1 different, 2 usage, printing the dump-date/rows/keys difference). `--offline` makes the command
  a usage error, and a build without `offline-geo` answers with the feature message instead of
  installing a table it cannot read.
- ✅ Freshness nudge, never a fetch: `[geo] update = "off" | "check"` (default `off`),
  `update_interval_days = 90`, `update_url = ""` (empty = the official dump; a mirror otherwise).
  With `check`, a run that resolved a name from a table whose `SNAPSHOT` dump date is older than the
  interval prints one stderr note (suppressed by `-q`, throttled to once per 24 h through a state
  file in the cache dir read/written with the injected clock), naming the table and
  `location update-data`. Nothing in the query path ever opens a socket for data.
- ✅ Documentation: a README "Updating the city data" section — the bundled default, the command,
  the four config keys, the privacy statement ("the tool never fetches city data by itself"), and a
  ready-to-paste `systemd --user` timer plus a cron line as the supported form of *automatic*
  update; the config schema, CLI surface and phase rows in `docs/plans/README.md`; a CHANGELOG
  entry; `REUSE.toml` annotations for the two new fixtures (hand-written sample dump → GPL,
  hand-built zip of it → GPL).
- ✅ Tests. Unit: the ZIP reader (valid, truncated, multi-member, zip64 and encrypted rejection)
  against `tests/fixtures/geo/cities-sample.zip`; `geo::table` encode→decode round trip and
  determinism; source resolution (user over bundled, fallback on a corrupt user table, `bundled`
  ignoring it, `user` failing without it); the freshness note's throttle with the fake clock.
  Integration (real binary, `CIRROCAST_FORBID_NETWORK=1`, throwaway XDG): `location update-data
  --from tests/fixtures/geo/cities-sample.zip` installs, then `location search --offline <sample
  city>` resolves from the user table and `-v` names it; `--check` exit codes; `--offline` +
  `update-data` = exit 2; a URL source under the network guard fails loudly and changes nothing; the
  user table removed → bundled table answers again.

## Design notes

* **Why the user table lives in `$XDG_DATA_HOME` and not the cache.** The cache is evictable
  (`cache clean --all` deletes namespaces, TTLs expire entries); user-installed data is state the
  user chose to keep. `paths.rs` already resolves the data directory for exactly this kind of
  artifact, and `SNAPSHOT` travels beside the blobs so freshness is readable without decoding them.
* **Why the runtime encoder and not a prebuilt download.** GeoNames publishes the raw dump only, and
  a "download our release asset" design would update only when *we* release — no freshness over
  upgrading the package. Building the table in-process from the same dump the developer path uses
  keeps one format, one credit line and one refresh story; the encoder is a few hundred lines and
  rides in the existing `flate2`/`unicode-normalization` dependencies.
* **Why a hand-written ZIP reader.** The official dump is a single-member deflate ZIP and has been
  for years; the reader is ~100 lines, testable against a committed fixture, and avoids a new
  dependency (and its licence, MSRV and supply-chain surface) for one maintenance command. Zip64,
  encryption and stored/unknown methods fail with a message naming the file, and `--from` a local
  `.txt` remains the escape hatch.
* **Rejected: fetching during a query.** `update = "auto"` meaning "fetch when stale" would put a
  hidden 3.3 MB request and a multi-second latency spike into a 50 ms command, make runs
  non-deterministic and machine-dependent, and contradict the privacy rule (a request the user did
  not ask for) and the offline modes. The freshness *note* plus an externally scheduled
  `update-data` gives the same outcome without any of that; the README documents the timer.
* **Rejected: writing the user table without validation.** The candidate blobs are decoded back
  before the rename, so a dump that no longer fits the format (or a truncated download) can never
  replace a working table; the previous table stays installed until the new one is proven.
* **Format compatibility.** The blobs carry magic + version; a user table written by a *newer*
  format version is refused with a message naming `location update-data` (re-run it with the current
  build) rather than silently ignored. The dump's own schema has been stable for years, and extra
  columns are already tolerated by the parser.
* **No new dependency is expected.** `flate2` (already a direct dependency) covers deflate; the ZIP
  container, the encoder move and the freshness state file need nothing else. If implementation
  proves otherwise, the dependency policy applies: licence, TLS/async, MSRV and what it replaces,
  written into this file before it lands.

## Out of scope

Incremental/delta updates (each refresh is the full dump), background/daemon behaviour (the tool has
no daemon and the timer is the user's), user-authored city lists that are not `GeoNames` dumps, any
change to the network geocoder fallback (`geo.strategy` stays step 18's), and verifying the download
against an upstream signature (GeoNames publishes none; integrity is the ZIP parse, the decode-back
check and the `SNAPSHOT` SHA-256 record).

## Verification

```bash
cargo fmt --check && cargo clippy --workspace --all-targets --locked -- -D warnings
CIRROCAST_FORBID_NETWORK=1 cargo test --workspace --locked && reuse lint

# install the fixture dump into a throwaway XDG tree, then resolve from it
export XDG_DATA_HOME=$(mktemp -d)
cargo run -q -- location update-data --from tests/fixtures/geo/cities-sample.zip
cargo run -q -- location search --offline Sampleville -v   # the fixture's synthetic city; -v names the table
cargo run -q -- location update-data --check --from tests/fixtures/geo/cities-sample.zip; echo $?  # 0
cargo run -q -- location update-data --offline; echo $?    # 2, usage
CIRROCAST_FORBID_NETWORK=1 cargo run -q -- location update-data; echo $?   # loud failure, table unchanged
# the bundled table still answers when the user table is absent or corrupt
```

## Exit criteria

- ✅ Every existing test still passes with no user table present: the default path is byte-for-byte
  the step-18 behaviour, and `cargo test --no-default-features` still passes.
- ✅ `location update-data --from <fixture zip>` installs a table the runtime then uses, `-v` names
  it, `--check` reports 0 for the installed dump and 1 for a different one, and a corrupt user table
  falls back with a warning (`auto`) or fails with the fix named (`user`).
- ✅ No query-path run opens a socket for data; `update = "check"` only ever prints the throttled
  note; `--offline` refuses `update-data` with exit 2.
- ✅ The builder consumes `geo::table` (no second format implementation in the tree) and its output
  for the committed dump is byte-identical to step 18's.
- ✅ `cargo fmt --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`,
  `cargo test --workspace --locked`, `cargo test --workspace --no-default-features --locked` and
  `reuse lint` clean; README/CHANGELOG/plan contract updated in the same commits.

## Risks

* The upstream dump changes shape (zip64, multiple members, a different file name): the reader
  fails with a typed message and `--from` a local `.txt` still works; the fixtures pin the accepted
  shapes.
* A user table drifts far from the installed build (e.g. installed by an older release): the
  version byte refuses it with the fix named, and `data = "bundled"` is the one-key escape.
* The freshness note becomes noise: it is opt-in, throttled to once a day and silenced by `-q`.
* The encoder moving into the main crate grows the binary by a few kilobytes and the crate's test
  surface; the round-trip test is the compensation (it catches format drift in-crate).
* Scope pressure to make `update` fetch during a query: rejected here explicitly, with the README's
  timer as the supported alternative.

## Progress log

- 2026-10-04 — step opened after the step-18 follow-up discussion. Decision recorded here: built-in
  table stays the default and the fallback; user updates are explicit (`location update-data`),
  installed under `$XDG_DATA_HOME/cirrocast/geo/`; "automatic" means the user's scheduler running
  the same command; an in-run auto-fetch was rejected (privacy, latency, determinism, offline
  modes). Step 18's Out of scope now points here for the user-supplied-table half.
- 2026-10-04 — step implemented. Deviations from this file, amended in the deliverables above:
  `geo::update` exposes `build_candidate` + `install` + `compare` instead of one `install` (the
  `--check` path needs the candidate without writing, and the split is what makes it testable); the
  decoder half of `geo::table` is `#[cfg(feature = "offline-geo")]` so a build without the feature
  compiles the encoder alone (the builder needs exactly that); the HTTP layer had to learn bytes —
  `HttpResponse` now keeps the body as `Vec<u8>` with `body()` (text) and `bytes()` accessors,
  because the dump is a ZIP and the old text-only body would have mangled it; the freshness note is
  throttled through the cache dir's `geo/update-notice.json` state file, and its policy is a pure
  `note_due(now, dump_date, interval, last_notice)` unit-tested with fixed times instead of a fake
  clock. The builder's output for the committed dump is byte-identical to step 18's (verified
  against both the pre-refresh and the current snapshot), and the fixture zip is hand-built and
  committed with a first-party REUSE annotation.
- 2026-10-04 — verification: `cargo fmt --check`, `cargo clippy --workspace --all-targets --locked
  -- -D warnings`, `cargo test --workspace --locked`, `cargo test --workspace --no-default-features
  --locked` and `reuse lint` clean; the smoke runs in `## Verification` were executed in a throwaway
  XDG sandbox (install → resolve from the user table → `--check` 0/1 → `--offline` refusal → the
  network guard failing a URL without installing anything → the note firing once and being silenced
  by `-q`).
