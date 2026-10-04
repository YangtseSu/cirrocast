<!--
SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
SPDX-License-Identifier: GPL-3.0-or-later
-->

# AGENTS.md — working rules for `cirrocast`

`cirrocast` is a terminal weather client written in Rust: a from-scratch replacement for `wego`,
using `wttr.in` as the *output* reference and no code from either. It ships pluggable weather
backends (keyless first, BYOK for commercial ones), geocoding by city name, optional IP-based
location, configurable units and output languages, and full XDG directory compliance.

Read [`docs/plans/README.md`](docs/plans/README.md) before touching code: it is the binding
architecture contract (module map, data model, provider/renderer/config/CLI interfaces). When the
contract and this file disagree, the contract wins for interfaces, this file wins for process.

## Non-goals (do not build these)

No GUI/TUI, no daemon or server mode, no telemetry or analytics, no account system, no scraping of
`wttr.in` (it is a reference for *layout*, never a data source), no async runtime, no `unsafe`.

## Repo map

| Path | Contents |
|---|---|
| `src/main.rs` | argv → `cli` → dispatch → exit code. Nothing else. |
| `src/cli.rs` | clap definitions, location argument parsing, subcommand dispatch |
| `src/error.rs` | `Error` + `Result` + exit-code mapping (1 generic, 2 usage, 3 network/upstream, 4 config, 5 location, 6 missing key) |
| `src/paths.rs` | XDG config/cache/data resolution via `etcetera` |
| `src/config/` | `config.toml` schema + load/merge/save, `keys.rs` BYOK store |
| `src/model/` | canonical WMO-condition/unit/metric-SI data model |
| `src/geo/` | geocoders (Open-Meteo, Nominatim), IP locators |
| `src/http.rs`, `src/cache.rs` | shared HTTP client and on-disk cache |
| `src/template.rs`, `src/parallel.rs` | the shared `%`-token template engine and the ordered parallel map behind multi-location runs |
| `src/provider/` | one file per backend + registry/chain selection |
| `src/render/` | `art-table` (default), `one-line`, `plain`, `json`, art blocks, colour |
| `locales/` | Fluent `.ftl` catalogs |
| `tests/` | integration tests; `tests/fixtures/` = recorded upstream responses |
| `docs/plans/` | step-by-step build plan with in-file progress markers |

## Golden rules

1. **English everywhere** — code, comments, docs, commit messages, branch names.
2. **GPL-3.0-or-later + REUSE compliance.** Every new file carries SPDX file-copyright and
   licence tags as its first lines, in that file's comment syntax (Rust `//`, Markdown HTML comment,
   TOML `#`):
   <!-- REUSE-IgnoreStart -->
   ```
   SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
   SPDX-License-Identifier: GPL-3.0-or-later
   ```
   <!-- REUSE-IgnoreEnd -->
   Files that cannot hold comments (`Cargo.lock`, JSON fixtures, binaries) are declared as
   `[[annotations]]` in `REUSE.toml`. `reuse lint` must pass; never add a file without licensing
   information.
3. **Never copy from `wego` or `wttr.in`** — not code, not art, not data tables, not docs. Icon art,
   colour palettes and condition text are re-authored here. Third-party material that is genuinely
   reused keeps its upstream licence and is listed in `REUSE.toml`.
4. **No stub deliverables.** Do not commit `todo!()`, `unimplemented!()`, placeholder output,
   no-op fallbacks or "will be wired later" code. A feature is committed when it works end to end.
   Scaffolding for a *later* step is declared in `docs/plans/`, not in `src/`.
5. **One conversion point.** All data is stored metric/SI (`temp_c`, `wind_kmh`, `precip_mm`,
   `pressure_hpa`, `visibility_km`); only `src/render/` converts for display. Providers must ask
   upstream APIs for metric.
6. **Canonical conditions.** Everything is a WMO 4677 code (`model::Condition`). Each provider owns
   the mapping from its native codes; renderers and translations never branch on provider codes.
7. **Synchronous HTTP only** (`ureq` + rustls) through `src/http.rs`. No direct socket/file access
   inside providers or geocoders — they receive the shared `Env` (client, cache, config).
8. **No `unwrap`/`expect`/`panic!`/slice-indexing that can fail** outside `#[cfg(test)]`. User-triggered
   failures must become typed `Error` values. `unsafe_code` is `forbid` at the crate level.
9. **XDG only.** Config `$XDG_CONFIG_HOME/cirrocast/`, cache `$XDG_CACHE_HOME/cirrocast/`, data
   `$XDG_DATA_HOME/cirrocast/`, resolved via `etcetera`. Never write into `$HOME` directly, never
   write into the source tree at runtime.
10. **Secrets.** API keys never enter `config.toml`, never appear in argv (read them from stdin or
    `keys.toml`, 0600), never get logged or included in error messages or `--verbose` output.
    `key list` prints masked values only.
11. **Privacy.** No telemetry, no analytics, no phone-home. The public-IP lookup happens only when
    the user asks for it (`--ip`) or when no location is configured anywhere; it is documented in
    `--help` and the README.
12. **Determinism.** Same inputs → same output. Tests never touch the network; anything time- or
    tty-dependent is injected (`RenderContext::now`, `TermCaps`, `Transport`).

## Commands

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

A change is not done until: `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
`cargo test --workspace` and `reuse lint` are clean **and** the changed surface was actually run and
observed (for a CLI change, that means executing `cirrocast` and looking at the output, not only
tests). The workspace flag matters because the root manifest is a package: without it a bare
`cargo clippy`/`cargo test` in the repository root covers only `cirrocast` itself, not the dev-only
`build/geo-table` builder.

CI (`.github/workflows/ci.yml`) runs exactly those commands on `ubuntu-26.04` and `macos-26` — the
images are named explicitly, never `<os>-latest` — with the test matrix spanning `stable` and the
MSRV `1.98.0`, every third-party action pinned to a commit SHA, and `CIRROCAST_FORBID_NETWORK=1`
exported for the whole test job. The `gates` job additionally enforces the render-layer import rule
(`src/render` and `src/model` may not import `http`, `provider` or `cache`) and that
`LICENSES/GPL-3.0-or-later.txt` is byte-identical to `LICENSE`. Dependency policy lives in
`deny.toml`: an audited licence allow list, duplicates and wildcards denied, crates.io as the only
source, and no advisory ignore without a reason and an expiry date.

### Releasing

The bundled city table is refreshed and verified **locally, before the tag**: the release workflow
stays off the network for it, and a tag must never be cut against a stale snapshot. The check
downloads the official dump, so run it where the network is reachable — and not under
`CIRROCAST_FORBID_NETWORK=1`:

```bash
cargo run -p geo-table -- https://download.geonames.org/export/dump/cities15000.zip --check
```

`unchanged` for all three files → tag. `CHANGED` → refresh, test, re-record, commit, then tag:

```bash
cargo run -p geo-table -- https://download.geonames.org/export/dump/cities15000.zip   # writes src/geo/data
cargo test --workspace          # the suite pins rows of the committed snapshot
# re-record the blob sizes and timings in docs/plans/21-perf-and-resource-budget.md
git add src/geo/data docs/plans/21-perf-and-resource-budget.md && git commit
```

The README's release checklist mirrors this. The full development/release guide is step 28's
`CONTRIBUTING.md`; this rule moves there once that file exists.

## Plan-driven workflow

1. Pick the lowest-numbered step in `docs/plans/README.md` with `Status: ⬜ not-started` (or continue a
   `🚧 in-progress` one). Steps are executed in order; do not start a step whose dependencies are open.
   `⏸ backlog` items (`B01`, `B02`, …) are outside the schedule and are skipped unless the pull is
   recorded as described in `docs/plans/README.md`. A step appended after an earlier phase may be
   started ahead of its number once every entry of its `Depends on` line is done — record that
   deviation in the step's `## Progress log`.
2. Work item by item through that step's `## Deliverables`. One deliverable = one commit.
3. In the same commit as the code, update the step file: flip the item to `- ✅` and append a dated
   line to `## Progress log`. Never rewrite history in the log.
4. When every item and every `## Exit criteria` item is `- ✅`, set `Status: ✅ done` and update the
   step table in `docs/plans/README.md`. Do not mark a step done while any `- ⬜` item remains.
5. If reality diverges from the plan, fix the plan in the same commit and say why in the log line.
   Interfaces change → `docs/plans/README.md` first.

Progress markers are emoji (`⬜ not-started` / `🚧 in-progress` / `⛔ blocked` / `✅ done` /
`⏸ backlog`, tasks `- ⬜` / `- ✅`) so that scanning a step file shows its state at a glance.

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

## Testing policy

* Unit tests live next to the code; integration tests (CLI behaviour via `assert_cmd`) in `tests/`.
* **No test may open a network connection.** HTTP is exercised through `StubTransport`; upstream
  payloads come from `tests/fixtures/`. A test that needs the network must be `#[ignore]`d with a
  comment naming the environment variable that enables it.
* Snapshot tests (`insta`) are for renderer output; snapshots are reviewed by eye, never
  blanket-accepted (`INSTA_UPDATE=always` is not used in CI).
* Assert behaviour, boundaries, error taxonomy and invariants — not incidental formatting of
  internals. New permanent tests must be able to catch a realistic consumer-visible regression.
* Every bug report gets a failing test first, then the fix; keep the test unless the bug class is
  already covered elsewhere.

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
rejected unless the contract is amended first.
