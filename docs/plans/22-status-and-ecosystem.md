<!--
SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Step 22 — status probe, ecosystem recipes and output contracts

Status: ⬜ not-started
Depends on: 13 (packaging and release), 19 (multi-location and templates)
Touches: `src/status.rs`, `src/cli.rs`, `contrib/statusbar/*`, `.github/workflows/ci.yml`,
`tests/status_contract.rs`, `tests/plain_order.rs`, `tests/fixtures/cache/`, `docs/ecosystem.md`,
`docs/{formats,providers,configuration}.md`, `docs/plans/README.md`, `README.md`, `CHANGELOG.md`,
`REUSE.toml`

## Goal

Other tools can consume cirrocast without parsing prose: `cirrocast status` is a one-line,
colourless, never-aborting probe for status bars with a written contract and runnable examples for
waybar, polybar, i3blocks, tmux, starship and bash/zsh prompts; and the JSON, `one-line` and `plain`
outputs are frozen contracts with a written breaking-change policy. Packaging beyond the AUR package
and the release archives of step 13 is deliberately out of scope here — the multi-platform matrix
lives in backlog B02.

## Deliverables

- ⬜ `src/status.rs` + `src/cli.rs`: `cirrocast status [--format <TEMPLATE>] [--location <SPEC>]
      [--max-age <SECS>] [--offline] [--placeholder <TEXT>] [--color never|always]`; `--template` is
      accepted as a synonym of `--format` here; the global `-f/--format <NAME>` enum does not apply to
      `status` (documented in `--help` and `docs/formats.md`).
- ⬜ `src/status.rs` output contract: exactly one line plus `\n` (a template newline becomes a space, the
      line is trimmed); colour off unless `--color always`; default template `%c %t`; placeholder `n/a`,
      overridable by `--placeholder` or `[status] placeholder`; `--max-age` defaults to
      `[cache] weather_ttl_secs` (600) and serves a younger entry without revalidating while an older one
      takes the normal fetch path; `--offline` never opens a socket and serves a stale entry.
- ⬜ `src/status.rs` exit-code contract: `0` for success **and** for every transient or data failure
      (`Error::Network`, `Upstream`, `LocationNotFound`, `MissingKey` — placeholder on stdout, one
      `error: …` line on stderr); `2` for usage (bad template, unknown flag); `4` for config
      (unreadable config, no location configured). "Never exits non-zero because the network was
      unavailable" is the promise status bars rely on.
- ⬜ `src/status.rs` privacy rule: `status` never performs the public-IP lookup; with no location
      argument, `[location] default` or `--location` must supply one, otherwise exit 4 telling the
      user to set it.
- ⬜ `contrib/statusbar/`: runnable examples, each with the SPDX header in its own comment syntax —
      `waybar.jsonc` (custom module, `interval` 900), `polybar.ini` (`[module/weather]`), `i3blocks.conf`
      (`interval=900`), `tmux.conf` (`status-interval 900` + `#(…)`), `starship.toml`
      (`[custom.weather]`), and `bash-prompt.sh` / `zsh-prompt.zsh` (both cache the rendered line under
      `$XDG_RUNTIME_DIR/cirrocast/status` and only re-render when it is older than the interval).
- ⬜ `contrib/statusbar/verify.sh` + `tests/fixtures/cache/weather/open-meteo-39.90-116.40-3-<date>.json`:
      builds a temp `XDG_CACHE_HOME` from the fixture, runs every example's command with `--offline`, and
      asserts exit 0 plus exactly one line each — no network, so it runs in CI.
- ⬜ `tests/status_contract.rs`: single-line guarantee with a template containing `\n`; colour default vs
      `--color always`; placeholder + exit 0 under an injected upstream failure and an offline cache miss;
      `--max-age` freshness; exit 2 for an unknown token; exit 4 with no location configured.
- ⬜ `docs/ecosystem.md`: the status-bar contract, the contrib snippets explained, and the **Output
      contracts** section (JSON schema + version policy, `one-line` token stability, `plain` field-order
      stability, breaking-change policy). The install-per-platform half of this document is backlog
      B02's; until then the file documents the AUR package and the release archives of step 13.
- ⬜ Output contracts enforced: `tests/plain_order.rs` snapshot of the field order; `docs/formats.md`
      freezing token meanings; `docs/schema/json-v2.json` current with `json-v1.json` retained
      read-only (step 28); `CHANGELOG.md` `### Breaking` template. A breaking output change requires a
      minor bump, one release of dual emission where feasible, and an entry naming the old and new
      shape.
- ⬜ `.github/workflows/ci.yml`: job `statusbar` (`contrib/statusbar/verify.sh`, offline against the
      committed fixture cache).
- ⬜ `README.md` + `docs/formats.md` + `docs/providers.md`: pointer to `docs/ecosystem.md`, the status
      snippet, and the output-contract summary linked from the formats doc.

## Design notes

* **`status` is not a second renderer.** It calls the step 19 template engine, the step 05 cache and the
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
* **A breaking output change is a release event, not a commit.** The JSON schema version, the token
  meanings and the `plain` field order change only with a minor bump, one release of dual emission
  where feasible, and a `CHANGELOG.md` entry naming both shapes; the mechanics of cutting that release
  stay in step 13's checklist.

## Out of scope

The multi-platform packaging matrix — Nix flake, Homebrew tap, `.deb`/`.rpm` metadata, the static musl
archive, macOS/Windows archive verification and the declined-container-options write-up — is backlog
B02, deferred on 2026-10-04 because the release target is Linux, mainly Arch. Also out of scope here:
localising the status contract, a `--watch`/interval mode (the examples re-run the process), and any
usage telemetry (forbidden by the privacy rule).

## Verification

```bash
cargo run -q -- status --location Beijing --format '%c %t' --max-age 900 ; echo $?
cargo run -q -- status --location Beijing --offline --placeholder '-' ; echo $?          # cached → 0
XDG_CACHE_HOME=/tmp/empty cargo run -q -- status --location Beijing --offline ; echo $?  # n/a, 0
cargo run -q -- status --format '%y' ; echo $?                                          # exit 2
contrib/statusbar/verify.sh                                                             # offline
```

Observable result: `status` prints one line and exits 0 in every offline/placeholder case and 2 on the
bad token; `verify.sh` reports one line per example with exit 0; the fixture-backed cache makes every
check network-free.

## Exit criteria

- ⬜ `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`, `reuse lint` clean.
- ⬜ `status` contract covered by `tests/status_contract.rs`: single line, colour default, placeholder with
      exit 0, `--max-age` freshness, exit 2 and exit 4 cases.
- ⬜ Six status-bar examples plus two prompt snippets exist, run as written, and pass
      `contrib/statusbar/verify.sh` against the committed fixture cache.
- ⬜ `docs/ecosystem.md` documents the status-bar contract, the output contracts and the breaking-change
      policy (the install-path sections arrive with backlog B02).
- ⬜ The `plain` field-order snapshot and the `one-line` token meanings are pinned by tests, and the
      `CHANGELOG.md` `### Breaking` template exists.

## Risks

* Status-bar markup differs per tool (waybar's `{}`, others' bare output); mitigated by executing the
  exact examples in CI.
* Contract drift: a renderer change can silently break a consumer; mitigated by the `plain` order
  snapshot, the token table test of step 19 and the documented dual-emission policy.
* The examples follow each bar's own configuration format, which moves; the versions exercised are
  recorded in `docs/ecosystem.md` and the CI job runs the committed files, not the upstream docs.

## Progress log

- 2026-09-30 — step opened (as step 24, "ecosystem integration and packaging"): status contract (single
  line, colour, exit codes, `--max-age`), output contracts and the packaging matrix were fixed together.
- 2026-10-04 — split and renumbered to step 22 by the plan reorganization: the packaging half (Nix,
  Homebrew, deb/rpm, musl, macOS/Windows verification, declined container options) moved to backlog
  B02, so this step keeps only the status probe, the status-bar recipes and the output contracts; the
  `wttr-compatible service` dependency is gone with it (B01 is backlog).
