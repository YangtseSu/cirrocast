<!--
SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Contributing to `cirrocast`

`cirrocast` is a terminal weather client written in Rust: a from-scratch replacement for `wego` that
uses `wttr.in` as the *output* reference and no code from either. Thanks for helping; the rules below
keep a small, single-maintainer project reviewable.

Read these two files before touching anything:

* [`AGENTS.md`](AGENTS.md) — the operating rules: golden rules, commands, testing policy, commit
  conventions and dependency policy. This guide summarises the parts a contributor needs and points
  back to it; when they disagree, `AGENTS.md` wins for process.
* [`docs/plans/README.md`](docs/plans/README.md) — the binding architecture contract and the
  step-by-step build plan. When the contract and `AGENTS.md` disagree, the contract wins for
  interfaces.

## Prerequisites

The project tracks the **stable** Rust toolchain and supports no floor below it.
[`rust-toolchain.toml`](rust-toolchain.toml) names the channel and the two components the gates need,
and `rustup` honours it automatically inside this checkout:

```toml
[toolchain]
channel = "stable"
components = ["rustfmt", "clippy"]
```

`rust-version` in `Cargo.toml` names the stable the crate is built with (`1.99` at `v1.3.x`) and is
bumped together with the toolchain; there is no separate MSRV job. You also need `git`, and for the
gates that no compiler enforces:

| Tool | Used for |
|---|---|
| [`reuse`](https://github.com/fsfe/reuse-tool) | the licence/SPDX gate (`reuse lint`) |
| [`cargo-deny`](https://github.com/EmbarkStudios/cargo-deny) | the licence, advisory, ban and source gate (`cargo deny check`) |
| [`cargo-audit`](https://github.com/rustsec/rustsec) | the independent advisory check (`cargo audit`) |
| Python 3 | `scripts/check-render-imports.py`, the render-layer invariant |

## Build and run

```bash
cargo run -q -- Beijing                 # run the CLI
cargo run -q -- Beijing Shanghai Tokyo -f one-line   # several locations: argument order, up to four at a time
cargo run -q -- @home --template '%l %c%t'           # a [locations] alias and a literal template
cargo fmt                               # required before every commit
cargo clippy --workspace --all-targets --locked -- -D warnings   # includes build/geo-table
cargo test --workspace --locked         # unit + integration, no network
CIRROCAST_FORBID_NETWORK=1 cargo test --workspace   # what CI runs: any socket attempt fails loudly
cargo test -- --ignored                 # live smoke tests, manual only (CIRROCAST_LIVE_TESTS=1)
reuse lint                              # licence/SPDX gate
cargo deny check                        # licence, advisory, ban and source gate
cargo audit                             # independent advisory check beside cargo deny
```

A change is not done until `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
`cargo test --workspace` and `reuse lint` are clean **and** the changed surface was actually run and
observed (for a CLI change, that means executing `cirrocast` and looking at the output, not only
tests). The `--workspace` flag matters: the root manifest is a package, so a bare `cargo clippy` or
`cargo test` in the repository root covers only `cirrocast` itself, not the dev-only `build/geo-table`
builder.

## Test policy

* Unit tests live next to the code; integration tests (CLI behaviour through `assert_cmd`) in
  `tests/`. HTTP is exercised through `StubTransport`, and upstream payloads come from
  `tests/fixtures/`.
* **No test may open a network connection.** CI exports `CIRROCAST_FORBID_NETWORK=1` for the whole
  test job, which makes `src/http.rs` refuse every non-loopback request before DNS or connect, so an
  accidentally network-dependent test fails loudly instead of passing quietly on a connected machine.
  A test that genuinely needs the network must be `#[ignore]`d with a comment naming the environment
  variable that enables it (`CIRROCAST_LIVE_TESTS=1 cargo test -- --ignored`, live smoke tests only).
* **Snapshot tests are reviewed by eye.** Snapshots (`insta`) cover renderer output; read the diff
  before accepting it. `INSTA_UPDATE=always` is never used in CI. The long `--help` snapshot and the
  man page change only in the commit that changes a flag, so their diffs are reviewed in the same
  view.
* Assert behaviour, boundaries, error taxonomy and invariants — not incidental formatting of
  internals. A new permanent test must be able to catch a realistic consumer-visible regression.
* Every bug report gets a failing test first, then the fix; keep the test unless the bug class is
  already covered elsewhere.

## Plan-driven workflow

Work follows the plan set, not a free-form backlog:

1. Pick the lowest-numbered step in [`docs/plans/README.md`](docs/plans/README.md) that is
   `⬜ not-started` (or continue a `🚧 in-progress` one). Steps run in order; do not start a step whose
   `Depends on` entries are open. `⏸ backlog` items (`B01`, `B02`, …) are skipped unless the pull is
   recorded as described in that file.
2. Work item by item through the step's `## Deliverables`; one deliverable is one commit.
3. In the same commit as the code, flip the item to `- ✅` and append a dated line to the step's
   `## Progress log`. Never rewrite history in the log.
4. When every deliverable and exit-criterion item is `- ✅`, set `Status: ✅ done` and update the step
   table in `docs/plans/README.md`. A step is not done while any `- ⬜` item remains.
5. If reality diverges from the plan, fix the plan in the same commit and say why in the log line.
   Interfaces change → `docs/plans/README.md` first.

Progress markers are emoji (`⬜ not-started` / `🚧 in-progress` / `⛔ blocked` / `✅ done` /
`⏸ backlog`, tasks `- ⬜` / `- ✅`) so a step file's state shows at a glance.

## Recipes

**Add a provider** (one step, one commit series): new `ProviderId` variant + registry metadata row in
`src/provider/mod.rs`; `src/provider/<id>.rs` implementing `Provider` (native codes → `Condition`,
day-parts aggregated in the location timezone); recorded fixtures under `tests/fixtures/<id>/`;
tests for decoding, unit invariance and error mapping; `provider info <id>` output verified; README
provider matrix row. No new CLI flag — backends are selected with `--provider`.

**Add a renderer/format**: implement `Renderer` in `src/render/<name>.rs`, register it in the format
enum + `--help`, add a snapshot test (`insta`) for at least metric/us, 1/3/7 days and a narrow width,
and document the format in the README with an example block.

**Use a location alias or a named template**: add a key to `[locations]` (`home = "@39.9,116.4"`,
including another alias — chains are cycle-checked at load) or `[templates]` (`compact = "%c%t"`) in
`config.toml`; then `cirrocast @home` / `cirrocast -f compact` / `--template @compact`. The engine,
the exported `TOKENS` table and the width/precision rules live in `src/template.rs`; adding a token
means a `TokenSpec` row, a `value` arm, a test row in `tests/templates.rs` (the count is asserted
against the table) and the `--help` epilogue in `src/cli.rs`.

**Add a language**: drop `locales/<tag>/main.ftl` (copy `en-US`), translate every key, add the
`("<tag>", include_str!("../locales/<tag>/main.ftl"))` line to `CATALOGS` in `src/i18n.rs` (the
`include_str!` embedding is what requires that one line), run `cargo test i18n` — the completeness
test fails if any condition key, renderer key or `en-US` key is missing. Nothing else changes.

**Change the config schema**: bump `schema_version`, add the migration arm in `src/config/mod.rs`,
update the schema block in `docs/plans/README.md`, add a migration test from the previous version.

## Licensing and REUSE

`cirrocast` is GPL-3.0-or-later and REUSE-compliant. **Every new file carries SPDX file-copyright and
licence tags as its first lines**, in that file's comment syntax (Rust `//`, Markdown HTML comment,
TOML/YAML `#`):

<!-- REUSE-IgnoreStart -->

```
SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
SPDX-License-Identifier: GPL-3.0-or-later
```

<!-- REUSE-IgnoreEnd -->

Files that cannot hold comments (`Cargo.lock`, JSON fixtures, binaries) are declared as
`[[annotations]]` in [`REUSE.toml`](REUSE.toml). `reuse lint` must pass; never add a file without
licensing information. Third-party material that is genuinely reused keeps its upstream licence and
is listed in `REUSE.toml` — and nothing may be copied from `wego` or `wttr.in`, not code, not art,
not data tables, not docs.

## Commit and branch conventions

* Conventional Commits, English, imperative: `feat(provider): add open-meteo hourly decoding`,
  `fix(cache): honour ttl on offline reads`, `docs(plans): tick step 05 deliverables`.
* One logical change per commit; no WIP commits on `main`; no GPT/agent attribution trailers.
* Branch names: `<step>-<slug>` (e.g. `06-open-meteo-provider`). `main` stays green — CI must pass
  before pushing.
* Never commit secrets, `keys.toml`, editor configs, or generated caches.

## Dependency policy

Prefer std and crates already in `Cargo.toml`. A new dependency needs, in the step doc's design
notes: what it replaces, its licence (must be GPL-3.0-or-later compatible), whether it pulls TLS or
an async runtime, and its MSRV. Heavyweight frameworks (tokio, reqwest, any TUI toolkit) are
rejected unless the contract is amended first. [`deny.toml`](deny.toml) enforces the machine-checkable
half: an audited licence allow list, duplicate versions and wildcard requirements denied, crates.io
as the only source, and no advisory ignore without a reason and an expiry date.

## Documentation rules

Every fact has exactly one authoritative home, and a behaviour change lands with its documentation in
the same commit — the docs are not written once and abandoned.

* `docs/configuration.md` owns config keys, types, defaults and precedence.
* `docs/providers.md` owns per-provider rate limits, quotas and attribution duties.
* `docs/formats.md` owns the `%` token table (cross-checked against `template::TOKENS`).
* `docs/schema.md` owns the JSON/config schema and the key index.
* `docs/ecosystem.md` owns the status probe and the frozen output contracts.
* `docs/getting-started.md`, `docs/location.md`, `docs/i18n.md`, `docs/troubleshooting.md` and
  `docs/architecture.md` own install/first-run, location syntax, localisation, failure diagnosis and
  the module map respectively.

Machine-readable artifacts cannot drift from the binary: `docs/reference/help-long.txt` is compared
byte-for-byte with `--help` by a test, `docs/reference/flags.txt` is set-compared with the flags in
`--help`, and `docs/schema/json-v2.json` validates live `-f json` output. The two generated
documents are regenerated on purpose — never by the normal test run — and the diff is the review:

```bash
cargo test --test help_snapshot -- --ignored regenerate_help   # docs/reference/help-long.txt
cargo test --test help_snapshot -- --ignored regenerate_man    # man/cirrocast.1
```

**`docs/schema/json-v1.json` is frozen.** It records what `v1.0.0` printed and is the compatibility
promise for existing consumers: once released, it must never be edited, corrected or regenerated.
A schema change means a new file (`json-vN.json`) and a `schema_version` bump, never an edit to v1.

## Releasing

Release steps, the signed tag and the AUR bump live in
[README.md → Release checklist](README.md#release-checklist) — it is the one authoritative copy.
Before a tag, the bundled city and country layers are verified locally against the official dumps
(the exact commands are in [`AGENTS.md`](AGENTS.md)); a tag is never cut against a stale snapshot.
