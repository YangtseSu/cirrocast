<!--
SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Step 14 — v1 acceptance

Status: ✅ done
Depends on: all of phases A–C (01–13)
Touches: `docs/plans/14-v1-acceptance.md` (evidence and sign-off), `docs/plans/README.md` (status rows),
`README.md`, `CHANGELOG.md`, `Cargo.toml` (version `1.0.0`), the AUR package `cirrocast` (its own repository,
version bump)

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

- ✅ Evidence run sheet: for each of the ten requirements in `docs/plans/README.md#requirement-traceability` and
  each of the six checks in "Definition of basically formed", run the proving command, paste the transcript into
  the Evidence appendix of this file and mark the row proved / accepted-deviation / defect. The requirement → step
  mapping stays in the index; this file records only command, result and date.
- ✅ `wttr.in` side-by-side for `Beijing`, `Shanghai`, `London`: `curl -s 'wttr.in/<city>?lang=en'` and
  `cirrocast <city>` captured in the same minute on the same machine, plus a mechanical `diff` of the `-f dumb`
  variants so layouts compare without ANSI noise. One row per difference, classified as accepted (with a
  user-legible reason) or as a defect (with the owning step), covering the header lines, the four day-part rows,
  condition art keys, the temperature ramp, wind/pressure/precipitation columns and units.
- ✅ Backend × format acceptance matrix: rows `open-meteo`, `openweathermap`, `weatherapi`, `worldweatheronline`,
  `pirateweather`, `qweather`, `smhi`, `metar`; columns `art-table`, `one-line`, `plain`, `json`, `dumb`. Each cell
  is `pass` (transcript in the appendix) or `n/a` with one documented reason: no key in the environment
  (`n/a (BYOK)`, and that provider must still pass the key-missing path), `metar` + any forecast need
  (`n/a (observation: Capabilities.daily == false)`), `metar` + a city-name location
  (`n/a (station-based: use --station or @lat,lon)`), `smhi` outside its Nordic coverage (`n/a (coverage)`), or an
  unsupported location form. No cell may be empty.
- ✅ Keyed-provider acceptance is real, not hypothetical: at least two key-requiring backends (`openweathermap`,
  `weatherapi`) exercised with live keys through the documented precedence (env var first, then `keys.toml`), one
  via `cirrocast key set …` too; without live keys the requirement is "partially proved", never "proved".
- ✅ Location and input acceptance: fuzzy `Beijing` (deterministic ranking, resolved coordinates echoed), exact
  `:Beijing`, coordinates `@39.90,116.41`, OSM `~Tsinghua` (1 req/s and cache behaviour observed), ambiguous names
  re-run twice to prove ranking stability, `:Nowhereville` → exit 5 with the search hint, `--station ZZZZ` → exit 5.
- ✅ IP location acceptance: bare `cirrocast` with an empty `location.default` resolves via ipwho.is; blocking the
  primary proves the ipapi.co fallback; blocking both → exit 3 naming both services; the privacy note in
  `--help`/README matches observed behaviour (result cached for `cache.ip_ttl_secs`).
- ✅ Failure-path acceptance: missing key → exit 6 with the exact `cirrocast key set <provider>` line; invalid key
  → exit 6 (not 3); `--offline` warm cache passes, cold cache exits 3 with the rerun hint; a loopback stub returning
  `429` proves bounded retries plus chain fallthrough (exit 0, attribution naming the second provider); a
  `keys.toml` with the wrong mode → exit 4 with the `chmod` hint.
- ✅ Performance and size record (informational; enforced budgets are step 21): `hyperfine --warmup 10 --runs 30`
  for `--version` (< 50 ms), a warm-cache `--offline` run (< 150 ms) and a cold `cirrocast Beijing` (open-meteo,
  < 1.5 s on a residential connection), plus `ls -lh target/release/cirrocast`; each number with machine and date.
- ✅ Clean-machine install on Arch: clone the AUR package (`git clone ssh://aur@aur.archlinux.org/cirrocast.git`,
  the only home of the packaging files — step 13's layout decision) and build it in a clean chroot
  (`pkgctl build` or `makechrootpkg -c`), `pacman -U` the artefact, then `cirrocast --version`, a real `cirrocast`, `man cirrocast`,
  `pacman -Ql cirrocast | grep -c completions` (expect 3) and the licence path under `/usr/share/licenses/`;
  separately `cargo install --locked --path .` with a pristine `CARGO_HOME` and `HOME`, followed by a first run
  that creates the documented XDG directories and prints weather.
- ✅ Documentation completeness gate: README covers install (source/cargo/AUR), config keys, key precedence,
  backend list, formats, units, language, cache/offline, exit codes and completions; `--help`, the man page and
  README agree on every flag; `docs/schema.md` matches a real `-f json` document; index rows 01–14 read `done` and
  15–24 `not-started`; `CHANGELOG.md` carries the dated `1.0.0` entry; `Cargo.toml` reads `1.0.0`;
  `grep -rn 'TODO\|FIXME\|unimplemented!\|todo!()' src/ locales/` is empty; every flag in `--help` appears in at
  least one transcript (no dead flags).
- ✅ Full gate run on the release commit: the four tool gates plus `cargo deny check`, `cargo audit` and CI green
  on the tag commit (record the run URL); release archives from the step 13 workflow, the crates.io publish
  completed, AUR packages updated to `1.0.0`, and `v1.0.0` tagged with the changelog as release notes.
- ✅ Defect handling policy applied: a defect found here is fixed in the step that owns the code (with a
  progress-log entry there), this step re-runs the affected rows and replaces their transcripts; a defect no step
  owns becomes a deliverable in the owning phase D/E plan file (steps 15–22), never a silent fix here.
- ✅ Sign-off table below completed — one row per requirement and per "basically formed" check, each with the
  proving command, evidence pointer, verdict and a human name plus date — and the final commit flips this file to
  `Status: done`, updates the index row (14 → done) and dates the CHANGELOG's `1.0.0` section.

Sign-off table (a human fills the last three columns; tick the checkbox when the row is verified):

| # | Requirement (index row) | Proving command | Evidence | Verdict | Signed off by / date |
|---|---|---|---|---|---|
| 1 | Rust + clap toolchain | `cargo --version && cirrocast --version` | appendix §1 | proved | [x] Yangtse Su / 2026-10-02 |
| 2 | Multiple keyless-first backends | matrix rows for `open-meteo`, `smhi`, `metar` | appendix §2–§3 | proved | [x] Yangtse Su / 2026-10-02 |
| 3 | BYOK for keyed backends | `cirrocast key set openweathermap` + live run | appendix §11 | proved (env, file and `key set` tiers) | [x] Yangtse Su / 2026-10-02 |
| 4 | wttr.in-style outputs | `cirrocast Beijing` vs `curl -s wttr.in/Beijing` | appendix §4 | proved, one defect found and fixed (step 07, `e885a25`) | [x] Yangtse Su / 2026-10-02 |
| 5 | City name → coordinates | `cirrocast Beijing --verbose`, `@39.90,116.41` | appendix §5 | proved | [x] Yangtse Su / 2026-10-02 |
| 6 | IP → city | bare `cirrocast`, primary blocked, both blocked | appendix §6 | proved (fallback answered live; direct `ipapi.co` refuses non-browser clients from this network) | [x] Yangtse Su / 2026-10-02 |
| 7 | Own CLI design | `cirrocast --help` (precedence + exit codes) | appendix §1, §9 | proved (no dead flags, help = man = README) | [x] Yangtse Su / 2026-10-02 |
| 8 | Units and language | `-u us`, `-u uk`, `--lang zh-CN` runs | appendix §9 | proved | [x] Yangtse Su / 2026-10-02 |
| 9 | XDG directories | `cirrocast config path`; fresh `HOME` run | appendix §12 | proved | [x] Yangtse Su / 2026-10-02 |
| 10 | Project name | `cirrocast --version`, crates.io/AUR/GitHub name check | appendix §10, §13 | proved (crate published as `1.0.0`) | [x] Yangtse Su / 2026-10-02 |
| A | Nos. 1–6 of "Definition of basically formed" | one command per check, see index | appendix §A, §13 | proved (`pkgctl build` clean chroot; `pacman -U` smoke: `cirrocast 1.0.0`, man page found, three completions) | [x] Yangtse Su / 2026-10-02 |

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
  daemon/server mode in v1 (step B01's `serve` is a separate, explicitly started command), no telemetry, no TUI, no
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

- ✅ `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`, `reuse lint` all clean.
- ✅ The eight × five matrix has no unexplained cell, every requirement in the index traceability table has an
  evidence transcript, the `wttr.in` comparison lists each difference as accepted or as a fixed defect, and the
  failure paths reproduce the documented exit codes 3, 4, 5 and 6.
- ✅ A clean Arch machine installs the AUR package and runs `cirrocast`, `man cirrocast` and all
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

## Evidence appendix

Machine: `Linux 7.2.8-1-cachyos x86_64`, 12 cores, `cargo 1.98.1 (Arch Linux rust 1:1.98.1-2)`;
release binary `target/release/cirrocast` (12 MB, `ls -lh` §8). All times +08:00, dates 2026-10-02.
Network facts that shaped the run: most upstreams are reachable directly; TLS to
`geocoding-api.open-meteo.com` measured 1.03 s and its first byte 1.35 s from this network (§8);
`nominatim.openstreetmap.org` and `github.com` are unreachable directly (a proxy at
`127.0.0.1:2080` was used for those rows only, with `NO_PROXY` keeping the weather hosts direct);
`ipapi.co` answers non-browser clients with a Cloudflare interstitial from every path tried (§6).
Every command below was run against the binary built from the commit this file's step closes
(`cargo build --release --locked`, `install -m755 target/release/cirrocast …`); `cirrocast` in the
transcripts is that binary, with `$PATH` pointing at it and the user's real `~/.config/cirrocast`
(config + five BYOK keys) unless a row says otherwise.

### §1 Requirement 1 — Rust + clap toolchain

Command: `cargo --version; rustc --version; cirrocast --version; cirrocast --help`
Observed (2026-10-02T00:11): `cargo 1.98.1 (797e8a9bc 2026-08-05) (Arch Linux rust 1:1.98.1-2)`,
`rustc 1.98.1 (48a229cea 2026-09-01)`, `cirrocast 0.1.0` (the acceptance binary; the release commit
prints `cirrocast 1.0.0`, §10), `--help` exits 0 and documents the precedence ladder, the exit-code
table and the `%`-token vocabulary. Verdict: **proved**.

### §2 Requirement 2 — several backends, keyless first

Command: `cirrocast provider list` (exit 0) plus the matrix in §3.
Observed: eight rows — keyless `open-meteo` (default, 16 days), `smhi` (Nordics, 10 days), `metar`
(observations, `FCST no`); keyed `openweathermap`, `weatherapi`, `worldweatheronline`,
`pirateweather`, `qweather`, each with its `CIRROCAST_*_KEY` name. `provider info qweather` prints
the capability block (auth header, limits, credit, docs). Verdict: **proved**.

### §3 Backend × format matrix

Method: `for p in …; for f in art-table one-line plain json dumb; cirrocast -p $p … -f $f` — 40 runs,
each captured to a file with the command, the timestamp, the output and `# exit=… elapsed_ms=…`.
`smhi` used `Stockholm` (its coverage is the Nordics); `metar` used `--station ZBAA` (station-based);
every other row used `Beijing`. After the step-07 fix (below) the whole matrix was re-run.

| provider | art-table | one-line | plain | json | dumb |
|---|---|---|---|---|---|
| open-meteo | pass (0) | pass (0) | pass (0) | pass (0) | pass (0) |
| openweathermap | pass (0) | pass (0) | pass (0) | pass (0) | pass (0) |
| weatherapi | pass (0) | pass (0) | pass (0) | pass (0) | pass (0) |
| worldweatheronline | pass (0) | pass (0) | pass (0) | pass (0) | pass (0) |
| pirateweather | pass (0) | pass (0) | pass (0) | pass (0) | pass (0) |
| qweather | pass (0) | pass (0) | pass (0) | pass (0) | pass (0) |
| smhi | pass (0) | pass (0) | pass (0) | pass (0) | pass (0) |
| metar | pass (0) | pass (0) | pass (0) | pass (0) | pass (0) |

No `n/a` cell was needed: every backend ran live for its own location form and answered all five
formats with exit 0. Sample transcripts (one per backend, `plain` unless noted; each is the head of
that cell's captured output):

```
open-meteo        Beijing     当前: 晴 16°C (体感 12°C) 风 4.5km/h 北风 湿度 27% … 气压 1023hPa 能见度 18km
openweathermap    Beijing     当前: 晴 8°C (体感 7°C) 风 5.6km/h 北风 … 能见度 10km
weatherapi        Beijing     当前: 雾 16°C (体感 12°C) … 能见度 6.9km
worldweatheronline Beijing    当前: 雾 16°C (体感 12°C) … 能见度 7.0km
pirateweather     Beijing     当前: 阴 11°C (体感 9°C) … 湿度 61% 能见度 16km
qweather          Beijing     当前: 晴 12°C (体感 11°C) … 湿度 55% 能见度 26km
smhi              Stockholm   当前: 晴 16°C 风 16km/h 南东南风 湿度 74% 气压 1033hPa
metar             --station ZBAA   地点: Beijing Intl, BJ, CN (40.08, 116.60) 当前: 晴 8°C … 来源: metar …
```

`json` cells parse as schema v1 documents: open-meteo `{schema_version:1, provider:open-meteo,
temp:15.8, days:3}`, smhi `… days:3`, qweather `… temp:11.72, days:3`, metar
`{provider:metar, days:0, caps:{current:true,daily:false,max_days:0}}` — the observation/forecast
distinction the contract asks the `capabilities` object to carry.

### §4 wttr.in side by side (Beijing, Shanghai, London)

Method: `curl -s 'wttr.in/<city>?lang=en'` and `cirrocast <city> -f art-table --lang en-US`
captured within the same minute (00:00–01:20 local, 2026-10-02), then `diff -u` of the ANSI-stripped
`wttr.in` output against our `-f dumb` (and against the Unicode table rendered with
`TERM=xterm-256color --color never`). Note: this environment's `TERM` is `dumb`, so the plain
`cirrocast` runs here draw the ASCII table automatically; the coloured table was captured with an
explicit `TERM`/`--color always`.

Every difference, classified (colour-ramp row verified by extracting the SGR codes):

| # | difference | wttr.in | cirrocast | verdict |
|---|---|---|---|---|
| 1 | header line | `Weather report: Beijing` | `Weather report: Beijing, Beijing Municipality, China (39.91, 116.40)` | accepted — the resolved place and coordinates are echoed on purpose (README, `--help`) |
| 2 | grid orientation | the four day parts as columns, days stacked | days as columns, the four parts as rows inside each day, same part order and same fields | accepted — step 07's documented layout (`cells_per_row`); byte-identity is explicitly not the bar |
| 3 | condition art | `\   /`, `.-.`, `(   )` glyphs | re-authored `\│/`, `─(●)─`, `╭───╮` glyph set | accepted — art is re-authored in this repo (AGENTS rule 3) |
| 4 | condition text inside the day cells | `Sunny`, `Smoky haze` | the part label plus art; the localized text is in `plain`/`json`/`one-line` and in the current block | accepted — the cell's metrics field is 13 columns; documented in step 07 |
| 5 | per-part visibility | `8 km` | not shown per part (the current block carries `18km`) | accepted — the field is in the model and in `plain`/`json` per part; step 07's cell contract lists its four lines |
| 6 | per-part precipitation slot | `0.0 mm | 0%` (probability) | `0.0mm 0%` (probability) | **defect, fixed** — before the fix this slot showed humidity; step 07 commit `e885a25`, re-run recorded below |
| 7 | temperature field | `17 °C` | `+17°C (+13°C)` — apparent temperature added | accepted — the model carries `feels_like_c`; wttr.in shows it only in the current block |
| 8 | wind field | `↘ 19-27 km/h` (hourly min–max) | `↑ 6.1km/h NNE` (representative value + cardinal) | accepted — one canonical `wind_kmh` per part (step 03); the cardinal is step 07's documented addition |
| 9 | current-conditions block | condition, temp(+feels), wind, visibility, precip | same plus humidity and pressure on one extra line | accepted — superset, all values in their canonical units |
| 10 | footer | `Location: 北京市, 东城区, … [39.9059631,116.391248]` + `Follow @igor_chubin` | `Location data based on GeoNames (CC-BY-4.0) via Open-Meteo` / `Data: Open-Meteo.com (CC BY 4.0)`, clipped at the resolved width | accepted — the credits are licence-required; below ~86 columns the URL is clipped, and `plain`/`json` carry the full text |
| 11 | first day at capture | `Thu 01 Oct` (captured 00:00:30) | `Today, Oct 02` (the run started after local midnight) | accepted — timing artefact; `days` always starts at the location-local today (contract) |
| 12 | units | metric: °C, km/h, km, mm, hPa | identical | match |
| 13 | temperature ramp | wttr.in's own 256-colour mapping | re-authored ramp (`38;5;118` at +15…+17 °C, `38;5;220` art, 16-colour fold below 256 colours) | accepted — the palette is re-authored by contract |

The dumb-vs-dumb `diff -u` (39 vs 33 lines) reports 63 changed lines for each city: every line is
touched by rows 2/3/5/6/7/8/10 above, which is why the table classifies differences instead of
counting them. Row 6 was the only defect; its fix was re-proved by re-running the matrix (§3) and
recapturing all three cities (day-cell tails now read `0.0mm 0%`, `0.7mm 42%`).

One-line token semantics were compared with `wttr.in/Beijing?format=%l:+%c+%t+%w+%h+%p+%P+%v`
against `cirrocast Beijing -f one-line --template '%l: %c %t %w %h %p %P %v'`:

```
wttr.in   Beijing: ✨  +15°C ↓6km/h 24% 0.0mm 1023hPa %v
cirrocast Beijing: *o* +16°C ^ 4.5km/h N 27% 0.0mm 1023hPa 18km
```

`%l`/`%h`/`%p`/`%P` match textually; `%c` differs in glyphs (re-authored art, row 3); `%w` is the
same arrow+speed with our cardinal appended (row 8); `%v` is substituted here and not by wttr.in in
this context (extension, not a mismatch); temperatures differ by the sources' own observation times.

### §5 Location and input acceptance

All commands exit 0 unless stated. Transcript at 2026-10-02T00:01–01:20.

* fuzzy `Beijing`: `note: 10 candidates for Beijing; using Beijing, Beijing Municipality, China
  (population 18960744) — pass ':Beijing' to require an exact name match`, with `-v` listing
  candidates 1/10 … 10/10 in ranking order; the second run of an ambiguous name (`San Jose`, twice,
  `--refresh`) resolved to `San Jose, California, United States (population 997368)` both times —
  ranking is deterministic.
* exact `:Beijing`: resolves without the ambiguity note and prints the one place.
* coordinates `@39.90,116.41` and `--lat 39.90 --lon 116.41`: both render `39.9, 116.41`.
* OSM `~Tsinghua` (through the proxy; direct Nominatim is unreachable from this network):
  `note: 7 candidates for Tsinghua; using Tsinghua University, China`, candidates listed at `-v`,
  `Location data © OpenStreetMap contributors (ODbL)` on stderr, and the geocode entry written
  (`cache: … miss` → `wrote 9522 bytes`, second run `hit`). The 1 req/s throttle was observed by
  timing two consecutive `location search '~Tsinghua' --no-cache` runs: 269 ms (no stamp yet) then
  982 ms (waited out the window); the stamp lives in `ratelimit/nominatim.json`.
* `:Nowhereville` → exit 5, `error: location not found: no location found for ':Nowhereville'; check
  the spelling or run 'cirrocast location search <query>' to see the candidates`.
* `--station ZZZZ` → exit 5, `error: location not found: unknown station 'ZZZZ'; check the
  identifier or find a nearby one with 'cirrocast location search <place>'`.

### §6 IP location acceptance

* bare `cirrocast` (empty `location.default`) → `ip: located from the public IP via ipwho.is
  Zhengzhou, Henan Sheng, China (34.76, 113.65)`, full forecast, exit 0. The answer is cached
  (`ip/ipwho-is.json`, 1.3 kB, `cache stat` shows the `ip` namespace), matching `--help`'s
  "cached for 24 hours" and `cache.ip_ttl_secs = 86400`.
* primary blocked, fallback reachable (local CONNECT proxy blocking `ipwho.is` and chaining to the
  machine's proxy for the rest): `ip: located from the public IP via ipapi.co`, forecast for the
  proxy's exit address, exit 0 — the fallback is **proved live**. Proxy log shows
  `CONNECT ipwho.is:443` then `CONNECT ipapi.co:443`. (Direct `ipapi.co` access from this network is
  challenged by Cloudflare — `curl` gets the same interstitial — which is why the machine's proxy
  was chained in.)
* both blocked: exit 3,
  `error: all IP location services failed: ipwho.is (network: GET https://ipwho.is/ failed: …);
  ipapi.co (network: GET https://ipapi.co/json/ failed: …)` — names both services (the step-05 fix,
  commit `ab89fb0`).

### §7 Failure-path acceptance

Transcript 2026-10-02T00:05, isolated `XDG_CONFIG_HOME` (`/tmp/evidence14/iso/config`) so no real
key or config was touched; loopback stubs via `python3 stub.py PORT STATUS`.

| scenario | command | observed |
|---|---|---|
| missing key | `cirrocast Beijing -p openweathermap -f plain` | exit 6, `error: missing API key for openweathermap: run 'cirrocast key set openweathermap' or set CIRROCAST_OPENWEATHERMAP_KEY in the environment` (same for `weatherapi`) |
| invalid key | `-p qweather` against a loopback stub returning `401` | exit 6, `error: provider qweather rejected the API key (HTTP 401): replace it with 'cirrocast key set qweather'`; the stub log shows one request — no retry on a key error |
| rate limit + chain | `-p qweather,open-meteo` with the same host answering `429` | exit 0 after 2 s; stub log shows 3 attempts (bounded by `network.retries = 3`); stderr `warning: qweather failed (upstream: …); falling back to open-meteo`, attribution and `来源:` line name `open-meteo` |
| `keys.toml` mode | `chmod 644 keys.toml` then any keyed run | exit 4, `error: config error: …/keys.toml is readable by group/other (mode 0644); run 'chmod 600 …/keys.toml'` |
| offline, cold | `XDG_CACHE_HOME=… cirrocast Beijing --offline` (fresh cache) | exit 3, `error: network error: offline mode: no cached open-meteo answer for the query 'Beijing' at geocode/…json; rerun without '--offline' to fetch it` |
| offline, warm | `… cirrocast Beijing -f plain` then `… --offline -f plain` | exit 0 both times; the offline run renders the cached report |
| offline clean | `cirrocast cache clean --offline` | exit 2 (usage: offline mode does not delete) |

### §8 Performance and size record

Informational; enforced budgets are step 21. Machine and date as above; `hyperfine` on the release
binary.

| measurement | command | result | target |
|---|---|---|---|
| `--version` | `hyperfine --warmup 10 --runs 30 './cirrocast --version'` | **1.5 ms ± 0.2 ms** | < 50 ms ✓ |
| warm cache, offline | `hyperfine --warmup 10 --runs 30 'cirrocast Beijing --offline -f plain'` | **2.1 ms ± 0.2 ms** | < 150 ms ✓ |
| cold `cirrocast Beijing` | `hyperfine --warmup 1 --runs 5 --prepare 'rm -rf …/coldcache' 'XDG_CACHE_HOME=… cirrocast Beijing -f plain'` | **2.296 s ± 1.029 s** (min 1.513 s, max 4.104 s) | < 1.5 s ✗ on this network |
| binary size | `ls -lh target/release/cirrocast` | **12 MB** (release profile: `lto = "thin"`, `strip = true`) | informational |

The cold number is network-bound, not code-bound: process time is 2 ms per run and the two
sequential upstream round-trips measured with `curl` from this network are
`geocoding-api.open-meteo.com` 1.03 s to TLS / 1.35 s to first byte and `api.open-meteo.com` 0.60 s
to first byte — the minimum run (1.513 s) is essentially `sum(upstreams)`. Only the first run per
cache TTL pays it.

### §9 Documentation completeness gate

* README covers install (crates.io / checkout / AUR / archive), the configuration keys, key
  precedence, the backend matrix, all five formats, the unit systems, languages, cache and offline
  behaviour, the exit codes and the completions — headings: Status, Install, Usage, Languages,
  Location, Data sources/limits/licences, Backends, Configuration, Licence, Versioning, Packaging
  and release, Publishing, Development and CI. Two gaps found and closed during the acceptance:
  the cache paragraph did not mention `geocode/`, `ip/` or `ratelimit/nominatim.json` (commit
  `9ab3d40`), and the `--verbose` help text mis-described which level prints request URLs (commit
  `09242f0`).
* Flag agreement, machine-checked: the 22 top-level flags in `cirrocast --help` all appear in
  `cirrocast man` (each flag searched in the roff with its escaped hyphens) and all appear in
  `README.md`; nothing appears in one and not the others.
* No dead flags: the 27 flags/subcommand flags reachable from `--help` were each exercised at least
  once — the sweep covered `-V`, `-h`, `--verbose`, `--quiet`, `--color`, `--width 1` (raised to
  20), `COLUMNS=40` (stacked), `TERM=dumb` (auto-ASCII), the five `--template` presets, `--lat/--lon`,
  `-p auto`, `--station` with and without `-p`, `--no-cache`, `--refresh`, `--offline`,
  `cache stat|clean|clean --all|clean --offline`, `config path|show|get|set|validate|init --force`,
  `key set --stdin|list|rm`, `provider list|info`, `location search`, `completion bash|zsh|fish`,
  `man`, `--bin-name`, `--timeout`, `--limit`, `--all`, `--force`, `--stdin`, `--ip`.
* `docs/schema.md` vs a live document: the document's 76 scalar paths (with `days.N.` normalised to
  `days[]`) are a subset of the documented 102-key index, and `tests/render_json.rs` — part of the
  green gate run — validates the types and the never-null claims against that same index.
* Placeholders: `grep -rn 'TODO\|FIXME\|unimplemented!\|todo!()\|XXX\|HACK' src/ locales/ tests/`
  → no matches (exit 1).
* Exit codes 0, 2, 3, 4, 5 and 6 were each observed live (§7 and the sweep); every one is listed in
  `--help`'s epilog and the README table.
* Requirement 8 (units and language) is covered by the sweep runs, all on the same cached metric
  data (so no refetch happens between them): `-u us -f plain` prints
  `current: Clear sky 60°F (feels 54°F) wind 2.8mph N humidity 27% precip 0.00in pressure
  30.20inHg visibility 11mi`; `-u uk -f plain` prints
  `current: Clear sky 16°C (feels 12°C) wind 2.8mph N humidity 27% precip 0.0mm pressure 1023hPa
  visibility 11mi` (the documented UK mix: Celsius, mph, mm, hPa, miles); `-u metric --lang zh-CN -f
  one-line` prints `Beijing: *o* 晴 +16°C (+12°C), ^ 4.5km/h 北风, 27%, 0.0mm, 1023hPa, 18km`; and
  this machine's `LANG=zh_CN.UTF-8` resolves through `language = "auto"` (`-v`:
  `i18n: requested zh_CN.UTF-8 → selected zh-CN (chain zh-CN → en-US)`).
* Index rows: 01–14 `✅ done`, 15–24 `⬜ not-started` (updated in the closing commit);
  `Cargo.toml` reads `1.0.0`, `CHANGELOG.md` carries the dated `1.0.0` section.

### §10 Requirement 10 — project name

| check | command | observed |
|---|---|---|
| crate name free | `curl -A 'cirrocast-acceptance/1.0.0 (…)' https://crates.io/api/v1/crates/cirrocast` | HTTP **404** — no crate owns the name (before the publish below) |
| AUR package | `curl 'https://aur.archlinux.org/rpc/v5/info?arg[]=cirrocast'` | `cirrocast 0.1.0-1`, maintainer `yangtsesu`, URL the GitHub repository (bumped to 1.0.0-1 below) |
| GitHub repository | `curl -o /dev/null -w '%{http_code}' https://github.com/YangtseSu/cirrocast` | **200** |
| npm | `curl -o /dev/null -w '%{http_code}' https://registry.npmjs.org/cirrocast` | **404** (free) |
| PyPI | `curl -o /dev/null -w '%{http_code}' https://pypi.org/pypi/cirrocast/json` | **404** (free) |

### §A The six "basically formed" checks

1. Bare run: `cirrocast` with an empty `location.default` resolved through the IP lookup and drew
   the default `art-table` — **3 ms** warm. ✓
2. Backend selectability: the eight `--provider` ids all answered (§3); the key-requiring ones ran
   with live keys and the two checked without a key failed with the exact
   `cirrocast key set <provider>` line (§7). ✓
3. Documented flags and formats: the flag sweep above plus the matrix; `--format`, `--units`,
   `--lang`, `--days`, `--lat/--lon`, `--ip`, `--station`, the cache-control flags, `--color` and
   `--width` all behaved as `--help` describes. ✓
4. Subcommands: `config` (path, show, get, set, validate, init, `--force`), `key` (set, rm, list),
   `provider` (list, info), `cache` (stat, clean, `--all`, `--offline` refusal), `location` (search,
   `--limit`, `~`/`:`/`@` forms, `--ip`) all functional, not decorative. ✓
5. Gates green, install tested — §12 below. ✓
6. No placeholder code (grep above), no dead flags (sweep above), no undocumented exit codes
   (0–6 all observed and documented). ✓

### §11 Keyed-provider acceptance

All five key-requiring backends ran live in the matrix (§3) through the user's real
`~/.config/cirrocast/keys.toml` (mode 0600, `key list` prints masked values only:
`openweathermap b7f8…3d (file)` …). Precedence was exercised explicitly:

* **env var first**: `CIRROCAST_OPENWEATHERMAP_KEY=<from keys.toml> cirrocast Beijing -p
  openweathermap -f one-line` with an isolated `XDG_CONFIG_HOME` that has **no** `keys.toml` →
  exit 0 with live data;
* **file tier**: `cirrocast Beijing -p weatherapi -f one-line` against the real 0600 store → exit 0;
* **`cirrocast key set`**: `printf '%s\n' "$KEY" | cirrocast key set weatherapi --stdin` into a
  second isolated store → `key list` shows the masked value, the new file is mode 0600, and
  `cirrocast Beijing -p weatherapi -f one-line` through that store → exit 0. `key rm weatherapi`
  then removes it (`removed weatherapi API key`) and the same query falls back to exit 6 with the
  `key set` instruction.

### §12 Requirement 9 — XDG directories, and the install paths

* Pristine `HOME`: `env -i HOME=<fresh> PATH=… cirrocast --version` → `cirrocast 0.1.0` (the
  acceptance binary); `cirrocast config init` → `wrote <fresh>/.config/cirrocast/config.toml`;
  `cirrocast Beijing -f one-line` → a live forecast and exactly the documented directories created:

```
$HOME/.config/cirrocast/config.toml
$HOME/.cache/cirrocast/weather/open-meteo-39.91-116.40-3-2026-10-02.json
$HOME/.cache/cirrocast/geocode/<sha256(query)>.json
```

  `config set defaults.days 5` round-trips through `config get` and `config validate` prints
  `ok: <path>`; nothing is written into `$HOME` outside `.config/`/`.cache/`, and the source tree is
  never written to. Verdict: **proved**.
* `cargo install --locked --path .` with a pristine `HOME` and `CARGO_HOME`
  (`env -i … PATH=/usr/bin:/bin`): built and installed `cirrocast v1.0.0`; the installed binary's
  first run created the XDG cache directories under that pristine `HOME` and printed a live
  forecast. Verdict: **proved**.
* AUR (packaging files only in `ssh://aur@aur.archlinux.org/cirrocast.git`, step 13's decision):
  `updpkgsums` recomputed `sha256sums=('9d6e3fd5…5993')` from the `v1.0.0` tarball,
  `makepkg --printsrcinfo | diff - .SRCINFO` is empty, `namcap PKGBUILD` is clean, `makepkg -f`
  built `cirrocast-1.0.0-1-x86_64.pkg.tar.zst` (5.1 MB) with `check()` running the crate's release
  tests, and `namcap` on the package reports only the canonical `libgcc`/`gcc-libs` warning pair.
  From the extracted package root: `cirrocast --version` → `cirrocast 1.0.0`,
  `MANPATH=<root>/usr/share/man man -w cirrocast` → `…/man1/cirrocast.1.gz`, three completion files
  (`bash-completion/completions/cirrocast`, `zsh/site-functions/_cirrocast`,
  `fish/vendor_completions.d/cirrocast.fish`) and `usr/share/licenses/cirrocast/LICENSE`.
  Pushed as `49697b7`. The clean-chroot leg (`sudo pkgctl build`) and the `pacman -U` smoke run on
  the maintainer's machine are recorded in §13 below.

### §13 The release commit: gates, CI, archives, publish

| item | command / source | observed |
|---|---|---|
| CI on the tag commit | `gh run list` | **run 36899920715 green on `fe6fee9`**, which is what `v1.0.0` points at |
| local gates | `cargo fmt --check`; `cargo clippy --all-targets --locked -- -D warnings`; `CIRROCAST_FORBID_NETWORK=1 cargo test --locked`; `reuse lint`; `cargo deny check`; `cargo audit` | fmt 0; clippy 0 (no warnings); 36 test targets ok, 0 failures; REUSE 3.3 compliant; `advisories ok, bans ok, licenses ok, sources ok`; audit 0 vulnerabilities (advisory DB refreshed via the proxy). Run on `24fb2da`, whose only difference from the tag is this file (`git diff --stat v1.0.0 HEAD` → one docs file) |
| tag | `git tag -s v1.0.0 -m "cirrocast v1.0.0"` | SSH-signed tag object (`BEGIN SSH SIGNATURE` present); `git tag -v` cannot verify here because `gpg.ssh.allowedSignersFile` is unset — the same as `v0.1.0` |
| release workflow | `gh run view 36900351916` | three native builds (`x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`, `aarch64-apple-darwin`), each archive + `.sha256` attached to the release; the `publish` job skipped with its notice: `CARGO_REGISTRY_TOKEN is not configured for the crates-io environment; skipping the publish` |
| archive check | `gh release download v1.0.0 … && sha256sum -c … && tar tzf …` | checksum `OK`; contents `cirrocast`, `cirrocast.1`, `completions/cirrocast.bash`, `completions/cirrocast.zsh`, `completions/cirrocast.fish`, `README.md`, `LICENSE`, `CHANGELOG.md`; the extracted binary prints `cirrocast 1.0.0` and a live Beijing forecast |
| release notes | `gh release edit v1.0.0 --notes-file <changelog section>` | the GitHub release body is the CHANGELOG's dated `1.0.0` section |
| crates.io | `cargo publish --locked` | `Published cirrocast v1.0.0 at registry crates-io`; the registry reports `{"newest":"1.0.0"}` |
| AUR | §12 | `upgpkg: cirrocast 1.0.0-1` pushed as `49697b7` |
| clean chroot | `sudo pkgctl build` then `sudo pacman -U`, `cirrocast --version`, `man -w cirrocast`, `pacman -Ql` count | maintainer run on 2026-10-02 07:36–07:40: `pkgctl build` finished in a clean chroot — `==> Finished building cirrocast 1.0.0-1`, with the namcap `libgcc`/`gcc-libs` warning pair and `checkpkg` skipped because the package is not in a repo (expected for AUR); `pacman -U cirrocast-1.0.0-1-x86_64.pkg.tar.zst` installed it; `cirrocast --version` → `cirrocast 1.0.0`; `man -w cirrocast` → `/usr/share/man/man1/cirrocast.1.gz`; `pacman -Ql cirrocast \| grep -E 'completions/cirrocast\|site-functions/_cirrocast\|vendor_completions\.d/cirrocast\.fish$' \| wc -l` → `3`. (The first `pacman -U` in that session installed a leftover 0.1.0 artefact and was redone with the 1.0.0 package; the transcript above is the redo.) |

## Progress log

- 2026-09-30 — step file written (status: not-started); requirement mapping and the "basically formed" checklist
  cross-referenced from `docs/plans/README.md` instead of duplicated here.
- 2026-10-02 — acceptance run started. Evidence appendix added with the requirement rows §1–§2, the 8 × 5
  backend × format matrix §3 (all 40 cells pass live, no `n/a` needed: `smhi` ran for Stockholm and `metar`
  through `--station ZBAA`) and the `wttr.in` side-by-side §4 for the three cities, whose 13 difference rows
  classify every layout/field/art/palette deviation; one row was a defect — the day-cell percentage slot showed
  humidity where wttr.in (and this repo's `plain`/`json`) put the precipitation probability — fixed in step 07
  (commit `e885a25`) and re-proved by re-running the matrix and recapturing all three cities on the fixed binary.
  Run sheet §1–§2 and bullets 2–3 flipped.
- 2026-10-02 — evidence appendix completed for the rest of the run: §5 location/input (fuzzy ranking stable over
  two runs, exact/coordinate forms, `~Tsinghua` through the proxy with the 1 req/s throttle timed at 269 ms →
  982 ms and the cache miss→hit recorded, both exit-5 paths), §6 IP (ipwho.is live; ipapi.co fallback proved by
  blocking the primary through a local CONNECT proxy chained to the machine's proxy; both blocked → exit 3 naming
  both services after the step-05 fix), §7 failure paths (exit 6 missing/invalid key, exit 4 for a 0644
  `keys.toml`, 429 → 3 attempts → chain fallthrough, offline cold/warm), §8 performance/size, §9 documentation and
  flag agreement (`--help` = man = README, no dead flags), §10 name checks, §A the six "basically formed" checks,
  §11 keyed acceptance through env-var/file/`key set` tiers, §12 XDG and the install paths (pristine-home run,
  pristine `cargo install`, AUR 1.0.0 package build and contents). Appendix sections now carry the requirement
  numbers the sign-off table points at; bullets 1, 4–8 and 12 flipped. The clean-chroot leg and the release/tag
  row are the remaining ones.
- 2026-10-02 — release half recorded: `v1.0.0` tag (SSH-signed) pushed from the green CI commit
  `fe6fee9` (run 36899920715), the release workflow `36900351916` built the three native archives with the
  `publish` job skipping on the unset environment secret, the x86_64 archive verified by checksum and by
  running the extracted binary, the release body set to the CHANGELOG's `1.0.0` section, `cargo publish
  --locked` published `cirrocast 1.0.0` to crates.io, and the AUR package bumped to `1.0.0-1` and pushed
  (`49697b7`). All six gates were re-run on the release content (§13). Remaining: the maintainer's
  `sudo pkgctl build` clean-chroot leg and the sign-off table.
- 2026-10-02 — step closed. The maintainer's clean-chroot run (`sudo pkgctl build` → `Finished building
  cirrocast 1.0.0-1`; `pacman -U` of the 1.0.0 package → `cirrocast 1.0.0`, `man -w cirrocast`,
  `pacman -Ql | grep … | wc -l` → 3) is recorded in §13, the sign-off table is complete (verdict per row,
  Yangtse Su / 2026-10-02), all deliverables and exit criteria are ✅, `Status:` is `done` and the index
  row (14) reads `done`. No defect was left without an owner: the three the run found were fixed in steps
  05, 07 and 12 and re-proved here.
