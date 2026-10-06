<!--
SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Step 21 — performance and resource budget

Status: ✅ done
Depends on: 12 (quality hardening and CI), 18 (offline city database), 19 (multi-location and templates)
Touches: `scripts/bench/{run.sh,cold.sh,compare.py,record.py}`, `perf/baseline.json`,
`.github/workflows/ci.yml`, `Cargo.toml`, `src/render/{mod,art_table,one_line,plain,color}.rs`,
`src/model/mod.rs`, `src/template.rs`, `src/geo/offline.rs`, `src/cli.rs`, `tests/{cli,common}.rs`
and the render/astro/air/template test files, `docs/performance.md`, `REUSE.toml`

## Goal

The informal budgets become enforced numbers with one measurement harness, one committed baseline and
a CI gate: `--version` under 20 ms, a cached single-location run under 60 ms, a cold run under 1.5 s on
a 100 Mbps link, RSS under 15 MB (the `--version` probe; a cached run that decodes the offline name
index gets the re-derived 32 MB budget), the default release binary under the re-derived 17 MB (the
original 5 MB predates the 3.41 MB embedded city table), `--help` under 225 lines (200 until
step 23 added `--date`, `--history` and `--marine`, 215 until step 26 added `--normals`). The
step also does the rendering work the numbers demand (`fmt::Write` hot path, one timezone lookup per
report instead of one per field, lazy offline-geo decode), audits dependency weight with `cargo
bloat`/`cargo llvm-lines`, and re-evaluates "no async runtime / no TUI toolkit" against the measured
evidence — keeping the decision, recording why.

## Deliverables

- ✅ `Cargo.toml`: `[profile.release] strip = "symbols"`, `lto = "thin"`, `codegen-units = 1`,
      `panic = "abort"`; the step-18 `offline-geo` feature stays **default-on**, with the
      `--no-default-features` proof landing here, so `cargo build --release --no-default-features`
      yields the reduced build (same CLI, city search via the network geocoder only). Feature
      inventory documented in `docs/performance.md`.
- ✅ `scripts/bench/run.sh`: `cargo build --release --locked`, then hyperfine
      (`hyperfine --warmup 5 --runs 30 -N`) over `--version`, `--help`,
      `--offline Beijing -f plain` and a warm-cache `Beijing -f plain`, with `XDG_CACHE_HOME`,
      `XDG_CONFIG_HOME` and `TZ` pinned to a temp tree seeded from `tests/fixtures/`; RSS via
      `/usr/bin/time -v` (`Maximum resident set size`), binary size via `stat -c %s`, `--help` lines
      via `wc -l`; everything written to `target/bench/raw.json`.
- ✅ `scripts/bench/cold.sh`: manual cold-run harness — empty cache dir, `--refresh`, three timed
      runs, prints median; run by hand on the documented reference machine and link, never in CI
      (a network number cannot be gated).
- ✅ `scripts/bench/compare.py`: Python 3 (stdlib only) reader of `perf/baseline.json` and
      `target/bench/raw.json`; prints a metric-by-metric table with budget, baseline and delta; exits
      1 when a gated median exceeds `baseline × 1.20` or a hard budget.
- ✅ `perf/baseline.json`: `{"schema": 1, "recorded": "YYYY-MM-DD", "commit": …, "machine": {…},
      "metrics": {…}}` with medians for every gated metric plus the cold-run figure and its conditions.
- ✅ `.github/workflows/perf.yml`: the `budget` job (release build, `scripts/bench/run.sh`,
      `scripts/bench/compare.py`, `cargo bloat --profile release-audit --crates -n 20` and
      `cargo llvm-lines --release --lib -p cirrocast | head -n 20` uploaded as artifacts),
      dispatch-only, pinned to the same image class as the baseline was recorded on, with the
      `record_baseline` input as the only way to write a baseline.
- ✅ `docs/performance.md`: machine specification (`lscpu` summary, kernel, rustc/cargo/hyperfine
      versions, commit), methodology (commands, cache priming, RSS and size measurement, why the cold
      run is not gated), the budget table with the measured numbers, the dependency-weight audit, and
      the async/TUI re-evaluation with evidence.
- ✅ `src/render/{mod,art_table,one_line,plain}.rs`: `fmt::Write` rendering into one pre-sized
      `String` (`with_capacity(4096)`), no per-cell `format!`, no intermediate `Vec<String>` of lines
      or per-line `String`; colour codes written as escapes into the same buffer; `Renderer::render`
      keeps its signature.
- ✅ `src/model/mod.rs` + `src/cli.rs`: `LocalTimes` computed once per report (`chrono_tz` lookup,
      day-part `DateTime<FixedOffset>`, preformatted clock strings), carried in `RenderContext`;
      renderers and the template engine stop converting per field.
- ✅ `src/geo/offline.rs`: lazy table decode (header first, `OnceLock` per shard) when step 18
      measured an eager startup decode; `--version`/`--help` and any run that never performs a city
      lookup must not touch the table at all (asserted by a test that removes the data file and runs
      `--version`).
- ✅ Dependency-weight audit in `docs/performance.md`: the top contributors from `cargo bloat
      --crates` with sizes, and a decision per heavy crate — `chrono-tz` (accepted: timezone-correct
      day parts are a contract requirement), `rustls`/`ring` or `aws-lc-rs` (accepted: TLS is not
      optional), `clap_builder` (accepted; `wrap_help` reviewed), `serde_json` (accepted),
      `flate2` (accepted, feature-reviewed), `idna`/`url` (accepted via `ureq`), plus the gate list
      (`keyring`, `offline-geo`, `ureq` compression features, `clap` `wrap_help`) and the measured
      delta of turning each off.
- ✅ `tests/cli.rs`: `--help` line count < 225 and `--help` exit 0 asserted deterministically.

## Design notes

* **Gate on ratios, report absolute budgets.** Shared CI runners fluctuate by more than the 20 %
  tolerance in absolute terms, so the gate compares fresh medians against the committed baseline with
  a 20 % allowance; the absolute budgets (20 ms, 60 ms, 15 MB, 5 MB, 225 lines) are printed and
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

### Measured inputs from step 18 (2026-10-04, re-recorded 2026-10-06)

The offline city database is the largest single asset the binary carries, so its numbers are recorded
here as the budget's input rather than discovered later. Machine: the maintainer's CachyOS box,
`x86_64`, release profile as shipped in `Cargo.toml` (`lto = "thin"`, `strip = true`), rustc stable.
Re-recorded on 2026-10-06 after the snapshot was refreshed to the official 2026-10-05 dump for the
`1.3.0` release (see step 13's progress log); rows that name a step keep that step's measurement.

| Measurement | Value |
|---|---|
| `src/geo/data/cities.bin.gz` | 878 049 B |
| `src/geo/data/keys.bin.gz` | 2 558 455 B |
| embedded total | 3 436 504 B ≈ **3.28 MiB** (the plan projected 2.55 MiB; the rows carry display *and* ascii name plus the geonameid, and the snapshot is the 2026-10-05 dump: 34 153 rows, 310 536 keys) |
| release binary before step 18 | 12 270 752 B |
| release binary after step 18 | 15 928 616 B (+3 657 864 B ≈ +3.49 MiB: the data plus ≈220 KB of decoder/`unicode-normalization` code) |
| release binary after step 18b | 16 047 296 B (+118 KB over step 18 for the shared table codec, the update command and its ZIP reader; the members are embedded once — a `const` holding `include_bytes!` was duplicated across codegen units and cost 3.4 MB until it became a `static`) |
| release binary at `v1.3.0` | 16 191 744 B (the 2026-10-05 refresh plus steps 19–28: the template engine, the extra backends, alerts, normals and the panel renderers on top of the 18b build) |
| `--version`, median of 5 | **1.5 ms** (budget: 20 ms) |
| `--help`, median of 3 | 1.8 ms |
| `location search --offline Beijing`, median of 5 | **52 ms** (index + row decode, ranking; the step-18 contract is "well under a second") |
| `location search --offline --all Springfield`, median of 5 | 60 ms |
| RSS, `--version` / an offline search | ≈12 MB / ≈25 MB (`ru_maxrss` of the child, coarse; the step-18 decode holds the decompressed members transiently) |

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
cargo bloat --profile release-audit --crates -n 10   # the audit profile resolves more of the tail than the stripped `release`
cargo llvm-lines --release --lib -p cirrocast | head -n 10   # the workspace needs a single target
stat -c '%s bytes' target/release/cirrocast                # < 5242880
cargo run -q -- --help | wc -l                             # < 225
/usr/bin/time -v cargo run -q -- --offline Beijing -f plain 2>&1 | grep 'Maximum resident'
scripts/bench/cold.sh                                      # manual, documents the link used
```

Observable result: the compare tool prints the table and exits 0 with every gated metric inside its
budget; after a deliberately injected regression (a 300 ms sleep in the `--offline` path) the same
command exits 1 and names the metric — verified once by hand and reverted, with the transcript pasted
into `docs/performance.md`.

## Exit criteria

- ✅ `cargo fmt --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`,
      `cargo test --workspace --locked`, `reuse lint` clean.
- ✅ Budget table in `docs/performance.md` filled with real numbers from the reference machine:
      `--version` 2.10 ms (< 20 ms), cached run 51.4 ms (< 60 ms), cold run 845 ms (< 1.5 s, with
      link), RSS 6.4 MiB for `--version` (< 15 MiB) and 25.6 MiB for the cached run (< 32 MiB,
      re-derived), binary 14.36 MiB (< 17 MiB, re-derived; the goal's 5 MiB predates the 3.41 MiB
      offline table), `--help` 198 lines (< 200 then; the budget is 225 since step 23 added
      three flags and step 26 `--normals`).
- ✅ `perf/baseline.json` committed and consumed by the dispatch-only `perf.yml` gate; the artificial-regression proof
      is recorded in the doc and in the progress log.
- ✅ `cargo bloat`/`cargo llvm-lines` top contributors named with sizes, every heavy crate accepted or
      gated with a one-line reason.
- ✅ The `offline-geo`-less build compiles, passes `cargo test`, and its size delta is recorded.
- ✅ Re-evaluation of no-async/no-TUI recorded with the measured evidence, decision unchanged.

## Risks

* CI runner variance: mitigated by the 20 % allowance, by `-N` (no shell), by fixed core count via
  `taskset -c 0` for the timing runs, and by re-recording the baseline only on an explicit
  `workflow_dispatch` with the new machine spec committed alongside.
* `/usr/bin/time -v` is a GNU time feature; on machines without it (the reference machine
  included) the harness falls back to hyperfine's per-child peak, recorded in the baseline as
  `machine.rss_method`. `ps -o rss= -p` sampling was rejected (a 50 ms run is too short to sample),
  and `getrusage(RUSAGE_CHILDREN)` read from a forked parent was measured and rejected too: it
  counts the parent's resident pages and overstated `--version` by ~6 MiB.
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
- 2026-10-05 — the render-buffer rewrite and the `LocalTimes` hoist landed in **one** commit, a
  deviation from "one deliverable = one commit": the hoist rewrites the same functions the rewrite
  moves (day headings, the metrics, the template tokens), and splitting them would have meant
  reconstructing an intermediate tree by hand. `color::write_paint`/`write_paint_severity` were
  added beside `paint` (outside the step's `Touches` list, but the only place the escape-writing
  twin belongs); `RenderContext` now carries `times: LocalTimes` and is `Clone` rather than `Copy`
  (its two `Copy` users — `panel_context` and the CLI's slot assembly — clone once per render and
  per location). Behaviour verified beyond the suite by diffing the old and new release binaries
  over 21 format/width/colour/alert/multi-location cases: byte-identical output.
- 2026-10-05 — release profile tuned and the feature switch proven: `--no-default-features` builds
  (11,483,928 B vs 15,057,960 B default: the `offline-geo` table and decoder are 3,574,032 B) and
  passes `cargo test --workspace --no-default-features`. `release-audit` added for `cargo bloat`.
  Feature inventory in `docs/performance.md`; the measured dependency switches there are
  `clap`'s `wrap_help` (−936 B, kept) and `ureq`'s default features (−12,496 B, kept).
- 2026-10-05 — the harness landed: `scripts/bench/run.sh` (release build, pinned XDG sandbox with
  a cache seeded from `tests/fixtures/open_meteo/forecast_beijing_2026-07-15.json`, alerts off,
  `CIRROCAST_FORBID_NETWORK=1`, `taskset -c 0`, hyperfine `--warmup 5 --runs 30 -N`, RSS, binary
  size and `--help` length into `target/bench/raw.json`), `cold.sh` (manual, three `--refresh` runs
  on an empty cache) and `compare.py` (ratio gate at 1.20 plus the hard budgets, with the
  "baseline already over budget" warning path). A fourth script, `record.py`, composes
  `perf/baseline.json` from a fresh `raw.json` plus the machine spec, so the local and CI
  re-recording paths cannot drift. `rss.py` was written, tried and deleted: reading
  `getrusage(RUSAGE_CHILDREN)` from a forked parent counts the parent's pages and overstated
  `--version` by ~6 MiB, so the fallback is hyperfine's per-child peak instead.
- 2026-10-05 — `perf/baseline.json` recorded on the reference machine (commit `c0f1b5b`):
  `--version` 2.10 ms, `--help` 2.34 ms / 198 lines, cached `--offline` run 51.4 ms, warm-cache run
  47.5 ms, RSS 6,604 KiB (`--version`) and 26,228 KiB (cached run), binary 15,057,960 B, cold run
  845 ms. The 5 MiB binary budget and the 15 MiB RSS budget are re-derived in the doc: the offline
  table makes both unreachable by design (3.41 MiB embedded, ~20 MiB decoded index).
- 2026-10-05 — CI: a `perf` job (release build, `run.sh`, `compare.py`, `cargo bloat
  --crates`/`cargo llvm-lines --lib` into the artifact bundle) on `ubuntu-26.04`, and a
  `record-baseline` job behind the new `workflow_dispatch` input `record_baseline` that measures
  and uploads a fresh `perf/baseline.json` (the cold figure carries over, since it is manual by
  design). `upload-artifact` is pinned to `330a01c4…` (v5.0.0, resolved with `git ls-remote`);
  the three measurement tools are installed with `cargo install --locked`, so no new third-party
  action enters the workflow.
- 2026-10-05 — `docs/performance.md` completed: reference machine, methodology (why the cold run
  is not gated, why alerts are off in the harness, how RSS is measured and which two fallbacks were
  rejected), the budget table with the re-derived binary/RSS numbers, the dependency-weight audit
  (`cargo bloat --crates` and `cargo llvm-lines --lib`) and the async/TUI re-evaluation. The
  lazy-decode deliverable is re-measured, not re-implemented: step 18's `OnceLock` decode plus
  `tests/offline_lazy.rs` already keep `--version` away from the table (6.4 MiB RSS, 2.10 ms), so
  the entry records the measurement and changes nothing. The gate's artificial-regression proof
  (a 300 ms sleep on the `--offline` path → exit 1 naming `offline_plain_ms`, then reverted) is
  transcribed in the doc.
- 2026-10-05 — `--help` was 200 lines, exactly the budget's edge, so the epilogue was compacted
  (the syntax paragraph to two lines, the six preset rows to four, both with the same content) to
  198, and `tests/cli.rs` now asserts the count with `COLUMNS=100` pinned — clap wraps the epilogue
  to the terminal width, so an inherited `COLUMNS` would make the test depend on the developer's
  shell.
- 2026-10-05 — step closed. Plan corrections made in this commit, per the "fix the plan" rule: the
  exit criterion's binary budget is re-derived (17 MiB, with the measurement and the reason in
  `docs/performance.md`), the verification block names `--profile release-audit` for `cargo bloat`
  (the shipped profile strips symbols) and `--lib -p cirrocast` for `cargo llvm-lines` (the
  workspace needs one target), and the risks section records the RSS fallback that was actually
  used (hyperfine's per-child peak) instead of the `ps` sampling that was rejected as too coarse.
  Full gate re-run on the final tree: `cargo fmt --check`, `cargo clippy --workspace --all-targets
  --locked -- -D warnings`, `cargo test --workspace --locked`, `reuse lint` — all clean, and the
  harness/gate run prints `no regression` with exit 0.
- 2026-10-06 — CI trimmed on the maintainer's instruction; recorded here because this step owns the
  budget's enforcement path. `ci.yml` loses the `perf` and `record-baseline` jobs and the
  `workflow_dispatch` input — a budget run belongs to a release preparation or a dependency change,
  not to every push — and regains them as the single `budget` job of the new dispatch-only
  `.github/workflows/perf.yml`, `record_baseline` input included; the dependency-weight tools
  (`cargo bloat`, `cargo llvm-lines`) stay in that workflow's measurement mode, where they are paid
  for only when someone asks for the numbers. The render-layer scan moved out of the workflow's
  heredoc into `scripts/check-render-imports.py` (same token-aware logic, now runnable locally as
  the one command CI runs); the `gates` job keeps it and the LICENSE `cmp`. The
  `--no-default-features` proof is now the second command of the single `cargo test` job (the
  separate build job was redundant: `cargo test` builds first) and the test matrix collapsed to
  `ubuntu-26.04` on stable — the project tracks stable, `rust-version` moves with the toolchain
  (now `1.99`) and carries no floor below it, and macOS is covered where it ships, by `release.yml`'s
  `cargo test --release` on `macos-26`. Per-push runner allocations: 16 → 8.
- 2026-10-06 — first manual dispatch of `perf.yml` on pushed `main`, and it did what the docs say a
  baseline must not do: the compare step failed because the committed baseline was the dev box's and
  the runner measured some 15 % slower that day — `--version` 3 ms (+26.5 %), `--help` 3 ms
  (+20.1 %), `--offline` 57 ms (+10.3 %), warm-cache 56 ms (+19.1 %), RSS 6 % *lower*. Both failures
  were one millisecond of timer resolution, not a regression, so `scripts/bench/compare.py` now
  requires an absolute floor beside the ratio (`FLOORS`: 5 ms timing, 1 MiB RSS, 0.5 MiB binary,
  5 lines `--help`; the docstring and this doc's gate section explain why), and the synthetic cases
  (runner-like spread → ok, +7 ms startup → FAIL, +21 % offline → FAIL) were run before the fix was
  committed.
- 2026-10-06 — the baseline was then re-recorded from the runner class (`record_baseline` dispatch,
  `perf/baseline.json` at the commit above): `--version` 2.11 ms, `--help` 2.31 ms / 199 lines,
  `--offline` 48.6 ms, warm-cache 48.4 ms, RSS 6,316 / 24,364 KiB, binary 15,018,608 B, machine
  `AMD EPYC 9V74` / `ubuntu-26.04` / hyperfine 1.19.0 / `gnu-time` RSS. The dev-box figures stay in
  `docs/performance.md` as the recording the budgets were re-derived from, and the doc now documents
  both reference machines.
- 2026-10-06 — the gate's rule settled on the evidence above: the four timing metrics are judged by
  their hard budgets alone (`compare.py`'s `BUDGET_ONLY`), because the host spread of one commit on
  one image class (±18 % `--offline`, ±30 % `--version`) is the size of the ratio allowance itself —
  a ratio there reports the host, not the change. The size/RSS/count metrics keep the 20 % ratio plus
  their floors, and a timing ratio over the allowance prints a warning without failing the run.
  Synthetic cases were run before the commit: `--offline` +20.5 % (under the 60 ms promise) →
  warn/exit 0, 65 ms → `FAIL over budget`, `--version` 25 ms → `FAIL over budget`, RSS +25 % →
  `FAIL +25% vs baseline`. The dependency-weight audit moved behind a second dispatch input
  (`audit`): its `release-audit` build was 82 s of the 185 s run, and it belongs to a
  dependency-change session, not to a budget check.

- 2026-10-06 — the `--help` line budget was raised from 200 to 215 by step 23: `--date`,
  `--history` and `--marine` are three documented flags (8 lines with clap's spacing), and no
  amount of prose-trimming elsewhere fits them under the old ceiling without deleting a flag's
  documentation. The parser gate (`scripts/bench/compare.py`) and `tests/cli.rs` carry the new
  number; the measured line count is 210 at `COLUMNS=100`.

- 2026-10-06 — the budget was raised again, 215 → 225, by step 26: `--normals` is a documented flag
  (5 lines with clap's spacing) plus the two `CIRROCAST_NORMALS_*` names the epilogue lists, and the
  measured count at `COLUMNS=100` is 220. `tests/cli.rs`, `scripts/bench/compare.py` and
  `docs/performance.md` carry the new number, the same rationale as step 23's raise from 200.
