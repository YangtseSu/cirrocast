<!--
SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Step 14 — v1 acceptance

Status: ⬜ not-started
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

- ⬜ Evidence run sheet: for each of the ten requirements in `docs/plans/README.md#requirement-traceability` and
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
- ⬜ Clean-machine install on Arch: clone the AUR package (`git clone ssh://aur@aur.archlinux.org/cirrocast.git`,
  the only home of the packaging files — step 13's layout decision) and build it in a clean chroot
  (`pkgctl build` or `makechrootpkg -c`), `pacman -U` the artefact, then `cirrocast --version`, a real `cirrocast`, `man cirrocast`,
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
- ⬜ A clean Arch machine installs the AUR package and runs `cirrocast`, `man cirrocast` and all
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

Command: `cirrocast provider list` (exit 0) plus the matrix in §3 table below.
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
