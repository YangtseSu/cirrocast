<!--
SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Step 01 — project scaffold

Status: ✅ done
Depends on: —
Touches: `Cargo.toml`, `rust-toolchain.toml`, `.gitignore`, `LICENSE`, `LICENSES/`, `REUSE.toml`,
`README.md`, `AGENTS.md`, `src/{main,lib,cli,error,paths}.rs`, `src/provider/mod.rs`, `tests/cli.rs`,
`docs/plans/`

## Goal

A compiling, tested, lint-clean cargo project that already has the parts which are independent of any
weather source: the clap command surface for `config`/`provider`, typed errors with stable exit codes,
XDG path resolution, and the provider registry metadata that later steps consume. It must be possible
to install the binary and get useful, non-placeholder answers (`--version`, `provider list`,
`config path`) before any HTTP code exists.

## Deliverables

- ✅ `Cargo.toml`: package metadata (`cirrocast`, edition 2024, MSRV 1.85), `license = "GPL-3.0-or-later"`,
      deps `clap` (derive/env/wrap_help), `etcetera`, `thiserror`; dev-deps `assert_cmd`, `predicates`,
      `tempfile`; `unsafe_code = "forbid"`, clippy `all = deny`, `pedantic = warn`.
- ✅ `rust-toolchain.toml` (stable + rustfmt + clippy), `.gitignore`, SPDX-tagged `LICENSE` (GPL-3.0-or-later
      full text) with `LICENSES/GPL-3.0-or-later.txt` as the REUSE-visible link, `REUSE.toml` for
      `Cargo.lock` and `tests/fixtures/**`.
- ✅ `src/lib.rs` + `src/main.rs`: library target for testability; `main` parses argv, dispatches, prints
      `error: …` on stderr and returns the mapped exit code.
- ✅ `src/error.rs`: `Error` enum (`Usage`, `Network`, `Upstream`, `Config`, `LocationNotFound`,
      `MissingKey`, `Other` for the generic exit code), `Result` alias, `exit_code()` (1 generic / 2 usage /
      3 network+upstream / 4 config / 5 location / 6 missing key) with unit tests.
- ✅ `src/paths.rs`: `Paths` resolved through `etcetera` honouring `XDG_CONFIG_HOME`, `XDG_CACHE_HOME`,
      `XDG_DATA_HOME` with the documented fallbacks; no side effects.
- ✅ `src/provider/mod.rs`: `ProviderId` (8 backends), `LocationKinds`, `ProviderMeta`, `metadata()`,
      `all()`, `FromStr` — data only, marked as declared-from-docs.
- ✅ `src/cli.rs`: `cirrocast config path`, `cirrocast provider list|info <ID>`; global `-v/--verbose`
      (count) and `-q/--quiet`; no weather flags yet.
- ✅ `tests/cli.rs`: version/help/exit-code/provider-table/XDG-override integration tests.
- ✅ `docs/plans/README.md` (contract + step index) and this file with progress markers.
- ✅ `AGENTS.md` (repo operating manual: rules, commands, plan workflow, recipes) and `README.md`
      (what/why, status, usage, backend matrix, licence).

## Design notes

* **No speculative weather flags.** `--provider`, `--format`, `--units`, `--days` … are added by the
  step that implements them (06/08). Declaring flags whose code paths return "not implemented" would
  ship a stub CLI; the flag matrix is documented in the contract and lands with its behaviour.
* **`lib.rs` + `main.rs`** instead of a single binary: integration tests and (later) doctests need to
  call into the crate; `main` stays a thin shell that only maps argv to a call and `Error` to an exit code.
* **`thiserror` and not `anyhow`**: exit codes require categorising errors, which an opaque `anyhow::Error`
  cannot do. No `anyhow` in the dependency tree.
* **`etcetera` and not `dirs`**: `dirs` ignores `XDG_CONFIG_DIRS`; `etcetera` implements the XDG base
  directory spec including the read-only search path, which we need for reading system-wide config.
* **No async runtime, no `reqwest`**: the client makes a handful of sequential requests per run, so an
  async runtime buys nothing and costs binary size and startup time. HTTP arrives in step 05 on `ureq`.
* **Provider metadata lives in the binary** (not in docs only) so `provider list`/`provider info` never
  drift from what the code can actually do. The declared limits are explicitly flagged as
  taught-from-docs and are re-verified in step 10.
* **REUSE layout**: GitHub only detects `./LICENSE`, while `reuse lint` requires the licence text under
  `LICENSES/`; the link keeps one copy of the text and satisfies both. Known consequence: a Windows
  checkout turns the link into a text file, which is accounted for in step 12's CI matrix.

## Out of scope

Config file loading/validation and the BYOK key store (step 02), the data model and units (step 03),
geocoding (step 04), HTTP/cache/IP location (step 05), any real backend (step 06+), rendering (step
07+), fill-in completions and man pages (step 08).

## Verification

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
reuse lint

cargo run -q -- --version                       # cirrocast 0.1.0
cargo run -q -- provider list                   # 8 rows, KEY column shows none/env vars
cargo run -q -- provider info open-meteo        # keyless capabilities
cargo run -q -- provider info nope; echo $?     # usage error on stderr, exit 2
XDG_CONFIG_HOME=/tmp/cc cargo run -q -- config path   # /tmp/cc/cirrocast
```

## Exit criteria

- ✅ `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`, `reuse lint` clean.
- ✅ `cirrocast provider list` prints all eight backends with key requirements, from the registry.
- ✅ `cirrocast config path` follows `XDG_CONFIG_HOME`.
- ✅ Exit codes observable: `provider info nope` → 2; `--help` → 0.
- ✅ Repository initialised with git, private GitHub remote configured, first commit pushed.
- ✅ No `unwrap`/`expect`/`panic!` outside tests; no placeholder output anywhere.

## Risks

* Registry capability numbers (days, key requirements, coverage) are from documentation and may be wrong;
  mitigated by the explicit re-verification task in step 10.
* The `LICENSES/` link is not portable to Windows checkouts; mitigated by the CI decision in step 12.
* MSRV 1.85 is declared but only exercised once MSRV CI exists (step 12).

## Progress log

- 2026-09-30 — step opened: cargo project initialised, licence switched to GPL-3.0-or-later with the
  REUSE layout, contract written in `docs/plans/README.md`, scaffold implementation delegated.
- 2026-09-30 — step done: `cargo fmt --check` clean, `cargo clippy --all-targets -- -D warnings` clean,
  `cargo test` 9 unit + 5 integration passing, `reuse lint` 38/38 files compliant. Smoke: `--version` →
  `cirrocast 0.1.0`; `provider list` → 8 aligned rows with key env vars; `provider info metar` → station-only
  capabilities; `XDG_CONFIG_HOME=/tmp/cc config path` → `/tmp/cc/cirrocast`; `provider info nope` → exit 2
  with the known-provider list. Git repository initialised on `main` with a private GitHub remote.
- 2026-09-30 — registry rows corrected by step 10's re-verification pass: `ProviderMeta` gained
  `verified` (printed by `provider info`), SMHI's `docs_url` points at the live SNOW1gv1 docs, WWO's
  `max_days` dropped from 3 to 5 and its `docs_url` moved to the Local Weather API page, QWeather's note
  no longer calls the backend China-focused, and the free-tier headlines of Open-Meteo, OWM, WeatherAPI
  and PirateWeather were added. Evidence per row in `docs/providers.md`; the rows themselves stay
  "declared, not measured" until their backends land.
