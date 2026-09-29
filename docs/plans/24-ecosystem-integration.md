<!--
SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Step 24 — ecosystem integration and packaging

Status: not-started
Depends on: 13 (packaging and release), 20 (wttr-compatible service), 21 (multi-location and templates)
Touches: `src/status.rs`, `src/cli.rs`, `contrib/statusbar/*`, `flake.nix`, `flake.lock`, `Cargo.toml`,
`packaging/homebrew/cirrocast.rb`, `.github/workflows/{ci.yml,release.yml}`, `tests/status_contract.rs`,
`tests/fixtures/cache/`, `docs/ecosystem.md`, `docs/{formats,providers,configuration,performance}.md`,
`README.md`, `REUSE.toml`

## Goal

Other tools can consume cirrocast without parsing prose: `cirrocast status` is a one-line, colourless,
never-aborting probe for status bars with a written contract and runnable examples for waybar, polybar,
i3blocks, tmux, starship and bash/zsh prompts; the JSON, `one-line` and `plain` outputs are frozen
contracts with a breaking-change policy; and the binary is installable beyond Arch — Nix flake,
Homebrew tap, `.deb`, `.rpm`, static musl build, plus verification of the macOS/Windows archives — with
the GPL-3.0-or-later text shipped in every package.

## Deliverables

- [ ] `src/status.rs` + `src/cli.rs`: `cirrocast status [--format <TEMPLATE>] [--location <SPEC>]
      [--max-age <SECS>] [--offline] [--placeholder <TEXT>] [--color never|always]`; `--template` is
      accepted as a synonym of `--format` here; the global `-f/--format <NAME>` enum does not apply to
      `status` (documented in `--help` and `docs/formats.md`).
- [ ] `src/status.rs` output contract: exactly one line plus `\n` (a template newline becomes a space, the
      line is trimmed); colour off unless `--color always`; default template `%c %t`; placeholder `n/a`,
      overridable by `--placeholder` or `[status] placeholder`; `--max-age` defaults to
      `[cache] weather_ttl_secs` (600) and serves a younger entry without revalidating while an older one
      takes the normal fetch path; `--offline` never opens a socket and serves a stale entry.
- [ ] `src/status.rs` exit-code contract: `0` for success **and** for every transient or data failure
      (`Error::Network`, `Upstream`, `LocationNotFound`, `MissingKey` — placeholder on stdout, one
      `error: …` line on stderr); `2` for usage (bad template, unknown flag); `4` for config
      (unreadable config, no location configured). "Never exits non-zero because the network was
      unavailable" is the promise status bars rely on.
- [ ] `src/status.rs` privacy rule: `status` never performs the public-IP lookup; with no location
      argument, `[location] default` or `--location` must supply one, otherwise exit 4 telling the
      user to set it.
- [ ] `contrib/statusbar/`: runnable examples, each with the SPDX header in its own comment syntax —
      `waybar.jsonc` (custom module, `interval` 900), `polybar.ini` (`[module/weather]`), `i3blocks.conf`
      (`interval=900`), `tmux.conf` (`status-interval 900` + `#(…)`), `starship.toml`
      (`[custom.weather]`), and `bash-prompt.sh` / `zsh-prompt.zsh` (both cache the rendered line under
      `$XDG_RUNTIME_DIR/cirrocast/status` and only re-render when it is older than the interval).
- [ ] `contrib/statusbar/verify.sh` + `tests/fixtures/cache/weather/open-meteo-39.90-116.40-3-<date>.json`:
      builds a temp `XDG_CACHE_HOME` from the fixture, runs every example's command with `--offline`, and
      asserts exit 0 plus exactly one line each — no network, so it runs in CI.
- [ ] `tests/status_contract.rs`: single-line guarantee with a template containing `\n`; colour default vs
      `--color always`; placeholder + exit 0 under an injected upstream failure and an offline cache miss;
      `--max-age` freshness; exit 2 for an unknown token; exit 4 with no location configured.
- [ ] `docs/ecosystem.md`: installation and usage per platform (Arch/AUR, Nix, Homebrew, `.deb`, `.rpm`,
      musl tarball, macOS/Windows archives), the status-bar contract, the contrib snippets explained,
      and the **Output contracts** section (JSON schema + version policy, `one-line` token stability,
      `plain` field-order stability, breaking-change policy).
- [ ] Output contracts enforced: `tests/plain_order.rs` snapshot of the field order; `docs/formats.md`
      freezing token meanings; `docs/schema/json-v2.json` current with `json-v1.json` retained
      read-only (step 23); `CHANGELOG.md` `### Breaking` template. A breaking output change requires a
      minor bump, one release of dual emission where feasible, and an entry naming the old and new
      shape.
- [ ] `flake.nix`: `rustPlatform.buildRustPackage` with `cargoLock.lockFile = ./Cargo.lock`,
      `nativeBuildInputs = [ installShellFiles ]`, installing binary, `man/cirrocast.1`, the three
      completion files and `LICENSE` into `$out/share/licenses/cirrocast/`;
      `meta = { license = lib.licenses.gpl3Plus; mainProgram = "cirrocast"; }`; `devShells.default`
      with rustc/cargo/rustfmt/clippy/reuse/cargo-deny/hyperfine/cargo-bloat/cargo-deb/
      cargo-generate-rpm/lychee; committed `flake.lock` (REUSE annotation — JSON holds no header).
- [ ] `Cargo.toml`: `[package.metadata.deb]` and `[package.metadata.generate-rpm]` asset lists (binary,
      man page, bash/zsh/fish completions, licence text), `section = "utils"`, `depends = "$auto"`, no
      library dependency beyond glibc (rustls, not OpenSSL).
- [ ] `packaging/homebrew/cirrocast.rb`: formula for the `yangtse/homebrew-tap` tap —
      `license "GPL-3.0-or-later"`, `bin.install "cirrocast"`, `man1.install "man/cirrocast.1"`,
      `generate_completions_from_executable(bin/"cirrocast", "completion", base_name: "cirrocast")`,
      `pkgshare.install "LICENSE"`, `sha256` per release asset, `brew install yangtse/tap/cirrocast`.
- [ ] Static musl build: `x86_64-unknown-linux-musl` in CI (`musl-tools`), archived with man page,
      completions and `LICENSE`, verified by `file` ("statically linked"), `ldd` ("not a dynamic
      executable") and an offline fixture-backed run in a scratch directory; its ≤ 6 MB budget (separate
      from step 22's 5 MB glibc budget) is recorded in `docs/performance.md`.
- [ ] Release-archive verification for macOS (`aarch64-apple-darwin`, `x86_64-apple-darwin`) and Windows
      (`x86_64-pc-windows-msvc`): a `release.yml` step unpacks the archive, asserts `LICENSE` is present,
      runs `--version`, `--help` and an offline fixture-cache run. The macOS archives are unsigned and
      un-notarised and the Windows binary unsigned, with the user-visible consequence (Gatekeeper /
      SmartScreen prompt) documented.
- [ ] `.github/workflows/ci.yml`: job `ecosystem` (`cachix/install-nix-action@v31`, `nix build -L
      .#default`, `./result/bin/cirrocast --version`); job `packages` (`cargo install cargo-deb
      cargo-generate-rpm --locked`, `dpkg-deb --info/--contents`, `dpkg-deb -x` into a temp root then
      run `--version`, `rpm -qip`/`rpm -qpl`, extraction via `rpm2cpio | cpio -idm` then run
      `--version`); job `statusbar` (`contrib/statusbar/verify.sh`).
- [ ] Declined, with reasons in `docs/ecosystem.md`: **container images** (the only long-running mode is
      the loopback-by-default `serve`, whose security model assumes a host; an image invites
      `--allow-remote` exposure and adds a pipeline plus GPL source-offer surface), **Snap** (confinement
      blocks `$XDG_CACHE_HOME` writes and the `serve` bind; Ubuntu-only), **Flatpak** (desktop sandbox,
      portal restrictions, no desktop integration to gain), **editor plugins** (third parties shell out to
      the CLI), **MSI/Chocolatey/Scoop** (deferred: the Windows archive plus a `PATH` step suffices).
- [ ] `README.md` + `docs/formats.md` + `docs/providers.md`: pointer to `docs/ecosystem.md`, the status
      snippet, and the output-contract summary linked from the formats doc.

## Design notes

* **`status` is not a second renderer.** It calls the step 21 template engine, the step 05 cache and the
  provider chain with the same options a normal run uses, plus a freshness override (`--max-age`) and the
  failure policy; a dedicated minimal fetch path would need its own cache handling and decoding.
* **`--max-age` versus the cache TTL.** The TTL decides whether an entry is *valid*; `--max-age` decides
  whether it is *fresh enough to skip the network for*. A bar refreshing every 15 min against a 10 min
  TTL would otherwise revalidate every refresh; both knobs are documented together in the config doc.
* **Never non-zero on transient failure.** A bar must render something stable while the network is down;
  a non-zero exit would make it drop the module or show an error where no stderr is visible, so only
  permanent usage and config problems fail.
* **Colour default.** waybar and polybar strip ANSI, tmux `#()` output does not reliably, so `status`
  defaults to `never` and only `--color always` opts in.
* **Nix flake rather than a nixpkgs submission.** A flake works the day it lands, is reproducible from
  its committed, REUSE-annotated `flake.lock`, gates our CI, and needs no network in the sandbox.
* **Homebrew via a tap, not core.** Core needs notability and maintainer review; the tap works from one reviewed file.
* **`cargo-deb`/`cargo-generate-rpm` over hand-written trees.** Both read metadata from `Cargo.toml`
  (MIT; 3.8 and 0.21), so no `debian/` tree or spec file has to track every version bump.
* **musl for x86_64 only.** aarch64 musl needs a cross toolchain (`cross` pulls a banned container,
  `cargo-zigbuild` adds zig for one archive), so aarch64 ships as a glibc build, documented as such.
* **Unsigned artifacts are stated, not hidden.** Notarisation needs a paid Apple account; the doc names the prompt to expect.
## Out of scope

nixpkgs, Homebrew core, Debian/Ubuntu/Fedora official repositories (artifacts plus the step 13 PKGBUILD
are the deliverable; distro inclusion is their process), container images, Snap/Flatpak, editor plugins,
code-signing certificates, an auto-updater, and any usage telemetry (forbidden by the privacy rule).

## Verification

```bash
cargo run -q -- status --location Beijing --format '%c %t' --max-age 900 ; echo $?
cargo run -q -- status --location Beijing --offline --placeholder '-' ; echo $?          # cached → 0
XDG_CACHE_HOME=/tmp/empty cargo run -q -- status --location Beijing --offline ; echo $?  # n/a, 0
cargo run -q -- status --format '%y' ; echo $?                                          # exit 2
contrib/statusbar/verify.sh                                                             # offline
nix build -L .#default && ./result/bin/cirrocast --version && ls result/share/licenses/cirrocast/
cargo deb && dpkg-deb -c target/debian/cirrocast_*.deb | grep -E 'bin/|man1/|licenses/'
cargo build --release --target x86_64-unknown-linux-musl && file target/x86_64-unknown-linux-musl/release/cirrocast
```

Observable result: `status` prints one line and exits 0 in every offline/placeholder case and 2 on the
bad token; `verify.sh` reports one line per example with exit 0; `nix build` yields a store path with
binary, man page, completions and licence text; the `.deb`/`.rpm` listings contain `/usr/bin/cirrocast`,
the man page and the licence text at the platform's canonical location; `file` reports the musl binary
as statically linked.

## Exit criteria

- [ ] `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`, `reuse lint` clean.
- [ ] `status` contract covered by `tests/status_contract.rs`: single line, colour default, placeholder with
      exit 0, `--max-age` freshness, exit 2 and exit 4 cases.
- [ ] Six status-bar examples plus two prompt snippets exist, run as written, and pass
      `contrib/statusbar/verify.sh` against the committed fixture cache.
- [ ] `nix build -L .#default` succeeds from a clean checkout with the committed `flake.lock`; CI `ecosystem` green.
- [ ] `cargo deb`/`cargo generate-rpm` packages contain binary, man page, completions and the licence text;
      the CI `packages` job extracts and runs both.
- [ ] The musl build is static per `file`/`ldd`, runs offline from the fixture cache, and its size is
      recorded against the 6 MB budget.
- [ ] macOS and Windows archives are unpacked and exercised by `release.yml`; a missing `LICENSE` or a
      failed `--version` fails the release.
- [ ] `docs/ecosystem.md` documents every install path, the licence location per package, the output contracts,
      the breaking-change policy and the declined options.

## Risks

* Status-bar markup differs per tool (waybar's `{}`, others' bare output); mitigated by executing the exact examples in CI.
* Homebrew on unsupported architectures builds from source, needing Rust on the user's machine;
  documented, with the prebuilt archives as the alternative.
* Brew tap drift after a release (stale `sha256`); mitigated by computing checksums in the release job
  and by `brew audit --strict` plus `brew install --build-from-source` in the tap repository's CI.
* rpm extraction in CI needs `rpm2cpio` and `cpio`; the job installs the distro packages when absent —
  no container images are pulled.
* Unsigned artifacts warn on macOS/Windows; the doc states it and the archives stay checksum-verifiable.

## Progress log

- 2026-09-30 — step opened: status contract (single line, colour, exit codes, `--max-age`), output
  contracts, packaging matrix and declined options fixed; Homebrew/Nix/deb/rpm/musl paths decided.
