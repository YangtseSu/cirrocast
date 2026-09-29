<!--
SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Step 21 — multi-location runs, templates and aliases

Status: ⬜ not-started
Depends on: 08 (CLI surface and formats), 14 (v1 acceptance)
Touches: `src/cli.rs`, `src/config/mod.rs`, `src/geo/mod.rs`, `src/template.rs`, `src/parallel.rs`,
`src/render/{one_line,json,art_table,plain}.rs`, `src/error.rs`, `src/lib.rs`,
`tests/{multi_location,templates,aliases}.rs`, `tests/fixtures/multi/`, `docs/plans/README.md`,
`AGENTS.md`, `README.md`

## Goal

One invocation can ask for several places: `cirrocast Beijing Shanghai Tokyo -f one-line` fetches them
in parallel, prints them in argument order whatever order the network answers in, isolates one
location's failure from the others, and exits with the worst error it saw. Above one location `json`
becomes an array, `art-table` grows a 2–4 location summary layout. Separately, the `%`-token template
engine (`--template`, `--template-file`, `--format full|minimal`, `[templates]` config presets) is the
single implementation behind every one-line output, including the wttr.in compatibility surface of
step 20, and `[locations]` aliases make `cirrocast @home` work.

## Deliverables

- ⬜ `src/cli.rs`: positional `[LOCATION]...` (was `[LOCATION]`); `--lat/--lon`, `--ip` and
      `--station` refuse to combine with more than one positional (`Error::Usage`, exit 2);
      `--format` gains `full` and `minimal` (one-line presets); new `--template <STRING>` and
      `--template-file <PATH>` (`-` reads stdin), both refused together with `--format json|plain|
      art-table|dumb` and with each other.
- ⬜ `src/parallel.rs`: `par_map_ordered<T, U>(items: &[T], workers: usize, f: impl Fn(usize, &T)
      -> Result<U> + Sync) -> Vec<Result<U>>` built on `std::thread::scope`, an `AtomicUsize` work
      counter and slot-indexed result writes; `workers = min(len, min(4, available_parallelism()))`.
      No channel-based unordered collection: the output order is the input order by construction.
- ⬜ Multi-location reporting in `src/lib.rs`: per-location `Result<Report>`, a failed slot renders
      the placeholder line `error: <query>: <message>` on stdout (one line, no colour, occupying the
      slot) while the full `error: …` line goes to stderr; the process exit code is the numerically
      largest `Error::exit_code()` among the failures (documented rule, tested with 3/5 and 5/6 pairs).
- ⬜ `src/render/json.rs`: above one location the top level is an array of report objects, each with
      `"schema_version": 2`; the single-location document stays a plain object with
      `"schema_version": 2`; a failed slot becomes
      `{"schema_version": 2, "query": "<as typed>", "error": {"code": <exit code>, "message": "…"}}`.
      `docs/schema/json-v2.json` (step 23) admits both shapes.
- ⬜ `src/render/art_table.rs`: combined summary layout for 2–4 locations — one header line per
      location, then that location's current condition plus today's high/low in a compact grid, one
      blank line between blocks, never wider than the resolved width; 5+ locations fall back to the
      per-location full tables and log `note: art-table summary layout is limited to 4 locations` to
      stderr once (silenced by `-q`).
- ⬜ `src/template.rs`: token table shared by `one-line`, the `full`/`minimal` presets, the
      wttr.in compat surface and `status` (step 24): `%c %C %x %t %f %H %L %w %h %p %P %e %u %m %M %v
      %l %d %D %T %Z %z %S %s %A %q`, with `TOKENS: &[TokenSpec]` exported so tests and the compat
      help page enumerate the same list.
- ⬜ `src/template.rs`: width specifiers `%[-][0][<width>][.<prec>]X` (right-aligned unless `-`,
      zero-pad for numeric tokens, precision truncates text from the right and rounds numbers),
      escapes `%%` → literal `%`, `%{…}` → the braced content as a token when it is exactly one known
      letter and verbatim otherwise, trailing lone `%` literal.
- ⬜ `src/template.rs` + `src/serve/query.rs` (step 20): unknown-token policy — `Error::Usage`
      (exit 2) for `--template`, `--template-file` and `[templates]` presets; literal passthrough for
      a wttr.in compat request. The asymmetry is deliberate and stated in `docs/wttr-compat.md`.
- ⬜ `src/config/mod.rs`: `[locations]` (`home = "@39.9,116.4"`) and `[templates]`
      (`compact = "%c%t"`) tables; `schema_version` 1 → 2 with a migration arm that stamps the new
      version (absent tables mean defaults) and a migration test from a v1 file.
- ⬜ `src/geo/mod.rs`: `@NAME` resolution order — parse as coordinates when the text after `@` has
      exactly one comma and both sides are `f64` with lat ∈ [-90, 90] and lon ∈ [-180, 180], otherwise
      alias lookup; alias values may be any location spec (§ alias-to-alias chains), expanded with a
      visited set and depth cap 8; a cycle or depth exhaustion → `Error::Config` naming the chain; an
      unknown alias → `Error::Usage` suggesting up to 3 configured names within edit distance 2.
- ⬜ `docs/plans/README.md`: contract updates — variadic location, the `full`/`minimal` formats, the
      template/exit-code rules, the `json` array rule, and the two new config tables.
- ⬜ `README.md` + `AGENTS.md`: usage examples for multi-location and templates; the aliases recipe.
- ⬜ Tests: `tests/templates.rs` walks `TOKENS` (every token renders against a fixture report,
      snapshot per token, width/precision table, escape cases, unknown-token policy both ways);
      `tests/aliases.rs` (expansion, nesting, cycle, coordinate-vs-alias disambiguation, suggestion
      list); `tests/multi_location.rs` (ordering under injected per-location delays, partial failure
      exit codes, placeholder slot in four formats, array-vs-object JSON).

## Design notes

* **Deterministic ordering without a serialising collector.** Results are written by index into a
  `Vec<Option<Result<..>>>` guarded by a `Mutex`, so a slow Tokyo request cannot reorder the output
  and no completion-order channel exists to get it wrong. Rejected: `JoinHandle` join in spawn order
  with `thread::scope` returning a tuple vector (fine, but the index slots also carry the per-slot
  error, and the same helper is reused by step 24's status path).
* **Exit-code rule is "largest mapped code wins".** This means a missing key (6) outranks a location
  miss (5), which is intended: the more actionable problem wins. The alternative (first failure in
  argument order) makes the code depend on argument order and is rejected as untestably arbitrary.
* **JSON is an array above one location.** The literal reading of the requirement, and it is what
  `jq '.[0]'` users expect; each element already carries `schema_version`, so an envelope object
  (rejected alternative, `{"schema_version": 2, "reports": […]}`) would add a nesting level without
  adding information. The cost — the top-level JSON type depends on the number of locations — is
  documented in `docs/formats.md` and in the schema (`oneOf`, step 23); a consumer that needs type
  stability passes exactly one location or reads `.[]` after `jq -s`.
* **`%L` is the day's low, not the location.** wttr.in's documented one-line table defines `H` as
  high and `L` as low; compatibility wins over the adjacency of `%l`/`%L` in the design brief, and
  the long/qualified location form is deliberately not a token — `%l` plus the art-table header
  already carry it.
* **Preset resolution order** for `--format <NAME>`: built-in format → built-in template preset →
  `[templates]` key → `Error::Usage` listing the three namespaces. `--template` is always a literal
  template and `--template-file` always a path, so no new addressing syntax is needed and a config
  preset named `json` cannot shadow the JSON format.
* **Unknown-token asymmetry.** The CLI user is authoring a template and wants the typo reported;
  the compat consumer is reusing a script written against wttr.in's larger token set (which includes
  tokens we do not implement, e.g. `%x`-adjacent glyph variants) and must not break. Literal
  passthrough keeps the script printing something; the same request logs the unknown token once on
  stderr in the service's access log.
* **Aliases live in config, not in a new location namespace.** `@NAME` is the only addressable form,
  and it is unambiguous because coordinates are recognised first. Cycle detection is a visited set
  with a depth cap (8), not a fixed expansion count: a legal chain of three aliases must work while
  `a → b → a` fails with the chain printed.
* **Deliberately not a new template language.** Everything is wttr.in's `%`-notation plus width and
  precision, because step 20 has to serve that surface verbatim; format strings (Rust `fmt::Arguments`,
  named fields) were rejected as a second syntax to document and test.

## Out of scope

`--format` gaining arbitrary new renderers (formats are added by their own step), per-location unit or
language overrides, `xargs`-style fan-out from stdin, saving rendered output to a file, and any change
to provider selection (one `--provider` chain applies to every location; a per-location provider is
not a thing).

## Verification

```bash
cargo run -q -- Beijing Shanghai Tokyo -f one-line
cargo run -q -- Beijing Shanghai Nope-9x -f one-line ; echo $?     # 5, error slot in place
cargo run -q -- Beijing Shanghai -f json | jq 'length'             # 2
cargo run -q -- Beijing -f json | jq -r 'type, .schema_version'    # object, 2
cargo run -q -- Beijing -f full
cargo run -q -- Beijing -f minimal
cargo run -q -- Beijing --template '%l %-12C %05.1t'
cargo run -q -- Beijing --template '%y' ; echo $?                  # unknown token: exit 2
cargo run -q -- @home --template '%l %c%t'                         # alias from config
time cargo run -q -- Beijing Shanghai Tokyo -f one-line            # wall clock ≈ slowest, not the sum
```

Observable result: three lines in argument order for the first command, one `error: …` line in the
second slot with exit 5, `2` from `jq length`, `object`/`2` for the single-location JSON, a template
rendering with the padded condition and zero-padded temperature, exit 2 for the unknown token, and a
wall clock clearly below the sum of three sequential fetches.

## Exit criteria

- ⬜ `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`, `reuse lint` clean.
- ⬜ Every token in `TOKENS` has a rendering test; the count in the test equals the table length
      (a new token without a test fails the build).
- ⬜ Ordering is proven deterministic with injected delays (test asserts argv order for 8 locations
      with descending delays).
- ⬜ Partial failure: exit code 5 for one location miss, 6 for one missing key, 6 when both occur;
      every other location still printed.
- ⬜ JSON: single location is an object, two locations an array of length 2, a failed slot is an
      error object with the exit code.
- ⬜ Alias cycle `home → work → home` fails with `Error::Config` and the chain in the message.
- ⬜ `docs/plans/README.md` contract updated in the same commit as the code.

## Risks

* Making the positional variadic is a breaking CLI change for scripts passing exactly one location
  plus a stray argument; mitigated by the usage error and by the fact that v1 shipped with a single
  positional, so the only affected callers are already wrong.
* Parallel fetching multiplies upstream load for keyed providers (a 4-location run is 4 concurrent
  requests); mitigated by the hard worker cap of 4, by sequential behaviour inside a single
  location's provider chain, and by cache reuse.
* Template `%` in shell contexts (tmux, prompt) needs quoting and `%%`; documented in
  `docs/formats.md` and in the contrib snippets of step 24.
* Alias suggestion via edit distance can suggest a surprising name; capped at 3 suggestions and never
  auto-selected (the user must type the corrected name).

## Progress log

- 2026-09-30 — step opened: ordering, exit-code, JSON-shape, token-table and alias rules fixed;
  `%L` resolved in favour of wttr.in's documented low-temperature meaning.
