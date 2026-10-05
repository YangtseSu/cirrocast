<!--
SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Step 20 — location candidate selection

Status: ✅ done
Depends on: `04-geocoding-and-location-syntax.md` (ranking, `location search`), `05-http-cache-and-ip-location.md` (cache, IP chain), `08-cli-surface-and-formats.md` (flag surface), `18-offline-city-database.md` (the offline candidate source and the `--all` listing share its index)
Touches: `src/geo/{mod,pick}.rs`, `src/cli.rs`, `src/main.rs`, `src/config/mod.rs`, `src/error.rs`, `tests/location_pick.rs`, `tests/cli_pick.rs`, `docs/plans/README.md`, `README.md`, `CHANGELOG.md`

## Goal

When resolving a place yields more than one plausible candidate — the ten Beijing hits the geocoder
returns, nine Springfields from the offline table, several towns inside the radius step 25's reverse
lookup names — the user picks one instead of the tool silently ranking it. On a terminal the run
prints the ranked list and asks; in a script, with `--yes`, or with `[location] pick = "never"` the
ranked winner is used as today and the existing note still says how to make the choice explicit. The
chosen candidate is echoed with a deterministic `@lat,lon` spec that skips the prompt next time.

## Deliverables

- ✅ `src/geo/mod.rs`: `pub struct Resolved { pub location: Location, pub candidates: Vec<Location>,
  pub resolution: Resolution }` and a `resolve_candidates(results, spec, limit) -> Result<Resolved>`
  beside the shipped `resolve` (which becomes a thin wrapper), so both `location search` and the
  weather path see the same ranked list. Ranking stays exactly step 04's three keys (exact
  case-insensitive name, then population, then upstream order); the winner is element 0.
- ✅ `src/geo/pick.rs`: `pub struct Picker<'a> { input: &'a mut dyn BufRead, output: &'a mut dyn
  Write }` with `choose(&mut self, query: &str, candidates: &[Location]) -> Result<Location>`:
  one line per candidate, `[1]`…`[N]` with the ranked winner marked `*`, formatted through the
  shipped `location_line` plus a `pop. <n>` tail when the population is known; prompt
  `choose a location [1-<N>, Enter=1, q=quit]: ` on the injected writer. Accepted input: an index in
  range, an empty line (= 1), `q`/`Q`; three consecutive invalid answers are `Error::Usage` (exit 2)
  naming the accepted input, and EOF behaves like `q`. `q` is
  `Error::LocationNotFound` — message `no location selected for Beijing` (exit 5).
- ✅ Prompt policy, resolved in one place in `src/cli.rs`: prompt iff candidates > 1 **and**
  (`--pick` was given **or** (stdin and stderr are terminals **and** config
  `[location] pick = "auto"`)). `--yes` and `[location] pick = "never"` suppress; `--pick` and
  `--yes` are a clap conflict group; `--yes` beats the config. Non-interactive runs keep today's
  behaviour: ranked winner on stdout, the step-04 note on stderr extended with `--pick`/`--yes`
  hints. `-q` still silences notes but never suppresses a prompt the policy asked for.
- ✅ After a selection, one stderr note (suppressed by `-q`):
  `selected: Beijing, Beijing Municipality, China — use @39.9042,116.4074 to skip the prompt` — the
  spec is always `@lat,lon` (two decimals is not enough to round-trip; use the full precision the
  location carries), because names re-query a geocoder whose ranking can drift.
- ✅ `location search <query>`: never prompts (it is the automation-friendly discovery command) and
  gains `--all`, printing the ranked candidate table (number, name, admin1, country, coordinates,
  zone, population when known) on stdout; the default output remains the winner line, so existing
  scripts keep working. `--limit` (default 10, max 100) bounds both forms; `-v` keeps printing the
  cache key path.
- ✅ `src/config/mod.rs`: `[location] pick = "auto"` (`auto` | `never`) with a `KEY_TABLE` row and
  `CIRROCAST_LOCATION_PICK`; an invalid value is `Error::Config` (exit 4) listing the two.
- ✅ `docs/plans/README.md`: the location-syntax paragraph of the architecture contract changes from
  "ambiguity is resolved by ranking and reported" to the picker policy above; `README.md` gains the
  flag rows and an example session; `CHANGELOG.md` records the behaviour change (a terminal run can
  now ask; non-terminals are unchanged).
- ✅ Tests: `tests/location_pick.rs` drives `Picker` with injected reader/writer — Enter, `2`, `q`,
  junk×3, EOF, out-of-range index, and the rendered list against a fixture — no TTY involved;
  `tests/cli_pick.rs` (assert_cmd) covers the wiring: `--pick` with stdin `2\n` selects the second
  candidate end to end (`--pick` is the deterministic hook that bypasses the TTY test), `--yes` never
  reads stdin and prints the winner with the note, `--pick --yes` is a usage error (exit 2),
  `CIRROCAST_LOCATION_PICK=never` behaves like `--yes`, and `location search --all` prints the full
  ranked table while `location search Beijing` still prints one line. All offline, by fixtures or the
  bundled table.

## Design notes

* **Prompt on stderr, not stdout.** `cirrocast Beijing | tee` must keep stdout a pure weather
  document; the question and the list are diagnostics, and the TTY test therefore looks at *stdin
  and stderr*, not stdout. `--yes` is the script switch; cron and CI have no stdin terminal and take
  the winner path automatically.
* **`--pick` is a real flag, not a test hook.** It forces the picker when the TTY check cannot (a
  shell function reading a here-doc, a wrapper driving several candidates), and it is the only way
  to test the interaction deterministically without a pseudo-terminal; the repo already refuses to
  spawn a PTY for tests (`rustix` is used for terminal size, not for test scaffolding).
* **Ranking stays upstream-faithful and total.** The picker does not re-rank, filter or fuzzy-match
  the list: the three shipped keys give a deterministic order, the winner is what a non-interactive
  run would have taken, and pressing Enter reproduces the old behaviour exactly.
* **Why `@lat,lon` as the canonical echo.** A name spec's result depends on the geocoder's current
  ranking (step 04's risk note says so); coordinates pin the choice and skip every lookup. The
  offline path may add a `:name, CC` habit later, but the prompt must not teach a spec that can
  drift.
* **Not a TUI.** `AGENTS.md` forbids a TUI; a numbered list plus a single line read is one prompt,
  the same shape as `key set`'s password read, and it works over a serial console, ssh and tmux.
* **Step 18 owns the offline list's shape.** Its `location search --offline` deliverable is adjusted
  in the same commit as this plan update (winner line by default, `--all` table) so the network and
  offline search surfaces do not diverge.

## Out of scope

Fuzzy filtering inside the picker (the list is bounded by `--limit` and already ranked), a
persistent favourites store (step 19's `[locations]` aliases are the named-location mechanism),
shell-completion integration for candidate values (step 08's completions remain static), and any
prompting from non-interactive contexts.

## Verification

```sh
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && reuse lint

printf '2\n' | cargo run -q -- --pick Beijing -f plain
#   shows [1]…[N] with `*` on the ranked winner, accepts `2`, then renders for the second hit;
#   stderr ends with `selected: … — use @39.9042,116.4074 to skip the prompt`
printf 'q\n' | cargo run -q -- --pick Beijing; echo $?      # exit 5, `no location selected`
cargo run -q -- --yes Beijing -f plain                      # winner printed, stdin never read
cargo run -q -- location search Beijing                     # one line, unchanged
cargo run -q -- location search --all Beijing | head -4     # ranked table, [1] marked
CIRROCAST_LOCATION_PICK=never cargo run -q -- Beijing -f plain   # no prompt, winner as in v1
```

## Exit criteria

- ✅ `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`, `reuse lint` clean.
- ✅ `--pick` end-to-end selects the second candidate from scripted stdin; `q` and EOF exit 5 with the
      message above; three invalid answers exit 2.
- ✅ A non-terminal run and `--yes` produce exactly the pre-step stdout and the extended note; no
      stdin is consumed.
- ✅ `location search` keeps its one-line default; `--all` matches the picker's list order for the
      same query and limit.
- ✅ README contract and CHANGELOG updated in the same commit as the behaviour.

## Risks

* A prompt in an unexpected context hangs a script; mitigated by the stdin/stderr TTY test, by
  `--yes`/`[location] pick = "never"`, and by the fact that a piped or file-backed stdin is not a
  terminal.
* **Prompt text stays English** — one string lives in the CLI, not the Fluent catalogs (which render
  weather), matching `key set`'s prompt; step 28's docs review can revisit it if a CLI string domain
  ever appears.
* `location_line` is a display format; reusing it for the candidate list couples the prompt to a
  renderer helper. Accepted: it is the same data (name, admin, country, coordinates, zone) the
  choice needs, and one formatting path cannot drift from the other.

## Progress log

- 2026-10-03 — step opened. Requirement recorded by the maintainer on 2026-10-03: choosing a place by
  name or by IP must offer the candidates when more than one exists, instead of ranking silently.
  Step 04's determinism contract (the ambiguity note, `:query`, `~query`) is preserved as the
  non-interactive path; the picker is additive on top of it. Written as a new step because steps 04
  and 08 are shipped and their tests pin the current behaviour.
- 2026-10-04 — renumbered from 26 to 20 by the plan reorganization (serve → B01, packaging matrix → B02); dependencies unchanged, `18-offline-city-database.md` is still the offline candidate source.
- 2026-10-04 — deliverable 1: `Resolved`/`resolve_candidates` land in `src/geo/mod.rs` and the CLI
  resolves through them, so the ranked list is built once and shared (`keep_candidates` and the
  second `rank` pass in `ranked_candidates` are gone); `resolve` stays as the same decision without
  the list. `location search --all`, the `-v` listing and the upcoming picker therefore print the
  one ranking; the `--exact` filter now applies to the network path's `--all` rows too.
- 2026-10-04 — deliverable 6: `[location] pick = "auto" | "never"` is part of the document, the
  `KEY_TABLE`/`config get|set` vocabulary and the `CIRROCAST_LOCATION_PICK` family; the built-in
  default is `auto`, an invalid value is refused with the two spellings listed. The CLI policy that
  reads it lands with the picker wiring.
- 2026-10-04 — deliverable 2: `src/geo/pick.rs` ships the stream-injected `Picker` (numbered list,
  `*` on the ranked winner, `pop. <n>` tail, one-line prompt) plus `prompt_lock`, which serializes
  prompts so a multi-location run cannot interleave two of them on the shared stdin/stderr.
  `tests/location_pick.rs` drives it with a `Cursor` and a `Vec<u8>`: Enter, `2`, `q`/`Q`, EOF,
  out-of-range and junk answers, the three-strike usage error and the exact rendered list.
- 2026-10-04 — deliverable 3: `--pick`/`--yes` are a clap conflict group on the query surface, and
  `should_pick`/`pick_policy` decide the prompt in one place (candidates >= 2, `--pick`, or
  terminal stdin+stderr with `[location] pick = "auto"` from config or `CIRROCAST_LOCATION_PICK`).
  The prompt is serialized across worker threads and reads the shared stdin; the ambiguity note
  gained the `--pick`/`--yes` tail, and a pick suppresses the note the picker just answered.
- 2026-10-04 — deliverable 4: a selection echoes `selected: <place> — use @<lat>,<lon> to skip the
  prompt` on stderr (full float precision, so the spec round-trips), suppressed by `-q` like every
  other note; the ambiguity note is not printed for a choice the prompt already made.
- 2026-10-04 — deliverables 5 and 8: `location search`'s one-line default, `--all` table and
  `--limit` were already shipped by step 18 under this step's agreed shape; `tests/cli_pick.rs` now
  pins that the search form never prompts and that its `--all` order is byte-identical to the
  picker's list for the same query, and adds the `--pick`/`--yes`/policy/reply taxonomy end to end
  through the real binary (recorded geocode fixture + cached forecast, network guard on).
- 2026-10-04 — deliverable 7 and closure: the architecture contract paragraph now spells the prompt
  precisely (`Error::LocationNotFound`/exit 5, the three-strike usage error, the `selected:` echo),
  the module map lists `geo/pick.rs`, README gained the flag row, the precedence row, the
  `location.pick` key, the env variable, the example session and the picker paragraph, and the
  CHANGELOG records the behaviour. Gates: `cargo fmt --check`, `cargo clippy --workspace
  --all-targets -D warnings`, `CIRROCAST_FORBID_NETWORK=1 cargo test --workspace` and `reuse lint`
  all clean, plus the offline smoke run over the bundled table. Unrelated time bomb surfaced by the
  clock rollover: `tests/fixtures/alerts/meteoalarm-heat-cap.xml` expires 2026-10-04T16:00Z and the
  alert pipeline drops expired alerts, so that test began failing mid-step; the CAP's two `expires`
  stamps are now shifted to now+6h at seed time, the way `live_gale_fixture` already did for FPAS.
