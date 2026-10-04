<!--
SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Step 21 — performance and resource budget

Status: ⬜ not-started
Depends on: 12 (quality hardening and CI), 18 (offline city database), 19 (multi-location and templates)
Touches: `scripts/bench/{run.sh,cold.sh,compare.py,ci.sh}`, `perf/baseline.json`,
`.github/workflows/ci.yml`, `Cargo.toml`, `src/render/{mod,art_table,one_line,plain}.rs`,
`src/model/mod.rs`, `src/geo/offline.rs`, `src/cli.rs`, `tests/cli.rs`, `docs/performance.md`,
`REUSE.toml`

## Goal

The informal budgets become enforced numbers with one measurement harness, one committed baseline and
a CI gate: `--version` under 20 ms, a cached single-location run under 60 ms, a cold run under 1.5 s on
a 100 Mbps link, RSS under 15 MB, the default release binary under 5 MB, `--help` under 200 lines. The
step also does the rendering work the numbers demand (`fmt::Write` hot path, one timezone lookup per
report instead of one per field, lazy offline-geo decode), audits dependency weight with `cargo
bloat`/`cargo llvm-lines`, and re-evaluates "no async runtime / no TUI toolkit" against the measured
evidence — keeping the decision, recording why.

## Deliverables

- ⬜ `Cargo.toml`: `[profile.release] strip = "symbols"`, `lto = "thin"`, `codegen-units = 1`,
      `panic = "abort"`; the step-18 `offline-geo` feature stays **default-on**, with the
      `--no-default-features` proof landing here, so `cargo build --release --no-default-features`
      yields the reduced build (same CLI, city search via the network geocoder only). Feature
      inventory documented in `docs/performance.md`.
- ⬜ `scripts/bench/run.sh`: `cargo build --release --locked`, then hyperfine
      (`hyperfine --warmup 5 --runs 30 -N`) over `--version`, `--help`,
      `--offline Beijing -f plain` and a warm-cache `Beijing -f plain`, with `XDG_CACHE_HOME`,
      `XDG_CONFIG_HOME` and `TZ` pinned to a temp tree seeded from `tests/fixtures/`; RSS via
      `/usr/bin/time -v` (`Maximum resident set size`), binary size via `stat -c %s`, `--help` lines
      via `wc -l`; everything written to `target/bench/raw.json`.
- ⬜ `scripts/bench/cold.sh`: manual cold-run harness — empty cache dir, `--refresh`, three timed
      runs, prints median; run by hand on the documented reference machine and link, never in CI
      (a network number cannot be gated).
- ⬜ `scripts/bench/compare.py`: Python 3 (stdlib only) reader of `perf/baseline.json` and
      `target/bench/raw.json`; prints a metric-by-metric table with budget, baseline and delta; exits
      1 when a gated median exceeds `baseline × 1.20` or a hard budget.
- ⬜ `perf/baseline.json`: `{"schema": 1, "recorded": "YYYY-MM-DD", "commit": …, "machine": {…},
      "metrics": {…}}` with medians for every gated metric plus the cold-run figure and its conditions.
- ⬜ `.github/workflows/ci.yml`: new `perf` job (release build, `scripts/bench/run.sh`,
      `scripts/bench/compare.py`, `cargo bloat --release --crates -n 20` and
      `cargo llvm-lines --release | head -n 20` uploaded as artifacts). Runner pinned to the same
      image class as the baseline was recorded on; the job is allowed to re-record only via an explicit
      `workflow_dispatch` input.
- ⬜ `docs/performance.md`: machine specification (`lscpu` summary, kernel, rustc/cargo/hyperfine
      versions, commit), methodology (commands, cache priming, RSS and size measurement, why the cold
      run is not gated), the budget table with the measured numbers, the dependency-weight audit, and
      the async/TUI re-evaluation with evidence.
- ⬜ `src/render/{mod,art_table,one_line,plain}.rs`: `fmt::Write` rendering into one pre-sized
      `String` (`with_capacity(4096)`), no per-cell `format!`, no intermediate `Vec<String>` of lines
      or per-line `String`; colour codes written as escapes into the same buffer; `Renderer::render`
      keeps its signature.
- ⬜ `src/model/mod.rs` + `src/cli.rs`: `LocalTimes` computed once per report (`chrono_tz` lookup,
      day-part `DateTime<FixedOffset>`, preformatted clock strings), carried in `RenderContext`;
      renderers and the template engine stop converting per field.
- ⬜ `src/geo/offline.rs`: lazy table decode (header first, `OnceLock` per shard) when step 18
      measured an eager startup decode; `--version`/`--help` and any run that never performs a city
      lookup must not touch the table at all (asserted by a test that removes the data file and runs
      `--version`).
- ⬜ Dependency-weight audit in `docs/performance.md`: the top contributors from `cargo bloat
      --crates` with sizes, and a decision per heavy crate — `chrono-tz` (accepted: timezone-correct
      day parts are a contract requirement), `rustls`/`ring` or `aws-lc-rs` (accepted: TLS is not
      optional), `clap_builder` (accepted; `wrap_help` reviewed), `serde_json` (accepted),
      `flate2` (accepted, feature-reviewed), `idna`/`url` (accepted via `ureq`), plus the gate list
      (`keyring`, `offline-geo`, `ureq` compression features, `clap` `wrap_help`) and the measured
      delta of turning each off.
- ⬜ `tests/cli.rs`: `--help` line count < 200 and `--help` exit 0 asserted deterministically.

## Design notes

* **Gate on ratios, report absolute budgets.** Shared CI runners fluctuate by more than the 20 %
  tolerance in absolute terms, so the gate compares fresh medians against the committed baseline with
  a 20 % allowance; the absolute budgets (20 ms, 60 ms, 15 MB, 5 MB, 200 lines) are printed and
  enforced as hard failures only when they are exceeded *and* the baseline also sits near the limit.
  `--version`, `--help`, binary size and RSS are machine-stable enough to gate directly.
* **Cold networking is measured, not gated.** A 1.5 s cold budget depends on the link, the provider and
  the city; gating it would produce a flaky job that blocks unrelated work. `scripts/bench/cold.sh`
  records it in the baseline with the link description (`tc`-shaped 100 Mbps, documented), and the
  release checklist in backlog B02 re-runs it.
* **`panic = "abort"` is compatible with the error policy.** Every user-triggered failure is a typed
  `Error` and no code path unwinds; abort only removes the landing pads, not behaviour. Tests keep
  unwinding because the profile applies to `--release` only.
* **`fmt::Write` and one buffer.** The hot path for `-f plain` is ~30 lines of at most 80 columns;
  the win is not the byte count but removing ~200 short-lived `String` allocations per run, which is
  also where the RSS target is easiest to lose. Snapshot tests already exist for these renderers, so
  the refactor is behaviour-guarded.
* **Timezone hoisting changes `RenderContext`, not the contract's spirit.** One `chrono_tz` lookup and
  one `DateTime<FixedOffset>` per (day, day-part) replaces one per rendered field; the README contract
  line for `RenderContext` is amended in the same commit to list `LocalTimes`.
* **Lazy offline table is conditional on measurement.** If step 18 measured eager decode as
  negligible, the deliverable degrades to "recorded as measured, no change" — but the `offline-geo`
  feature split still lands, because the size argument is independent of timing.
* **No async runtime, no TUI toolkit: decision kept.** Evidence recorded in `docs/performance.md`:
  process startup (the `--version` number, which is a proxy for runtime-init cost), the size share of
  the sync HTTP stack (`ureq`+`rustls`: sync code is not smaller by itself, but `tokio` would add
  200 KB+ and a second thread pool), and the fact that a run performs 1–3 dependent requests, where
  async latency hiding buys nothing. The TUI question is settled by the repo's non-goals (no
  interactive screen) and by RSS: a TUI toolkit would add render-loop machinery to a process whose
  entire job is one screen of text.
* **Rejected measurement tooling:** `criterion` benches (they measure functions, not the process
  budgets that are actually promised; hyperfine covers the CLI-level claim), `cargo-benchcmp`
  (unmaintained), wall-clock asserts inside `cargo test` (flaky and load-dependent).

### Measured inputs from step 18 (2026-10-04)

The offline city database is the largest single asset the binary carries, so its numbers are recorded
here as the budget's input rather than discovered later. Machine: the maintainer's CachyOS box,
`x86_64`, release profile as shipped in `Cargo.toml` (`lto = "thin"`, `strip = true`), rustc stable.
Re-recorded on 2026-10-04 after the snapshot was refreshed to the official 2026-10-04 dump (see
step 18's progress log).

| Measurement | Value |
|---|---|
| `src/geo/data/cities.bin.gz` | 878 055 B |
| `src/geo/data/keys.bin.gz` | 2 558 155 B |
| embedded total | 3 436 210 B ≈ **3.28 MiB** (the plan projected 2.55 MiB; the rows carry display *and* ascii name plus the geonameid, and the snapshot is the 2026-10-04 dump: 34 152 rows, 310 502 keys) |
| release binary before step 18 | 12 270 752 B |
| release binary after step 18 | 15 928 616 B (+3 657 864 B ≈ +3.49 MiB: the data plus ≈220 KB of decoder/`unicode-normalization` code) |
| release binary after step 18b | 16 047 296 B (+118 KB over step 18 for the shared table codec, the update command and its ZIP reader; the members are embedded once — a `const` holding `include_bytes!` was duplicated across codegen units and cost 3.4 MB until it became a `static`) |
| `--version`, median of 5 | **1.6 ms** (budget: 20 ms) |
| `--help`, median of 3 | 2.0 ms |
| `location search --offline Beijing`, median of 5 | **52 ms** (index + row decode, ranking; the step-18 contract is "well under a second") |
| `location search --offline --all Springfield`, median of 5 | 51 ms |
| RSS, `--version` / an offline search | ≈14 MB / ≈25 MB (`ru_maxrss` of the child, coarse; the step-18 decode holds the decompressed members transiently) |

Two consequences for this step: the 5 MB default-binary budget is already exceeded by the pre-step-18
build (12.3 MB) and is not reachable by turning the table off (that saves 3.6 MB of 15.9 MB), so the
budget itself needs re-deriving from the measured baseline; and if the table must go, step 18's
default is a one-line flip (`default = []` in `Cargo.toml` plus the README/CHANGELOG notes it
promised). The lazy-decode deliverable here is already satisfied by step 18's `LazyLock` + the
`tests/offline_lazy.rs` assertion, so this step re-measures rather than re-implements it.

## Out of scope

Cross-platform performance parity (the Windows and macOS numbers are reported in backlog B02's release
archives verification, not gated here), musl static size (backlog B02 sets and measures its own budget),
profile-guided optimisation and `-Zbuild-std` (nightly-only), and any change to caching semantics or
provider request counts (steps 05 and 10 own those).

## Verification

```bash
scripts/bench/run.sh
```

```bash
cat target/bench/raw.json | python3 -c 'import json,sys; d=json.load(sys.stdin); print({k: round(v["median"],3) for k,v in d.items()})'
python3 scripts/bench/compare.py perf/baseline.json target/bench/raw.json    # exit 0, "no regression"
cargo bloat --release --crates -n 10
cargo llvm-lines --release | head -n 10
stat -c '%s bytes' target/release/cirrocast                # < 5242880
cargo run -q -- --help | wc -l                             # < 200
/usr/bin/time -v cargo run -q -- --offline Beijing -f plain 2>&1 | grep 'Maximum resident'
scripts/bench/cold.sh                                      # manual, documents the link used
```

Observable result: the compare tool prints the table and exits 0 with every gated metric inside its
budget; after a deliberately injected regression (a 300 ms sleep in the `--offline` path) the same
command exits 1 and names the metric — verified once by hand and reverted, with the transcript pasted
into `docs/performance.md`.

## Exit criteria

- ⬜ `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`, `reuse lint` clean.
- ⬜ Budget table in `docs/performance.md` filled with real numbers from the reference machine:
      `--version` < 20 ms, cached run < 60 ms, cold run < 1.5 s (with link), RSS < 15 MB, binary < 5 MB,
      `--help` < 200 lines.
- ⬜ `perf/baseline.json` committed and consumed by the CI `perf` job; the artificial-regression proof
      is recorded in the doc and in the progress log.
- ⬜ `cargo bloat`/`cargo llvm-lines` top contributors named with sizes, every heavy crate accepted or
      gated with a one-line reason.
- ⬜ The `offline-geo`-less build compiles, passes `cargo test`, and its size delta is recorded.
- ⬜ Re-evaluation of no-async/no-TUI recorded with the measured evidence, decision unchanged.

## Risks

* CI runner variance: mitigated by the 20 % allowance, by `-N` (no shell), by fixed core count via
  `taskset -c 0` for the timing runs, and by re-recording the baseline only on an explicit
  `workflow_dispatch` with the new machine spec committed alongside.
* `/usr/bin/time -v` is a GNU time feature; on runners without it the script falls back to
  `getrusage`-based `ps -o rss= -p` sampling with the difference documented — the fallback measures
  peak *sampled* RSS, which is a lower bound.
* `strip = "symbols"` breaks `cargo bloat` symbol attribution; the audit therefore uses a
  `--profile release-audit` (inherits release, `strip = "none"`) build, documented in the doc and the
  script.
* Size creep is easy to reintroduce: the CI job fails on the 5 MB budget rather than on a trend, and
  the dependency audit's gate list is the place new crates must be argued for.

## Progress log

- 2026-09-30 — step opened: budget table, harness layout, baseline schema and the ratio-based gate
  fixed; measurement tooling chosen (hyperfine + `/usr/bin/time -v`) and `criterion` rejected.
- 2026-10-04 — renumbered from 22 to 21 by the plan reorganization (backlog split: serve → B01,
  packaging matrix → B02); depends on 18 added because the lazy-decode deliverable needs the offline
  table, the feature name unified to step 18's `offline-geo`, and the release-archive/musl references
  now point at backlog B02.
