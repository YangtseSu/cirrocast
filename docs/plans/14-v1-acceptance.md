<!--
SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Step 14 — v1 acceptance

Status: ⬜ not-started
Depends on: all of phases A–C (01–13)
Touches: `docs/plans/14-v1-acceptance.md` (evidence and sign-off), `docs/plans/README.md` (status rows),
`README.md`, `CHANGELOG.md`, `Cargo.toml` (version `1.0.0`), `packaging/aur/PKGBUILD` (version bump)

## Goal

The "basically formed" gate: prove by execution, not by argument, that v1.0.0 does everything the original
requirement list asked for. Every requirement in `docs/plans/README.md#requirement-traceability` gets a recorded
command and its observed output; the eight v1 backends are exercised against all five formats with the not-applicable
cells explained; the default `art-table` output is compared side by side with real `wttr.in` for the same three
cities at the same minute and every difference is classified as accepted or as a defect; the non-happy paths
(geocoding miss, IP failure, missing/bad key, offline, rate limit) are run end to end; startup, cached, cold-run and
binary-size numbers are recorded; the package is installed from the PKGBUILD on a clean Arch machine and beside it
via `cargo install`; the documentation set is complete; and a human signs off requirement by requirement. This step
ships no new features: defects found are fixed in the step that owns the code, never here.

## Deliverables

- ⬜ Evidence run sheet: for each of the ten requirements in `docs/plans/README.md#requirement-traceability` and
  each of the six checks in "Definition of basically formed", run the proving command, paste the transcript into
  the Evidence appendix of this file and mark the row proved / accepted-deviation / defect. The requirement → step
  mapping stays in the index; this file records only command, result and date.
- ⬜ `wttr.in` side-by-side for `Beijing`, `Shanghai`, `London`: `curl -s 'wttr.in/<city>?lang=en'` and
  `cirrocast <city>` captured in the same minute on the same machine, plus a mechanical `diff` of the `-f dumb`
  variants so layouts compare without ANSI noise. One row per difference, classified as accepted (with a
  user-legible reason) or as a defect (with the owning step), covering the header lines, the four day-part rows,
  condition art keys, the temperature ramp, wind/pressure/precipitation columns and units.
- ⬜ Backend × format acceptance matrix: rows `open-meteo`, `openweathermap`, `weatherapi`, `worldweatheronline`,
  `pirateweather`, `qweather`, `smhi`, `metar`; columns `art-table`, `one-line`, `plain`, `json`, `dumb`. Each cell
  is `pass` (transcript in the appendix) or `n/a` with one documented reason: no key in the environment
  (`n/a (BYOK)`, and that provider must still pass the key-missing path), `metar` + any forecast need
  (`n/a (observation: Capabilities.daily == false)`), `metar` + a city-name location
  (`n/a (station-based: use --station or @lat,lon)`), `smhi` outside its Nordic coverage (`n/a (coverage)`), or an
  unsupported location form. No cell may be empty.
- ⬜ Keyed-provider acceptance is real, not hypothetical: at least two key-requiring backends (`openweathermap`,
  `weatherapi`) exercised with live keys through the documented precedence (env var first, then `keys.toml`), one
  via `cirrocast key set …` too; without live keys the requirement is "partially proved", never "proved".
- ⬜ Location and input acceptance: fuzzy `Beijing` (deterministic ranking, resolved coordinates echoed), exact
  `:Beijing`, coordinates `@39.90,116.41`, OSM `~Tsinghua` (1 req/s and cache behaviour observed), ambiguous names
  re-run twice to prove ranking stability, `:Nowhereville` → exit 5 with the search hint, `--station ZZZZ` → exit 5.
- ⬜ IP location acceptance: bare `cirrocast` with an empty `location.default` resolves via ipwho.is; blocking the
  primary proves the ipapi.co fallback; blocking both → exit 3 naming both services; the privacy note in
  `--help`/README matches observed behaviour (result cached for `cache.ip_ttl_secs`).
- ⬜ Failure-path acceptance: missing key → exit 6 with the exact `cirrocast key set <provider>` line; invalid key
  → exit 6 (not 3); `--offline` warm cache passes, cold cache exits 3 with the rerun hint; a loopback stub returning
  `429` proves bounded retries plus chain fallthrough (exit 0, attribution naming the second provider); a
  `keys.toml` with the wrong mode → exit 4 with the `chmod` hint.
- ⬜ Performance and size record (informational; enforced budgets are step 22): `hyperfine --warmup 10 --runs 30`
  for `--version` (< 50 ms), a warm-cache `--offline` run (< 150 ms) and a cold `cirrocast Beijing` (open-meteo,
  < 1.5 s on a residential connection), plus `ls -lh target/release/cirrocast`; each number with machine and date.
- ⬜ Clean-machine install on Arch: build `packaging/aur/PKGBUILD` in a clean chroot (`pkgctl build` or
  `makechrootpkg -c`), `pacman -U` the artefact, then `cirrocast --version`, a real `cirrocast`, `man cirrocast`,
  `pacman -Ql cirrocast | grep -c completions` (expect 3) and the licence path under `/usr/share/licenses/`;
  separately `cargo install --locked --path .` with a pristine `CARGO_HOME` and `HOME`, followed by a first run
  that creates the documented XDG directories and prints weather.
- ⬜ Documentation completeness gate: README covers install (source/cargo/AUR), config keys, key precedence,
  backend list, formats, units, language, cache/offline, exit codes and completions; `--help`, the man page and
  README agree on every flag; `docs/schema.md` matches a real `-f json` document; index rows 01–14 read `done` and
  15–24 `not-started`; `CHANGELOG.md` carries the dated `1.0.0` entry; `Cargo.toml` reads `1.0.0`;
  `grep -rn 'TODO\|FIXME\|unimplemented!\|todo!()' src/ locales/` is empty; every flag in `--help` appears in at
  least one transcript (no dead flags).
- ⬜ Full gate run on the release commit: the four tool gates plus `cargo deny check`, `cargo audit` and CI green
  on the tag commit (record the run URL); release archives from the step 13 workflow, the crates.io publish
  completed, AUR packages updated to `1.0.0`, and `v1.0.0` tagged with the changelog as release notes.
- ⬜ Defect handling policy applied: a defect found here is fixed in the step that owns the code (with a
  progress-log entry there), this step re-runs the affected rows and replaces their transcripts; a defect no step
  owns becomes a deliverable in the owning phase D/E plan file (step 15–24), never a silent fix here.
- ⬜ Sign-off table below completed — one row per requirement and per "basically formed" check, each with the
  proving command, evidence pointer, verdict and a human name plus date — and the final commit flips this file to
  `Status: done`, updates the index row (14 → done) and dates the CHANGELOG's `1.0.0` section.

Sign-off table (a human fills the last three columns; tick the checkbox when the row is verified):

| # | Requirement (index row) | Proving command | Evidence | Verdict | Signed off by / date |
|---|---|---|---|---|---|
| 1 | Rust + clap toolchain | `cargo --version && cirrocast --version` | appendix §1 | | [ ] |
| 2 | Multiple keyless-first backends | matrix rows for `open-meteo`, `smhi`, `metar` | appendix §2 | | [ ] |
| 3 | BYOK for keyed backends | `cirrocast key set openweathermap` + live run | appendix §3 | | [ ] |
| 4 | wttr.in-style outputs | `cirrocast Beijing` vs `curl -s wttr.in/Beijing` | appendix §4 | | [ ] |
| 5 | City name → coordinates | `cirrocast Beijing --verbose`, `@39.90,116.41` | appendix §5 | | [ ] |
| 6 | IP → city | bare `cirrocast`, primary blocked, both blocked | appendix §6 | | [ ] |
| 7 | Own CLI design | `cirrocast --help` (precedence + exit codes) | appendix §7 | | [ ] |
| 8 | Units and language | `-u us`, `-u uk`, `--lang zh-CN` runs | appendix §8 | | [ ] |
| 9 | XDG directories | `cirrocast config path`; fresh `HOME` run | appendix §9 | | [ ] |
| 10 | Project name | `cirrocast --version`, crates.io/AUR/GitHub name check | appendix §10 | | [ ] |
| A | Nos. 1–6 of "Definition of basically formed" | one command per check, see index | appendix §A | | [ ] |

## Design notes

* This is a gate, not a feature step: every row must produce an artefact (a timestamped transcript with the exact
  command), because a claim without output is not evidence. Anything unobtainable is "partially proved", not
  assumed.
* Side-by-side methodology: capture `wttr.in` and `cirrocast` within the same minute; compare the `-f dumb`
  outputs textually (`diff`) so layout differences show without ANSI escapes, and compare the coloured `art-table`
  visually in a 256-colour terminal. Day/night art depends on local time, so the local time is recorded and a
  second run after sunset is optional, not a defect signal. Byte-identical output is explicitly *not* the bar:
  glyphs, palette and art are re-authored here, so layout, field set, units and token semantics must match.
* Matrix policy: `n/a` needs a stated reason, and a keyed backend is never simply skipped — it must at least prove
  the key-missing path, so a reader can tell "works with a key" from "told me how to get a key".
* Rate-limit simulation uses the step 12 loopback base-URL seam: offline, deterministic, real retry/chain code.
* The requirement matrix itself lives in `docs/plans/README.md#requirement-traceability`; this file deliberately
  does not restate it, so a mapping change is a one-place edit and cannot drift between two documents.
* Version semantics for this milestone: `1.0.0` means the CLI, config, cache and JSON output are stable contracts
  for the scope of the index's "Definition of basically formed"; the JSON and config schemas carry their own
  versions (step 13), so phases D and E cannot silently break consumers.

## Out of scope

* Everything planned as follow-up work with its own plan file, not a roadmap wish: alerts and severity (step 15),
  air quality and pollen (16), moon phase and astro (17), the offline bundled city database (18), additional
  providers such as met.no/visualcrossing/open-meteo archive and marine (19), the wttr.in-compatible local service
  (20), multi-location output and user templates (21), enforced performance and resource budgets (22), the
  documentation set and guides (23), ecosystem packaging (24) — phases D and E of `docs/plans/README.md`.
* True non-goals for v1 and beyond, recorded so no future step is expected to deliver them: no GUI, no
  daemon/server mode in v1 (step 20's `serve` is a separate, explicitly started command), no telemetry, no TUI, no
  plugin or scripting engine, no PNG/SVG image output, no code signing/notarization or build attestations (no step
  file owns those; revisit if a target platform requires them).
* Fixing defects inside this step: every defect is fixed in the owning step (or its phase plan file) and re-proved.

## Verification

```
curl -s 'wttr.in/Beijing?lang=en' > /tmp/wttr-beijing.txt && cirrocast Beijing > /tmp/cirrocast-beijing.txt
CLICOLOR_FORCE=1 cirrocast Beijing -f dumb > /tmp/cirrocast-beijing.dumb && diff -u /tmp/wttr-beijing.txt /tmp/cirrocast-beijing.dumb
for p in open-meteo openweathermap weatherapi worldweatheronline pirateweather qweather smhi metar; do \
  for f in art-table one-line plain json dumb; do echo "== $p $f"; cirrocast Beijing -p "$p" -f "$f" || true; done; done
cirrocast --station KJFK -p metar -f art-table && cirrocast --station KJFK -p metar -f json | jq .
cirrocast Beijing -p qweather                 # exit 6, prints: cirrocast key set qweather
cirrocast :Nowhereville                       # exit 5   ;   cirrocast --station ZZZZ   # exit 5
cirrocast cache clean && cirrocast Beijing --offline      # exit 3 + rerun hint
hyperfine --warmup 10 --runs 30 'target/release/cirrocast --version' && ls -lh target/release/cirrocast
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && reuse lint && cargo deny check
```

## Exit criteria

- ⬜ `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`, `reuse lint` all clean.
- ⬜ The eight × five matrix has no unexplained cell, every requirement in the index traceability table has an
  evidence transcript, the `wttr.in` comparison lists each difference as accepted or as a fixed defect, and the
  failure paths reproduce the documented exit codes 3, 4, 5 and 6.
- ⬜ A clean Arch machine installs from `packaging/aur/PKGBUILD` and runs `cirrocast`, `man cirrocast` and all
  three completions; `cargo install --locked --path .` does the same on a pristine `HOME`; the sign-off table is
  complete with a human name and date per row.

## Risks

* Upstream outages or provider drift during the acceptance window would leave rows unprovable: each row records
  the timestamp, a single time-boxed retry is allowed, and a still-failing row is recorded as a defect for the
  owning step rather than as `n/a`.
* `wttr.in` itself is rate-limited and occasionally returns its own error page: requests are spaced, the response
  is sanity-checked before being used as a reference, and the comparison is re-run if the reference is invalid.
* Missing live keys for the slower-moving providers (`weatherapi`, `worldweatheronline`, `pirateweather`,
  `qweather`) would weaken requirement 3: the environment supplies at least two live keys, and anything weaker is
  recorded as "partially proved".
* Narrow coverage (`smhi`: Nordics) or a narrow location kind (`metar`: stations) inflates the `n/a` count: the
  reason column exists so a reader can tell "not applicable" from "not tested".
* Time-of-day and locale dependence rules out byte-identical comparison: local time and locale are recorded.

## Progress log

- 2026-09-30 — step file written (status: not-started); requirement mapping and the "basically formed" checklist
  cross-referenced from `docs/plans/README.md` instead of duplicated here.
