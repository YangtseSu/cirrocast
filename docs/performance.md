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

`[profile.release-audit]` inherits `release` with `strip = "none"`. `cargo bloat` attributes the
top crates identically on the stripped binary, but the audit profile resolves more of the tail (51
crates left in its "and more" bucket, against 63 on the shipped profile). It is never shipped and
never measured by the harness; it exists for the audit below.

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

Both exit criteria are enforced rather than only asserted: CI's `build --no-default-features` and
`test --no-default-features` jobs run exactly those two commands on `ubuntu-26.04` and `macos-26`,
so an edit to a stub path that no longer compiles fails the build instead of shipping.

Dependency-level switches that were measured rather than assumed (the audit's "gate list"; each was
built and reverted to obtain the number):

| Switch | Size delta | Decision |
|---|---|---|
| `clap` without `wrap_help` | −936 B | **kept**: the help must wrap to the terminal, and a kilobyte is not a reason to break it |
| `ureq` without its default features (`gzip`, `charset`, `socks-proxy`) | −12,496 B | **kept**: gzip is how providers serve responses; `charset` decodes non-UTF-8 bodies; the saving is noise against a 15 MB binary |
| `keyring` | n/a | not a dependency: v1 stores keys in `keys.toml` (0600) and lists OS keyring storage as out of scope (step 10), so the plan's gate-list entry has nothing to turn off |

Feature-gating the remaining dependency defaults was considered and rejected: every one of them
either is required at runtime (TLS, gzip, time zones) or saves less than 0.1 % of the binary.

## Reference machine

The committed baseline was recorded on 2026-10-05 on the maintainer's machine:

| | |
|---|---|
| CPU | AMD Ryzen 5 3600X 6-Core Processor (6 cores / 12 threads) |
| Memory | 32 GiB |
| OS | CachyOS (Arch Linux), kernel `7.2.8-1-cachyos` |
| Toolchain | rustc/cargo 1.99.0 |
| Harness | hyperfine 1.20.0, Python 3.14.8 |
| RSS method | `hyperfine-wait4` (GNU time is not installed) |

`perf/baseline.json`'s `machine` object carries the same spec, so a baseline is never detached
from the machine it describes. The CI gate compares a runner's fresh medians against it with the
20 % ratio allowance; if the runner class is materially slower, the baseline is re-recorded *from
the runner* (the `record_baseline` dispatch input) and the resulting file is reviewed and committed
— a baseline must come from the machine class the gate runs on, and re-recording is a commit, not a
side effect.

## Methodology

`scripts/bench/run.sh`, in order:

1. `cargo build --release --locked` — the shipped profile, never `release-audit`.
2. A throwaway XDG tree under `target/bench/sandbox`, with `[alerts] enabled = false` and a cache
   primed from `tests/fixtures/open_meteo/forecast_beijing_2026-07-15.json` under the exact key the
   run asks for (`weather/open-meteo-39.91-116.40-3-<location-local date>.json`).
3. `CIRROCAST_FORBID_NETWORK=1` for every measured run, so an accidental request fails loudly
   instead of quietly becoming part of the number. That is also why alerts are off: a
   default-config run fetches the WMO SWIC index and one FPAS CAP document per covered area.
4. hyperfine `--warmup 5 --runs 30 -N` (no shell), prefixed with `taskset -c 0` where available, so
   one core and one shell-less exec decide the median.
5. Peak RSS: GNU time's `Maximum resident set size` when `/usr/bin/time` is GNU time; otherwise the
   per-child peak hyperfine recorded for the same command. A `/proc`-sampling fallback was rejected
   (a 50 ms run is too short to sample), and `getrusage(RUSAGE_CHILDREN)` read from a forked parent
   was tried and rejected: it counts the parent's resident pages and overstated `--version` by about
   6 MiB.
6. Binary size with `stat`, `--help` length with `wc -l`.

Two cached shapes are measured, because they differ by the path they take rather than by the work
they do: `--offline Beijing -f plain` (cache read only) and `Beijing -f plain` (the online path
enabled, still served from the seeded cache — the network guard proves it).

The **cold run** is measured by hand (`scripts/bench/cold.sh`: empty cache, `--refresh`, three timed
runs, median) and is never gated: the number depends on the link, the provider and the city. The
recorded figure is 845 ms on a direct fibre link with alerts disabled. A default-config cold run
measured ~8.5 s on the same link, because the alert sources fan out (the WMO index plus one CAP
document per FPAS area); that figure is informational and is not a budget.

## The budget table

| Metric | Budget | Baseline (2026-10-05) | Notes |
|---|---|---|---|
| `--version` | 20 ms | **2.03 ms** | startup: no runtime, no table |
| `--help` | < 200 lines | **198 lines** | clap wraps to the width; the test pins `COLUMNS=100` |
| cached run, `--offline` | 60 ms | **51.4 ms** | includes the city-table name index decode |
| warm-cache run | 60 ms | **47.5 ms** | the online path with a fresh cache |
| RSS, `--version` | 15 MiB | **6.4 MiB** | the goal's "RSS under 15 MB", met |
| RSS, cached run | 32 MiB (re-derived) | **25.6 MiB** | the decoded name index is ~19 MiB |
| binary | 17 MiB (re-derived) | **14.36 MiB** | see the re-derivation below |
| cold run | 1.5 s, not gated | **845 ms** | link-dependent, manual, recorded with its conditions |

### Budget re-derivation

Two of the goal's numbers were written before step 18 scoped the offline city table:

* **binary < 5 MiB.** The embedded GeoNames members are 3.28 MiB and their decoder, TLS and the
  catalogs put the floor near 15 MiB; the table is a documented default-on feature, so the budget
  moves to 17 MiB — the measured size plus headroom for one more dependency.
* **RSS < 15 MB.** A run that never resolves a name (the `--version` probe) sits at 6.4 MiB and
  keeps the 15 MiB budget. A cached run decodes the name index to resolve `Beijing`, which costs
  ~19 MiB, so its budget is 32 MiB — the measured peak plus headroom.

The original figures stay in this table's history (and in step 21's goal) so the change is visible;
what moved is the enforced number, not the measurement. Dropping `offline-geo` to satisfy 5 MiB was
rejected: it trades a user-visible feature (offline city search, step 18) for a number.

## The gate, and the proof that it bites

`scripts/bench/compare.py` exits 1 when a fresh median exceeds the committed baseline by more than
20 %, or when it exceeds a hard budget the baseline was inside. A budget the baseline already misses
is printed as "budget re-derivation due" rather than failing every run — that state means the budget
needs review, not that the run regressed.

The gate was verified against a deliberately injected regression (2026-10-05): a 300 ms sleep on
every `--offline` run, rebuilt, then the harness and the gate re-run:

```text
metric                   budget     baseline        fresh    delta  verdict
---------------------------------------------------------------------------
binary_bytes       17,825,792 B 15,057,960 B 15,058,352 B        +0.0%  ok
help_lines            200 lines    198 lines 198 lines    +0.0%  ok
help_ms                       -         2 ms 2 ms       -1.4%  ok
offline_plain_ms          60 ms        51 ms 352 ms     +585.5%  FAIL +585% vs baseline
plain_rss_kib        32,768 KiB   26,228 KiB 26,348 KiB      +0.5%  ok
version_ms                20 ms         2 ms 2 ms       +3.5%  ok
version_rss_kib      15,360 KiB    6,604 KiB 6,636 KiB      +0.5%  ok
warm_plain_ms             60 ms        47 ms 52 ms       +9.5%  ok

cold run (not gated): 845 ms — CachyOS box, direct fibre (no proxy); Beijing via open-meteo, alerts disabled as in the bench config
exit=1
```

The sleep was then removed, the binary rebuilt, and the same two commands printed `no regression:
every gated metric is inside its budget and the baseline` with exit 0.

## Dependency weight

`cargo bloat --profile release-audit --crates -n 20` (2026-10-05, rustc 1.99.0, `.text` is 4.0 MiB
of a 14.36 MiB binary):

```text
 File  .text     Size Crate
 3.9%  23.8% 982.1KiB cirrocast
 3.1%  18.9% 779.6KiB std
 1.4%   8.3% 343.0KiB rustls
 1.1%   7.0% 288.8KiB serde_core
 1.1%   6.9% 286.2KiB ring
 1.0%   6.0% 246.3KiB clap_builder
 0.7%   4.3% 177.5KiB serde_json
 0.5%   2.9% 120.7KiB ureq
 0.3%   2.1%  86.1KiB toml
 0.3%   1.6%  67.4KiB [Unknown]
 0.3%   1.6%  65.4KiB clap_complete
 0.2%   1.3%  53.4KiB chrono
 0.2%   1.2%  50.1KiB ureq_proto
 0.2%   1.2%  49.7KiB miniz_oxide
 0.2%   1.2%  49.6KiB webpki
 0.2%   1.1%  44.9KiB toml_parser
 0.2%   1.0%  40.9KiB clap_mangen
 0.2%   1.0%  39.4KiB http
 0.1%   0.8%  34.7KiB tzf_rs
 0.1%   0.7%  29.5KiB fluent_bundle
 1.0%   6.0% 247.7KiB And 51 more crates. Use -n N to show more.
16.4% 100.0%   4.0MiB .text section size, the file size is 24.5MiB
```

`.text` is only a quarter of the file: the rest is data — the embedded city table (3.28 MiB),
`chrono-tz`'s compiled tzdata, `tzf-rs`'s polygon index, the Fluent catalogs and the art blocks.
Every heavyweight crate is accepted with a reason:

| Crate | Decision |
|---|---|
| `chrono-tz` | accepted: timezone-correct day parts are a contract requirement (providers aggregate hourly data in the location's zone) and the zone data must work offline |
| `rustls` + `ring` + `webpki` | accepted: TLS is not optional, and rustls is the stack the dependency policy allows (no C toolchain) |
| `clap_builder` + `clap_complete` + `clap_mangen` | accepted: the documented CLI surface, completion scripts and man page; `wrap_help` alone was measured at 936 B |
| `serde_core` + `serde_json` | accepted: provider payload decoding and the `json` renderer |
| `toml` + `toml_parser` | accepted: `config.toml` |
| `ureq` + `ureq_proto` + `http` | accepted: the sync HTTP contract |
| `miniz_oxide` (through `flate2`) | accepted: the embedded table's gzip and (via ureq) response bodies, pure Rust |
| `tzf_rs` | accepted: METAR's coordinate → zone lookup, offline; 35 KiB of `.text` plus its data |

No crate is a removal candidate at this size. The only switches with measured deltas are the two in
the feature inventory above, and both are kept.

`cargo llvm-lines --release --lib -p cirrocast` (2026-10-05): **695,663** lines of LLVM IR across
**13,634** copies. The largest single items are clap's derive-generated `augment_args` /
`augment_args_for_update` (2,543 / 2,423 lines, one copy each), `Config::set_key` (2,264), the serde
visitors of each provider payload (~1,200–1,700 each), `Alerts::render` (1,515) and `Error`'s
`Display` (1,321). Nothing is a duplicated monomorphisation — the biggest entries are single copies
— so there is no generic to collapse; the total is the CLI and serde surface the features require.

## No async runtime, no TUI toolkit: decision kept

Step 21 re-evaluated both exclusions against the measured evidence; both stay.

* **Startup is the runtime-cost proxy.** `--version` runs in 2.03 ms at 6.4 MiB RSS: there is no
  runtime to initialise, no reactor, no worker pool.
* **The sync stack's size share.** `ureq` + `rustls` + `ring` + `webpki` + `ureq_proto` are about
  850 KiB of the 4.0 MiB `.text` (21 %). `tokio` would add a runtime, a reactor and a second thread
  pool on top of that — for a process whose run makes one to three dependent requests (forecast,
  then optionally alerts and air quality), where latency hiding has nothing to hide.
* **The concurrency that exists is already there.** A multi-location run overlaps up to four
  locations with threads (step 19's ordered parallel map), which is exactly the concurrency async
  would buy, without the runtime.
* **TUI.** The repository's non-goals exclude an interactive screen. A toolkit would add a render
  loop, event handling and terminal-state management to a process whose entire job is one screen of
  text — against a 6.4 MiB idle RSS and a 14.36 MiB binary that the budgets above are trying to
  hold, not grow.
