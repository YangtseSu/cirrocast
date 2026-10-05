<!--
SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Performance and the resource budget

`cirrocast` promises interactive numbers — a version probe, a help screen and a cached forecast are
supposed to feel instant — and a binary small enough to package. Step 21 turns those promises into
enforced numbers: one measurement harness (`scripts/bench/run.sh`), one committed baseline
(`perf/baseline.json`), one CI gate (`perf` job) and this file, which records what is measured, on
what machine, with what budget, and which decisions the measurements justify.

## Release profile

`[profile.release]` in `Cargo.toml`:

| Setting | Value | Why |
|---|---|---|
| `lto` | `"thin"` | whole-program optimisation without the full-LTO build time |
| `codegen-units` | `1` | gives thin LTO one unit to optimise across |
| `strip` | `"symbols"` | the shipped binary needs no symbol table |
| `panic` | `"abort"` | every user-triggered failure is a typed `Error`; no release path unwinds, so the landing pads only cost size |

`[profile.release-audit]` inherits `release` with `strip = "none"`, because `cargo bloat` cannot
attribute bytes to crates once the symbol table is gone. It is never shipped and never measured by
the harness; it exists for the audit below.

`panic = "abort"` is compatible with the error policy in `AGENTS.md`: user-triggered failures are
typed errors, `unwrap`/`expect`/`panic!` are banned outside tests, and the profile applies to
`--release` only, so the test suite still unwinds.

## Feature inventory

The crate has one feature:

| Feature | Default | What it adds | Measured cost |
|---|---|---|---|
| `offline-geo` | **on** | the embedded GeoNames city table (`src/geo/data/*.bin.gz`) and its decoder, so a city name resolves without a network round trip | **+3,574,032 B** (14.36 MiB → 10.95 MiB binary), and ~19.7 MiB of RSS while a name is resolved (the decoded name index) |

`cargo build --release --no-default-features` is the reduced build: the same CLI, with city search
falling through to the network geocoder (`src/geo/offline.rs` is feature-gated at `src/geo/mod.rs`,
and `src/cli.rs`'s `local_lookup` becomes a stub). The exit criteria are met by measurement, not by
assumption:

* the reduced build compiles (`cargo build --release --no-default-features --locked`);
* `CIRROCAST_FORBID_NETWORK=1 cargo test --workspace --no-default-features --locked` passes;
* the size delta is the 3.41 MiB above, recorded here and in `perf/baseline.json`'s sibling metric
  set (the harness always builds the default feature set — that is what ships).

Dependency-level switches that were measured rather than assumed (the audit's "gate list"; each was
built and reverted to obtain the number):

| Switch | Size delta | Decision |
|---|---|---|
| `clap` without `wrap_help` | −936 B | **kept**: the help must wrap to the terminal, and a kilobyte is not a reason to break it |
| `ureq` without its default features (`gzip`, `charset`, `socks-proxy`) | −12,496 B | **kept**: gzip is how providers serve responses; `charset` decodes non-UTF-8 bodies; the saving is noise against a 15 MB binary |
| `keyring` | n/a | not a dependency: v1 stores keys in `keys.toml` (0600) and lists OS keyring storage as out of scope (step 10), so the plan's gate-list entry has nothing to turn off |

Feature-gating the remaining dependency defaults was considered and rejected: every one of them
either is required at runtime (TLS, gzip, time zones) or saves less than 0.1 % of the binary.
