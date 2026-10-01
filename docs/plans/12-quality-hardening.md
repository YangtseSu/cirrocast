<!--
SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Step 12 — Quality hardening

Status: 🚧 in-progress
Depends on: 08 (cli-surface-and-formats), 09 (localization), 10 (additional-providers)
Touches: `src/**/*.rs` (error text and logging only), `src/http.rs` (network guard), `src/config/mod.rs`
(validation), `src/cli.rs` (help text), `deny.toml` (new), `.github/workflows/ci.yml` (new),
`LICENSES/GPL-3.0-or-later.txt`, `REUSE.toml`, `tests/{exit_codes,no_network,decoder_robustness}.rs` (new),
`tests/fixtures/malformed/**` (new), `README.md`, `AGENTS.md` (lint trap only)

## Goal

No new features: make everything already shipped trustworthy and continuously verifiable. Every user-facing
error names the offending value and the next command; `--verbose` explains a failure from cause to symptom
without leaking key material; `--offline` provably never touches the network; contradictory config is rejected
with the key path; `--help` documents precedence and exit codes; dependency licences are checked for
GPL-3.0-or-later compatibility and advisories are tracked; the crate builds on its declared 1.98 MSRV; every
upstream JSON decoder survives truncated and hostile input with `Error::Upstream`, not a panic; all gated by CI.

## Deliverables

- ⬜ Error-message audit across every module (`error.rs`, `cli.rs`, `config/`, `geo/`, `http.rs`, `cache.rs`,
  `provider/*`): every message names the offending value plus the accepted forms or the next command — missing
  key → `cirrocast key set openweathermap` and `CIRROCAST_OPENWEATHERMAP_KEY`; unknown provider/format/units/
  language → the accepted values; bad `--days` → the `0..=14` range and the provider's `max_days`; `keys.toml`
  wrong mode → `chmod 600 <path>`; config error → the key path; unknown location → `cirrocast location search
  <q>`; offline miss → provider, location, "rerun without `--offline`".
- ⬜ `tests/exit_codes.rs`: one deterministic scenario per binding exit code — 0 (`--version`), 2 (`--days 99`,
  `--format yaml`, `--station 12`), 4 (unreadable config, `keys.toml` at 0644), 6 (`--provider qweather`, no key,
  scratch config dir), 3 (`--refresh` with `CIRROCAST_FORBID_NETWORK=1`), 5 (unknown ICAO, stub stationinfo
  response). Asserts the code and that stderr names the offending value, never a full sentence.
- ⬜ `--verbose` review: `-v` prints one indented `caused by:` line per error source; `-vv` adds the request URL
  (secrets redacted), HTTP status, retry attempt and cache decision; `--quiet` mutes warnings, never errors; no
  log line ever contains a key or a `keys.toml` body.
- ✅ `--offline` correctness: with the guard on, `--offline` constructs no request at all; a warm cache renders
  identically to a run without the flag; a cold cache fails with exit 3 plus the rerun hint; `--no-cache
  --offline` and `--refresh --offline` are usage errors naming both flags; `cache stat` separates
  expired-but-present from valid entries.
- ✅ `cirrocast config validate` returns 0 on the shipped template and 4 with the key path for: unknown keys,
  unknown enum values, a non-ICAO `[providers.metar] station`, `render.width` below the minimum,
  zero/negative timeouts and TTLs, a `location.default` that fails the location grammar, and
  `cache.enabled = false` combined with `--offline`; the precedence rule `[units] <key> > defaults.units` is
  stated in the message when an override conflicts.
- ⬜ Help-text review: `--help`/`--version` read no config, XDG or cache state; a `CONFIG PRECEDENCE` block
  states CLI flag > `CIRROCAST_*` env var > `config.toml` > built-in default; an `EXIT CODES` block lists all
  seven codes with triggers; the man page renders from the same `cli.rs` text.
- ⬜ Startup gate recheck: `hyperfine --warmup 10 --runs 50 'target/release/cirrocast --version'` under 50 ms,
  and `strace -f -e trace=network target/release/cirrocast --version` with no `socket(` (enforced budgets with
  CI thresholds are step 22).
- ⬜ `deny.toml` + `cargo deny check` green: `[licenses]` allow-list (MIT, Apache-2.0, ISC, BSD-2-Clause,
  BSD-3-Clause, Zlib, Unicode-3.0, CDLA-Permissive-2.0, CC0-1.0, MPL-2.0, each with a one-line justification),
  deny list for licences that cannot combine with GPL-3.0-or-later (`GPL-2.0-only`, pre-3.0 `OpenSSL`,
  `SSPL-1.0`, `BUSL-1.1`, `Elastic-2.0`, `Commons-Clause`, `JSON`, `BSD-4-Clause`), `[bans]` denying duplicate
  versions and wildcard dependencies, `[sources]` restricted to crates.io, and `[advisories]` ignore entries
  each carrying a reason and a review date.
- ⬜ Dependency licence audit: enumerate every crate with `cargo metadata --all-features` and commit the
  name/version/licence/GPL-3.0-or-later verdict table into this file's Design notes in the same commit as
  `deny.toml`; expected exceptions needing an allow-list entry are `Unicode-3.0` (through `idna`/`url`) and
  `CDLA-Permissive-2.0` (through `webpki-roots`); an incompatible dependency is a blocker with a chosen
  replacement, never a silent allow-list. `cargo audit` runs beside `cargo deny` since the sources disagree.
- ⬜ MSRV at 1.98: `cargo +1.98.0 build --all-targets --locked` and `cargo +1.98.0 test --locked` pass locally
  and in a dedicated CI job; `cargo msrv verify` (cargo-msrv, optional) confirms the declared `rust-version`;
  `rust-toolchain.toml` keeps pinning `stable` for development, the `+1.98.0` override wins in the job.
  (The floor tracks the latest stable release rather than lagging behind it: step 11 raised 1.85 → 1.98 in
  one move, which is also what `tzf-rs` 2.x and the crate's let-chains require.)
- ⬜ `reuse lint` reaches 0 problems and stays there. The tree currently reports 1 invalid SPDX expression and
  1 file with no licensing information: `AGENTS.md` demonstrates a header with prose on the same line, so REUSE
  parses the prose as part of the licence expression (the value must sit alone on its line, or the example must
  be wrapped in `REUSE-IgnoreStart`/`REUSE-IgnoreEnd`), and `src/main.rs` needs its header. Fixture licensing
  follows the upstream rule — exact-path annotations with the upstream licence (CC-BY-4.0, ODbL-1.0, a
  public-domain `LicenseRef` for NOAA data) — so no blanket `tests/fixtures/**` override may remain. Windows
  checkout decision (Design notes): replace the `LICENSES/GPL-3.0-or-later.txt` symlink with a real copy of
  `LICENSE` and gate the pair with `cmp -s LICENSE LICENSES/GPL-3.0-or-later.txt` in CI.
- ⬜ `tests/no_network.rs`: the `CIRROCAST_FORBID_NETWORK=1` guard is enforced in `src/http.rs` before DNS and
  connect; the test proves the guard intercepts (a cold-cache request to a real upstream exits 3 with the guard
  message) and, on Linux, reruns the CLI under `unshare -rn` to prove no external DNS is required.
- ⬜ Decoder robustness: new malformed fixtures plus a sweep over every upstream JSON decoder (open-meteo, the
  six key-requiring providers, metar, geocoding, IP location, config) feeding a truncation sweep at every byte
  offset, empty/`{}`/`[]`/`null` bodies, wrong types (`"temp": "abc"`) and single-byte mutations — each input
  yields `Error::Upstream`/`Error::Config` with a cause chain and never panics, hangs or allocates unboundedly
  (payload cap enforced in `http.rs`).
- ⬜ Render-path audit: `src/render/**` and `src/model/**` import nothing from `http`, `provider` or `cache`,
  enforced by a CI grep gate (`! grep -rn 'use crate::\(http\|provider\|cache\)' src/render src/model`), with
  `cargo tree` confirming only `serde`/`serde_json`/`chrono`/locale data beyond std.
- ⬜ XDG audit: no write outside `$XDG_{CONFIG,CACHE,DATA}_HOME/cirrocast` (verified by running with all three
  pointed at a temporary tree and diffing it), `XDG_CONFIG_DIRS` honoured for reads, and `--offline`/`--no-cache`
  creating no cache directory.
- ⬜ Secret-handling audit: keys never land in `config.toml`, `key list` masks values, `keys.toml` is written
  0600 and refused when wider, and a test greps captured `-vv` stderr for the fake key it exported and finds
  nothing.
- ⬜ `.github/workflows/ci.yml`: jobs `fmt`, `clippy --all-targets -- -D warnings`, `test` (matrix
  ubuntu-latest + macos-latest × stable + 1.85.0, `CIRROCAST_FORBID_NETWORK=1`, `--locked`), `reuse`
  (`fsfe/reuse-action`), `deny` (`EmbarkStudios/cargo-deny-action`), `audit` (`rustsec/audit-check`), plus the
  layer and `cmp` gates; every third-party action pinned to a commit SHA, `concurrency` cancelling superseded
  runs, no Windows job. README/AGENTS document the matrix, MSRV, deny policy, the no-network rule and the
  resulting CI summary.

## Design notes

* `LICENSES/GPL-3.0-or-later.txt` is currently a symlink to `../LICENSE`; a Windows checkout with
  `core.symlinks=false` (default outside Developer Mode) materialises it as a text file containing `../LICENSE`,
  so the lint verdict depends on the platform. **Decision: ship a real copy** beside the root `LICENSE` that
  GitHub detects, with `cmp -s` in CI against drift. Verified while writing this plan: replacing the symlink with
  a copy in a scratch checkout leaves `reuse lint` output identical, so it costs one duplicated 35 KiB file.
  Rejected: lint on Linux/macOS only (Windows contributors keep a red local lint, no documented fix); requiring
  `core.symlinks=true` (an undocumented per-developer setup step); making the root `LICENSE` the pointer (GitHub
  licence detection needs a real root file).
* Guard design: `CIRROCAST_FORBID_NETWORK=1` is read once by `src/http.rs` and blocks every non-loopback
  connection before DNS, so a blocked run cannot even resolve a name. Loopback stays reachable so tests can aim a
  provider's base URL at an in-process stub; the guard's own test asserts blocking against a real upstream,
  because CI has network and a silently broken guard would otherwise pass. The whole `test` job exports the
  variable, so an accidentally network-dependent test fails loudly.
* Robustness approach: deterministic truncation and mutation sweeps instead of a fuzzing dependency —
  `cargo-fuzz`/`honggfuzz` need nightly plus corpus infrastructure that a CLI decoder does not justify, while a
  byte-offset sweep covers exactly the realistic failure class (unchecked slicing, `serde` type assumptions).
  The sweep is driven by fixtures already committed in steps 06/10/11, so it stays reproducible and offline.
* Licence policy: GPL-3.0-or-later admits permissive licences (MIT/Apache-2.0/ISC/BSD/Zlib/CC0), MPL-2.0
  (compatible, with a source-availability duty for modified MPL files) and the permissive data licences
  `Unicode-3.0` and `CDLA-Permissive-2.0`; the denied classes above are GPL-incompatible. The verdict table for
  the full dependency set is committed here together with `deny.toml`.
* `cargo deny` gates licences, advisories, bans and sources; `cargo audit` stays as an independent advisory check
  because mirrored databases can lag behind live RUSTSEC data.
* No Windows CI job in v1: no Windows packaging exists (step 13 ships Linux/macOS archives, ecosystem packaging
  is step 24), no Windows-specific code path exists, and the symlink trap would make lint platform-dependent.

## Out of scope

* Feature work, including the error wording of features that do not exist yet: alerts (step 15), air quality and
  pollen (step 16), moon/astro (step 17), offline city database (step 18), more providers (step 19), the
  wttr.in-compatible service (step 20), multi-location output (step 21).
* Enforced performance/resource budgets with CI thresholds: step 22 (here only the startup gate is rechecked).
* Packaging and release mechanics: step 13; ecosystem artefacts and Windows/macOS installers: step 24.

## Verification

```
cargo deny check && cargo audit && reuse lint                    # all green, 0 lint problems
cargo +1.85.0 build --all-targets --locked && cargo +1.85.0 test --locked
CIRROCAST_FORBID_NETWORK=1 cargo test                            # suite green with the guard active
strace -f -e trace=network target/release/cirrocast --version 2>&1 | grep -c 'socket('   # expect 0
hyperfine --warmup 10 --runs 50 'target/release/cirrocast --version'                     # under 50 ms
target/release/cirrocast cache clean && target/release/cirrocast Beijing --offline        # exit 3, rerun hint
```

## Exit criteria

- ⬜ `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`, `reuse lint` all clean.
- ⬜ `cargo deny check` and `cargo audit` green, `cargo +1.85.0 build --all-targets --locked` green, and the
  dependency licence verdict table filled in with no unresolved GPL-3.0-or-later incompatibility.
- ⬜ `CIRROCAST_FORBID_NETWORK=1 cargo test` passes, the guard test shows a blocked request exiting 3 with the
  guard message, and `hyperfine` reports `--version` under 50 ms with no socket opened.

## Risks

* Advisory-database or mirror flakiness in CI would destabilise red/green: pin actions by SHA and give every
  `[advisories] ignore` entry a reason plus a review date.
* A dependency bump can raise MSRV above 1.85 or add a denied licence: `--locked` in CI forces such a bump into a
  reviewed commit that the licence gate can fail.
* The loopback exception could hide a network-dependent test: the blocking test targets a real host, while the
  layer gate and the `unshare -rn` run give independent signals.
* Message-contract tests could ossify wording: assertions target the offending value and the next-command
  substring only, keeping rewordings cheap.
* macOS runners lack `strace`: the no-network check is Linux-only, while macOS runs `--version` and the timing check.

## Progress log

- 2026-09-30 — step file written (status: not-started); `reuse lint` state (1 invalid expression, 1 headerless
  file) and the symlink-versus-copy behaviour recorded while drafting.
- 2026-10-01 — step opened (status: in-progress). Landed: the `CIRROCAST_FORBID_NETWORK` guard and the 8 MiB
  body cap in `src/http.rs` (blocked before DNS; loopback exempt for in-process stubs); the offline-miss error
  now names provider, place, key path and the rerun hint (`Cache::read_or_fetch_json` gained the `place`
  argument, so `fetch_json` takes the `Location` and metar its station); `cache stat` counts expired entries
  separately; `config validate` got the strict key check (`check_known_keys`), the ICAO station rule
  (`provider::metar::is_icao_station`, shared with `--station`), the `location.default` grammar check, the
  `cache.enabled = false` + `--offline` combination (also enforced on a weather run) and the `[units]` override
  precedence notes; `MissingKey` and `LocationNotFound` name their next command. Two deliverables ticked;
  the verbose review and the message audit are still open.
