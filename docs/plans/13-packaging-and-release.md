<!--
SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Step 13 — Packaging and release

Status: ✅ done
Depends on: 08 (cli-surface-and-formats), 12 (quality-hardening)
Touches: `Cargo.toml` (metadata only), `CHANGELOG.md` (new), `docs/schema.md` (new), `README.md`,
`.github/workflows/release.yml` (new), `.github/workflows/ci.yml` (publish dry-run job), `src/render/json.rs`,
`tests/render_json.rs`, `docs/plans/{14-v1-acceptance,22-status-and-ecosystem}.md` (reference updates), and the
AUR package `cirrocast` — which lives in its own git repository
(`ssh://aur@aur.archlinux.org/cirrocast.git`), not in this tree.

## Goal

Turn the working CLI into a releasable artefact: a written versioning policy for the crate, the JSON output
schema and the config schema; a Keep a Changelog file whose first entries come from the completed steps; one
release workflow that builds Linux and macOS binaries with the GPL licence text, README and man page inside each
archive; a crates.io publish checklist that `cargo package --list` actually satisfies; an Arch package (tagged
release, maintained in the AUR repository) that installs the binary, the bash/zsh/fish completions, `cirrocast.1`
and the licence; and documented install paths for source, `cargo install` and AUR users.

## Deliverables

- ✅ Versioning policy in `README.md#versioning`: `Cargo.toml` stays `0.x` while phases A–C land, `1.0.0` is
  cut in step 14, phase D releases are `1.1.0`–`1.2.0` (steps 15–20), phase E is `1.3.0` (steps 21–22) and
  phases F and G are `1.4.0` (steps 23–28), with the backlog (B01–B02) unscheduled, matching
  `docs/plans/README.md`; the crate version, the JSON schema version and the config schema version move
  independently.
- ✅ `docs/schema.md` (new): the JSON output schema v1 (every field, its type, its canonical unit, optionality,
  `schema_version`) and the config schema v1 (every key, default, accepted values, migration hooks); the rule that
  additive fields are a schema-minor change while removing, renaming or retyping a field is a schema-major change
  that bumps `schema_version`; a worked example document for each. Its key index table is the machine-checked list
  the next bullet reads, so the document and the code cannot drift apart unnoticed.
- ✅ `src/render/json.rs` + `tests/render_json.rs`: the top-level `"schema_version": 1` and the stable field names
  and units are pinned by tests that read the *parsed* document (round-trip through `serde_json`, not a text
  snapshot) and compare it with the key index of `docs/schema.md`; the hand-maintained `EXPECTED_KEYS` array is
  deleted rather than kept as a second list that can drift.
- ✅ `CHANGELOG.md` (new, Keep a Changelog 1.1.0 + SemVer, SPDX header): an `Unreleased` section plus `0.x`
  entries summarising the user-visible changes of steps 01–11 (backends, formats, localization, cache/offline
  behaviour), each entry naming the CLI surface it affects; the `1.0.0` entry is added in step 14; compare links
  point at GitHub tags.
- ✅ `.github/workflows/release.yml` (new): triggered by tags matching `v[0-9]*.[0-9]*.[0-9]*` (GitHub tag
  filters are globs, not regexes; the build job also asserts that the tag equals `Cargo.toml`'s version),
  matrix
  `ubuntu-26.04` → `x86_64-unknown-linux-gnu`, `ubuntu-26.04-arm` → `aarch64-unknown-linux-gnu`, `macos-26` →
  `aarch64-apple-darwin` (image names explicit, never `<os>-latest`; each runner builds its own target, so no
  cross-compilation); each job runs `cargo build --release --locked`, `cirrocast man > cirrocast.1`,
  `cirrocast completion bash|zsh|fish`, `cargo test --release --locked`, then packs
  `cirrocast-v$VERSION-$TARGET.tar.gz` containing `cirrocast`, `README.md`, `LICENSE`, `CHANGELOG.md`,
  `cirrocast.1` and `completions/`, writes a `.sha256` per archive and publishes with the preinstalled `gh`
  (`gh release create "$GITHUB_REF_NAME" … --generate-notes`).
- ✅ `publish` job in the same workflow: `cargo package --list` review and `cargo publish --locked` through
  **crates.io trusted publishing** (OIDC) — the job runs in the `crates-io` environment with `id-token: write`
  and exchanges the workflow identity for a short-lived token via `rust-lang/crates-io-auth-action` (pinned by
  SHA), so no token is stored in GitHub; a `workflow_dispatch` run executes only the `verify` job, which proves
  the crate's trusted-publisher record matches this repository/workflow/environment without cutting a release,
  and the publish step skips a version that is already on crates.io. (Original design, 2026-10-01: a
  `CARGO_REGISTRY_TOKEN` environment secret with a skip-with-notice while unset; replaced on 2026-10-02 —
  see the progress log.) Plus a documented dry-run path (`cargo publish --dry-run --locked`) that runs for
  every PR in `ci.yml`.
- ✅ `Cargo.toml` metadata: keep `license = "GPL-3.0-or-later"` with no `license-file` (cargo forbids both),
  `repository`, `readme`, `keywords`, `categories` and `rust-version` present and correct; replace the blunt
  `exclude = ["docs/", ".github/"]` with an explicit `include` list (`src/**`, `locales/**`, `LICENSE`,
  `README.md`, `CHANGELOG.md`, `Cargo.toml`, `Cargo.lock`) so `docs/`, `.github/`, `tests/`, `examples/` and
  `packaging/` stay out of the crate; `Cargo.lock` must be committed because every release path uses `--locked`.
- ✅ crates.io checklist executed and recorded in `README.md#publishing`: name `cirrocast` still free,
  `cargo package --list` shows no `docs/`, no `tests/`, no `examples/` and no `.github/`, and does show
  `LICENSE`/`README.md`/`CHANGELOG.md`, `cargo publish --dry-run --locked` passes,
  `cargo doc --no-deps --all-features` builds (the library target is the documented half, so its module doc
  comments carry the description), and `[package.metadata.docs.rs] all-features = true` is set.
- ✅ AUR package `cirrocast` created in its own repository at `0.1.0` (pushed as commit `b0652b1`): an
  obfuscated `# Maintainer:` line and a
  `0BSD` packaging-licence header (the licence the AUR asks for, separate from the project's GPL; the first
  lines are
  <!-- REUSE-IgnoreStart -->
  `# SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>` and `# SPDX-License-Identifier: 0BSD`),
  <!-- REUSE-IgnoreEnd -->
  `arch=('x86_64' 'aarch64')`, `license=('GPL-3.0-or-later')`, `depends=('gcc-libs' 'glibc')`,
  `makedepends=('cargo' 'rust')`, `options=('!lto' '!debug')`, source
  `https://github.com/YangtseSu/cirrocast/archive/refs/tags/v$pkgver.tar.gz` with the real `sha256sums` produced
  by `updpkgsums`; `build()` maps `CARCH` (`x86_64` → `x86_64-unknown-linux-gnu`, `aarch64` →
  `aarch64-unknown-linux-gnu`) and runs `cargo build --release --locked --target "$triple"`; `check()` runs
  `cargo test --release --locked` and compares `cargo metadata --format-version 1`'s package version with
  `$pkgver` so a stale PKGBUILD fails loudly; `package()` installs `usr/bin/cirrocast`, the completions to
  `usr/share/bash-completion/completions/cirrocast`, `usr/share/zsh/site-functions/_cirrocast`,
  `usr/share/fish/vendor_completions.d/cirrocast.fish`, the man page to `usr/share/man/man1/cirrocast.1`, and
  `LICENSE` to `usr/share/licenses/cirrocast/LICENSE` (GPL requires shipping the licence text). A `LICENSE` file
  with the 0BSD text lives in the same repository, because the AUR requires the packaging to carry a licence.
  `makedepends` also carries `jq` (the version assertion reads `cargo metadata`), and `source` uses the
  unique-name form (`cirrocast-$pkgver.tar.gz::https://…`) that namcap asks for. Result: `updpkgsums` filled
  `sha256sums=('05cde6d4…c5a')` from the `v0.1.0` tarball (fetched twice independently, same bytes), and
  `makepkg --printsrcinfo` is byte-identical to the committed `.SRCINFO`.
- ✅ AUR verification recorded in `README.md#packaging` and the progress log: `namcap PKGBUILD` (exit 0),
  `makepkg --printsrcinfo | diff - .SRCINFO` (empty), `makepkg -f` (built `cirrocast-0.1.0-1-x86_64.pkg.tar.zst`,
  5.1 MB, `check()` running the crate's release tests), `namcap` on the package (its two warnings are the
  canonical `libgcc`/`gcc-libs` pair for a Rust binary; no errors), then the package extracted and run from a
  temporary root: `cirrocast --version` → `cirrocast 0.1.0`, `man -w cirrocast` → the installed `cirrocast.1.gz`
  (rendered), and exactly three completion files (`bash-completion/completions/cirrocast`,
  `zsh/site-functions/_cirrocast`, `fish/vendor_completions.d/cirrocast.fish`). The `grep -c completions` form
  originally written here counts the two completion *directories* as well and prints 4; the checklist and the
  README now match the three file paths explicitly. A clean-chroot build (`sudo pkgctl build`) is the one step
  that needs root and is left to the maintainer; the same PKGBUILD was additionally built under a pristine
  `HOME`/`CARGO_HOME` with `--cleanbuild` to prove it fetches its own dependencies.
- ✅ `cargo install` path documented in `README.md#install`: `cargo install --locked cirrocast` (and
  `cargo install --locked --path .` from a checkout), plus the two commands that install what the packages
  place for you — `cirrocast completion <bash|zsh|fish> > <completion path>` and `cirrocast man > <man path>` —
  with the exact target paths per shell.
- ✅ `README.md#packaging` documents all three install paths (source build / `cargo install`, the AUR
  package, the release archive), the files each one installs, the release checklist (bump `Cargo.toml`, bump the
  AUR package with `updpkgsums`, regenerate `.SRCINFO`, update `CHANGELOG.md`, push a signed `vX.Y.Z` tag,
  confirm the workflow's archives and the crates.io publish) and how to verify a downloaded archive
  (`sha256sum -c`), plus the glibc floor of the Linux archives and the clean-chroot verification commands.
- ✅ The pre-tag checklist is part of the step file (`## Pre-tag checklist`): version bumped in `Cargo.toml`, `CHANGELOG.md` released
  section dated, `cargo package --list` reviewed, CI green on the tag's commit, archive contents inspected
  (`tar tzf`), the crates.io publish confirmed, and the AUR package bumped *after* the tag exists
  (`updpkgsums`, `makepkg --printsrcinfo > .SRCINFO`, `git push`).

## Design notes

* Release tooling: **plain `cargo build --release --locked` in a GitHub Actions matrix**, not `cargo-dist`.
  `cargo-dist` can do the job (its `include` option and README/LICENSE auto-includes cover most of the archive
  requirements), but the archives must also contain artefacts generated *at release time by the binary itself*
  (`cirrocast man`, `cirrocast completion …`), which a static `include` list cannot express; we would have to
  commit generated files and add a fidelity check. The hand-written workflow is roughly sixty lines, needs no
  extra tool pinned in CI, keeps the third-party action surface at zero beyond the pinned checkout action, and
  already has to exist for the `publish` job. Revisit if we ever want shell/PowerShell installers (step B02).
* Release archives ship `LICENSE`, `README.md` and `CHANGELOG.md` inside every tarball because GPL-3.0-or-later
  distribution requires the licence text alongside the binary, and the man page plus completions because a tarball
  user has no package manager to install them.
* The crate ships a library target as well as the binary: `src/lib.rs` has existed since step 07, because the
  integration tests compile against the modules. It is published and documented as it is, but the contract the
  project promises is the CLI plus the documented schemas — the module paths carry no stability promise of their
  own, and `docs/schema.md` is where a consumer's guarantees live.
* **AUR layout (decided 2026-10-01)**: the AUR repositories are the **only** home of the packaging files — this
  tree keeps no `packaging/aur/` copy. A release PKGBUILD's `sha256sums` can only be computed after the tag
  exists, so the file is written after the release; a copy here would mean a second post-release commit, a second
  edit surface and a drift risk, while the only consumers of a PKGBUILD (AUR web, `pkgctl`, AUR helpers) read the
  AUR repository. The packaging licence (`LICENSE`, RFC 0040) and the obfuscated `# Maintainer:` line are AUR
  obligations and would sit awkwardly beside this tree's GPL headings, and the install surface is verified where
  it is installed: step 14 builds the AUR package in a clean chroot and runs the binary, the man page and the
  three completions from the installed package. `REUSE.toml` therefore needs no `.SRCINFO` annotation.
* **No `cirrocast-git` package** (same decision): a VCS package repeats the same `build()`/`check()`/`package()`
  bodies and needs them kept in sync by hand, while the tagged package already follows every release by the
  ordinary PKGBUILD bump. Revisit in step B02 if users ask for a tracking package.
* Arch packaging is hand-written rather than generated: the project has no Rust-to-PKGBUILD generator in its
  dependency set, the package is small, and a hand-written PKGBUILD keeps the `CARCH` mapping, the licence path
  and the completions paths explicit and reviewable.
* The PKGBUILD does not vendor crates: `cargo build --locked` fetches from crates.io inside `build()`, which is
  the AUR-accepted practice; vendoring would add a second checksum surface for no benefit at this size.
* The Linux release archives are built on the `ubuntu-26.04` image, the same image CI tests on, so an archive
  needs that image's glibc or newer; `README.md#packaging` says so and points older distributions at the AUR
  package, which builds against their own glibc.

## Out of scope

* Windows, deb/rpm, Homebrew, AppImage/Flatpak, container images and other ecosystem packages: step B02.
* A `cirrocast-git` AUR package and a copy of the packaging files in this tree: declined 2026-10-01 (see the
  design notes); revisit in step B02.
* Documentation set beyond README/schema/step files (guides, website, translations of docs): step 28.
* Performance and size budgets printed in release notes: step 21 (the binary size note in step 14 is informational).
* Code signing, notarization and reproducible-build attestations: no step file owns them; they are recorded in
  step 14's non-goals until a platform requires them.
* Feature work of any kind, including the `serve` mode of step B01 and multi-location output of step 19.

## Verification

```
cargo package --list                                  # no docs/, tests/, examples/; LICENSE and README present
cargo publish --dry-run --locked                      # succeeds without uploading
cargo doc --no-deps --all-features                    # renders the binary crate without warnings
cargo test --test render_json                         # the doc key index and the rendered document agree

# in a clone of ssh://aur@aur.archlinux.org/cirrocast.git
updpkgsums && makepkg --printsrcinfo > .SRCINFO        # real checksums for the tag tarball
makepkg --printsrcinfo | diff - .SRCINFO               # empty
namcap PKGBUILD && makepkg -f && namcap cirrocast-*.pkg.tar.zst
pkgctl build                                           # clean chroot
pacman -U cirrocast-0.1.0-1-x86_64.pkg.tar.zst         # then: --version, man -w, three completions
# the count must match the three *files*: `grep -c completions` also counts the two directories
pacman -Ql cirrocast | grep -E 'completions/cirrocast$|site-functions/_cirrocast$|vendor_completions\.d/cirrocast\.fish$' | wc -l

# after the tag push
gh release view v0.1.0 --json assets
tar tzf cirrocast-v0.1.0-x86_64-unknown-linux-gnu.tar.gz   # binary, LICENSE, README.md, CHANGELOG.md, man, completions/
sha256sum -c cirrocast-v0.1.0-x86_64-unknown-linux-gnu.tar.gz.sha256
```

## Exit criteria

- ✅ `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`, `reuse lint` all clean
  (locally on rust 1.98.1 and in CI run 36881479150, which is the commit the tag points at).
- ✅ `cargo package --list` shows the intended file set (46 files: `src/**`, `locales/**`, `Cargo.toml`,
  `Cargo.lock`, `LICENSE`, `README.md`, `CHANGELOG.md`) and `cargo publish --dry-run --locked` succeeds;
  `docs/schema.md`, `CHANGELOG.md` and `README.md#versioning` agree with each other and with `Cargo.toml`
  (the config example in the schema is byte-identical to `cirrocast config init`'s output).
- ✅ The AUR repository holds `PKGBUILD`, `.SRCINFO` and the packaging licence; `namcap` reports no errors on
  the PKGBUILD and none on the built package (only its canonical `libgcc`/`gcc-libs` warning pair),
  `makepkg --printsrcinfo` matches the committed `.SRCINFO`, and `makepkg -f` yields a package whose contents
  provide a working `cirrocast --version`, a `man -w cirrocast` hit and three completion files. The
  clean-chroot leg (`sudo pkgctl build`) needs root and is the maintainer's to run; the same PKGBUILD was
  built under a pristine `HOME`/`CARGO_HOME` with `--cleanbuild` instead.
- ✅ The `v0.1.0` tag produced the three documented archives (Linux x86_64/aarch64, macOS aarch64) with
  `.sha256` files; the workflow's `release` job failed on this first run (a missing `--repo`, fixed on `main`
  afterwards) and the release was recreated from the run's own artifacts — step 14 repeats the whole path for
  `1.0.0`, which is what the fix is for.

## Pre-tag checklist

In order, on the commit that will carry the tag. Items 6–8 can only be checked after it:

1. ✅ `cargo fmt --check`, `cargo clippy --all-targets --locked -- -D warnings`, `cargo test --locked`
   (with `CIRROCAST_FORBID_NETWORK=1`), `reuse lint` and `cargo deny check` are clean locally.
2. ✅ `Cargo.toml`'s `version` matches the tag (`vX.Y.Z`) and `CHANGELOG.md` carries the dated section for it.
3. ✅ `cargo package --list --locked` shows the intended 46 files — no `docs/`, `tests/`, `examples/`,
   `.github/`.
4. ✅ `cargo publish --dry-run --locked` and `cargo doc --no-deps --all-features` pass on that commit (the
   `package` CI job runs the first of the two on every pull request).
5. ✅ CI is green on the commit pushed to `main` (run 36881479150), and the signed `v0.1.0` tag is pushed
   from that commit.
6. ✅ Each archive is inspected (`tar tzf`) and verified (`sha256sum -c`), and the GitHub release exists.
   The workflow's `release` job failed on this first run — `gh` had no git remote to read the repository
   from in a job that does not check the tree out; the fix (`--repo "$GITHUB_REPOSITORY"`) is on `main`, and
   `v0.1.0`'s release was recreated from that run's own artifacts: three archives, three `.sha256` files,
   checksums verified and the x86_64 binary run from the extracted archive (`cirrocast 0.1.0`).
7. ✅ The crates.io publish is confirmed, or the reason it did not run is recorded. — the `v0.1.0` `publish`
   job took the skip branch (`CARGO_REGISTRY_TOKEN is not configured for the crates-io environment; skipping the
   publish`, exit 0); `1.0.0` was published from the maintainer's machine on 2026-10-02, and since that date the
   job publishes through crates.io trusted publishing instead of a token (see the progress log).
8. ✅ The AUR package is bumped from the tag tarball (`updpkgsums`, `makepkg --printsrcinfo > .SRCINFO`,
   `git push`) and the `## Verification` commands are re-run. — pushed as commit `b0652b1`.

## Risks

* GitHub runner drift (an image being renamed or retired) would break one matrix entry: the matrix is a one-line
  change and the workflow comments record the alternatives (`ubuntu-24.04`, `macos-15-intel`).
* `cargo publish` is irreversible: mitigated by a dry-run job on every PR, a protected environment, the
  trusted-publishing exchange (which carries no long-lived token to leak or forget) and publishing after the
  GitHub release artefacts exist.
* AUR rules require `.SRCINFO` to match the PKGBUILD exactly: the release checklist regenerates it in the same
  commit as the `pkgver` bump, and the `check()` version assertion fails a stale PKGBUILD loudly.
* `--locked` fails in the source tarball if `Cargo.lock` is not committed or if a dependency needs a newer MSRV:
  `Cargo.lock` is committed and step 12's MSRV job runs on the same lockfile.
* The PKGBUILD checksum must be updated for every release: `updpkgsums` does it from the tag tarball, the
  `check()` version assertion catches a forgotten bump, and the checklist names the step.
* The AUR package is the only copy of the packaging files, so an edit made there is not reviewed in this tree:
  accepted 2026-10-01 (the files change at most once per release and the verification commands live in
  `README.md#packaging`); step B02 may add an automated AUR bump if the manual one becomes a nuisance.

## Progress log

- 2026-09-30 — step file written (status: not-started); `cargo-dist` capability (static `include`, README/LICENSE
  auto-includes) checked before rejecting it in favour of the hand-written workflow.
- 2026-10-01 — plan revised before the first deliverable: the packaging files live only in the AUR repository
  (no `packaging/aur/` copy here, no `--git` package) and the AUR package is created for the `v0.1.0` tag; the
  Linux/macOS release images are `ubuntu-26.04`, `ubuntu-26.04-arm` and `macos-26` (Intel macOS dropped); the
  `publish` job skips with a notice until a token is configured. Reason: a copy of a release PKGBUILD cannot be
  complete before the tag exists (the sums), so the copy only adds a post-release commit and a drift surface,
  while the AUR repository is mandatory anyway. Step 14's clean-chroot deliverable now builds the AUR package.
- 2026-10-01 — `docs/schema.md` written: the JSON v1 key index (the machine-checked list), the config v1 keys
  with their migration hooks, and a real `-f json` document plus the `config init` document as worked examples.
- 2026-10-01 — `tests/render_json.rs` now reads the documented key index and checks every value's JSON type and
  the never-null claims against it; the duplicated `EXPECTED_KEYS` array is gone, and a knowingly renamed doc
  key was verified to fail the suite.
- 2026-10-01 — `CHANGELOG.md` added: Keep a Changelog 1.1.0, an `Unreleased` placeholder and a dated `0.1.0`
  entry that names the CLI surface of every user-visible feature up to step 12.
- 2026-10-01 — `Cargo.toml` publishes only `src/`, `locales/` and the five root files (46 files, no `docs/`,
  `tests/`, `examples/` or `.github/`); `[package.metadata.docs.rs] all-features = true` added. Plan corrected in
  the same commit: the crate is not binary-only — `src/lib.rs` has existed since step 07 for the integration
  tests, so the docs.rs note and the checklist wording now say "library target".
- 2026-10-01 — `release.yml` added: three native build legs (`ubuntu-26.04`, `ubuntu-26.04-arm`, `macos-26`), a
  tag/version assertion, archives under a `cirrocast-vX.Y.Z-<target>/` directory with the man page and three
  completions generated by the binary itself, `.sha256` per archive, a `gh release create --generate-notes`
  job, and the `publish` job behind the `crates-io` environment. The pack step was rehearsed locally for the
  x86_64 leg (build → man/completions → tar → `sha256sum -c`), since the tag push is what exercises the rest.
- 2026-10-01 — `ci.yml` gained a `package` job: `cargo package --list --locked` plus
  `cargo publish --dry-run --locked` on every pull request, so the packaged file set and the packaging build
  are reviewed before a tag exists.
- 2026-10-01 — `README.md#versioning` added: the four independent version numbers (crate, JSON schema, config
  schema, cache envelope), the release schedule matching the plan index, and the minor/major rule for schema
  changes.
- 2026-10-01 — `README.md#install` rewritten around the four real paths (crates.io, checkout, AUR, release
  archive) with the `sha256sum -c` step and the exact per-shell completion targets `cargo install` cannot
  place.
- 2026-10-01 — `README.md#packaging` added: the install-path table, the native build matrix and the glibc floor
  of the Linux archives, the seven-step release checklist with the `updpkgsums`/`.SRCINFO` AUR bump and the
  clean-chroot verification commands.
- 2026-10-01 — crates.io checklist executed and recorded in `README.md#publishing`: the name is free on the
  registry API, `cargo package --list --locked` is 46 files with no `docs/`, `tests/`, `examples/` or `.github/`,
  `cargo publish --dry-run --locked` built the packaged crate and stopped at the dry-run upload, and
  `cargo doc --no-deps --all-features` is warning-free (the eight pre-existing rustdoc warnings are fixed in the
  same commit). The `crates-io` environment and the `CARGO_REGISTRY_TOKEN` secret are the remaining manual step
  before the workflow can publish, and the job skips with a notice until then.
- 2026-10-01 — `## Pre-tag checklist` added to this file (items 1–4 already satisfied locally: the five gates,
  the 46-file package listing, the passing dry run and the warning-free doc build); the release tag is the next
  step, then the AUR package, which is the part that needs the tag tarball.
- 2026-10-01 — the first `main` push went red in the clippy job: stable had moved to 1.99, whose pedantic
  `assert_is_empty` fires on all 48 `assert!(x.is_empty())` sites (Arch ships rust 1.98.1, so it could not be
  reproduced locally; the lint is unknown to 0.1.98). The sites now use the element-typed comparison the lint
  asks for (`assert_eq!(x, Vec::<T>::new())`, `assert_eq!(s, "")`, `assert_eq!(slice, [] as [T; 0])`,
  `assert_ne!` for the negated ones) — no suppression, and a failure now prints the value. CI run 36881479150 is
  green on the resulting commit, which is the one the tag is cut from.
- 2026-10-01 — AUR: the package repository was cloned, `PKGBUILD` + `.SRCINFO` + a 0BSD packaging `LICENSE`
  written, and the first `updpkgsums` exposed an infrastructure problem rather than a packaging one — the
  GitHub repository was still private, so the tag tarball answers 404 to every unauthenticated client
  (authenticated fetches return 200 for the same URLs). It is public now (after scanning the whole history for
  credentials: no private-key blocks, no `ghp_`/`sk-`/`AKIA`/`AIza` tokens, no key-bearing fixture values), and
  the tarball then hashed to `05cde6d4…c5a` in two independent downloads.
- 2026-10-01 — AUR package pushed (commit `b0652b1`). `namcap` on the first draft asked for two changes, both
  applied: the target triple is derived from `$CARCH` instead of listing architectures, and the source URL is
  named `cirrocast-$pkgver.tar.gz::…`. `namcap PKGBUILD` is then silent; `makepkg -f` built
  `cirrocast-0.1.0-1-x86_64.pkg.tar.zst` (5.1 MB, the crate's release tests running in `check()`), and the
  extracted package ran `cirrocast --version`, resolved `man -w cirrocast` and installed exactly three
  completion files. The same PKGBUILD also built under a pristine `HOME`/`CARGO_HOME` with `--cleanbuild`.
  The clean-chroot leg needs root (`sudo pkgctl build`) and is left to the maintainer.
- 2026-10-01 — release run 36882589122: the three native build legs are green and produced the documented
  archives; the `publish` job skipped on the missing token exactly as designed; the `release` job failed
  because `gh` has no git remote to read in a job that never checks the tree out — `--repo "$GITHUB_REPOSITORY"`
  is the fix on `main`, and the `v0.1.0` release was recreated from that run's own artifacts (all three
  `.sha256` files verified, the x86_64 binary run from the archive). Step 14's `1.0.0` tag is what exercises the
  fixed job end to end.
- 2026-10-02 — the publish path moved from a stored token to **crates.io trusted publishing**. `1.0.0` had been
  published from the maintainer's machine (the workflow's `publish` job having skipped on the unset environment
  secret), and the crate's crates.io settings now enable *"Require trusted publishing for all new versions"*,
  so token publishing is refused from here on. `release.yml`'s `publish` job runs with `id-token: write` inside
  the `crates-io` environment and exchanges the workflow identity for a short-lived token via
  `rust-lang/crates-io-auth-action` (pinned to `c6f97d42243bad5fab37ca0427f495c86d5b1a18`, v1.0.5); the
  skip-with-notice branch is gone — a mis-configured trusted-publisher record now fails the job loudly, which is
  the point. A `verify` job, reachable only through `workflow_dispatch`, proves the record matches the
  repository/workflow/environment without cutting a release. The deliverable, pre-tag-checklist and risk wording
  above were corrected in the same commit, and README.md#publishing documents the new setup.
- 2026-10-02 — the exchange was proved before any release depends on it: `gh workflow run release.yml --ref main`
  (run 36944713379, the `verify` job only — build/release/publish skip on a dispatch). The log shows
  `Requesting token from: https://crates.io/api/v1/trusted_publishing/tokens` → `Retrieved token successfully` →
  `##[notice]crates.io issued a temporary token for this workflow; the trusted-publisher record matches`, and the
  action's post step `Token revoked successfully`. The next tag therefore publishes without a stored secret; the
  only way it can fail is a change to the repository name, the workflow file name or the `crates-io` environment
  without updating the crate's trusted-publisher record.
