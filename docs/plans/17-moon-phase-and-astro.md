<!--
SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Step 17 — moon phase and astronomy

Status: not-started
Depends on: 03, 08
Touches: `src/astro/{mod,moon,sun,julian}.rs`, `src/render/art.rs`, `src/render/{art_table,plain,
one_line,json}.rs`, `src/cli.rs`, `src/i18n.rs`, `locales/{en-US,zh-CN}/main.ftl`, `tests/astro.rs`,
`tests/fixtures/astro/`, `tests/fixtures/astro/README.md`, `REUSE.toml`, `docs/plans/README.md`,
`CHANGELOG.md`

## Goal

`cirrocast` computes the moon phase, illuminated fraction, moonrise/moonset, and — as a fallback for
providers that do not send them — local sunrise/sunset/day length entirely on the machine, with no
network call of any kind. The results appear in `one-line` as the wttr.in-compatible `%m` (moon art)
and `%M` (phase name) tokens, in a `--moon` view for `art-table`/`plain`, and as an `astro` object in
`json`. Polar day/night and DST-transition days produce defined, non-NaN output.

## Deliverables

- [ ] `src/astro/julian.rs`: Julian Day from a civil date/time (Meeus, *Astronomical Algorithms*,
      ch. 7) and `jd -> DateTime<Utc>` conversion; ΔT applied as a single documented function
      (Espenak–Meeus polynomial fits, 1900–2150) so all solar/lunar math runs in TT and all output
      is UTC.
- [ ] `src/astro/moon.rs`: geocentric ecliptic longitude and latitude of the Moon (Meeus ch. 47/48,
      truncated ELP-2000/82 series — the same term set prototyped below), `synodic_phase(jd)`
      returning the synodic elongation `E ∈ [0,360)` (`0` = new, `90` = first quarter, `180` = full),
      `illuminated_fraction(jd) = (1 − cos E)/2`, `MoonPhase::{New, WaxingCrescent, FirstQuarter,
      WaxingGibbous, Full, WaningGibbous, LastQuarter, WaningCrescent}` by 45°-wide windows of `E`
      (`[337.5,22.5)` wraps to New), `age_days(jd)` since the preceding new moon,
      `next_phases(jd, 4) -> [(MoonPhase, DateTime<Utc>)]` (Meeus ch. 49 phase-moment series), and
      `moonrise_moonset(date, lat, lon, tz) -> (Option<DateTime<FixedOffset>>,
      Option<DateTime<FixedOffset>>)` using the standard h₀ = +0.125° (lunar parallax, refraction,
      semi-diameter) with Meeus' interpolation over three hour angles.
- [ ] `src/astro/sun.rs`: apparent solar longitude and declination (Meeus ch. 25),
      `sunrise_sunset(date, lat, lon, tz)` with h₀ = −0.833°, and `daylight_secs(...)`; returns
      `None` for a rise/set that does not occur on that day and sets `polar: Some(PolarDay)` /
      `polar: Some(PolarNight)` instead of clamping to 00:00.
- [ ] `src/astro/mod.rs`: `Astro { moon: Moon { phase, illuminated_fraction, age_days, moonrise,
      moonset, next: Vec<(MoonPhase, DateTime<FixedOffset>)> }, sun: Option<Sun { sunrise, sunset,
      daylight_secs, polar: Option<Polar> }>, computed_at }`; a `sun {}` block is produced only when
      the report lacks provider sunrise/sunset (fallback), otherwise the provider values are used
      and `sun.source = "provider"` is recorded.
- [ ] **No network in this module**: `src/astro/**` must not reference `crate::http`, `crate::cache`
      or `ureq`; enforced by the exit-criteria grep and by the module taking no `Env` parameter.
- [ ] `src/render/art.rs`: an authored 8-entry moon art table keyed by `MoonPhase::art_key()`
      (`moon/new`, `moon/waxing-crescent`, …) built from `█ ▓ ▒ ░` and `◐ ◑`-class block glyphs, plus
      a 2-cell ASCII fallback used when `TERM=dumb`/`--format dumb`; re-authored here, never copied
      from wego or wttr.in.
- [ ] `one-line`: `%m` expands to the moon art glyph, `%M` to the localised phase name; both are
      empty-string safe and documented in the token table (`%A` = alerts, `%q` = AQI, `%m`/`%M` =
      moon, so no token is claimed twice across steps 15–17).
- [ ] `--moon` view for `art-table` (a 4-line block under the table) and `plain` (a `moon:`/`sun:`
      key block), `--format moon` for a standalone view; `json` gains
      `"astro": { "moon": {"phase","phase_key","illuminated_fraction","age_days","moonrise",
      "moonset","next":[{"phase","at"}]}, "sun": {"sunrise","sunset","daylight_secs","polar",
      "source"} }` (additive to `schema_version` 2, no bump).
- [ ] `src/i18n.rs` + catalogs: phase names (`moon-phase-{new,waxing-crescent,first-quarter,
      waxing-gibbous,full,waning-gibbous,last-quarter,waning-crescent}`), `astro-moonrise`,
      `astro-moonset`, `astro-sunrise`, `astro-sunset`, `astro-daylight`, `astro-polar-day`,
      `astro-polar-night`, `astro-no-rise`, `astro-illumination`, `astro-age-days`.
- [ ] Tests (`tests/astro.rs`, fixed instants, no clock access — `RenderContext::now` is injected):
      the eight 2026 phase instants below (name **and** illuminated fraction), the two Meeus book
      instants below, a DST-transition day, polar day and polar night at Longyearbyen, and a leap
      day; plus a `no_network` test that runs the whole module with a poisoned `Env`-style stub
      unavailable (the module is pure, so the test simply compiles it without the HTTP feature).

## Design notes

* **Algorithm and measured accuracy.** Prototyped on 2026-09-30 in a throwaway script implementing
  Meeus ch. 48 (27 largest longitude terms) + ch. 25 sun: against JPL Horizons DE441 (geocentric,
  quantity `10`) the illuminated fraction deviates by at most **0.19 pp** (typically 0.02–0.14 pp),
  and the book instant for the new moon of 1977-02-18 03:37:42 TD (Meeus example 49.a,
  JD 2443192.65118) falls at `E = 359.9834°`, i.e. ~1.8 minutes from the computed phase instant.
  Test tolerances are therefore **±0.5 pp** on the illuminated fraction and **±2 minutes** on phase
  instants, both tighter than the ±10″/±1 min figures usually quoted for the truncated series.
* **Fixture provenance** (recorded in `tests/fixtures/astro/README.md`): the moon rows come from
  JPL Horizons (`https://ssd.jpl.nasa.gov/api/horizons.api`, `COMMAND='301'`, `CENTER='500@399'`,
  `QUANTITIES='10,43'`, retrieved 2026-09-30); the sun rows come from the Open-Meteo archive API
  (`daily=sunrise,sunset,daylight_duration`, retrieved 2026-09-30). Both are recorded verbatim, and
  the expected values in the table below were derived from them, not from our implementation.
* **Moon fixture instants** (name derived from `E = 180° − phase_angle` with waxing/waning taken
  from the direction of the illumination change in the Horizons series):

  | UTC instant | Horizons illuminated % | `E` (°) | expected `MoonPhase` |
  |---|---|---|---|
  | 2026-01-18T12:00 | 0.20848 | 356.23 | New |
  | 2026-01-22T16:00 | 15.00160 | 45.45 | WaxingCrescent |
  | 2026-01-26T00:00 | 47.87048 | 87.40 | FirstQuarter |
  | 2026-01-31T00:00 | 94.97307 | 154.34 | WaxingGibbous |
  | 2026-02-02T00:00 | 99.95555 | 180.99 | Full |
  | 2026-02-04T04:00 | 93.95135 | 208.53 | WaningGibbous |
  | 2026-02-09T00:00 | 55.15727 | 264.20 | LastQuarter |
  | 2026-02-12T12:00 | 23.44829 | 302.33 | WaningCrescent |

* **Sun fixture instants**: New York `40.7128,-74.0060` on 2026-03-08 (DST transition, sunrise
  07:19, sunset 18:55, daylight 41 772.11 s) and on 2025-03-09 (07:17 / 18:56, 41 976.06 s);
  Longyearbyen `78.2232,15.6469` on 2026-06-21 and 2025-12-21, where the provider **clamps** to
  `00:00`/`00:00` and reports 86 400 s / 0 s — the case that proves the module must return `None`
  plus the `Polar` flag and the renderer must print `—`, never `00:00` or `NaN`; Beijing
  `39.9042,116.4074` on the leap day 2024-02-29 (06:49 / 18:05, 40 569.22 s).
* **Local day semantics**: rise/set are computed for the location-local calendar day containing
  `ctx.now` and converted through `ctx.tz`, so the DST spring-forward day is 23 h long; the module
  iterates hour angles in UT and never assumes a 24 h day. This is what the New York fixtures pin.
* **Dependency stance**: `chrono` and `chrono_tz` 0.10.4 (MIT OR Apache-2.0, MSRV 1.65) are already
  in the tree from steps 03/05 — GPL-3.0-or-later-compatible, no new licence surface. A dedicated
  astronomy crate (e.g. `sunrise`) was rejected: it covers only the solar half, its accuracy claim is
  unverifiable from the docs, and every added dependency enlarges the `cargo deny`/attribution
  surface for ~150 lines of testable arithmetic. `unsafe` stays forbidden.
* **Fallback only**: when the provider (e.g. `met-no`) sends no sunrise/sunset, the local computation
  fills the gap and `sun.source = "local"`; when it does (Open-Meteo `daily=sunrise,sunset`), the
  provider value wins and the local value is still computed in tests to keep the two paths honest.

## Out of scope

Twilight bands (civil/nautical/astronomical), planet or star ephemerides, eclipse prediction,
libration and the observer-position correction of the illuminated fraction (the geocentric value is
what the panel prints, labelled as such), and hourly moon altitude for a sky-glow chart. `%m`/`%M`
are the only new `one-line` tokens; the wttr.in `%a` astronomy slot is *not* implemented. Time-zone
resolution for locations without an IANA zone stays a step 04 concern.

## Verification

```bash
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && reuse lint

# no network primitives in the astronomy module
! grep -rn 'ureq\|crate::http\|crate::cache' src/astro/

cargo run -q -- --moon Beijing
#   phase name + illuminated % + age + moonrise/moonset + sun fallback block; exit 0 offline
cargo run -q -- -f one-line Beijing | tr ' ' '\n' | grep -c '%'   # 0: tokens expanded
cargo run -q -- -f one-line --template '%M (%m) %t' Beijing      # "Waxing Crescent (…) 18°C"
cargo run -q -- --offline --moon --lat 78.2232 --lon 15.6469
#   "polar day", moonrise/moonset "—" where not applicable, no NaN/0, exit 0
cargo run -q -- -f json --moon Beijing | jq '.astro.moon.phase, .astro.sun.polar'
cargo run -q -- --moon -p met-no --lat 59.91 --lon 10.75 -v
#   "sun: computed locally (provider sends none)" and identical solar values to the open-meteo run
```

## Exit criteria

- [ ] `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`, `reuse lint` clean.
- [ ] The eight moon fixtures and the two Meeus book instants pass within ±0.5 pp / ±2 minutes.
- [ ] `grep -rn 'ureq\|crate::http\|crate::cache' src/astro/` is empty (no network path exists).
- [ ] Longyearbyen polar day/night and the New York DST day render without `NaN`, `inf`, `0` or an
  impossible clock time; the polar labels are localised in both catalogs.
- [ ] `%m`/`%M` documented in the token table next to `%A`/`%q`, and `--moon` works from cache offline.

## Risks

* Truncated-series accuracy degrades far from the year 2000; mitigated by documenting the supported
  range (1900–2100) and asserting the two book instants (1977, 2044) at the edges of that range.
* Moonrise/moonset near the polar circle are the least accurate values in the module (the Moon's
  declination changes fast); the module returns `None` plus a `--verbose` note when the three-point
  interpolation brackets disagree by more than 10 minutes.
* Emoji-width assumptions differ between terminals; the art table is authored in single-width block
  characters and `dumb` mode uses the ASCII variant, so `--width` accounting stays exact.

## Progress log

- 2026-09-30 — step opened; Meeus prototype measured against JPL Horizons DE441 (max 0.19 pp on the
  illuminated fraction, 1.8 min on the 1977 book new moon) and the fixture instants above fixed.
