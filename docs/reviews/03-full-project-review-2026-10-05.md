<!--
SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Full-project review — 2026-10-05

Reviewed at `7de979b` (v1.2.0), ~57 000 lines of Rust across `src/`, `tests/`, `build/geo-table`,
`scripts/bench`, the CI workflows and ~200 KB of documentation.

Predecessors: [`01-full-project-review-2026-10-02.md`](01-full-project-review-2026-10-02.md) and its
fix record [`02-review-01-fixes-2026-10-02.md`](02-review-01-fixes-2026-10-02.md). Findings from
those documents are not repeated unless they regressed: three fixes left the defect behind or created
a new one — the METAR cache-write regression (§3.1), the QWeather fog code the fix itself introduced
(§3.6) and the unreachable rounding branch its own fix left behind plus two stale comments it
missed (§4.4 #41, §5 nit 2).

## 1. Method

### 1.1 What the coordinator ran

| Gate | Command | Result |
|---|---|---|
| format | `cargo fmt --check` | clean |
| clippy | `cargo clippy --workspace --all-targets --locked -- -D warnings` | clean, 0 diagnostics |
| tests | `CIRROCAST_FORBID_NETWORK=1 cargo test --workspace --locked` | pass: 417 integration test functions in `tests/`, 350 in-crate unit tests, 5 in the `geo-table` builder, 23 doc tests, 4 ignored (live network) |
| licence | `reuse lint` | 373/373 files with copyright and licence information, compliant with REUSE 3.3 |

Live smoke runs of the shipped surface were executed as part of the review (weather query in the
default `art-table`, `dumb`, `plain`, `one-line`, `json`, `alerts`, `aqi`, `moon`; multi-location
runs including a failed slot; every `config`/`key`/`provider`/`cache`/`location` subcommand; the
offline scopes; the width ladder from 20 to 200 columns; the colour ladder under
`NO_COLOR`/`CLICOLOR_FORCE`/`--color`; the locale matrix; METAR and SMHI provider runs). Eleven
findings below are marked **[reproduced]** — re-observed by running the binary — and three more
**[source-verified]**, checked against the dependency's or the vendor's own source.

### 1.2 How the review was run

Eight read-only slices, never more than five subagents at once:

| Slice | Scope |
|---|---|
| `CliSurface` | `src/cli.rs`, `src/main.rs`, `src/lib.rs`, `src/error.rs`, `src/paths.rs`, `src/parallel.rs` |
| `ConfigHttpCache` | `src/config/`, `src/cache.rs`, `src/http.rs` |
| `Providers` | `src/provider/`, `src/alerts/`, `src/air/` |
| `RenderI18nModel` | `src/render/`, `src/template.rs`, `src/i18n.rs`, `src/model/`, `locales/` |
| `GeoAstro` | `src/geo/`, `src/astro/`, `build/geo-table` |
| `SecurityPrivacyLicense` | secrets, egress, local file handling, licensing, supply chain |
| `DocsConsistency` | `README.md`, `CHANGELOG.md`, `AGENTS.md`, `docs/**` |
| `ContractConformance` | the binding contract in `docs/plans/README.md` versus the code |
| `TestsAndCI` (six scouts) | `tests/**`, `.github/workflows/`, `scripts/bench/`, `deny.toml`, `perf/baseline.json` |

Every slice worked from the binding contract (`docs/plans/README.md`) and AGENTS.md, was forbidden to
edit anything or to run cargo, and had to cite `file:line` for each finding. **Every Major finding
in this report was re-checked by the coordinator against the code, against the dependency's own
source, or by running the binary**; the severities in this document are the coordinator's, not the
slices'. Claims that did not survive that check are recorded in §7 instead of being silently dropped.

### 1.3 Severity

* **Blocker** — a shipped guarantee is broken: panic/abort, secret disclosure, data loss, or plainly
  wrong output for an ordinary input.
* **Major** — a binding contract rule is violated, or a real defect is reachable through a normal
  command; the user or a downstream consumer sees something wrong, or a gate that should have caught
  it does not exist.
* **Minor** — robustness, an edge case, a misleading message, a stale comment, a weak test.
* **Nit** — wording with no behavioural effect.

## 2. Summary

| Severity | Count | Of which reproduced or source-verified |
|---|---|---|
| Blocker | 0 | — |
| Major | 16 | 10 |
| Minor | 96 (71 code/test findings + 25 documentation rows) | 5 |
| Nit | 14 | 4 |

**No Blocker was found.** That is the headline result: there is no input a normal user can type, and
no upstream payload a provider can return, that panics, aborts, leaks a secret or corrupts state on
the shipped paths. The closest candidates were rated Major because each one falsifies a claim the
code itself makes (§3.10, §3.11) rather than because they crash.

The shape of the findings is telling: **the defects cluster where a rule was written down but not
enforced**, not where the code is complicated. Twelve of the sixteen Majors are a written promise
that the implementation does not keep — a cache key missing a field, a validator accepting a value
the runtime rejects, a gate documented in `docs/performance.md` that no CI job runs, a credit line
that drops the link its licence requires. The genuinely intricate parts — the astronomy, the binary
city-table decoder, the WMO mapping, the unit layer — came back nearly clean.

Priorities for the next session, in order:

1. §3.6 — two QWeather mappings that report the wrong weather (one of them introduced by the previous
   review's own fix).
2. §3.1, §3.2, §3.4, §3.5 — four "the validator says yes, the runtime says no" defects; each one
   ships a configuration file that cannot run.
3. §3.3, §3.12 — the METAR cache-write regression and the HKO cache-key collision.
4. §3.14 — the missing `--no-default-features` CI gate.

## 3. Major findings

### 3.1 A cache-write failure still throws away a successful METAR fetch and exits 4

**Where** — `src/provider/metar.rs:254`, with `src/cache.rs:812-824`.

`Cache::read_or_fetch_json` was fixed for exactly this in review-01 §2.1: a fetch that succeeded must
not be discarded because the cache could not be written (`src/cache.rs:667-673` logs and continues).
`src/provider/metar.rs` carries its own copy of that routine, `cached_json`, and it still ends with
`env.cache.write(key, response.status(), &body, ttl)?;`.

**[reproduced]**

```
$ chmod 0555 $XDG_CACHE_HOME/cirrocast
$ cirrocast -p metar --station ZBAA
error: config error: cannot create …/weather: Permission denied (os error 13)
$ echo $?
4
```

The observation was decoded successfully; the report was thrown away over a cache directory the user
made read-only. `--station ZBAA` without `-p` reaches the same code path. The same run with the
default provider exits 0 and prints the report, because the geocode path is the one that was fixed.

**Fix** — use the swallow-and-log shape already present at `src/provider/metar.rs:342`, and add
`Cache::write_best_effort` so the three callsites cannot drift apart again.

### 3.2 `[alerts] sources` accepts `auto` mixed with explicit ids; every run then exits 2

**Where** — `src/config/mod.rs:1010-1013` (validator) versus `src/alerts/mod.rs:157-159` and
`src/alerts/mod.rs:107` (runtime).

`check_alert_sources` skips the token `auto` wherever it appears; `alerts::is_auto` requires it to be
the *only* entry, and `explicit_sources` parses every entry with `AlertSource::from_str`, which knows
nothing about `auto`.

**[reproduced]**

```
$ cirrocast config set alerts.sources auto,fpas   # exit 0
$ cirrocast config validate                       # ok: …/config.toml
$ cirrocast Beijing
error: unknown alert source `auto`; known sources: nws, meteoalarm, qweather, hko, wmoswic, fpas, visualcrossing
$ echo $?
2
```

The error names the documented selector keyword as unknown. A file that passes both `config set` and
`config validate` fails every run.

**Fix** — reject a list mixing `auto` with anything else in `check_alert_sources`, with a message
that names the key.

### 3.3 The HKO alert cache key omits `lang`, so one run poisons the next

**Where** — `src/alerts/hko.rs:41,63` (`super::key(env, "hko-warnsum", loc)`) with
`src/alerts/mod.rs:266-270` (`CacheKey::alert(source, lat, lon, hour)`).

The summary request is the only place a request parameter varies with the user's language — the
request adds `?lang=tc|en` — and the cache key does not include it.

**[reproduced]**

```
$ cirrocast --lat 22.30 --lon 114.17 --alerts -vv --lang zh-CN
cache: …/alerts/hko-warnsum-22.30-114.17-20261005T02.json: miss
cache: …/alerts/hko-warnsum-22.30-114.17-20261005T02.json: wrote 189 bytes
$ cirrocast --lat 22.30 --lon 114.17 --alerts -vv --lang en-US
cache: …/alerts/hko-warnsum-22.30-114.17-20261005T02.json: hit      ← no request at all
```

An English run is served the Traditional Chinese warning text for the rest of the UTC hour.
`--refresh` hides it. The same collision makes a four-location run write four identical HKO entries,
since the request carries no place parameter.

**Fix** — include `lang` in the key (`CacheKey::hash("alerts", …)` as `document_key` already does
for FPAS identifiers).

### 3.4 `visualcrossing` is a valid `[alerts] sources` value that always fails at run time

**Where** — `src/model/alert.rs:224-232` (`AlertSource::ALL`, seven entries),
`src/config/mod.rs:1003-1024` (`check_alert_sources` validates against `ALL`),
`src/alerts/mod.rs:104-108` (`if !source.available() { return Err(Error::Usage(…)) }`).

`docs/schema.md:199` and `:448-449` document `visualcrossing` as a legal id; the CLI's `--help`
lists only six.

**[reproduced]**

```
$ cirrocast config set alerts.sources visualcrossing   # exit 0
$ cirrocast config validate                           # ok
$ cirrocast Beijing
error: alert source `visualcrossing` is not wired up yet
$ echo $?
2
```

AGENTS.md rule 4 forbids "will be wired later" code in `src/`, and `Error::Usage` (exit 2) is the
wrong class for a value that came from a file. `--alerts-from visualcrossing` has the same effect but
only *after* the location lookup has already opened a socket — the "usage error that must not cost a
request" its own doc comment (`src/cli.rs:1128-1131`) promises.

**Fix** — either drop `VisualCrossing` from `ALL` until its provider lands, or reject it in
`check_alert_sources` with an `Error::Config` that names the key.

### 3.5 An empty or whitespace-only argument bypasses `location.default` and queries the public IP

**Where** — `src/cli.rs:1680-1688` (`location_arg`), `src/config/mod.rs:2109-2112`
(`cli.location.clone().or_else(|| non_empty(&config.location.default))`), `src/geo/mod.rs:80-82`
(`LocationSpec::Default`), `src/cli.rs:814-825` (the verbose `location:` line).

clap's `StringValueParser` accepts an empty positional; `location_arg` forwards it verbatim, and
`Some("")` short-circuits the configured default. The contract says "empty = config
`location.default`, else public IP" — here a location *is* configured and the empty argument wins
anyway.

**[reproduced]** with `[location] default = "Beijing"`:

```
$ cirrocast "" --offline=all -v
location:  (from the command line)
error: all IP location services failed: ipwho.is (network: offline: no cached ipwho-is location
answer for the public IP …); ipapi.co (network: offline: no cached ipapi-co location answer …)
```

Without `--offline` that is a real request to ipwho.is and ipapi.co — a privacy-rule-11 violation
for a `$UNSET_VAR` expansion or a shell loop that yields an empty string. The verbose line also
prints `location:  (from the command line)` with nothing after the colon. `cirrocast "" --ip` is
exit 2 ("a location argument cannot be combined with `--ip`") for the same reason.

**Fix** — treat an all-whitespace single positional as absent in `location_arg`, and skip the
`location:` verbose line when the merged value is empty.

### 3.6 Two QWeather condition codes map to the wrong weather

**Where** — `src/provider/qweather.rs:588` and `:597`.

QWeather's own condition table (`dev.qweather.com/en/docs/api/weather/weather-conditions/`) reads:

| code | text |
|---|---|
| `307` | 大雨 / Heavy Rain |
| `308` | 极端降雨 / Extreme Rain |
| `309` | 毛毛雨/细雨 / **Drizzle Rain** |
| `310` | 暴雨 / Storm |
| `515` | **Extra Heavy Fog** (the strongest member of the 500 fog family) |

**[source-verified against the vendor's documentation]**

* `307..=312 | 316..=318 => 65` (heavy rain) swallows `309`, so drizzle renders as
  `cond-65 = Heavy rain` in every format, with the heavy-rain art block — and because
  `severity_rank` decides `dominant_condition` inside a day part, an afternoon drizzle is shown as
  the day's heaviest weather. Drizzle is one of the most common conditions in QWeather's largest
  market.
* `515 => 56` renders "Extra Heavy Fog" as `cond-56 = Light freezing drizzle`.

The second arm is a **regression introduced by the previous review's own fix**: review-01 §4.1
asserted "515 is freezing drizzle", fix 4.1 encoded that, and the premise was wrong — 515 is the top
of the 500 fog family, and QWeather has no freezing-drizzle code in the 500 range at all. No fixture
covers either code (`tests/provider_qweather.rs` exercises 100-103), so CI is green.

**Fix** — `307..=308 | 310..=312 | 316..=318 => 65` with `309 => 53` (drizzle), and fold `515` back
into the fog family; add fixture rows for both codes and amend the review-01 record, whose premise
was false.

### 3.7 Multi-location `--pick` is nondeterministic

**Where** — `src/cli.rs:1087` (`crate::fetch_reports(&targets, |_, target| slot_report(&context,
target))`), `src/cli.rs:1120-1123`, `src/geo/pick.rs:25-32` (`static PROMPT: Mutex<()>`),
`src/parallel.rs:57-60` (`next.fetch_add(1, Ordering::Relaxed)`).

The prompt mutex serialises the prompts but assigns them no order: `par_map_ordered` hands slot
indices to whichever worker thread reaches the counter first, so which location consumes the first
line of stdin is scheduling-dependent. `cirrocast Beijing Shanghai Guangzhou --pick` prints three
numbered lists, reads three lines, and the answer `2` may select Guangzhou on one run and Beijing on
the next — different stdout, different `-f json` `query` fields, different `selected:` echoes, for
identical input. That breaks AGENTS.md golden rule 12 and the contract's "same inputs → same output".
`-vv` interleaves for the same reason. No test covers multi-location plus `--pick`.

**Fix** — resolve every prompt serially, in argument order, before the parallel fetch (a pre-pass that
returns `Vec<Location>`), or refuse the combination in `validate_query` the way `--ip`, `--station`
and `--lat/--lon` are refused with more than one argument.

### 3.8 `days[0]` is not the location-local today for five of the eight backends

**Where** — `src/provider/dayparts.rs:59-79` (`covered_days` keeps only dates whose four day parts all
have a sample), called from `openweathermap.rs:365`, `qweather.rs:437`, `smhi.rs:234`,
`pirateweather.rs:313`, `worldweatheronline.rs:311`.

The binding model says `days` "is ordered oldest → newest and **always starts at the
location-local today**". `covers_every_part` requires a sample in all four six-hour bands, so a
run before 06:00 local (or against a series that starts late in the day) drops today entirely and
`days[0]` is tomorrow. Open-Meteo and WeatherAPI honour the clause, so the two families disagree
inside one binary. The behaviour is deliberate and pinned by `tests/provider_smhi.rs:67-84` ("the
first day a `DayPart` can be built for is the 1st"), which is exactly why the contract and the code
disagree without either gate noticing.

A consumer that reads `days[0].date` as "today" is wrong for those five backends in the first six
hours of the local day; the cache key's `<local-date>` component and the `--days` clamp message are
computed from the real local today, so the report and its own cache key disagree.

**Fix** — decide which side is wrong and make the other agree: either `covered_days` starts at local
today and carries a partial day (with the missing parts marked), or the contract sentence is amended
and `Report` documents that `days[0]` is the first *complete* local day.

### 3.9 The HTTP body cap does not bound a gzip response

**Where** — `src/http.rs:25-26` (the claim), `src/http.rs:531-536`
(`response.body_mut().with_config().limit(MAX_BODY_BYTES).read_to_vec()`), `Cargo.toml:71`
(`ureq = { version = "3.4.2", features = ["rustls"] }` — default features on).

**[source-verified in the dependency]**

`ureq` 3.4.2 builds the body reader as
`MaybeLossyDecoder<CharsetDecoder<ContentDecoder<LimitReader<BodySourceRef>>>>`
(`ureq-3.4.2/src/body/mod.rs:783`, constructed at `:519-521`): the `LimitReader` sits **inside** the
content decoder, so `MAX_BODY_BYTES` counts bytes off the socket and `flate2`'s `MultiGzDecoder`
output flows into `read_to_vec`'s `Vec<u8>` unconstrained. `Cargo.lock:1298-1312` confirms `flate2`
is live (pulled in by ureq's default `gzip` feature). The module doc's "a broken or hostile upstream
cannot make the client allocate without bound" is therefore false for any upstream that answers
`Content-Encoding: gzip`.

Trigger: a compromised or hostile weather/IP/alert endpoint — or an on-path attacker against one of
the three service URLs that accept plain `http://` (`network.nominatim_url`, `alerts.fpas_url`,
`geo.update_url`). A few hundred KiB of gzip expands to tens of GiB and the process is OOM-killed
(exit 137, an undocumented code). Expected: a typed `Error::Upstream` naming the cap and exit 3.

**Fix** — `ureq = { version = "3.4.2", default-features = false, features = ["rustls"] }` (no
upstream this project talks to needs transparent gzip, and `flate2` stays a direct dependency for
the embedded city table), or reject a non-identity `Content-Encoding` before `read_to_vec`. Either
way amend the doc sentence so it describes what is enforced.

### 3.10 No redirect policy: a custom credential header is forwarded to another host

**Where** — `src/http.rs:487-494` (the whole agent build: timeouts, UA, proxy — no
`max_redirects`, no `redirect_auth_headers`), `src/provider/qweather.rs:208-210` and
`src/alerts/qweather.rs:53-55` (`.header("X-QW-Api-Key", key)`).

**[source-verified in the dependency]** — `ureq` defaults to `max_redirects: 10`
(`ureq-3.4.2/src/config.rs:950`) and `ureq-proto-0.6.4/src/client/redirect.rs:76-92` removes only
`Authorization`, `Cookie` and `Content-Length` on a hop. QWeather's credential is a *custom* header,
so it survives. The provider's own doc (`src/provider/qweather.rs:178-181`) states the invariant this
breaks: "any other authority would send it to a third party".

Trigger: the account host (user-configurable via `[providers.qweather] host`) answers
`302 Location: https://elsewhere.example/v1/…`; `cirrocast` follows it, keeps the key, and delivers
it. Nothing in `-v`/`-vv` reveals it, because only the URL is logged. `MeteoAlarm` uses
`Authorization` and is covered by the default.

**Fix** — `.max_redirects(0)` on the agent: every upstream in the registry answers its final URL
directly, so nothing in the project needs redirect following, and a 3xx then maps to
`Error::Upstream` (exit 3) with the provider named.

### 3.11 The weather-data credit omits the service link its licence requires

**Where** — `src/provider/mod.rs:134` (`licence: Some("Open-Meteo.com (CC BY 4.0)")`), printed by
`src/render/plain.rs:92`, `src/render/art_table.rs:1065` and `src/render/json.rs:731`.

The binding attribution clause (`docs/plans/README.md:352-363`) fixes the line as
`Weather data by Open-Meteo.com (https://open-meteo.com/)`. The shipped string has neither the
wording nor the URL, and **[reproduced]**:

```
$ cirrocast Beijing --days 0
…
Location data by GeoNames (CC BY 4.0) — https://www.geonames.org/
数据： Open-Meteo.com (CC BY 4.0)
```

The GeoNames side of the credit does carry its link; the Open-Meteo side does not. CC-BY-4.0 asks for
credit *plus a link to the material*, and for a `--lat/--lon` or `--ip` run no URL appears anywhere in
the output at all (`src/geo/mod.rs:459-465` returns `None` for coordinates/IP/station). The
divergence is knowingly recorded at `docs/providers.md:119`, which means the contract sentence and the
shipped output are both deliberate and inconsistent — one of them has to change.

**Fix** — put the contracted string (link included) in the registry rows and pin each with a test;
amend `docs/providers.md:119` in the same commit.

### 3.12 `Current::wind_dir_deg` is not optional, so a variable wind is reported as due north

**Where** — `src/model/mod.rs:123-124` (`pub wind_dir_deg: u16`), `src/provider/metar.rs:608`
(`wind_dir_deg: decoded.wind_dir_deg.unwrap_or(0)`), consumed at `src/render/plain.rs:168`,
`src/render/art_table.rs:603` and `src/template.rs:836`.

The METAR decoder gets it right: `decode.rs:147-148` documents `None` for a variable (`VRB`) wind and
`:462` produces it. The provider then substitutes `0`, which every renderer prints as a definite
north wind. The day-part twin is already `Option<u16>` (`src/model/mod.rs:276-277`) — review-01 §6.6
fixed exactly this shape for `%w` and left `Current` behind. In-tree fixture:
`tests/fixtures/metar/LFPG/raw.metar:1` = `METAR LFPG 010000Z VRB02KT 9999 BKN016 18/16 Q1018 NOSIG`,
with `tests/metar.rs:109-114` asserting `wind: (None, 2.0 * 1.852, None, true, false)` — direction
`None`, `wind_variable = true`. The substitution then discards the decoder's `wind_variable` flag
entirely. The same collapse happens for an OpenWeatherMap observation with no `wind.deg`.

**Fix** — make `Current::wind_dir_deg` an `Option<u16>`; the three consumers already have the
day-part path's handling to copy.

### 3.13 A CAP document with `status` `Test`/`Exercise`/`Draft` is shown as a live warning

**Where** — `src/alerts/cap.rs:425` (`Leaf::Status => self.document.status = non_empty(&text)`) and
`src/alerts/cap.rs:512-524` (which returns early only for `msgType == "Cancel"`).

The GeoJSON twin of the same message *is* filtered: `src/alerts/nws.rs:111-113` drops `is_test`. So a
drill published through FPAS, WMO SWIC or MeteoAlarm — exactly the channels civil-protection exercises
use — renders in the banner and in `-f alerts` as a live
`⚠ Severe Thunderstorm Warning — Extreme`, while the identical NWS payload is dropped.

**Fix** — mirror the NWS guard in `alerts_from_cap`, and add one fixture row per aggregator.

### 3.14 No CI job builds or tests the `--no-default-features` build, although the docs call it a gate

**Where** — `.github/workflows/ci.yml` (ten jobs, none of them passes `--no-default-features`;
`grep -rn 'no-default-features' .github/` returns nothing), against
`docs/performance.md:47-48` ("the exit criteria are met by measurement, not by assumption: *the
reduced build compiles*; *`CIRROCAST_FORBID_NETWORK=1 cargo test --workspace --no-default-features
--locked` passes*"), `docs/plans/21-perf-and-resource-budget.md:199` and `CHANGELOG.md:85-87`.

The claim is true today — it was verified by hand on 2026-10-05 — but nothing re-verifies it. The
reduced build's own code has **zero** test coverage: `tests/offline_geo.rs:12` and
`tests/offline_lazy.rs:11` are `#![cfg(feature = "offline-geo")]`, so they compile to nothing, and
`grep 'cfg(not(feature' tests/` has no matches at all. The two stub paths
(`src/cli.rs:2050-2056` and `src/cli.rs:2272-2276`) are therefore never even compiled by CI.

A later edit to those blocks — or to any signature they must match — will not fail CI, and the
"city search falls back to the network geocoder" capability a user can build will silently stop
compiling.

**Fix** — a `cargo build --workspace --no-default-features --locked` +
`cargo test --workspace --no-default-features --locked` step, plus one integration test asserting the
stub's user-visible message under `#[cfg(not(feature = "offline-geo"))]`.

### 3.15 `index/capurl`/`hubLink` are fetched verbatim from an upstream payload

**Where** — `src/alerts/meteoalarm.rs:187-197` (`hub_url`: any absolute `http(s)` URL is used
as-is), `src/alerts/wmoswic.rs:101-110` (the same shape against `CAP_ROOT`).

Every other outbound host in the binary is checked: `is_service_url` for `network.nominatim_url` and
`alerts.fpas_url`, `is_qweather_host` for `[providers.qweather] host`. These two are chosen by the
*data*. A compromised aggregator — or an on-path attacker on a cleartext hop, which the `http://`
branch explicitly allows — can point `cirrocast` at any host, including a link-local address, and the
answer is fetched, cached and parsed as a CAP document.

No credential is attached to the document request, so this is a blind request-forgery primitive
rather than an exfiltration path, and the upstream is already trusted for the alert content itself.
That is why this is Major rather than Blocker: it breaks the project's own "every host is attributable
to a flag or a config key" model and is undocumented in the README's data-source table, not because it
crosses a new privilege boundary.

**Fix** — resolve relative links against the documented base (as the fallback branch already does)
and reject an absolute link whose scheme is not https or whose host is not the querying host, exactly
as `document_key` already refuses an FPAS identifier that would traverse.

### 3.16 A point-in-polygon test that never unwraps longitudes drops warnings across ±180°

**Where** — `src/alerts/geometry.rs:113-127` (CAP polygon) and `:63-88` (GeoJSON ring).

Both run the even-odd cast on raw longitudes. A polygon crossing the antimeridian — the normal way an
EEZ or national warning is drawn for Fiji, Kiribati, Tonga, Samoa or Chukotka — has its straddling
edges at +180 and −180, so `lon < xj` is false for every edge and `inside` never flips. A point at
`(−17.5, 178.45)` inside such a polygon is reported as outside, and the alert is silently dropped.
The same ring reports false positives near 0°, which is the sole client-side gate in
`meteoalarm.rs:70-73`.

**Fix** — unwrap each ring before casting (if `max_lon − min_lon > 180`, shift the western half by
±360 and test the point at `lon`, `lon±360`).

## 4. Minor findings

### 4.1 Provider decoding

| # | Where | Finding |
|---|---|---|
| 1 | `src/provider/qweather.rs:597` | `condition_of` maps an *unparsable* code string to `0` ("Clear sky") via `unwrap_or(0)`, while the arm below it keeps unknown codes undescribed. |
| 2 | `src/provider/qweather.rs:575-600` | Codes `154` (night overcast) and the `200..=213` wind family have no arm and fall through to `Unknown`. |
| 3 | `src/provider/weatherapi.rs:483,486` | `1243` ("moderate or heavy rain shower") and `1258` (…snow showers) share an arm with their *light* twins, so the WMO code understates severity by one band; WWO's 356/371 map correctly to 81/86. |
| 4 | `src/provider/worldweatheronline.rs:471` | `248 | 260 => 45` folds *freezing* fog (260) into plain fog, while WeatherAPI's equivalent 1147 maps to 48 — two providers disagree about one phenomenon, which is what the canonical-WMO rule exists to prevent. |
| 5 | `src/provider/openweathermap.rs:464` | `701..=781 => 45` maps smoke, haze, dust, sand, squalls **and tornado** to `Fog`, although the model describes WMO 4/5/6/7. The in-crate comment calls the collapse intentional; the justification ("WMO has no equivalent") is true only for 762/781. |
| 6 | `src/provider/pirateweather.rs:357-364` | A `-999` sentinel in `humidity`, `cloudCover` or `windBearing` propagates through `?` and discards the **entire** `current` block — `current: null` in `json` — for a fully decodable observation. |
| 7 | `src/provider/metar/decode.rs:231-255` | Only the wind group is first-wins. A `TEMPO`/`BECMG` block before `RMK` has its clock read as a metric visibility (`0600` → 6 km) and its temperature overwriting the observation's. |
| 8 | `src/provider/metar/decode.rs:25-26` | The decoder's own table promises `R06L/2000FT`, `VCSH` and `RE…` are handled; `is_rvr` requires an all-digit runway token and `WEATHER_DESCRIPTORS` has no `RE`, so all three are dropped. |
| 9 | `src/provider/open_meteo.rs:524-527` | An out-of-range `weather_code` is clamped into 0..255 instead of staying undescribed — `-1` becomes "Clear sky" — contradicting the doc comment directly above it. |
| 10 | `src/provider/open_meteo.rs:136-146` | `ForecastResponse` carries `elevation`, `utc_offset_seconds` and `timezone_abbreviation`, none of which any code reads; `utc_offset_seconds` has no `#[serde(default)]`, so an otherwise usable payload missing that one key fails the whole decode. |
| 11 | `src/provider/mod.rs:251`, `src/provider/smhi.rs:234` | **[reproduced]** The registry declares `max_days: 10`; `-p smhi -d 10` renders 6 day columns with no warning at any verbosity, because SMHI's 12-hour steps leave whole day parts empty. `provider info smhi` does document the widening, the run does not. |
| 12 | `src/air/open_meteo.rs:290-300` | A partially-null pollen block is completed with `0.0`, contradicting `src/model/air.rs:19-22` ("`0 grains` and `not measured` are different answers"). Two doc comments inside `model/air.rs` also contradict each other here. |
| 13 | `src/air/open_meteo.rs:211-218` | The `-v` "AQI above 500" note is printed before the reading is validated, so it can describe a panel the same function then refuses to produce. |
| 14 | `src/alerts/cap.rs:455-466` | An `<expires>` that does not parse (ISO-8601 basic offset `+0200`, fractional seconds) becomes "no end", and `model/alert.rs:391-399` reads a missing end as "live forever" — an immortal warning. |
| 15 | `src/alerts/geometry.rs:63-127` | An unparsable polygon vertex is dropped silently (the tested shape differs from the issued one) and a structurally broken GeoJSON ring reports `Some(false)` where the module's own rule says untestable geometry must be *kept*. |
| 16 | `src/model/alert.rs:286-290` | The QWeather coverage box is "over mainland China" (18–54 °N, 73–135 °E) and HKO's spans all of 22.1–22.7 °N, so Delhi is told `qweather` covers it and Shenzhen gets Hong Kong warnings; only coordinate-only locations reach this. |
| 17 | `src/cli.rs:1409` vs `src/alerts/mod.rs:20-22` | A source list from `config.toml` is not treated as an explicit request, so its failure degrades to a `-v` note — the module's documented "explicitly named sources are promises" applies only to the flag. |
| 18 | `src/alerts/meteoalarm.rs:105` | The token is read with `std::env::var` instead of the crate's `env_value`, so a non-UTF-8 value yields "no `CIRROCAST_METEOALARM_KEY` configured" although it is set. |

### 4.2 HTTP, cache and config

| # | Where | Finding |
|---|---|---|
| 19 | `src/config/mod.rs:1277-1281` | `atomic_write`'s temp name is keyed on the **process id** only, so two threads of one process writing the same cache path unlink each other's in-flight temp file. `cirrocast Beijing Beijing` reaches it: the cache is silently truncated (self-healing) and a spurious `cache write failed` line appears at `-vv`. |
| 20 | `src/http.rs:90-124` | `is_loopback_url` splits the authority on `:` before stripping userinfo, so `http://localhost:8080@evil.com/` is classified loopback and slips past the `CIRROCAST_FORBID_NETWORK` guard while `ureq` dials `evil.com`. Reachable via a hand-set `network.nominatim_url`, which `is_service_url` accepts. |
| 21 | `src/http.rs:456-466` | A `socks5://` proxy from the *environment* is accepted by `Proxy::try_from_env`, then ignored: ureq emits a `log::warn!` and the crate installs no logger, so the request goes out direct with no warning anywhere. A silent proxy bypass. |
| 22 | `src/http.rs:487-494` | No `timeout_global`, so `--timeout` bounds each phase but never the request: a body trickling one byte per `timeout − 1` seconds keeps the process alive indefinitely. The flag's help says "per-request timeout". |
| 23 | `src/http.rs:675-676` vs `:686-692` | The doc says `Retry-After` applies to `429`/`503`; the code consults it on every retried status, so a `500` carrying `Retry-After: 120` sleeps 60 s per retry instead of 0.5 s. |
| 24 | `src/config/mod.rs:1900` | `config set location.default …` routes to the whole-`[location]` checker, so an unrelated invalid `location.pick` or a cyclic `[locations]` alias makes the command fail — the same class of coupling review-01 §2.3 fixed for the other 40-odd keys. `[locations]` has no `config set` row at all, so there is no repair path. |
| 25 | `src/http.rs:514-516`, `src/provider/mod.rs:490-501` | `HttpRequest::timeout`/`timeout_duration` and the `ThreeHourly`/`Daily` variants of `HourlyResolution` have no production caller; `pub` items escape `dead_code`, so clippy cannot see it. |
| 26 | `src/cache.rs:841-861` | `entry_files` has no prefix filter, so `cache stat` counts a crashed run's `.<name>.tmp.<pid>` leftovers and `cache clean` (without `--all`) never removes them. |
| 27 | `src/cache.rs:46`, `:773` | `cache stat` reports a hardcoded five-name list; `ratelimit/` and `geo/` — both written by the code (`nominatim.rs:46`, `update.rs:40`) — are never reported, and `clean --all` leaves `geo/update-notice.json` (which suppresses the `[geo] update = "check"` freshness note for 24 h) behind. |
| 28 | `src/cache.rs:322-337`, `:376-389` | Two `weather/` key shapes — the `<part>` variant and the air-quality variant — are absent from the contract's cache-layout clause; only the air one is documented anywhere (step 16's file). |

### 4.3 Geo, city table and the builder

| # | Where | Finding |
|---|---|---|
| 29 | `src/geo/table.rs:475-500`, `:366-380` | `Index::decode` reserves two `Vec`s sized from the file-supplied `count` (`u32`, `usize::try_from` never fails on 64-bit) before anything is bounds-checked, and `gunzip` has no output cap. A crafted or bit-rotted user member turns `cirrocast <name>` into a multi-gigabyte allocation instead of the documented `UserTableIssue::Corrupt` fallback. |
| 30 | `src/geo/table.rs:626-631` | `wanted.len() - cities.len()` can underflow when a row section repeats a geonameid (which `parse_dump` does not reject): a debug build panics — forbidden outside `#[cfg(test)]` — and a release build prints `18446744073709551615 rows`. |
| 31 | `src/geo/rank.rs:20-24`, `:82-90`, `:118-124` | Three doc comments state the ranking is a total order over (tier, population); it is not — equal tier and equal population (including two `None` collapsed to 0) fall back to source order, so `~Springfield` can resolve to a different place between runs. |
| 32 | `src/geo/nominatim.rs:117-141` | The 1 req/s throttle is a read-modify-write of a state file with no lock, so `cirrocast "~Aachen" "~Berlin"` sends both requests inside the same second from the two worker threads — breaching the usage policy the file exists to honour. |
| 33 | `src/geo/ip.rs:220-221`, `:247-248`; `src/geo/open_meteo.rs:173-190` | These two JSON paths copy `latitude`/`longitude` into a `Location` without the finiteness/range check `nominatim.rs:305-312` performs, so `1e400` → `inf` travels into the provider URL and the header (`(inf, 116.40)`). |
| 34 | `src/geo/update.rs:301-341` | The decompression cap is compared against the size the ZIP *declares*, then `read_to_end` inflates without a limit and the length is only checked afterwards: a mirror declaring 1 byte and inflating 4 GB is bounded by nothing. |
| 35 | `src/geo/update.rs:204-213` | `install` renames the three members one at a time while its own doc comment says "the previous table [is] kept until every file is in place"; a crash between renames leaves a mismatched pair that passes eager validation and fails every lookup with exit 4. |
| 36 | `src/geo/update.rs:146-155` | A `--from` value that is not `http(s)://` goes to `fs::read` verbatim, so `file:///srv/dump.zip` fails with `cannot read file:///srv/dump.zip: No such file or directory` and exit 1 instead of a usage error naming the unsupported scheme. |
| 37 | `src/model/mod.rs:60-61` | `LocationSource::Config` is never constructed on any shipped path (only a match arm, two unit tests and the `json` mapping), yet `docs/schema.md:62` lists `config` as a legal `location.source` value — AGENTS.md rule 10 forbids dead variants, and the value set a consumer may match on is frozen. |
| 38 | `build/geo-table/src/main.rs:97` | `-h`/`--help` is routed through `Err(usage())`, so the builder prints `error: usage: …` and exits 1; and `candidate` builds its client from `Network::default()`, ignoring the maintainer's proxy configuration in the documented pre-tag `--check` step. |

### 4.4 Render, model and i18n

| # | Where | Finding |
|---|---|---|
| 39 | `src/render/moon.rs:63-68`, `src/render/air.rs:67-70` | Both standalone views fold to ASCII **after** clipping to `--width`, and `fold_ascii` widens `—` to `--`: `--format moon --width 30` on a no-moonrise day emits a 32-column line. `art_table` folds first and gets the order right (`art_table.rs:178-187`). |
| 40 | `src/render/air.rs:361-364` | The claim "escape sequences … contain no spaces, so a painted value survives a wrap intact" is false — `index_entry` paints `43 (Good)`, and `wrap` splits on spaces, so at widths 22-28 a closing `\x1b[0m` is pushed to the next line and the SGR is never reset. |
| 41 | `src/model/units.rs:452-497` | The "ulp nudge" branch in `round_half_away_from_zero` is unreachable (`delta.signum() == value.signum()` cannot hold after `round()`), and an 8.8 M-pattern sweep plus 200 k random values fired it zero times. The function is exactly `f32::round`; `ulp()` and the ~65-line doc/doctest exist only for the dead branch. This came from review-01 §5.4's fix. |
| 42 | `src/render/json.rs:85-110` | The multi-location document is re-serialised through `serde_json::Value` (a `BTreeMap` without `preserve_order`), so **object keys come out alphabetical** in the array form and in declaration order in the single form. [reproduced: two-location output starts `"air"`, single-location starts `"schema_version"`]. |
| 43 | `src/render/json.rs:195-211` | `location.lat`, `lon` and `elevation_m` skip the `-0.0` normalisation every other float gets, so `--lat=-0 -f json` emits `"lat": -0.0`, against the module's own "every number … has no negative zero" rule. [reproduced] |
| 44 | `src/render/alerts.rs:206-218` | `--format alerts` ignores the resolved width and writes CAP `headline`/`description`/`instruction` verbatim; the width clause exempts only `plain` and `json`, while `README.md:250` documents `alerts` as ignoring `--width`. |
| 45 | `src/cli.rs:1804-1805`, `src/render/art_table.rs:126-131` | `--format dumb` forces `ColorMode::Never` and `ColorDepth::Mono`, silently discarding an explicit `--color always`. |
| 46 | `src/render/mod.rs:257-264` | `--color always` under `TERM=dumb` *upgrades* `Mono` to `Ansi256`, emitting 256-colour sequences into a terminal that advertised fewer than sixteen — the clause says fold down to sixteen. |
| 47 | `src/render/art_table.rs:513-521` | The observed line converts a clock per rendered field (`with_timezone(&Utc).format("%H:%M")`), which the `times` clause forbids, and prints `Z` rather than the location offset. |
| 48 | `src/model/mod.rs:90-95` | `Location::station` carries no `#[serde(default)]` although the clause names it; the back-compat promise currently rests on serde's implicit `Option` handling. |
| 49 | `src/template.rs:625-635`, `:708-720` | Every unknown token recomputes `template[..offset].chars().count()`, so CLI template validation is Θ(len²): a 1 MB `--template-file -` appears to hang instead of printing the typo report. |
| 50 | `src/render/mod.rs:646-649` | `renderer_for` re-resolves the template against an **empty** `[templates]` map, so `-f <name>` for any configured template whose body starts with `@` fails with `unknown one-line preset … configured [templates]: none`. |
| 51 | `src/template.rs:80-81`, `src/render/moon.rs:133`, `src/model/air.rs:110-113` | Four user-visible English literals reach stdout in every language: the `sun` preset's "sunrise"/"sunset" (both catalogs have `astro-sunrise`), the `uv` preset's "UV", `moon.rs`'s " at " in `本地计算，无需网络 at …`, and the air-quality credit line. |

One more in this area, unnumbered because it is the only instance of its kind: `src/i18n.rs:621`,
`:729` and `:734` call `.expect("… is a valid BCP-47 tag")` on three constants. The literals are
constants, so the paths are unreachable today, but AGENTS.md rule 8 and the contract's error clause
state the ban without an exception, and a scan of all 65 runtime files found no other production
`unwrap`/`expect`/`panic!`.

### 4.5 CLI surface and flags

| # | Where | Finding |
|---|---|---|
| 52 | `src/cli.rs:1384-1396` | `--severity` with `--no-alerts` is silently ignored, while `--alerts-from` and `--format alerts` in the same position are refused with `Error::Usage`. |
| 53 | `src/cli.rs:1916-1918`, `:1992-1994` | `location search`/`update-data` resolve the timeout as `args.timeout.unwrap_or(config.network.timeout_secs)`, skipping the `CIRROCAST_TIMEOUT` tier that `KEY_TABLE` declares and `config get` reports. |
| 54 | `src/cli.rs:292-294` vs `src/config/mod.rs:82` | `--width` accepts 1..=500; the same setting in `config.toml` accepts only `0` or 40..=500 (`config set render.width 39` → exit 4), with no stated rationale. [reproduced] |
| 55 | `src/geo/mod.rs:80-82` | An out-of-range or non-numeric `@lat,lon` such as `@91.0,0` or `@nan,0` silently becomes an alias lookup, so a typo reports `unknown location alias @91.0,0`. The contract's "otherwise `@name`" sanctions it; a diagnostic would not contradict the contract. |
| 56 | `src/cli.rs:1572-1581` | The `selected: <place> — use @<lat>,<lon>` echo is suppressed by `-q`, although the contract states it unconditionally — the reproducibility affordance disappears in the run that was told to pick. |
| 57 | `src/cli.rs:900-906` | `--station` with a *configured* `defaults.provider` chain replaces the chain with `metar` alone rather than prepending, and the same combination is refused with exit 2 when the chain was typed on the command line. |
| 58 | `src/cli.rs:1832-1845` | `--template-file -` preserves the template's trailing newline and then appends one, so `printf '%%l\n' \| cirrocast -f one-line --template-file -` emits two lines from a one-line format. [reproduced] |

### 4.6 Tests and CI

| # | Where | Finding |
|---|---|---|
| 59 | `.github/workflows/ci.yml:117-125` | The render/model import gate is a line-based `grep`, defeatable by ordinary Rust: a multi-line grouped `use crate::{ cache::Cache, … };` and `crate :: cache::Cache` both pass. Verified against a synthetic tree; **no current violation exists** in `src/render` or `src/model` — the gate's coverage, not its verdict, is the finding. |
| 60 | `tests/cli.rs:23-25` | A second bare `cirrocast()` helper bypasses `common::Sandbox` entirely, so six CLI tests run against the developer's real `HOME`, real `XDG_CONFIG_DIRS`, real `CIRROCAST_*` and **without the network guard** — against the comment at `tests/common/mod.rs:105` ("Every CLI test runs with the network guard on"). |
| 61 | `tests/no_network.rs:110-114`, `tests/cli.rs:362`, `tests/cli_flags.rs:44`, `tests/multi_location.rs:50` | Four files build the seeded cache key from `chrono::Utc::now().date_naive()`, so a run crossing local midnight between seeding and executing the binary misses the entry and fails with a message that reads like a product bug. Two of the four use the UTC date where the key is keyed on the Asia/Shanghai date. |
| 62 | `tests/cli_flags.rs:70-95` | Thirteen of the twenty "both spellings" flag cases assert only `"Weather report:"`, a header the renderer prints regardless of the flag — nine of them would pass with the flag deleted. |
| 63 | `tests/cli_flags.rs:523-531` | Inside `the_exit_code_table_is_reachable_end_to_end`, code 6 is a library assertion on a locally constructed `Error`, with a stale comment ("only reachable once a key-requiring backend exists"); `qweather` exists and `tests/exit_codes.rs:141` already drives it through the binary. |
| 64 | `tests/cli_offline.rs:329-357` | `no_fixture_carries_an_api_key` reads the developer's real `$HOME/.config/cirrocast/keys.toml` and returns early twice — on CI it asserts nothing — and walks `Path::new("tests/fixtures")` relative to the cwd instead of `fixture_path()`. |
| 65 | `tests/exit_codes.rs` | The file that advertises "one deterministic scenario per binding code" has no scenario for code 1, and reaches code 3 only through the guard's own message — `Error::Upstream` and `Error::InvalidToken` are unpinned at the process boundary (both are asserted only as constructed values). |
| 66 | `tests/exit_codes.rs:141-152` vs `tests/cli_flags.rs:458-531` vs `src/error.rs:158-210` | The 0–6 table is driven end-to-end in two files with different fixtures, and asserted a third time as a hand-built value table; six of seven scenarios are near-duplicates. |
| 67 | `tests/alerts.rs:718-742` | The MeteoAlarm bad-token test also passes `--offline`, so no HTTP request is made and the `InvalidToken` conversion (`src/alerts/meteoalarm.rs:117-122`) is never exercised — the regression it guards is exactly "tell the user to run `cirrocast key set meteoalarm`", a command that does not exist for that service. |
| 68 | `tests/` (grep) | No integration test runs the binary with `NO_COLOR` or `CLICOLOR_FORCE`; the only colour assertion uses the explicit `--color always`, which bypasses the whole ladder (`src/render/mod.rs:195-200`). Swapping the two branches would turn every `NO_COLOR` user green-on-escapes with a fully green suite. |
| 69 | `tests/` (grep) | No test writes `[network] offline = …` and runs the binary, so "the config supplies the default and the flag wins" (`src/cli.rs:2408-2412`) is asserted only as a method (`src/config/mod.rs:2849-2878`). |
| 70 | `tests/` (grep) | `--offline=all` — the spelling `CHANGELOG.md:135-136` tells users to adopt — is never passed; only the bare `--offline` and `--offline=weather|geo` are. |
| 71 | `ci.yml` (no `permissions:`) | The CI workflow declares no `permissions:` block, so every job — including `pull_request` runs of contributor-modified test code — inherits the repository's default `GITHUB_TOKEN` scope, and three third-party actions receive it implicitly. `release.yml` gets this right. |

### 4.7 Documentation drift (compressed)

| Where | Finding |
|---|---|
| `README.md:778-779` | Still documents a third credential tier, "→ the OS keyring (feature-gated, later release)". Review-01 fix 7.2 removed it from the contract and `keys.rs` but missed the README. |
| `README.md:75`, `:89-90` | "Steps 01–20 of 29 … are in place" and "Phase E — the performance and resource budgets … is next", while step 21 is `Status: ✅ done`; `docs/performance.md` (the enforced budget document) is linked from nowhere in the README. |
| `README.md:949-960` | The CI table claims "the check matrix is the same locally and in CI" while listing `cargo clippy --all-targets` / `cargo test --locked` (no `--workspace`, the exact trap `AGENTS.md:102` warns about) and omitting the `perf` and `record-baseline` jobs entirely. |
| `AGENTS.md:99-106` | "CI … runs exactly those commands" — it now also runs `scripts/bench/run.sh`, `compare.py`, `record.py`, `cargo bloat` and `cargo llvm-lines`, none of which appears in AGENTS.md. |
| `docs/plans/README.md:149-208` | The binding module map omits 15 shipped modules: `lib.rs`, the whole `src/alerts/` tree, `geo/table.rs`, `geo/update.rs`, `provider/dayparts.rs` and `provider/metar/{decode,station_table}.rs`. |
| `docs/plans/README.md:375-401` | The binding config block omits the shipped `[air] index` key that `config init` writes into every new file. |
| `docs/plans/README.md:388-389`, `:398` | The same block lists `[geo] search`, `[geo] reverse` and the whole `[normals]` table — keys `check_known_keys` rejects. A file transcribed verbatim from the binding block **fails `config validate` with exit 4**. |
| `docs/plans/README.md:449-478` | The binding CLI synopsis omits `--aqi`, `--aqi-index` and `--moon` (all shipped, all in `--help`), and the `-f` list omits the `short`/`default`/`uv`/`sun` presets the same document lists 75 lines earlier. |
| `docs/plans/README.md:266-269` | "the interim fixed list `open-meteo,met-no,smhi`" — `met-no` is not a `ProviderId`; `auto_chain()` derives `[open-meteo, smhi]` and never consults coverage. |
| `docs/plans/README.md:464` vs `docs/schema.md:449` | The contract's `--alerts-from` list omits `visualcrossing`; `docs/schema.md` lists it. (See §3.4 — one of the two must change.) |
| `docs/providers.md:68-73` | Four wrong rows in the status-code table: `403` is mapped to exit 6/"chain stops" (only `401` is refined, `src/provider/mod.rs:570-572`); a `400` invalid-parameter is mapped to `Error::Usage` (no such mapping exists); QWeather is credited with a `404` (it signals `400`); Open-Meteo is listed under "200 with an error envelope" (it sends it with `400`, and `ForecastResponse` has no error field). The document's own §393-394 contradicts row `:68`. |
| `docs/providers.md:93-96` and six more sections | Seven of the eight "Response fields consumed" lists describe the upstream payload rather than the client's `Deserialize` structs (they cite `current_units`, `alerts`, `astro.moonrise`, `clouds[]`, `wind.compass`, … that no struct carries). Only SMHI's matches its wire struct. |
| `docs/plans/21-…md:19` | The step is closed `✅ done` while its Goal still promises "the default release binary under 5 MB" and "RSS under 15 MB", numbers its own exit criteria record as exceeded by ~2.9× (14.36 MiB / 25.6 MiB in `perf/baseline.json`). |
| `docs/plans/04-…md:27`, `20-…md:35` | Two ticked deliverables of closed steps specify `Error::Location`, a variant that does not exist; the enum has only `Error::LocationNotFound`. |
| `docs/plans/04-…md:9` / `05-…md:9` vs `README.md:70` | Steps 04 and 05 declare a mutual `Depends on`, which the workflow rule ("every entry of its `Depends on` line is done") cannot satisfy; the index table records only half of it. |
| `docs/plans/README.md:49` | The phase-D milestone still says steps 19 and 20 "remain"; both are `✅ done` and shipped in 1.2.0. |
| `docs/plans/README.md:512` | Calls `LICENSES/GPL-3.0-or-later.txt` a "sibling **link**"; it is a deliberate copy that a CI `cmp` gate protects (restoring a symlink is the exact defect step 12 removed). |
| `docs/performance.md:113`, `:190` | The "Baseline (2026-10-05)" column quotes `--version` at 2.10 ms; `perf/baseline.json` records 2.029 ms (the quoted figure is the regression-proof run). |
| `docs/providers.md:37`, `:238` | Table headers say "verified 2026-09-30" for `metar` and `qweather`, whose registry rows carry `verified: "2026-10-01"` — and the document promises the two agree. |
| `docs/providers.md:234` | Calls METAR "the second keyless backend"; it is the third in registry and step order, and `auto` excludes it entirely. |
| `docs/providers.md:233-274` | METAR is the only backend section with no printed-credit paragraph, although the registry carries one. |
| `docs/plans/README.md:403-405`, `src/cli.rs:133-136` | The override ladder lists seven `CIRROCAST_*` variables; three more exist (`CIRROCAST_LOCATION_PICK`, `CIRROCAST_NOMINATIM_URL`, `CIRROCAST_IP_SERVICE`) and neither document nor `--help` mentions them. |
| `README.md:494-497` | Offers `--exact` as "the `:` prefix equivalent" on the weather query, where the flag does not exist (`cirrocast Beijing --exact` → exit 2 from clap). |
| `README.md:176-182` | The subcommand synopsis omits `location update-data` and `config init --force`. |
| `deny.toml` `[bans]` comment | The recorded reason for not skipping `getrandom` ("only dependent chain … through a dev-dependency") is false on the face of `Cargo.lock`: `0.2.17` is reached through `ring` (runtime), `0.4.3` through `tempfile` (dev-only). Either the CI job is red or the justification misleads the next reviewer. |

## 5. Nits

1. `src/render/json.rs:8` — the module doc says the document carries `"schema_version": 1`; the
   constant is 2.
2. `src/i18n.rs:1104`, `:1147` — the doc comments say `Wed, Sep 30`; the en-US catalog is
   `Wed 30 Sep`. Review-01 §6.11 fixed the same wording in `one_line.rs` and missed these two.
3. `src/template.rs:711-717` — the unknown-token message prints the letter, not the spelling the
   user typed (`%y` for `%12y`).
4. `src/render/art.rs:702-706` — the "every condition has a glyph" assertion compares against `""`
   while `one_line_art` ends in a `"???"` catch-all, so it can never fail.
5. `src/geo/rank.rs:12` — the intra-doc link target does not exist under `--no-default-features`.
6. `src/geo/pick.rs:81-82` — the prompt reads one line into an unbounded `String`.
7. `src/config/keys.rs:188-203` — a dangling symlink at `keys.toml` yields `No such file or
   directory` from a path `symlink_metadata` just reported as present.
8. `src/cache.rs:758-812` — `cache clean --all` counts crashed-run temp files as entries.
9. `docs/schema.md:214-346` — the worked example omits `"astro"`, which the emitter always writes;
   the key index has no row for the failed-slot keys `query`/`error.code`/`error.message` nor for
   the object-vs-array rule.
10. `AGENTS.md:22-40` — the repo map omits `src/lib.rs`, `src/air/`, `src/astro/`, `src/alerts/`,
    `src/i18n.rs` and the `build/geo-table` workspace member.
11. `src/provider/metar/decode.rs` / `src/provider/open_meteo.rs:136-146` — three unused deserialised
    fields on the Open-Meteo response (covered once, here, rather than twice).
12. `src/render/air.rs` and `src/render/moon.rs` — the standalone panels wrap to `--width` but the
    header line is not fitted, so a very narrow `--width` can still clip a panel.
13. `build/geo-table/src/main.rs:198-201` — the builder's `--help` exit code (see §4.3 #38).
14. `tests/multi_location.rs:75-97` — an ordering test that duplicates `src/parallel.rs:96` and names
    each report with the string it later asserts on, so a swap could not be diagnosed.

## 6. Verified clean

Recorded because a review that only lists defects misrepresents the work. Each item was checked in
the code, in the fixture, or by running the binary.

**Gates.** `cargo fmt --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`,
`CIRROCAST_FORBID_NETWORK=1 cargo test --workspace --locked` and `reuse lint` (373/373 files) are all
clean at `7de979b`.

**Runtime discipline.** No `unwrap()`, `panic!` or `Result::expect` outside `#[cfg(test)]` in any of
the 65 runtime source files — the only three are `.expect()` on constant BCP-47 literals in
`src/i18n.rs:621,729,734` (§4.4). `src/geo/table.rs`'s `cursor.expect(MAGIC)?` is the crate's own
method, `?`-propagated.

**Secrets.** `keys.*` is absent from `KEY_TABLE`, so no code path can write a key into `config.toml`.
`check_mode` runs *before* `read_to_string` and refuses any group/other bit
(`src/config/keys.rs:188-206`); writes go through `atomic_write(…, 0o600)`. `key set` takes no value
positional — the secret arrives from `rpassword` or stdin. `key list` masks
(`SUPE…ef (file)`, [reproduced end-to-end]). `redact` replaces both the raw and the percent-encoded
spelling of a secret, longest first. A live check with a planted `openweathermap` key and `-vv`
leaked it nowhere: not on stdout, not on stderr, not in the cache tree.

**Privacy.** No telemetry, no analytics, no phone-home. The IP services are address-of-the-caller
URLs with **no query parameter** (`src/geo/ip.rs:41-44`), so the address never appears in a URL the
code builds; a failed chain names each service tried. `ip_ttl_secs` is capped at 86 400 with a `-v`
note. §3.5 is the one hole found.

**Determinism.** `tests/cli_offline.rs:468-473` asserts byte-identical stdout *and* stderr across two
runs; `tests/render_snapshots.rs:435-462` across three locales including `tr_TR`. All time and tty
input is injected (`RenderContext::now`, `TermCaps`, `Transport`); no test reads `chrono::Local`.
Locale negotiation was exercised across twelve `--lang` values: POSIX spellings normalise
(`zh_CN.UTF-8` → `zh-CN`), the fallback chain is printed under `-v`, and an unknown tag falls back to
`en-US` rather than failing.

**Location.** All five argument forms behave as the contract specifies, including the edge cases
(`@39.9,116.4,5` → alias, `@ 39.9 , 116.4` → coordinates, `@,` → alias → usage error, `:`/`~`/`@`
alone → usage error). The bundled table was decoded independently of the Rust code: 34 152 rows, all
geonameids unique, 310 502 sorted keys with sorted, deduplicated id lists, zero trailing bytes, every
row referenced, matching `src/geo/data/SNAPSHOT`. Name folding cannot diverge between the builder and
the runtime because both call `geo::fold::fold`; `MÜNCHEN`, `São Paulo`, `saopaulo` and `peking` all
resolve through it. Lazy loading is real and asserted (`tests/offline_lazy.rs`). The Nominatim retry
loop no longer bypasses its throttle (review-01 §2.8 fix confirmed at `nominatim.rs:110-132`).

**Model and units.** Every provider maps its native codes to canonical WMO except the four rows in
§4.1; day-part aggregation (boundaries, the representative sample, sum/max/severity rules) was checked
against every backend's fixture and matches `wttr.in`'s four rows. Unit conversion happens only in
`src/render/` and `src/model/units.rs`; providers rescale only between SI units the API dictates
(m/s → km/h, kt → km/h, m → km, inHg → hPa). The `fetch`/`fetch_report` split means no backend can
skip the finite-reading guard. `parts_in_order` rejects a misordered report with a typed error.

**Astronomy.** The formulas were re-derived independently (Meeus 25, 22, 47, 12) and compared with
JPL Horizons fixtures: illumination within 0.13 pp (the module claims 0.14), Meeus example 49.a/49.b
phase instants to 0.54/0.39 min. ΔT is applied in exactly one place; every position series is
evaluated at TT while GMST and the hour angle are evaluated at UT; the year is clamped to 1900-2150;
polar days report `Polar::Day|Night` instead of a clamped `00:00`; the local-day window is
DST-correct (a spring-forward day is 23 h, the ±3 h margin covers it).

**Alerts and air.** CAP parsing is a hand-written state machine with no entity expansion; the chain
falls through only on `Network`/`Upstream`; a usage/key/location error aborts the chain before the
next backend runs; the key check precedes every credentialed call; `ProviderId::from_str` round-trips
every `as_str()` value case- and separator-insensitively and rejects unknown ids rather than dropping
them; `auto` excludes `metar` (station-only) and `--station` prepends it. The AQI breakpoints were
checked against the documented US and European scales.

**Providers.** No `std::fs`, `TcpStream`, `ureq` or `Command` anywhere in `src/provider/`,
`src/alerts/` or `src/air/`. Sentinel handling is correct per backend (`-999` for Pirate Weather, `9999`
for SMHI, no invented sentinels for QWeather). Open-Meteo's parallel arrays are length-checked with a
typed error and an absent `precipitation_probability` degrades to `None`, not to zero.

**Rendering.** Width was measured live from 20 to 200 columns: no line ever exceeds the resolved width
in `art-table`; below 60 the layout stacks; `COLUMNS=45` is honoured. Colour: no escapes in `json` or
`plain` even under `--color always`, escapes present in `art-table` under `--color always`,
`TERM=dumb` selects the ASCII charset automatically, `NO_COLOR` (including empty) is honoured.
Attribution is decided in exactly one place (`geo::attribution_line`) and each of its four branches
matches the contracted wording, licence included.

**Docs and licensing.** `reuse lint` reports full compliance, `LICENSE` and
`LICENSES/GPL-3.0-or-later.txt` are byte-identical, every CI action is pinned to a commit SHA, runner
images are named explicitly, `--locked` is used wherever a gate could otherwise update the lockfile,
`INSTA_UPDATE=no` is set for the matrix, and `cargo deny` runs with `--all-features --locked`.

## 7. Investigated and refuted

Recorded because a review that reports only what survived its own checks overstates its coverage.
Each of these was reported by a slice and then disproved by the coordinator.

1. **"`cirrocast` never calls `Config::validate()` on the weather-query path, so
   `[providers.metar] station` reaches a cache *filename* unvalidated and writes outside
   `$XDG_CACHE_HOME`."** — **False.** `Settings::resolve` calls `config.validate()?` at
   `src/config/mod.rs:2086`, and `run_query` calls `Settings::resolve` at `src/cli.rs:984`. A
   hand-edited config with `provider = "metar"` and `station = "/../../../tmp/pwned"` was run: exit 4,
   `providers.metar.station: `/../../../tmp/pwned` is not a four-character ICAO station identifier`,
   nothing created outside the cache root. Review-01 blocker 1.1 (the `socks5://` panic) is likewise
   still fixed on that path — a hand-edited `network.proxy = "socks5://127.0.0.1:1080"` produces
   `error: config error: network.proxy: `socks5://127.0.0.1:1080` is not an `http://` or `https://`
   proxy URL`, exit 4, no panic. The grep that produced the claim looked for `validate()` at the
   `cli.rs` call sites and missed the one inside `Settings::resolve`.
2. **"`[alerts] sources = ["nws"]` in `config.toml` is silently downgraded"** — **Partly false.** The
   configured list is indeed not treated as an explicit request (`src/cli.rs:1409` reads only
   `--alerts-from`), so the *failure* degrades to a note, exactly as §4.1 #17 records. The claim that
   this contradicts `src/alerts/mod.rs:20-22` is right, and it is kept at Minor.
3. **`serde_json` rejects an out-of-range float literal, so the missing coordinate validation in
   `src/geo/ip.rs` is only a style difference.** — **Not settled**, and §4.3 #33 is filed on that
   basis: the two modules differ (`nominatim.rs` validates, `ip.rs` and `geo/open_meteo.rs` do not) and
   the difference is observable only if `1e400` parses to `inf`. Left as Minor with the assumption
   stated.
4. **`tests/cli.rs` runs against the developer's real environment** — **True, reclassified.** It is
   filed as §4.6 #60 (Minor). The slice rated it Blocker; nothing a user can observe depends on it, and
   the six affected tests are `provider list`, `config path` and four config-syntax cases.
5. **"Four test files race against the local date when seeding cache keys."** — **True, reclassified**
   to §4.6 #61 (Minor): the window is milliseconds wide and opens once per day.

## 8. What this review did not cover

* Windows and non-Unix targets (the project ships none, and CI runs no such job).
* The `build/geo-table` output was **not** rebuilt from the upstream GeoNames dump: that check
  downloads the official dump, and AGENTS.md makes it a pre-tag step run deliberately outside
  `CIRROCAST_FORBID_NETWORK=1`. The builder's determinism was instead checked by reading
  (`BTreeMap` accumulation, sorted-and-deduplicated id lists, `gzip()` pinning `mtime(0)`, no wall
  clock in `snapshot_text`) and by its own test (`the_build_writes_all_three_files_and_is_deterministic`).
* The 8 ignored live-network tests were not executed.
* `cargo deny` and `cargo audit` were not run (the CI jobs own them); `deny.toml`'s `getrandom`
  justification (§4.7) was checked against `Cargo.lock` only and is filed as unverified on both
  branches.
* Performance was not re-measured: `perf/baseline.json` was read, not re-recorded. The `--no-default-
  features` gate it claims (`docs/performance.md:47-48`) was confirmed absent from CI but the build
  itself was not run here.

## 9. Suggested order of work

1. **§3.6** — the two QWeather mappings, with fixture rows and a corrected review-01 record. It is a
   wrong-forecast defect in a widely used backend and the fix is three lines.
2. **§3.2, §3.4** — make `check_alert_sources` agree with what the runtime accepts (`auto` mixing, and
   `visualcrossing`). Two validators, one message each, and no configuration file can then validate
   and fail.
3. **§3.1, §3.12** — the METAR cache-write `?` and the HKO cache key. One regression fix, one key fix.
4. **§3.14** — add the `--no-default-features` CI job plus one test for the stub paths, so the claim in
   `docs/performance.md` is enforced rather than asserted.
5. **§3.5** — map an empty/whitespace positional to "absent" and add the regression test; the privacy
   rule is cheap to keep and expensive to lose.
6. **§3.9, §3.10** — `max_redirects(0)` and the gzip decision; then amend the two doc sentences that
   currently promise more than the code delivers.
7. **§3.11** with **§4.7** — the credit line and the contract sentence in one commit, since either
   alone leaves them disagreeing.
8. **§3.7, §3.8, §3.13, §3.16** — determinism, the `days[0]` invariant, CAP `status`, antimeridian
   geometry. Each needs a design decision (serialise the prompts; carry a partial day; mirror the NWS
   guard; unwrap the rings) rather than a patch.
9. **§3.12** (`wind_dir_deg`) — make it `Option<u16>`; it is a model change with three call sites, all
   of which already handle the `None` case on the day-part path.
10. Everything in §4 that shares a file with the above, then §4.6 (test strength) and §4.7 (docs) as
    their own sweeps — the documentation drift alone is a single-commit fix per document.

## 10. Reproducing this review

```bash
# the gates
cargo fmt --check
cargo clippy --workspace --all-targets --locked -- -D warnings
CIRROCAST_FORBID_NETWORK=1 cargo test --workspace --locked
reuse lint

# §3.1 — a cache-write failure must not discard a successful fetch
cargo build && D=$(mktemp -d) && mkdir -p $D/c && chmod 0555 $D/c
XDG_CACHE_HOME=$D/c target/debug/cirrocast -p metar --station ZBAA; echo "exit $?"
chmod 0755 $D/c

# §3.2 / §3.4 — the validator accepts what the runtime rejects
D=$(mktemp -d); export XDG_CONFIG_HOME=$D/config XDG_CACHE_HOME=$D/cache XDG_DATA_HOME=$D/data
target/debug/cirrocast config set alerts.sources auto,fpas            # 0
target/debug/cirrocast config validate                               # ok
target/debug/cirrocast Beijing; echo "exit $?"                       # 2
target/debug/cirrocast config set alerts.sources visualcrossing        # 0
target/debug/cirrocast config validate; echo "validate exit $?"         # 0
target/debug/cirrocast Beijing; echo "exit $?"                          # 2
target/debug/cirrocast config set alerts.sources auto                   # put it back

# §3.5 — an empty argument bypasses location.default and queries the public IP
target/debug/cirrocast config set location.default Beijing
target/debug/cirrocast "" --offline=all -v 2>&1 | grep -E '^(location|error)'

# §3.3 — the HKO cache key omits lang: the second run is a cache hit, no request
target/debug/cirrocast --lat 22.30 --lon 114.17 --alerts -vv --lang zh-CN 2>&1 | grep hko-warnsum
target/debug/cirrocast --lat 22.30 --lon 114.17 --alerts -vv --lang en-US 2>&1 | grep hko-warnsum

# §4.5 #54 — the same width is legal as a flag and illegal in config.toml
target/debug/cirrocast --width 39 Beijing >/dev/null; echo "flag exit $?"
target/debug/cirrocast config set render.width 39; echo "config exit $?"

# §4.4 #43 — a -0 coordinate reaches the JSON document as -0.0
target/debug/cirrocast --lat=-0 --lon=0 -f json --days 0 | grep '"lat"'
```
