<!--
SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Step 28 — documentation set and repo hygiene

Status: ⬜ not-started
Depends on: 14 (v1 acceptance), 19 (multi-location and templates); its generated artifacts snapshot
the surface of every other scheduled step (18–27), so it lands last in the A-series
Touches: `docs/{getting-started,configuration,providers,formats,location,i18n,troubleshooting,
architecture}.md`, `docs/reference/{help-long.txt,flags.txt}`, `docs/schema/{json-v1.json,
json-v2.json}`, `docs/screenshots/*.txt`, `man/cirrocast.1`, `CONTRIBUTING.md`, `SECURITY.md`,
`README.md`, `.github/ISSUE_TEMPLATE/{bug,feature,provider-request}.yml`,
`.github/ISSUE_TEMPLATE/config.yml`, `.github/workflows/ci.yml`, `lychee.toml`,
`tests/{help_snapshot,docs_flags,json_schema}.rs`, `tests/fixtures/json/`, `Cargo.toml`, `REUSE.toml`

## Goal

Every claim a user can act on has exactly one authoritative document under `docs/`, and the
machine-readable artifacts cannot drift from the binary: the man page and the long `--help` are
generated and snapshot-tested, the flag list in the docs is compared with the real `--help` by a test,
and the `json` output is validated against a committed JSON Schema on every run. The repo grows the
files a stranger needs to contribute (build/test/provider recipe, REUSE rule, commit convention,
issue templates, security policy) and a link checker keeps them honest.

## Deliverables

- ⬜ `docs/getting-started.md`: install (the AUR package and the release archives; further package
      formats are backlog B02), first run, `--help` tour, config init, the keyless backends, setting
      a key for one keyed backend, offline mode.
- ⬜ `docs/configuration.md`: every config key with type, default, example and effect; the two new
      tables (`[locations]`, `[templates]`); the precedence table (CLI flag > env var > project
      config? — no: `CIRROCAST_*` env > `$XDG_CONFIG_HOME` user config > `XDG_CONFIG_DIRS` system
      config > built-in default, with keys resolved separately per the key store rules).
- ⬜ `docs/providers.md`: per provider — auth (env var name, key store, keyless), documented rate
      limits and what we do on 429/5xx, coverage and `max_days`, attribution requirement, accuracy
      caveats, and the WMO-mapping note. (Authored in step 10 as the registry re-verification record —
      endpoints, quotas with their wording, licence duties, traps; this step reviews it against the
      shipped binary and adds anything the implementation learned.)
- ⬜ `docs/formats.md`: every format (`art-table`, `one-line`, `full`, `minimal`, `plain`, `json`,
      `dumb`) with a real captured example block, the full `%` token table (identical to
      `template::TOKENS`), width/precision syntax, escape rules, and the unknown-token policy per
      context.
- ⬜ `docs/location.md`: fuzzy vs `:exact` vs `~osm` vs `@lat,lon` vs alias vs `--ip` vs `--station`
      syntax, and the deterministic ranking rules (population, then exact-name, then provider order)
      with a worked ambiguous example.
- ⬜ `docs/i18n.md`: how to add a language (copy `locales/en-US/main.ftl`, translate every key, run
      `cargo test i18n`), what the completeness test covers (every WMO condition key), `lang = "auto"`
      negotiation, and the rule that no translated string may be constructed by concatenation.
- ⬜ `docs/troubleshooting.md`: network/TLS/proxy failures, missing or unreadable keys (including the
      `0600` refusal and `key list` masking), cache corruption and how to clear it, provider rate
      limits, wrong city, absent alerts, offline mode, and how to produce a bug report
      (`-vv` output with secrets redacted).
- ⬜ `docs/architecture.md`: module map, request data flow as a mermaid diagram (argv → cli → geo →
      provider → cache → model → render → stdout), the `serve` thread model, on-disk state, and the
      invariants (metric-SI storage, single conversion point, cache unit-independence).
- ⬜ `docs/performance.md` reviewed against the binary (authored in step 21) and linked from
      `README.md` and the man page; `docs/wttr-compat.md` is reviewed and linked only if backlog B01
      has landed (that file is authored there).
- ⬜ Man page: `man/cirrocast.1` generated from the clap definitions (step 08), reviewed line by line
      against `--help`; `man --warn --local-file man/cirrocast.1 > /dev/null` is silent; the page gains
      EXIT STATUS (0–6), ENVIRONMENT (`CIRROCAST_*`, `NO_COLOR`, `CLICOLOR_FORCE`, `XDG_*`,
      `HTTPS_PROXY`) and SEE ALSO (`cirrocast(1)`, the docs directory).
- ⬜ `docs/reference/help-long.txt` + `tests/help_snapshot.rs`: the long `--help` output is the
      canonical flag reference, compared byte-for-byte with the snapshot and regenerated only by
      `cargo test -- --ignored regenerate_help` when a flag change is intended.
- ⬜ `docs/reference/flags.txt` (one long flag per line, sorted) + `tests/docs_flags.rs`: set
      equality between the file and the flags parsed out of `--help`, plus "every `` `--flag` `` token
      that appears in `docs/**/*.md` and `README.md` is in `--help`", and a duplicate/sort check.
- ⬜ `docs/schema/json-v1.json` (frozen: the single-location object with `"schema_version": 1` that
      v1.0.0 printed) and `docs/schema/json-v2.json` (current: `oneOf` a report object and an array of
      at least two report objects, `"schema_version": 2`), both Draft 2020-12 with `$defs` shared by
      `$ref`.
- ⬜ `tests/json_schema.rs`: dev-dependency `jsonschema = { version = "0.58", default-features =
      false }`; validates (a) the committed v1 fixture `tests/fixtures/json/v1-detroit.json` against
      `json-v1.json` and (b) live, fixture-backed output for one, two and three locations against
      `json-v2.json`, including a failed-slot error object.
- ⬜ `CONTRIBUTING.md`: build/test commands, the plan-driven workflow pointer, add-a-provider and
      add-a-language recipes (mirroring `AGENTS.md`), the REUSE requirement with the exact header
      lines, Conventional Commits rules, and the review expectations (snapshots reviewed by eye,
      no network in tests).
- ⬜ `.github/ISSUE_TEMPLATE/{bug,feature,provider-request}.yml` + `config.yml`: GitHub issue forms
      asking for version, platform, provider, config (redacted), and `-vv` output for bugs; the
      provider-request form asks for auth model, coverage, licence and attribution so a new backend
      can be judged from the issue alone.
- ⬜ `SECURITY.md`: the no-telemetry promise, key handling (env/`keys.toml` 0600/keyring, never
      logged), report of a vulnerability by private advisory to the maintainer address, scope
      (binary, packaging, and the `serve` surface if backlog B01 has landed), and "no bug bounty".
- ⬜ `README.md` refresh: what/why, one real ASCII screenshot per format from
      `docs/screenshots/*.txt` (captured from real runs, `script -q -c 'cirrocast Beijing' /dev/null`),
      the provider matrix, install paths (AUR and release archives; backlog B02 adds more), links to
      every doc, licence.
- ⬜ `.github/workflows/ci.yml` + `lychee.toml`: `docs` job running the snapshot test, the flag
      reconciliation test, the schema test, `man --warn`, and `lycheeverse/lychee-action@v2` with
      `--cache --max-cache-age 1d` over `*.md` plus a checked-in ignore list for known-hostile hosts.
- ⬜ `REUSE.toml`: annotations for `docs/reference/*.txt`, `docs/screenshots/*.txt`,
      `docs/schema/*.json`, `tests/fixtures/json/*.json`, `perf/baseline.json`, `lychee.toml` if it
      cannot carry a comment.

## Design notes

* **`jsonschema` over `valico`.** `jsonschema` 0.58 (MIT) implements Draft 2020-12, has an active
  maintainer and a conformance suite; `valico` (MIT) is unmaintained since 2021 and Draft-7 only.
  `default-features = false` is required: the default set enables `resolve-http`, which pulls
  `reqwest` → `tokio` into the build graph for a test that never fetches a remote `$ref` (our schemas
  use only internal `$defs`). It is a dev-dependency, so it never enters the shipped binary or the
  step 21 size budget. A hand-rolled `serde_json` walk over the schema is rejected: keywords like
  `multipleOf`, `unevaluatedProperties` and `oneOf` branching are exactly where hand-rolled validators
  diverge from the spec, and the point of this artifact is that third parties can trust it.
* **Schema v1 and v2 both ship.** v1 records what `v1.0.0` printed and must not be edited afterwards
  (it is the compatibility promise for existing consumers); v2 is the current family once the
  phase-F surface freezes (steps 19–27) and is what the live tests validate. The two files share
  nothing but a `$defs` copy, deliberately: a consumer pinning v1 must be able to read a
  self-contained file.
* **The flag reference is a plain list, not generated docs.** A Markdown table generated from clap
  would duplicate `--help` and rot in the same way as hand-written prose; `flags.txt` is a flat list
  whose only job is the set-equality test, and `--help` stays the human-facing canonical reference.
* **Snapshot acceptance is manual.** `help-long.txt` and the man page change only in the commit that
  changes a flag, and the reviewer sees both diffs in one view; the CI job never runs
  `INSTA_UPDATE=always` (matching the policy in `AGENTS.md`).
* **Lychee is CI-only.** It is an async Rust binary, but it runs as an action, never as a dependency
  of this crate, so the "no async runtime" rule is untouched; its local-file mode plus a 1-day cache
  keeps the job from hammering upstream hosts.
* **Docs are per-audience, not per-feature.** `configuration.md` is the only place defaults live,
  `providers.md` the only place rate limits live, `formats.md` the only place the token table is
  written down (and it is cross-checked against `template::TOKENS` by a test, since the table exists
  twice: in code and in prose).

## Out of scope

Translating the documentation (English only, per `AGENTS.md`), a documentation site generator
(Markdown in the repository tree is the deliverable; `mdbook` adds a build system for no reader we
have), shell completion documentation (completions are generated in step 08 and self-documenting),
and content beyond the pointers this step links.

## Verification

```bash
cargo test
man --warn --local-file man/cirrocast.1 > /dev/null && echo 'man page renders clean'
cargo run -q -- --help | diff - docs/reference/help-long.txt && echo 'help snapshot matches'
cargo run -q -- --help | grep -o -- '--[a-z-]\+' | sort -u | diff - docs/reference/flags.txt
cargo run -q -- --offline Beijing -f json | python3 -m json.tool > /dev/null && echo 'json parses'
cargo run -q -- Beijing Shanghai -f json | python3 -c 'import json,sys; print(type(json.load(sys.stdin)).__name__)'
# local links only, as a contributor runs it before pushing
lychee --config lychee.toml --offline --root-dir "$(pwd)" './**/*.md'
# everything, as CI runs it (the GitHub Action wraps this call)
lychee --config lychee.toml --root-dir "$(pwd)" --cache --max-cache-age 1d './**/*.md'
```

Observable result: `cargo test` green including the three new tests; the man page renders with no
warnings; the `--help` snapshot and the flag list diff empty; multi-location JSON prints `list`; the
link check reports 0 broken links.

## Exit criteria

- ⬜ `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`, `reuse lint` clean.
- ⬜ All ten `docs/*.md` files exist, are linked from `README.md`, and contain no placeholder text.
- ⬜ `tests/docs_flags.rs` fails when a flag is added/removed without touching `docs/reference/flags.txt`
      (proven once by hand, reverted) and when a doc mentions a flag the binary does not have.
- ⬜ Every documented config key, format and token exists in the binary; every existing one is
      documented (checked by the flags test for flags and by `tests/templates.rs` for tokens).
- ⬜ `docs/schema/json-v1.json` and `json-v2.json` validate the committed and the live outputs
      respectively; the schema test parses the CLI output, not a hand-written sample.
- ⬜ Man page reviewed line by line, `man --warn` silent, EXIT STATUS/ENVIRONMENT/SEE ALSO present.
- ⬜ `CONTRIBUTING.md`, `SECURITY.md`, three issue forms and the CI `docs` job committed; link check
      green; `README.md` shows real captured ASCII screenshots and the provider matrix.
- ⬜ `reuse lint` covers every new artifact (`.txt`, `.json`, `.yml`), via headers where the format
      supports comments and `REUSE.toml` annotations where it does not.

## Risks

* Documentation drift is the default failure mode; the flags test covers flags only, so prose about
  behaviour is guarded by review discipline and by the `## Progress log` rule that a step's doc
  changes land in the same commit as its code.
* External link rot will make CI red for reasons unrelated to the change; mitigated by the
  `lychee.toml` ignore list (with a comment per entry) and by `--max-cache-age 1d`.
* `jsonschema` is a fast-moving crate (several releases per month); pinned by minor version with
  `Cargo.lock` committed, and it is dev-only, so an upgrade can never break a release.
* The v1 schema file is a promise: once users generate output with `schema_version: 1`, editing that
  file is forbidden — added to `CONTRIBUTING.md` as a written rule.

## Progress log

- 2026-09-30 — step opened: document set, generated-artifact checks, validator choice (`jsonschema`
  with `default-features = false`) and repo-hygiene files fixed.
- 2026-10-04 — renumbered from 23 to 28 by the plan reorganization and moved to the end of the
  A-series: its man/flags/help/schema snapshots must see the frozen CLI and JSON surface, so the
  generated artifacts wait for steps 18–27, while the prose halves (getting-started, configuration,
  location, i18n, troubleshooting, architecture) can be written as their subjects land. The
  `wttr-compat.md` review is now conditional on backlog B01.
