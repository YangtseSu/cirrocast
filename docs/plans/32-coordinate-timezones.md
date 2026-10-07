<!--
SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Step 32 — a zone for coordinate locations

Status: ✅ done
Depends on: `04-geocoding-and-location-syntax.md` (the `@lat,lon` spec and the resolver), `05-http-cache-and-ip-location.md` (the IP answer path the naming also serves), `18-offline-city-database.md` (the bundled table, whose rows carry an IANA zone), `25-location-sources-2.md` (coordinate naming, `[geo] reverse`), `10`/`23`/`24` (the zone-less backends whose refusal this removes)
Touches: `src/geo/reverse.rs`, `src/cli.rs`, `src/config/mod.rs`, `src/provider/{qweather,openweathermap,smhi,worldweatheronline}.rs` (message only, if the wording needs the resolved name), `tests/{geo_reverse,cli,exit_codes}.rs`, `docs/{providers,location,configuration}.md`, `README.md`, `docs/plans/README.md`, `CHANGELOG.md`

## Goal

`cirrocast -p qweather @35.2,113.9` works. Today it fails with *"provider `qweather` needs the
location's time zone and its response carries only UTC instants"*, even though the run has just
named that point `Xinxiang, China` — from a bundled table row that carries `Asia/Shanghai`. A
coordinate's zone is a fact about the point, and the program already holds a good answer for it in
two places: the nearest bundled city (when it is close enough), and the user (`--tz`).

The refusal stays for a point whose zone nothing can supply — the providers that cannot repair a
zone must keep refusing rather than aggregate a day in UTC — but it becomes the exception instead
of the rule for `@lat,lon`.

## Deliverables

- ✅ **Zone adoption from a bundled naming hit.** `src/geo/reverse.rs` gains
      `ZONE_RADIUS_KM = 10.0` and `adoptable_zone(&Nearby) -> Option<Tz>`: the zone of a hit whose
      source is the bundled table and whose distance is within the tighter radius. The CLI's
      `name_coordinate` adopts it for a location whose zone is still the UTC placeholder (never over
      a zone the location already has), keeps the coordinate's own `lat`/`lon`, source and name
      handling untouched, and prints one `-v` line naming the city, the distance and the zone it
      lent. Unit tests in `reverse.rs` (within, beyond, Nominatim hit) and a CLI test that the named
      coordinate line shows `Asia/Shanghai` instead of `<timezone resolved at fetch time>`.
- ✅ **`--tz <IANA>` / `[location] tz` / `CIRROCAST_TZ`.** The explicit answer for every point the
      tables cannot zone, and for a user who knows better than the nearest city. Parsed through
      `chrono_tz` (an unknown name is a usage error naming an example), resolved with the usual
      precedence (flag → environment → configuration), and applied to every location of the run at
      the end of `resolve_location`, so a name, a coordinate, an IP answer and `location search` all
      honour it; a `-v` line says when it replaced a different zone. `[location] tz` is validated
      with the same parser, documented in `docs/configuration.md`, and listed in the `--help`
      epilogue.
- ✅ **Tests for both halves, end to end.** `@35.2,113.9` (9 km from Xinxiang) is no longer
      provisional, so `-p qweather` reaches the credential check instead of the zone refusal; a point
      named but *beyond* the zone radius keeps the placeholder and still refuses; `--tz` overrides
      both a provisional and a resolved zone; an unknown zone is exit 2; and the existing
      `-p qweather @39.9,116.4` exit-code test keeps its meaning (Beijing's city centre is ~1 km
      away, so the zone is adopted and the run still ends at the missing key, exit 6).
- ✅ **Docs.** `docs/providers.md`: the qweather refusal paragraph becomes conditional and records
      the `localTime` finding below; `docs/location.md`: the coordinate section gains the adoption
      rule, the radius and `--tz`; `docs/configuration.md` and the README flag table gain
      `[location] tz`; the CHANGELOG gains the entry; the plan index gains this step.

## Design notes

* **The API already takes coordinates — the zone was the blocker.** `/weather/v1/{current,hourly}`
  has taken `{lat}/{lon}` since step 10; nothing about the request needed to change. What the
  payload never carries is an IANA zone.
* **`localTime=true` is not the fix, and it is recorded here so it is not re-proposed.** The v1
  hourly endpoint accepts a `localTime` parameter (default `false` = UTC): probed live on 2026-10-07
  against the account, Berlin in DST, `hours=3`:
  `2026-10-07T14:00Z` by default and `2026-10-07T16:00+02:00` with `localTime=true` — per-instant
  **offsets**, still no zone name anywhere in the payload. Offsets would let the *provider* bucket
  the hours into days (DST-exact), but the rest of the report needs a name: the JSON
  `location.timezone`, the moon and sun local times, `%Z`, the `Today`/`今天` labels. A report whose
  JSON says `UTC` while its day labels are `+08:00` is worse than a refusal, and the `current` block
  carries no timestamp either way (the recorded fixture and the published field list agree).
* **Why adoption is limited to the bundled table and to 10 km.** A name is a label; a zone is a fact
  about the point. The naming radius (25 km) is deliberately loose because a nearby name is better
  than none, but a zone border can run between a point and a city 25 km away (Arizona/California,
  Spain/Portugal), so adoption uses half that radius, applies only when the hit came from the
  bundled table (whose rows carry GeoNames' per-city zone; a Nominatim object's zone tag is rare and
  describes the object, not the point), and never overwrites a zone the location already has.
* **`--tz` applies to every location of the run.** It is the run's answer to "which zone are these
  times in"; a multi-location run that needs per-location zones is asking the resolver, not the
  flag. The `-v` note says when it replaced something.
* **No new dependency.** `chrono-tz` already parses IANA names; the table's zones are already
  `Tz` values; `deny.toml` is untouched.
* **The refusal text is a follow-up, not this step.** When a zone genuinely cannot be supplied, the
  four providers' messages could name the place the coordinate resolved to (`cirrocast -p qweather
  Xinxiang`) instead of a hard-coded example; that touches four modules and their message tests and
  is deliberately left for its own change.

## Out of scope

* `localTime=true` (rejected above), and any use of an offset as a zone.
* Adopting a Nominatim result's zone tag.
* A zone finder from coordinates alone (`tzf`-style data): the bundled table plus `--tz` cover the
  real cases, and a new data set needs its own licence review.
* Changing the four providers' refusal behaviour or wording (a follow-up), and the IP path's zone
  handling beyond what the shared naming merge already gives it.

## Verification

```bash
cargo run -q -- -p qweather @35.2,113.9 -v     # zone adopted from Xinxiang; no zone refusal
cargo run -q -- location search @35.2,113.9 -v # the note, and Asia/Shanghai in the location line
cargo run -q -- location search @35.2,113.6 -v # ~18 km: named, but the zone stays provisional
cargo run -q -- -p qweather @35.2,113.9 --tz Asia/Tokyo -f json | jq '.location.timezone'
cargo run -q -- location search @35.2,113.9 --tz Mars/Olympus   # usage error, exit 2
cargo run -q -- -p qweather @0,0               # no zone anywhere: the documented refusal, exit 2
cargo test --workspace --locked && cargo clippy --workspace --all-targets --locked -- -D warnings
reuse lint
```

## Exit criteria

- ✅ A coordinate within the zone radius of a bundled city fetches from every zone-less backend
      (qweather, openweathermap, smhi, worldweatheronline) without the zone error, verified live for
      at least qweather and pasted into the progress log.
- ✅ A coordinate with no usable zone still refuses with the documented usage error, and `--tz` makes
      it work — both proven by tests.
- ✅ `--tz`, `[location] tz` and `CIRROCAST_TZ` agree on the spelling and the precedence, and an
      unknown zone is a usage error before any request.
- ✅ `docs/providers.md` records the `localTime` probe and the conditional refusal; the README,
      `docs/location.md` and `docs/configuration.md` document the adoption rule and the flag;
      `reuse lint` is clean.

## Risks

* **A zone border inside 10 km.** Rare, and the note plus `--tz` make it visible and fixable; the
  alternative (refusing everything) is what this step removes.
* **A wrong `--tz`** silently mislabels every time in the run. The `-v` line says when the flag
  replaced a resolved zone, so a mistake is visible to anyone looking.
* **The bundled table's coverage.** A point far from any `cities15000` row still has no zone; that is
  what `--tz` is for, and the refusal stays honest for the rest.

## Progress log

- 2026-10-07 — step created and started, from the maintainer's observation that the weather API takes
  coordinates directly and offers `localTime`. The `localTime` half was probed live against the
  account the same day (Berlin, DST: `2026-10-07T14:00Z` default versus `2026-10-07T16:00+02:00` with
  `localTime=true`) and rejected as the fix, with the reasoning recorded above; the zone adoption and
  `--tz` halves are the accepted design.
- 2026-10-07 — all four deliverables landed in one series: `reverse::ZONE_RADIUS_KM` (10 km) and
  `reverse::adoptable_zone` with their unit test; the adoption in `name_coordinate` (after the naming
  note, so the `-v` output reads as a consequence of it); `geo::parse_zone` shared by the flag,
  `CIRROCAST_TZ` and `[location] tz`; the global `--tz` flag and `apply_run_zone` at the end of
  `resolve_location`, so names, coordinates, IP answers and `location search` all honour it. The four
  zone-less backends' refusal messages now name `--tz` (their tests pin it). `tests/cli.rs` gained
  the adoption/radius/flag/environment/configuration cases, `tests/exit_codes.rs` the no-zone refusal
  (with a stored key, so the chain reaches the provider), and the config fixture and probes the new
  key. The `--help` budget went 225 → 230 (measured 227) with the raise recorded in `tests/cli.rs`,
  `scripts/bench/compare.py`, `docs/performance.md` and step 21's log, and the two generated
  documents were regenerated and reviewed.
- 2026-10-07 — verified live against the account: `cirrocast -p qweather @35.2,113.9` prints a real
  report (exit 0, `Xinxiang, China`, `Asia/Shanghai` adopted from the 9.0 km bundled hit);
  `location search @35.2,113.6` (18 km) keeps `<timezone resolved at fetch time>`;
  `--tz Asia/Tokyo` and `CIRROCAST_TZ=Asia/Tokyo` override with the `-v` note; `--tz Mars/Olympus`
  is a usage error (exit 2) and `[location] tz = "Mars/Olympus"` a configuration error (exit 4);
  `-p qweather @0,0` still refuses with exit 2 and a message naming `--tz`. `cargo fmt --check`,
  `clippy --workspace --all-targets -D warnings`, `cargo test --workspace` and `reuse lint` are
  clean.
