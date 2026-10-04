<!--
SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
SPDX-License-Identifier: GPL-3.0-or-later
-->

# B02 — multi-platform packaging matrix (backlog)

Status: ⏸ backlog — deferred on 2026-10-04: the release target is Linux, mainly Arch (the AUR
package and the release archives of step 13); other distributions are not considered for now, and
macOS keeps its existing GitHub archive with no expansion planned. Not scheduled, and it may be
dropped; scheduling it means renumbering it into the A-series tail first (`docs/plans/README.md`).
Depends on: 13 (packaging and release), 21 (performance and resource budget), 22 (status probe,
ecosystem recipes and output contracts)
Touches: `flake.nix`, `flake.lock`, `Cargo.toml`, `packaging/homebrew/cirrocast.rb`,
`.github/workflows/{ci.yml,release.yml}`, `docs/ecosystem.md`, `docs/performance.md`, `README.md`,
`REUSE.toml`

## Goal

The binary is installable beyond the Arch package and the plain release archives: a Nix flake, a
Homebrew tap (for the existing macOS archives), `.deb` and `.rpm` metadata read from `Cargo.toml`, a
static x86_64 musl archive, and automated unpack-and-run verification of the macOS/Windows release
archives — with the GPL-3.0-or-later text shipped in every package and the declined options
(container images, Snap, Flatpak, editor plugins, MSI/Chocolatey/Scoop) documented with reasons.

## Deliverables

- ⬜ `flake.nix`: `rustPlatform.buildRustPackage` with `cargoLock.lockFile = ./Cargo.lock`,
      `nativeBuildInputs = [ installShellFiles ]`, installing binary, `man/cirrocast.1`, the three
      completion files and `LICENSE` into `$out/share/licenses/cirrocast/`;
      `meta = { license = lib.licenses.gpl3Plus; mainProgram = "cirrocast"; }`; `devShells.default`
      with rustc/cargo/rustfmt/clippy/reuse/cargo-deny/hyperfine/cargo-bloat/cargo-deb/
      cargo-generate-rpm/lychee; committed `flake.lock` (REUSE annotation — JSON holds no header).
- ⬜ `Cargo.toml`: `[package.metadata.deb]` and `[package.metadata.generate-rpm]` asset lists (binary,
      man page, bash/zsh/fish completions, licence text), `section = "utils"`, `depends = "$auto"`, no
      library dependency beyond glibc (rustls, not OpenSSL).
- ⬜ `packaging/homebrew/cirrocast.rb`: formula for the `yangtse/homebrew-tap` tap —
      `license "GPL-3.0-or-later"`, `bin.install "cirrocast"`, `man1.install "man/cirrocast.1"`,
      `generate_completions_from_executable(bin/"cirrocast", "completion", base_name: "cirrocast")`,
      `pkgshare.install "LICENSE"`, `sha256` per release asset, `brew install yangtse/tap/cirrocast`.
- ⬜ Static musl build: `x86_64-unknown-linux-musl` in CI (`musl-tools`), archived with man page,
      completions and `LICENSE`, verified by `file` ("statically linked"), `ldd` ("not a dynamic
      executable") and an offline fixture-backed run in a scratch directory; its ≤ 6 MB budget (separate
      from step 21's 5 MB glibc budget) is recorded in `docs/performance.md`.
- ⬜ Release-archive verification for macOS (`aarch64-apple-darwin`, `x86_64-apple-darwin`) and Windows
      (`x86_64-pc-windows-msvc`): a `release.yml` step unpacks the archive, asserts `LICENSE` is present,
      runs `--version`, `--help` and an offline fixture-cache run. The macOS archives are unsigned and
      un-notarised and the Windows binary unsigned, with the user-visible consequence (Gatekeeper /
      SmartScreen prompt) documented. (The macOS archives themselves already exist; this bullet adds
      verification, not a new target, and no expansion of the target matrix is planned.)
- ⬜ `.github/workflows/ci.yml`: job `ecosystem` (`cachix/install-nix-action@v31`, `nix build -L
      .#default`, `./result/bin/cirrocast --version`); job `packages` (`cargo install cargo-deb
      cargo-generate-rpm --locked`, `dpkg-deb --info/--contents`, `dpkg-deb -x` into a temp root then
      run `--version`, `rpm -qip`/`rpm -qpl`, extraction via `rpm2cpio | cpio -idm` then run
      `--version`).
- ⬜ `docs/ecosystem.md`: the installation and usage half — Arch/AUR, Nix, Homebrew, `.deb`, `.rpm`,
      musl tarball, macOS/Windows archives — the licence location per package, and the declined options
      with reasons. (Step 22 authored the status-bar and output-contract half; this step extends the
      same file.)
- ⬜ `README.md`: install-path table rows for the new artefacts.

## Design notes

* **Nix flake rather than a nixpkgs submission.** A flake works the day it lands, is reproducible from
  its committed, REUSE-annotated `flake.lock`, gates our CI, and needs no network in the sandbox.
* **Homebrew via a tap, not core.** Core needs notability and maintainer review; the tap works from one
  reviewed file, and it packages the archives the release workflow already builds.
* **`cargo-deb`/`cargo-generate-rpm` over hand-written trees.** Both read metadata from `Cargo.toml`
  (MIT; 3.8 and 0.21), so no `debian/` tree or spec file has to track every version bump.
* **musl for x86_64 only.** aarch64 musl needs a cross toolchain (`cross` pulls a banned container,
  `cargo-zigbuild` adds zig for one archive), so aarch64 ships as a glibc build, documented as such.
* **Unsigned artifacts are stated, not hidden.** Notarisation needs a paid Apple account; the doc names
  the prompt to expect.
* **Why this is backlog.** The project's own release path (cargo install, the AUR package, the GitHub
  archives) covers the stated target of Linux/Arch users; each extra channel is a CI job and a
  support surface with no current demand. This file keeps the research so the work is ready if that
  demand appears.

## Out of scope

nixpkgs, Homebrew core, Debian/Ubuntu/Fedora official repositories (the artifacts plus the AUR package
step 13 created are the deliverable; distro inclusion is their process), container images, Snap/Flatpak,
editor plugins, code-signing certificates, an auto-updater, further target triples beyond the ones
named above, and any usage telemetry (forbidden by the privacy rule).

## Verification

```bash
nix build -L .#default && ./result/bin/cirrocast --version && ls result/share/licenses/cirrocast/
cargo deb && dpkg-deb -c target/debian/cirrocast_*.deb | grep -E 'bin/|man1/|licenses/'
cargo generate-rpm && rpm -qpl target/generate-rpm/cirrocast-*.rpm | grep -E 'bin/|man1/|licenses/'
cargo build --release --target x86_64-unknown-linux-musl && file target/x86_64-unknown-linux-musl/release/cirrocast
```

Observable result: `nix build` yields a store path with binary, man page, completions and licence text;
the `.deb`/`.rpm` listings contain `/usr/bin/cirrocast`, the man page and the licence text at the
platform's canonical location; `file` reports the musl binary as statically linked and it runs offline
from the fixture cache.

## Exit criteria

- ⬜ `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`, `reuse lint` clean.
- ⬜ `nix build -L .#default` succeeds from a clean checkout with the committed `flake.lock`; CI `ecosystem` green.
- ⬜ `cargo deb`/`cargo generate-rpm` packages contain binary, man page, completions and the licence text;
      the CI `packages` job extracts and runs both.
- ⬜ The musl build is static per `file`/`ldd`, runs offline from the fixture cache, and its size is
      recorded against the 6 MB budget.
- ⬜ macOS and Windows archives are unpacked and exercised by `release.yml`; a missing `LICENSE` or a
      failed `--version` fails the release.
- ⬜ `docs/ecosystem.md` documents every install path, the licence location per package and the declined
      options.

## Risks

* Homebrew on unsupported architectures builds from source, needing Rust on the user's machine;
  documented, with the prebuilt archives as the alternative.
* Brew tap drift after a release (stale `sha256`); mitigated by computing checksums in the release job
  and by `brew audit --strict` plus `brew install --build-from-source` in the tap repository's CI.
* rpm extraction in CI needs `rpm2cpio` and `cpio`; the job installs the distro packages when absent —
  no container images are pulled.
* Unsigned artifacts warn on macOS/Windows; the doc states it and the archives stay checksum-verifiable.
* Every added channel is maintenance in perpetuity; the matrix is therefore opt-in per channel and may
  be trimmed after a release or two of data.

## Progress log

- 2026-10-04 — opened as backlog B02 from the packaging half of the old step 24 ("ecosystem
  integration and packaging"): deferred because the release target is Linux, mainly Arch, and the
  macOS archives need no expansion. The status-probe and output-contract half stayed in the A-series
  as step 22.
