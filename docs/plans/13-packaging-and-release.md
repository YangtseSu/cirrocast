<!--
SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Step 13 — Packaging and release

Status: not-started
Depends on: 08 (cli-surface-and-formats), 12 (quality-hardening)
Touches: `Cargo.toml` (metadata only), `CHANGELOG.md` (new), `docs/schema.md` (new), `README.md`,
`.github/workflows/release.yml` (new), `.github/workflows/ci.yml` (publish dry-run job), `src/render/json.rs`,
`tests/render_json.rs`, `packaging/aur/PKGBUILD` (new), `packaging/aur/.SRCINFO` (new),
`packaging/aur/cirrocast-git/PKGBUILD` (new), `packaging/aur/cirrocast-git/.SRCINFO` (new), `REUSE.toml`

## Goal

Turn the working CLI into a releasable artefact: a written versioning policy for the crate, the JSON output
schema and the config schema; a Keep a Changelog file whose first entries come from the completed steps; one
release workflow that builds Linux and macOS binaries with the GPL licence text, README and man page inside each
archive; a crates.io publish checklist that `cargo package --list` actually satisfies; Arch packages (tagged
release and `-git`) that install the binary, the bash/zsh/fish completions, `cirrocast.1` and the licence; and
documented install paths for source, `cargo install` and AUR users.

## Deliverables

- [ ] Versioning policy in `README.md#versioning`: `Cargo.toml` stays `0.x` while phases A–C land, `1.0.0` is
  cut in step 14, phase D releases are `1.1.0`–`1.2.0` (steps 15–19) and phase E is `2.0.0` (steps 20–24),
  matching `docs/plans/README.md`; the crate version, the JSON schema version and the config schema version move
  independently.
- [ ] `docs/schema.md` (new): the JSON output schema v1 (every field, its type, its canonical unit, optionality,
  `schema_version`) and the config schema v1 (every key, default, accepted values, migration hooks); the rule that
  additive fields are a schema-minor change while removing, renaming or retyping a field is a schema-major change
  that bumps `schema_version`; a worked example document for each.
- [ ] `src/render/json.rs` + `tests/render_json.rs`: a top-level `"schema_version": 1` and stable field names and
  units, pinned by a test that renders a fixture `Report` and asserts the parsed document (round-trip through
  `serde_json`, not a text snapshot), so `docs/schema.md` cannot drift from the code unnoticed.
- [ ] `CHANGELOG.md` (new, Keep a Changelog 1.1.0 + SemVer, SPDX header): an `Unreleased` section plus `0.x`
  entries summarising the user-visible changes of steps 01–11 (backends, formats, localization, cache/offline
  behaviour), each entry naming the CLI surface it affects; the `1.0.0` entry is added in step 14; compare links
  point at GitHub tags.
- [ ] `.github/workflows/release.yml` (new): triggered by tags matching `v[0-9]+.[0-9]+.[0-9]+`, matrix
  `ubuntu-latest` → `x86_64-unknown-linux-gnu`, `ubuntu-24.04-arm` → `aarch64-unknown-linux-gnu`, `macos-latest` →
  `aarch64-apple-darwin`, `macos-13` → `x86_64-apple-darwin`; each job runs `cargo build --release --locked`,
  `cirrocast man > cirrocast.1`, `cirrocast completion bash|zsh|fish`, `cargo test --release --locked`, then packs
  `cirrocast-v$VERSION-$TARGET.tar.gz` containing `cirrocast`, `README.md`, `LICENSE`, `CHANGELOG.md`,
  `cirrocast.1` and `completions/`, writes a `.sha256` per archive and publishes with the preinstalled `gh`
  (`gh release create "$GITHUB_REF_NAME" … --generate-notes`).
- [ ] `publish` job in the same workflow: `cargo package --list` review and `cargo publish --locked` with
  `CARGO_REGISTRY_TOKEN`, gated on the build matrix succeeding and on a protected environment, and a documented
  dry-run path (`cargo publish --dry-run --locked`) that runs for every PR in `ci.yml`.
- [ ] `Cargo.toml` metadata: keep `license = "GPL-3.0-or-later"` with no `license-file` (cargo forbids both),
  `repository`, `readme`, `keywords`, `categories` and `rust-version` present and correct; replace the blunt
  `exclude = ["docs/", ".github/"]` with an explicit `include` list (`src/**`, `locales/**`, `LICENSE`,
  `README.md`, `CHANGELOG.md`, `Cargo.toml`, `Cargo.lock`) so `packaging/`, `docs/`, `.github/` and `tests/`
  stay out of the crate; `Cargo.lock` must be committed because every release path uses `--locked`.
- [ ] crates.io checklist executed and recorded in `README.md#publishing`: name `cirrocast` still free,
  `cargo package --list` shows no `docs/`, no `packaging/`, no `tests/fixtures/` and does show `LICENSE`/`README`,
  `cargo publish --dry-run --locked` passes, `cargo doc --no-deps --all-features` builds (binary target only, so
  module doc comments must carry the description), and `[package.metadata.docs.rs] all-features = true` is set.
- [ ] `packaging/aur/PKGBUILD` (new): `pkgname=cirrocast`, `arch=('x86_64' 'aarch64')`, `license=('GPL-3.0-or-later')`,
  `depends=('gcc-libs' 'glibc')`, `makedepends=('cargo' 'rust')`, `options=('!lto' '!debug')`, source
  `https://github.com/YangtseSu/cirrocast/archive/refs/tags/v$pkgver.tar.gz` with its `sha256sums`; `build()` maps
  `CARCH` (`x86_64` → `x86_64-unknown-linux-gnu`, `aarch64` → `aarch64-unknown-linux-gnu`) and runs
  `cargo build --release --locked --target "$triple"`.
- [ ] Same PKGBUILD `check()`/`package()`: `check()` runs `cargo test --release --locked` and compares
  `cargo metadata --format-version 1`'s package version with `$pkgver` so a stale PKGBUILD fails loudly;
  `package()` installs `usr/bin/cirrocast`, the completions to
  `usr/share/bash-completion/completions/cirrocast`, `usr/share/zsh/site-functions/_cirrocast`,
  `usr/share/fish/vendor_completions.d/cirrocast.fish`, the man page to `usr/share/man/man1/cirrocast.1`, and
  `LICENSE` to `usr/share/licenses/cirrocast/LICENSE` (GPL requires shipping the licence text).
- [ ] `packaging/aur/cirrocast-git/PKGBUILD` (new): `pkgname=cirrocast-git`, `provides=('cirrocast')`,
  `conflicts=('cirrocast')`, `source=('git+https://github.com/YangtseSu/cirrocast.git')`, a `pkgver()` built from
  `git describe --long --tags --abbrev=8` (e.g. `v1.0.0.r12.gabc12345`), `sha256sums=('SKIP')`, the same
  `build`/`check`/`package` bodies minus the version-equality assertion.
- [ ] Both `.SRCINFO` files generated with `makepkg --printsrcinfo` and committed; `REUSE.toml` gains an
  annotation for `packaging/aur/**/.SRCINFO` because the generated file cannot hold a comment header, and both
  PKGBUILDs carry the SPDX header in `#` comments.
- [ ] AUR verification documented in the PKGBUILD header comment and re-run on every bump: `namcap PKGBUILD`,
  `namcap cirrocast-*.pkg.tar.zst`, `makepkg --printsrcinfo | diff - packaging/aur/.SRCINFO`,
  `makepkg -f -i` in a clean chroot (`pkgctl build`), then `cirrocast --version`, `man -w cirrocast` and
  `pacman -Ql cirrocast | grep -c completions` (expect 3).
- [ ] `cargo install` path documented in `README.md#install`: `cargo install --locked cirrocast` (and
  `cargo install --locked --path .` from a checkout), plus the two commands that install what the packages
  place for you — `cirrocast completion <bash|zsh|fish> > <completion path>` and `cirrocast man > <man path>` —
  with the exact target paths per shell.
- [ ] `README.md#packaging` documents all three install paths (source build, cargo, AUR release/`-git`), the
  files each one installs, the release checklist (bump `Cargo.toml` and both PKGBUILDs, regenerate `.SRCINFO`,
  update `CHANGELOG.md`, push a signed `vX.Y.Z` tag, confirm the workflow's archives and the crates.io publish)
  and how to verify a downloaded archive (`sha256sum -c`).
- [ ] The pre-tag checklist is part of the step file: version bumped in `Cargo.toml` + PKGBUILDs, `.SRCINFO`
  regenerated, `CHANGELOG.md` released section dated, `cargo package --list` reviewed, CI green on the tag's
  commit, archive contents inspected (`tar tzf`), and the AUR packages updated after the GitHub release exists.

## Design notes

* Release tooling: **plain `cargo build --release --locked` in a GitHub Actions matrix**, not `cargo-dist`.
  `cargo-dist` can do the job (its `include` option and README/LICENSE auto-includes cover most of the archive
  requirements), but the archives must also contain artefacts generated *at release time by the binary itself*
  (`cirrocast man`, `cirrocast completion …`), which a static `include` list cannot express; we would have to
  commit generated files and add a fidelity check. The hand-written workflow is roughly sixty lines, needs no
  extra tool pinned in CI, keeps the third-party action surface at zero beyond the pinned checkout action, and
  already has to exist for the `publish` job. Revisit if we ever want shell/PowerShell installers (step 24).
* Release archives ship `LICENSE`, `README.md` and `CHANGELOG.md` inside every tarball because GPL-3.0-or-later
  distribution requires the licence text alongside the binary, and the man page plus completions because a tarball
  user has no package manager to install them.
* The crate is a binary-only package: docs.rs still builds and renders the binary target, so no `src/lib.rs` is
  introduced (that would create a public API with its own stability promise we do not need).
* Arch packaging is hand-written rather than generated: the project has no Rust-to-PKGBUILD generator in its
  dependency set, the two packages are small, and a hand-written PKGBUILD keeps the `CARCH` mapping, the licence
  path and the completions paths explicit and reviewable.
* The PKGBUILD does not vendor crates: `cargo build --locked` fetches from crates.io inside `build()`, which is
  the AUR-accepted practice; vendoring would add a second checksum surface for no benefit at this size.

## Out of scope

* Windows, deb/rpm, Homebrew, AppImage/Flatpak, container images and other ecosystem packages: step 24.
* Documentation set beyond README/schema/plan files (guides, website, translations of docs): step 23.
* Performance and size budgets printed in release notes: step 22 (the binary size note in step 14 is informational).
* Code signing, notarization and reproducible-build attestations: no step file owns them; they are recorded in
  step 14's non-goals until a platform requires them.
* Feature work of any kind, including the `serve` mode of step 20 and multi-location output of step 21.

## Verification

```
cargo package --list                                  # no docs/, packaging/, .github/; LICENSE and README present
cargo publish --dry-run --locked                      # succeeds without uploading
cargo doc --no-deps --all-features                    # renders the binary crate without warnings
makepkg --printsrcinfo | diff - packaging/aur/.SRCINFO   # empty diff after a version bump
makepkg -f && namcap PKGBUILD && namcap cirrocast-*.pkg.tar.zst
tar tzf cirrocast-v1.0.0-x86_64-unknown-linux-gnu.tar.gz  # binary, LICENSE, README.md, CHANGELOG.md, man, completions/
sudo pacman -U cirrocast-1.0.0-1-x86_64.pkg.tar.zst && cirrocast --version && man -w cirrocast
```

## Exit criteria

- [ ] `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`, `reuse lint` all clean.
- [ ] `cargo package --list` shows the intended file set and `cargo publish --dry-run --locked` succeeds;
  `docs/schema.md`, `CHANGELOG.md` and `README.md#versioning` agree with each other and with `Cargo.toml`.
- [ ] On an Arch machine (or clean chroot) `makepkg -f` builds from `packaging/aur/PKGBUILD`, `namcap` reports no
  errors, installing the package yields a working `cirrocast --version`, a `man -w cirrocast` hit and three
  installed completion files, and `packaging/aur/cirrocast-git/PKGBUILD` builds with a `pkgver` like
  `v1.0.0.r12.gabc12345`.

## Risks

* GitHub runner drift (the macOS x86_64 runner `macos-13` is being retired) would break one matrix entry: the
  matrix is a one-line change, and the workflow records the fallback (`macos-15-intel`) in a comment.
* `cargo publish` is irreversible: mitigated by a dry-run job on every PR, a protected environment, and publishing
  after the GitHub release artefacts exist.
* AUR rules require `.SRCINFO` to match the PKGBUILD exactly: the diff command above is part of the bump checklist
  and the annotation keeps the generated file lint-clean.
* `--locked` fails in the source tarball if `Cargo.lock` is not committed or if a dependency needs a newer MSRV:
  `Cargo.lock` is committed and step 12's MSRV job runs on the same lockfile.
* The PKGBUILD checksum must be updated for every release: the `check()` version assertion plus the checklist make
  a forgotten bump fail rather than ship the wrong version.

## Progress log

- 2026-09-30 — step file written (status: not-started); `cargo-dist` capability (static `include`, README/LICENSE
  auto-includes) checked before rejecting it in favour of the hand-written workflow.
