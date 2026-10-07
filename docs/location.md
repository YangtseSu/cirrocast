<!--
SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Location arguments

A location reaches `cirrocast` as one of the spellings below, and every spelling has exactly one
code path in `src/geo/`. Resolution is deterministic: the same argument resolves to the same place
on a re-run, whether the bundled table or a network geocoder answered, because the candidate order
is a total order over the candidates' own fields (`src/geo/rank.rs`, step 04). The chosen place is
echoed by the run, so a script never has to guess which "Springfield" it got.

The argument is the same on the command line and everywhere else it may come from —
`CIRROCAST_LOCATION`, `[location] default`, `status --location` — so a spec configured once behaves
exactly as it does when typed. One difference: the environment variable and the configuration key
each name **one** location, so a multi-location run (several `LOCATION` arguments) exists only on
the command line.

> The console blocks below were captured in the English locale (`--lang en-US`). Names,
coordinates and zones are locale-independent; the surrounding notes, credits and weather labels are
translated — see [i18n.md](i18n.md).

## The syntax forms

| Argument | What it means |
|---|---|
| `Beijing` | fuzzy search: the bundled city table first, then the `[geo] search` chain on a miss; the hits are ranked (see below) |
| `:Beijing` | only a candidate whose name matches the query exactly (folded); the same narrowing as `--exact` on `location search` |
| `~Tsinghua` | an OpenStreetMap/Nominatim `/search` query; never the Open-Meteo geocoder |
| `@39.9042,116.4074` | explicit coordinates; no geocoding request at all |
| `@home` | a name from the `[locations]` alias table, expanded before resolution |
| *(empty)* | `[location] default` when set, else the public-IP lookup |

Fuzzy and `:exact` names must be at least two characters (`MIN_QUERY_CHARS`); a shorter one is a
usage error (exit 2). Coordinates are accepted only when the comma-separated pair is inside the
world (`-90..=90`, `-180..=180`); anything else after `@` is treated as an alias name.

Three flags describe a single place instead of taking a positional argument, and they are weather
query flags, not `location search` flags: `--lat DEG`+`--lon DEG` (folded into the `@lat,lon`
spelling the resolver already understands), `--ip`, and `--station ICAO`.

### Fuzzy `Beijing`

```console
$ cirrocast location search Beijing
Beijing, CN (39.91, 116.40) Asia/Shanghai
note: 3 candidates for `Beijing`; using Beijing, CN (population 18960744) — pass `:Beijing` to require an exact name match, `--pick` to choose one, or `--yes` to keep the winner
Location data by GeoNames (CC BY 4.0) — https://www.geonames.org/
```

The winning line is stdout; the ambiguity note and the data credit are stderr, so stdout stays
pipeable. `-q` silences the note, not the credit.

### Exact `:Beijing`

```console
$ cirrocast location search :Beijing
Beijing, CN (39.91, 116.40) Asia/Shanghai
Location data by GeoNames (CC BY 4.0) — https://www.geonames.org/
```

The `:` prefix asks for candidates whose folded name or ASCII name *equals* the query
(`geo::rank::same_name`), so no ambiguity note is printed. On `location search` the same filter is
spelled `--exact`.

### OpenStreetMap `~Tsinghua`

```console
$ cirrocast location search '~Tsinghua University'
Tsinghua University, Beijing, China (40.00, 116.32) <timezone resolved at fetch time>
note: 2 candidates for `Tsinghua University`; using Tsinghua University, Beijing, China (population 47000) — `--pick` to choose one, or `--yes` to keep the winner
Location data © OpenStreetMap contributors (ODbL)
```

`~` never touches Open-Meteo; it goes straight to Nominatim `/search`, whose one-request-per-second
policy and ODbL credit are described in [providers.md](providers.md). A `~` result carries the
`address.state` of the hit as its admin-1 division when the response has one (here `Beijing`), a
provisional zone, and a population from the hit's `extratags` when present; its credit is
OpenStreetMap's, not GeoNames'. The public instance was unreachable directly from the verification
network (connect timeouts) when this was written, so the run above replays the recorded response
under `tests/fixtures/geo/` through `network.nominatim_url` — the same response the
[providers.md](providers.md) section was recorded from.

### Coordinates `@39.9042,116.4074`

```console
$ cirrocast location search @39.9042,116.4074
Beijing, China (39.90, 116.41) Asia/Shanghai
Location data by GeoNames (CC BY 4.0) — https://www.geonames.org/
```

The coordinates are the request key and are never sent to a geocoder. The point is *named* for
display only (see below) — here the bundled table named it `Beijing, China` 0.9 km away — and a
close bundled city also lends its IANA zone, which is why the line shows `Asia/Shanghai` rather than
the placeholder. The parenthesised coordinates are omitted when the name *is* the pair (a bare
`@lat,lon` that nothing could name), so a coordinate location prints its name only once.

### Alias `@home`

```console
$ cirrocast location search @home        # [locations] home = "Beijing", office = "Shanghai"
Beijing, CN (39.91, 116.40) Asia/Shanghai
note: 3 candidates for `Beijing`; using Beijing, CN (population 18960744) — pass `:Beijing` to require an exact name match, `--pick` to choose one, or `--yes` to keep the winner
Location data by GeoNames (CC BY 4.0) — https://www.geonames.org/
```

An alias value is any location argument, including another alias (`home = "@office"`), expanded
until a non-alias spec is left. A cycle (`a → b → a`) or a chain deeper than eight links is a
configuration error (exit 4) reported with the chain; an unknown `@name` is a usage error (exit 2)
that suggests the closest configured names by edit distance. The keys and their defaults are in
[configuration.md](configuration.md#locations--aliases).

```console
$ cirrocast location search @91,0
error: unknown location alias `@91,0` (`91,0` is not inside the world: latitude -90..=90, longitude -180..=180); known aliases: home, office (accepted forms: Beijing | :Beijing | ~Tsinghua | @39.9042,116.4074 | @name (an alias from [locations]))
```

### Empty argument

With no argument, `location search` uses `[location] default`; when that is empty too, the run
locates from the public IP address and says so:

```console
$ cirrocast location search
ip: located from the public IP via ipwho.is
Zhengzhou, Henan Sheng, China (34.76, 113.65) Asia/Shanghai
```

The IP lookup is disclosed on stderr and *only* happens with `--ip` or when no location is given
anywhere (flag, environment, configuration). A configured default is itself a spec, resolved through
the full chain:

```console
$ cirrocast location search          # [location] default = "Chamonix"
Chamonix, Rhône-Alpes, France (45.92, 6.87) Europe/Paris
Location data based on GeoNames (CC-BY-4.0) via Open-Meteo — https://open-meteo.com/
```

### Replacing the argument with a flag

`--lat`/`--lon` and `--ip` are alternatives to a positional argument; a command-line location
combined with either is refused, and `--station` conflicts with both. `--station ICAO` is not a
location argument at all: it names a METAR station that the `metar` provider resolves and heads the
provider chain with; the location the run renders is the station's own.

```console
$ cirrocast --station ZBAA -f one-line
Data: aviationweather.gov (NOAA/NWS, public domain)
Beijing Intl: *o* Clear sky +11°C (n/a), > 3.6km/h ENE, 76%, 0.0mm, 1021hPa, 10km

$ cirrocast --lat 39.9042 --lon 116.4074 -f one-line
Location data by GeoNames (CC BY 4.0) — https://www.geonames.org/
Data: Open-Meteo.com (CC BY 4.0) — https://open-meteo.com/
Beijing: *o* Clear sky +20°C (+16°C), , 11km/h SW, 30%, 0.0mm, 1021hPa, 18km
```

## The resolution order

A **plain name** is resolved by `geo::name_location` in this order:

1. **The bundled city table**, when the build has the `offline-geo` feature and
   `[geo] strategy` is not `network`. It answers from `[geo] data` (the user-installed table when
   present, else the embedded snapshot) and no socket is opened.
2. **The network geocoder chain**, when `[geo] strategy` is not `bundled` and the bundled table had
   no hit. `[geo] search` selects it: `auto` — the default — is `open-meteo`, then `geonames` when
   an account name is configured, then `nominatim` as the last resort. The per-source hits are
   merged and de-duplicated, then ranked.

`[geo] strategy` (`CIRROCAST_...` does not exist for this key; set it in `config.toml`) picks
`auto` (table first, network on a miss), `bundled` (table only) or `network` (geocoder only).
`--offline=geo` — and bare `--offline` (= `all`) — removes the network half entirely: names resolve
from the bundled table only, `~` is refused because it needs OpenStreetMap, and a name the table
does not know is reported as a miss, not a network error:

```console
$ cirrocast location search --offline=geo ~Tsinghua
error: network error: offline: `~Tsinghua` searches ask OpenStreetMap over the network; use a plain name (the bundled table) or `@lat,lon` instead

$ cirrocast location search --offline=geo Zzzzzville
error: location not found: no location found for `Zzzzzville` (no offline match)
```

The two sources rank with the *same* function, so a name resolves identically offline and online
where both know it. `-v` reports which source answered:

```console
$ cirrocast -v location search Chamonix
location: Chamonix not in the bundled city database; asking open-meteo, geonames, nominatim
location: open-meteo: 1 candidates
location: geonames: skipped (no account name; `cirrocast key set geonames` or CIRROCAST_GEONAMES_USER adds it)
location: nominatim: not asked (an earlier source answered)
Chamonix, Rhône-Alpes, France (45.92, 6.87) Europe/Paris
Location data based on GeoNames (CC-BY-4.0) via Open-Meteo — https://open-meteo.com/
location: candidate 1/1: Chamonix, Rhône-Alpes, France (45.92, 6.87) Europe/Paris (population 10614)
```

### The multi-source merge

Under `search = "auto"` more than one source may answer, and the same place comes back from each
with slightly different coordinates; showing it three times would make the picker useless. The
per-source lists are collapsed (`src/geo/merge.rs`, step 25) by these rules:

* two candidates are the same place when their folded names and country codes are equal and they lie
  within 5 km of each other — the **earlier source's record wins whole**, population and zone
  included, so a merge never mixes fields from two sources;
* a candidate whose country code is present but not two ASCII letters (GeoNames reports `-99` for
  the shapes without one) is dropped, because automatic provider selection reads that code;
* the surviving candidates keep source order, and the query-aware ranking runs afterwards, so a
  merge can only change which candidates are on the list, never how a query resolves.

`geonames` is **BYOK**: it is skipped under `auto` when no account name is set (the `-v` line names
the two commands that add one), and an explicitly selected `search = "geonames"` without an account
name is a missing-key error (exit 6). The endpoints, auth, quotas and licences of every geocoding
source are in [providers.md](providers.md#location-and-ip-services); this document does not repeat
them.

### The IP services

When no place is named at all, three donated services answer the same question in order:
`ipwho.is`, then `ipapi.co`, then `IP.SB`. `CIRROCAST_IP_SERVICE` pins one (`auto`, `ipwhois`,
`ipapi`, `ipsb`), the answer is cached for 24 hours (`cache.ip_ttl_secs`), and the credit each
service requires — or does not — is in [providers.md](providers.md#location-and-ip-services). An
answer that carries no city is named exactly like a typed coordinate (below); when nothing can name
it, the coordinates are the name and the header is never blank.

## The ranking

The order is implemented once in `src/geo/rank.rs` and used by every source, so the bundled table
and the network geocoder cannot order the same query differently. Candidates are compared by:

1. **match tier**, best first — `Exact` (the folded display name or ASCII name equals the folded
   query) beats `Prefix` (either starts with it) beats everything else;
2. **larger population** (a candidate without one counts as `0`);
3. **display name**, then **ASCII name**, ascending — the total-order tiebreak that keeps two equal
   candidates from falling back to the order the service happened to return them in.

`location search` cannot therefore resolve differently between two runs even if a service reorders
its hits, which is the equivalence the step-18 tests pin. `--limit N` (1..=100, default 10) bounds
how many candidates are ranked: it is passed to each source as its own `limit`/`count` and the
sorted list is truncated to N afterwards, so `--all` prints at most N rows. The winner is
unaffected as long as the cap is at least one, because the truncation keeps the head of the order:

```console
$ cirrocast location search --all --limit 3 Springfield
 1. Springfield, US (37.22, -93.30) America/Chicago (population 170188)
 2. Springfield, US (42.10, -72.59) America/New_York (population 154341)
 3. Springfield, US (39.80, -89.64) America/Chicago (population 114394)
Location data by GeoNames (CC BY 4.0) — https://www.geonames.org/
```

For the bundled table the index also splits keys into an exact-key tier and a prefix-key tier
(`src/geo/table.rs`); the index keys are built from *every* spelling of a row (display name, ASCII
name, and GeoNames alternate names), and the shared ranking is applied on top. The work below uses
the offline table.

### Worked example: `Springfield`

```console
$ cirrocast location search --all Springfield
 1. Springfield, US (37.22, -93.30) America/Chicago (population 170188)
 2. Springfield, US (42.10, -72.59) America/New_York (population 154341)
 3. Springfield, US (39.80, -89.64) America/Chicago (population 114394)
 4. Springfield, US (44.05, -123.02) America/Los_Angeles (population 60870)
 5. Springfield, US (39.92, -83.81) America/New_York (population 59680)
 6. Springfield, US (38.79, -77.19) America/New_York (population 30484)
 7. Springfield, US (39.93, -75.32) America/New_York (population 23363)
 8. Springfield, US (36.51, -86.89) America/Chicago (population 16808)
 9. Springfield Gardens, US (40.66, -73.76) America/New_York (population 30515)
10. Springfield Lakes, AU (-27.67, 152.92) Australia/Brisbane (population 15081)
Location data by GeoNames (CC BY 4.0) — https://www.geonames.org/
```

Rows 1–8 fold to exactly `springfield`, so they are the `Exact` tier and are ordered by population
descending. Rows 9–10 only start with the query and are the `Prefix` tier: row 9's population
(30 515) is larger than row 8's (16 808) and its coordinates are closer to several of the others,
but an exact name still outranks a prefix, which is exactly the rule above. The `:Springfield` /
`--exact` spelling asks only for the exact-key matches and the winner is the same:

```console
$ cirrocast location search Springfield
Springfield, US (37.22, -93.30) America/Chicago
note: 10 candidates for `Springfield`; using Springfield, US (population 170188) — pass `:Springfield` to require an exact name match, `--pick` to choose one, or `--yes` to keep the winner
Location data by GeoNames (CC BY 4.0) — https://www.geonames.org/
```

## The picker

When a fuzzy name resolves to several places, a terminal run can ask which one to use **before**
anything is fetched. The picker is not a TUI: it prints a numbered list and reads one line, so it
works over ssh, a serial console or a here-document.

| The prompt happens when… | `--pick` forces it; `--yes` never asks; `[location] pick = "auto"` asks only when **both stdin and stderr are terminals**; `[location] pick = "never"` always takes the ranked winner |
| Env override | `CIRROCAST_LOCATION_PICK` (`auto`, `never`) — a bad value is a configuration error (exit 4) |
| Where | Only the weather query. `location search` never prompts: it has no `--pick`/`--yes` and prints the ranked table with `--all` instead. The `status` probe never prompts either, whatever the policy says |

An empty line (or `1`) selects the winner, which is marked `*`; a number in range selects that row;
`q` (or `Q`, or EOF) gives up with exit 5; three invalid answers in a row are a usage error naming
the accepted input (exit 2). A multi-location run asks every question in one serial pre-pass in
argument order, before any fetch, so two questions never share the one input stream.

```console
$ printf 'q\n' | cirrocast Beijing --pick
[1] * Beijing, CN (39.91, 116.40) Asia/Shanghai (pop. 18960744)
[2]   Basingstoke, GB (51.26, -1.09) Europe/London (pop. 107642)
[3]   Beckingen, DE (49.40, 6.70) Europe/Berlin (pop. 15983)
choose a location [1-3, Enter=1, q=quit]: error: location not found: no location selected for Beijing
```

The prompt list is the ranking with a `pop. <n>` suffix and the winner pre-marked; it goes to
stderr, as does the prompt itself. A chosen candidate is echoed as a coordinate spec so the next run
can skip the ranking, and that echo is printed even under `-q` because it is the reproducibility
affordance for the choice:

```console
$ printf '1\n' | cirrocast Beijing --pick -f one-line -q
[1] * Beijing, CN (39.91, 116.40) Asia/Shanghai (pop. 18960744)
[2]   Basingstoke, GB (51.26, -1.09) Europe/London (pop. 107642)
[3]   Beckingen, DE (49.40, 6.70) Europe/Berlin (pop. 15983)
choose a location [1-3, Enter=1, q=quit]: selected: Beijing, CN — use @39.9075,116.39723 to skip the prompt
Location data by GeoNames (CC BY 4.0) — https://www.geonames.org/
Data: Open-Meteo.com (CC BY 4.0) — https://open-meteo.com/
Beijing: *o* Clear sky +20°C (+16°C), , 11km/h SW, 30%, 0.0mm, 1021hPa, 18km
```

A run whose stdin or stderr is not a terminal — a pipe, a redirect, a cron job — takes the ranked
winner silently, exactly as `--yes` does:

```console
$ cirrocast Beijing -f one-line -q < /dev/null
Location data by GeoNames (CC BY 4.0) — https://www.geonames.org/
Data: Open-Meteo.com (CC BY 4.0) — https://open-meteo.com/
Beijing: *o* Clear sky +20°C (+16°C), , 11km/h SW, 30%, 0.0mm, 1021hPa, 18km
```

## Naming a coordinate or an IP answer

`@39.9042,116.4074` is a request key, not a name, so the point is named for display only. A name is
a display attribute: the location keeps its own coordinates and its `Coordinates` (or `Ip`)
provenance, and only `name`/`admin1`/`country`/`country_code` and the `named_by` provenance are
filled in — the zone is the one exception, borrowed under the rule below. `[geo] reverse`
(`CIRROCAST_GEO_REVERSE`) picks the policy:

* `auto` (default) — the bundled city index is scanned for places within **25 km**
  (`reverse::RADIUS_KM`), nearest first, and the country layer supplies the country *name* the city
  table does not carry; when nothing is that close and the run may open a socket, Nominatim
  `/reverse` names the point;
* `offline` — the bundled tables only, never a socket (what `--offline` and `--offline=geo` force);
* `off` — do not name coordinates at all.

Naming never fails the query: a network or upstream failure of the online half becomes "no name"
plus a note a `-v` run prints. The credit follows `named_by`, so a coordinate named by the bundled
tables or Nominatim carries that source's credit and a bare one carries none.

### The zone a coordinate borrows

A coordinate has no zone of its own, and the backends whose responses carry no zone — QWeather,
OpenWeatherMap, SMHI, World Weather Online — refuse such a location rather than aggregate a day in
UTC. So a coordinate whose zone is still the placeholder adopts the zone of a bundled city when the
point is close enough to it:

* the hit must come from the **bundled table** (whose rows carry `GeoNames`' per-city IANA zone; a
  Nominatim object's zone tag describes the object, not the point) and be within **10 km**
  (`reverse::ZONE_RADIUS_KM`, half the naming radius — a zone border can run between a point and a
  city 25 km away);
* a zone the location already has is never overwritten, and `-v` says where a borrowed one came
  from;
* when nothing is close enough, `--tz <ZONE>` (or `CIRROCAST_TZ`, or `[location] tz`) states the zone
  outright: one IANA name for the run, applied to every location of it, with `-v` noting when it
  replaced a resolved zone.

`-v` reports the source, the distance and the zone:

```console
$ cirrocast -v location search @39.9042,116.4074
location: named by the bundled tables, 0.9 km away: Beijing, China
location: zone Asia/Shanghai adopted from Beijing (0.9 km away)
Beijing, China (39.90, 116.41) Asia/Shanghai
Location data by GeoNames (CC BY 4.0) — https://www.geonames.org/
location: candidate 1/3: Beijing, China (39.91, 116.40) Asia/Shanghai (population 18960744)
location: candidate 2/3: Daxing, China (39.74, 116.33) Asia/Shanghai (population 104904)
location: candidate 3/3: Tongzhou, China (39.90, 116.66) Asia/Shanghai (population 163326)
```

An IP answer whose service reported no city is named the same way, and borrows a zone the same way.
A zone nothing could supply stays `<timezone resolved at fetch time>` until a provider's own response
reports one (Open-Meteo and MET Norway do); the four zone-less backends refuse the run instead, and
`--tz` is the way out.

### `--all` and the candidate list shape

`--all` replaces the single winner line with the ranked candidate table. Each row is
`<rank>. <Name, admin1, country> (<lat>, <lon>) <zone>` plus ` (population <n>)` when the source
reports one; the rank is right-aligned in two columns. For a coordinate, `--all` lists the nearby
naming candidates, nearest first (bear in mind that the rank starts at 1 there, not at the
distance):

```console
$ cirrocast location search '@30.58,114.27' --all
 1. Wuhan, China (30.58, 114.27) Asia/Shanghai (population 13739000)
 2. Wuchang, China (30.54, 114.30) Asia/Shanghai (population 1102188)
 3. Pánlóngchéng Jīngjì Kāifāqū, China (30.69, 114.27) Asia/Shanghai (population 59207)
 4. Caidian, China (30.58, 114.03) Asia/Shanghai (population 71891)
Location data by GeoNames (CC BY 4.0) — https://www.geonames.org/
```

Everything that is commentary — the ambiguity note, the attribution, the IP disclosure and the `-v`
candidate list — goes to stderr; the candidate table or the winner line is the only thing on stdout.
The `json`, `one-line` and `plain` contracts that consume the resolved location are in
[ecosystem.md](ecosystem.md), and the formats in [formats.md](formats.md).

## Offline city data

The bundled snapshot is the default and the fallback; a newer GeoNames `cities15000` dump can be
installed per user without waiting for a release:

```console
$ cirrocast location update-data --check
error: https://download.geonames.org/export/dump/cities15000.zip differs from the bundled city table in cities.bin.gz, keys.bin.gz, SNAPSHOT; run without `--check` to install it
```

`location update-data` fetches through the same HTTP stack as everything else (proxy, timeout,
retries and the `CIRROCAST_FORBID_NETWORK` guard apply), builds the table with the tool's own
encoder, proves it decodes, and installs it atomically under `$XDG_DATA_HOME/cirrocast/geo/`. Name
resolution then prefers it, and the bundled table stays the fallback; `[geo] data` picks `auto`
(the user table when present and valid), `bundled` or `user`. A corrupt user table is diagnosed once
and the run falls back to the bundled one unless `data = "user"` was set. `--from <PATH|URL>` reads
a local `.txt`/`.zip` or a mirror, and `--offline` is refused on purpose. The full recipe, the
scheduled-update units and the refresh instructions for the *committed* snapshot are in
[README.md § Updating the city data](../README.md#updating-the-city-data); the `GeoNames` credit
prints for either table.

A build with `cargo build --no-default-features` drops the `offline-geo` feature and the embedded
table with it (`Cargo.toml`): `location search` then resolves every name through the network
geocoder only, `[geo] strategy` and `[geo] data` have nothing to pick between, and `location
update-data` fails with a configuration error saying the build has no table to install. The size
budget and the reduced-build CI leg are in [performance.md](performance.md).

## Exit codes

| code | when (location path) |
|---|---|
| 2 | a usage mistake: a malformed spec, a name shorter than two characters, an unknown `@alias`, three invalid picker answers |
| 3 | a network or upstream failure: a geocoding service unreachable, a `~` search under `--offline=geo` |
| 4 | a configuration problem: an alias cycle or over-deep chain, a bad `[geo]`/`[location]` value, a location that must be resolved through the configured path |
| 5 | no place: an unresolvable name or station, `q`/EOF at the prompt |
| 6 | a missing or invalid key: an explicitly selected `geonames` source with no account name |

The full ladder (including the weather path) is in `cirrocast --help`; the `status` probe's own
subset is in [ecosystem.md](ecosystem.md).

See also: [configuration.md](configuration.md) for every key and default,
[providers.md](providers.md#location-and-ip-services) for the geocoding and IP services,
[schema.md](schema.md) for the JSON document, and [formats.md](formats.md) for the output formats.
