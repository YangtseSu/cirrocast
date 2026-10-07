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
* This guide owns the build and test commands, the test policy, the recipes, the packaging and
  publishing flow and the [release checklist](#release-checklist); the version numbers a consumer
  sees are [`docs/schema.md`](docs/schema.md#versioning)'s.

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

## CI

| Job | Command |
|---|---|
| `fmt` | `cargo fmt --check` |
| `clippy` | `cargo clippy --workspace --all-targets --locked -- -D warnings` |
| `test` | `cargo test --workspace --locked`, then the reduced build with `--no-default-features` |
| `docs` | the generated-artifact tests (`help_snapshot`, `docs_flags`, `json_schema`), `man --warn`, lychee over every Markdown file |
| `package` | `cargo package --list --locked`, `cargo publish --dry-run --locked` |
| `gates` | `scripts/check-render-imports.py` and the `LICENSE` copy check |
| `reuse` / `deny` / `audit` | `reuse lint`, `cargo deny check`, `cargo audit` |

[`.github/workflows/ci.yml`](.github/workflows/ci.yml) runs these on `ubuntu-26.04`, on the stable
toolchain, with every third-party action pinned to a commit SHA and `CIRROCAST_FORBID_NETWORK=1`
exported for the whole test job. The performance budget is a separate, dispatch-only workflow
([`.github/workflows/perf.yml`](.github/workflows/perf.yml)): nothing in it runs on a push or a pull
request. A tag runs the suite once more with `--release` on `macos-26` before the archives are packed
([`.github/workflows/release.yml`](.github/workflows/release.yml)).

## Packaging and release

| Install path | What lands on the system | Built from |
|---|---|---|
| `cargo install --locked cirrocast` (or `--path .`) | the binary in `$CARGO_HOME/bin`; the man page and completions are generated by it on demand | crates.io, or this checkout |
| AUR [`cirrocast`](https://aur.archlinux.org/packages/cirrocast) | `/usr/bin/cirrocast`, `cirrocast.1`, the bash/zsh/fish completions, and `LICENSE` under `/usr/share/licenses/cirrocast/` | the `vX.Y.Z` tag tarball from GitHub |
| Release archive `cirrocast-vX.Y.Z-<target>.tar.gz` | `cirrocast-vX.Y.Z-<target>/` containing the binary, `README.md`, `LICENSE`, `CHANGELOG.md`, `cirrocast.1` and `completions/` — nothing is installed for you | the release workflow (`.github/workflows/release.yml`) |

Archives are built natively on `ubuntu-26.04` (x86_64), `ubuntu-26.04-arm` (aarch64) and `macos-26`
(Apple silicon), one job per target, with no cross-compilation; a Linux archive therefore needs that
image's glibc or newer. On an older distribution the AUR package is the better fit — it builds from
the tag tarball against the system's own glibc. Windows is not packaged in v1: `cargo install`
covers it, and further ecosystem packages are deferred to the backlog (B02). Every archive ships
next to the `.sha256` the workflow computed:

```bash
sha256sum -c cirrocast-v1.3.0-x86_64-unknown-linux-gnu.tar.gz.sha256   # macOS: shasum -a 256 -c
```

`cargo package --list --locked` and `cargo publish --dry-run --locked` run on every pull request
(the `package` job above), so the file set a release uploads is reviewed before any tag exists.

### Release checklist

A tag is never cut against a stale snapshot: steps 2 and 3 download the official dumps, so run them
where the network is reachable — never under `CIRROCAST_FORBID_NETWORK=1`.

1. The four gates plus `cargo deny check` are green on the commit to be tagged, and CI is green on it.
2. The bundled city data is current:
   `cargo run -p geo-table -- https://download.geonames.org/export/dump/cities15000.zip --check`
   reports every file `unchanged` against the official dump. If it reports `CHANGED`, refresh the
   snapshot (`cargo run -p geo-table -- <path-or-url>`), run `cargo test --workspace` (the suite
   pins rows of the committed data), re-record the size/timing numbers in
   [`docs/plans/21-perf-and-resource-budget.md`](docs/plans/21-perf-and-resource-budget.md), and
   commit all of it before tagging.
3. The bundled country layer is current too: `cargo run -p geo-table -- --countries
   https://raw.githubusercontent.com/nvkelso/natural-earth-vector/v5.1.2/geojson/ne_50m_admin_0_countries.geojson
   --check` reports both files `unchanged`. `CHANGED` → rebuild without `--check`, run
   `cargo test --workspace` (the layer's canary pins coordinates and the size budget), and commit
   `src/geo/data`.
4. `version` bumped in `Cargo.toml`; `CHANGELOG.md` gets its dated section and compare link; the
   version strings in `README.md`, this guide, `docs/getting-started.md`, `docs/troubleshooting.md`
   and the bug-report issue form move with it; all committed together.
5. `cargo package --list --locked` reviewed (`src/`, `locales/`, `Cargo.toml`, `Cargo.lock`,
   `LICENSE`, `README.md`, `CHANGELOG.md` — and nothing else).
6. A signed tag is pushed: `git tag -s vX.Y.Z -m "cirrocast vX.Y.Z" && git push origin vX.Y.Z`. The
   workflow refuses a tag that does not match `Cargo.toml`'s version.
7. The run is watched: three archives, three `.sha256` files, and a GitHub release with generated
   notes. One archive is inspected (`tar tzf`) and verified (`sha256sum -c`).
8. The `publish` job is approved in the `crates-io` environment — `cargo publish` cannot be undone.
9. The AUR package is bumped, which is only possible once the tag exists because the checksums come
   from the tag tarball:

```bash
git clone ssh://aur@aur.archlinux.org/cirrocast.git   # the packaging files live only there
cd cirrocast                                          # edit pkgver= in PKGBUILD, then:
updpkgsums                                            # recompute sha256sums from the tag tarball
makepkg --printsrcinfo > .SRCINFO                     # AUR metadata, regenerated in the same commit
makepkg --printsrcinfo | diff - .SRCINFO              # must print nothing
namcap PKGBUILD
makepkg -f && namcap cirrocast-*.pkg.tar.zst
git commit -am "upgpkg: cirrocast X.Y.Z-1" && git push
```

```bash
# what a bump is verified with, on a clean machine or in a chroot
pkgctl build                         # devtools clean chroot; makechrootpkg -c does the same
sudo pacman -U cirrocast-1.3.0-1-x86_64.pkg.tar.zst
cirrocast --version && man -w cirrocast
pacman -Ql cirrocast | grep -E 'completions/cirrocast$|site-functions/_cirrocast$|vendor_completions\.d/cirrocast\.fish$' | wc -l   # 3
```

(A plain `pacman -Ql cirrocast | grep -c completions` prints 4: the two completion *directories*
match as well, which is why the three file paths are matched explicitly.)

The AUR repository is the only home of the packaging files — this tree keeps no copy, which is why
the checklist edits them there (the reasoning is in
[`docs/plans/13-packaging-and-release.md`](docs/plans/13-packaging-and-release.md)). `check()` in the
PKGBUILD compares `cargo metadata`'s version with `$pkgver`, so a stale PKGBUILD fails the build
instead of shipping a mislabelled package.

## Publishing

The crate is prepared for crates.io, but publishing is a deliberate act rather than a side effect of
tagging: it runs in the release workflow's `publish` job, behind the protected `crates-io`
environment, and `cargo publish` cannot be undone.

```bash
cargo package --list --locked        # the exact file set that would be uploaded
cargo publish --dry-run --locked     # builds the packaged crate; uploads nothing
cargo doc --no-deps --all-features   # what docs.rs renders
```

Checklist, executed 2026-10-01 before the first release:

* `cirrocast` is free on crates.io (checked against the registry API);
* `cargo package --list --locked` shows 46 files — `src/**`, `locales/**`, `Cargo.toml` (plus
  cargo's own `Cargo.toml.orig`), `Cargo.lock`, `LICENSE`, `README.md`, `CHANGELOG.md` — and no
  `docs/`, `tests/`, `examples/` or `.github/`;
* `cargo publish --dry-run --locked` succeeds, building the crate from the packaged file set;
* `cargo doc --no-deps --all-features` builds the library target without warnings, and
  `[package.metadata.docs.rs] all-features = true` is set;
* `license = "GPL-3.0-or-later"` with no `license-file` (cargo forbids both);
* `Cargo.lock` is committed, because every release path builds with `--locked`.

`cirrocast 1.0.0` was published on 2026-10-02 with `cargo publish --locked` from the tagged tree
(the `v1.0.0` release workflow's `publish` job had skipped with its notice, because that was before
the crate moved to trusted publishing). The crate's crates.io settings now enable **"Require trusted
publishing for all new versions"**, so token-based publishing is refused from here on.

Since 2026-10-02 the release workflow publishes with **crates.io trusted publishing** (OIDC): no
token is stored in GitHub. The `publish` job runs inside the `crates-io` environment with
`id-token: write` and exchanges the workflow's OIDC identity for a short-lived crates.io token via
[`rust-lang/crates-io-auth-action`](https://github.com/rust-lang/crates-io-auth-action), pinned by
commit SHA in `.github/workflows/release.yml`. The crate's trusted-publisher record names three
things — the repository (`YangtseSu/cirrocast`), the workflow file (`release.yml`) and the
environment (`crates-io`) — and the exchange fails if any of them changes. `gh workflow run
release.yml` executes only the `verify` job, which proves that record matches without cutting a
release; a version that is already on crates.io is skipped by the publish job's own check.
