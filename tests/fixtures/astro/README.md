<!--
SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Astronomy fixtures

What is here, where it came from, and what it is allowed to prove.

## `open-meteo-*.json` — recorded sunrise/sunset archives

Verbatim responses of the Open-Meteo **archive** API
(`https://archive-api.open-meteo.com/v1/archive?latitude=…&longitude=…&start_date=…&end_date=…&daily=sunrise,sunset,daylight_duration&timezone=auto`),
retrieved 2026-10-04 for the five days the step-17 tests pin:

| file | point | day | sunrise | sunset | daylight |
|---|---|---|---|---|---|
| `open-meteo-ny-2026-03-08.json` | 40.7128, −74.0060 | 2026-03-08 (DST spring-forward) | 07:19 | 18:55 | 41 772.11 s |
| `open-meteo-ny-2025-03-09.json` | 40.7128, −74.0060 | 2025-03-09 | 07:17 | 18:56 | 41 976.06 s |
| `open-meteo-longyearbyen-2026-06-21.json` | 78.2232, 15.6469 | 2026-06-21 | 00:00 (clamped) | 00:00 next day (clamped) | 86 400.00 s |
| `open-meteo-longyearbyen-2025-12-21.json` | 78.2232, 15.6469 | 2025-12-21 | 00:00 (clamped) | 00:00 (clamped) | 0.00 s |
| `open-meteo-beijing-2024-02-29.json` | 39.9042, 116.4074 | 2024-02-29 (leap day) | 06:49 | 18:05 | 40 569.22 s |

The polar rows are the reason the module must return `None` plus a `Polar` flag: the provider
*clamps* to midnight and reports 86 400 s / 0 s, and copying that would print `00:00` as a sunrise
that never happened. The tests compare the locally computed values with these recordings within
two minutes; the archive rounds its times to the minute and snaps to a grid cell, so a tighter
bound would be measuring the recording, not the algorithm.

## Moon instants — measured against JPL Horizons DE441

The eight instants of 2026 were measured against JPL Horizons
(`https://ssd.jpl.nasa.gov/api/horizons.api`, `COMMAND='301'`, `CENTER='500@399'`,
`QUANTITIES='10,43'`) on 2026-09-30, as recorded in
[`docs/plans/17-moon-phase-and-astro.md`](../../../docs/plans/17-moon-phase-and-astro.md): the
illuminated percentage and the synodic elongation `E = 180° − phase_angle`, with waxing/waning
taken from the direction of the illumination change. The tests live in `tests/astro.rs`; the
tolerance is ±0.5 pp on the illuminated fraction (measured deviation ≤ 0.14 pp) and the phase name
must land in the recorded 45° window.

| UTC instant | Horizons illuminated % | `E` (°) | expected phase |
|---|---|---|---|
| 2026-01-18T12:00 | 0.20848 | 356.23 | New |
| 2026-01-22T16:00 | 15.00160 | 45.45 | WaxingCrescent |
| 2026-01-26T00:00 | 47.87048 | 87.40 | FirstQuarter |
| 2026-01-31T00:00 | 94.97307 | 154.34 | WaxingGibbous |
| 2026-02-02T00:00 | 99.95555 | 180.99 | Full |
| 2026-02-04T04:00 | 93.95135 | 208.53 | WaningGibbous |
| 2026-02-09T00:00 | 55.15727 | 264.20 | LastQuarter |
| 2026-02-12T12:00 | 23.44829 | 302.33 | WaningCrescent |

## Book instants — Meeus, *Astronomical Algorithms* 2nd ed., chapter 49

| instant (TD) | JDE | source |
|---|---|---|
| 1977-02-18 03:37:42 | 2 443 192.65118 | Example 49.a (New Moon) |
| 2044-01-21 23:48:17 | 2 467 636.49186 | Example 49.b (Last Quarter) |

The module's solver works in TT, so the tests compare its root in TT with these values within
two minutes. The measured offsets are 32 s and 23 s.

## Moonrise/moonset references — PyEphem 4.2.1

The three cases below were computed with PyEphem (its standard observer, pressure 1010 mbar) on
2026-10-04 as an independent reference for the rise/set search, which is the least accurate part of
the module (the Moon's declination changes fast). The tests allow ±10 minutes; the measured
differences are 11 s, 30 s and 76 s.

| point | day | moonrise | moonset |
|---|---|---|---|
| Beijing 39.9042, 116.4074 | 2026-10-01 | 20:28:27 | 11:24:16 |
| New York 40.7128, −74.0060 | 2026-03-08 | — (next one is March 9 00:41) | 09:22:58 |
| Longyearbyen 78.2232, 15.6469 | 2026-06-21 | 12:28:05 | 01:18:07 |

The New York row is the interesting one: the Moon rises at 00:41 on the *next* local day, so the
8th itself has no moonrise — a missing event, not a midnight.
