<!--
SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Step 04 — geocoding-and-location-syntax

Status: not-started
Depends on: `02-config-and-state.md` (`Config`/`Settings.location`, `KEY_TABLE`), `03-canonical-model-and-units.md` (`Location`, `LocationSource`) — and, for the network half only (`src/geo/open_meteo.rs`, `src/geo/nominatim.rs`, reachable `location search` results), `05-http-cache-and-ip-location.md` (`src/http.rs`, `src/cache.rs`, the `Clock` trait)
Touches: `src/geo/mod.rs`, `src/geo/open_meteo.rs`, `src/geo/nominatim.rs`, `src/config/mod.rs` (one `KEY_TABLE` row), `src/cli.rs`, `src/main.rs`, `tests/geo_spec.rs`, `tests/geo_open_meteo.rs`, `tests/geo_nominatim.rs`, `tests/fixtures/geo/`, `REUSE.toml`, `LICENSES/`, `docs/plans/README.md`

## Goal

The user can name a place in four ways — `Beijing`, `:Beijing`, `~Tsinghua`, `@39.9042,116.4074` —
and always gets one deterministic `Location` back, with the chosen hit echoed so nothing is guessed
twice. Bare names and `:names` resolve through the keyless Open-Meteo geocoding API, `~queries`
through OpenStreetMap Nominatim under its usage policy, and coordinates skip geocoding entirely.
Ambiguous matches are ranked (exact name, then population, then upstream order) and reported on
stderr; malformed input fails with `Error::Usage` and a message that lists the accepted forms.

## Deliverables

- [ ] `src/geo/mod.rs`: `pub enum LocationSpec { Default, Fuzzy(String), Exact(String), Osm(String), LatLon(f64, f64) }` with `impl FromStr` and `pub fn parse_arg(arg: Option<&str>) -> Result<LocationSpec>`: `None`/empty/whitespace-only → `Default`; `@lat,lon` → `LatLon`; leading `:` → `Exact` (at least one character after the colon); leading `~` → `Osm`; anything else → `Fuzzy` (trimmed, at least one character).
- [ ] Validation, each case with the accepted-forms hint appended: more than one comma, trailing junk or a missing part (`@39.9`, `@39.9,116.4,5`) ⇒ `Error::Usage("invalid coordinates `@39.9`: expected @<lat>,<lon>, e.g. @39.9042,116.4074")`; non-numeric parts, `NaN`, `inf` ⇒ same; out of range (`.abs()` on the parsed value) ⇒ `Error::Usage("latitude 91 is out of range -90..=90")` / `longitude` with `-180..=180`; empty `:`/`~` ⇒ `Error::Usage("`:`, `~` and `@` need a value")`; a one-character fuzzy query ⇒ `Error::Usage("search term `B` is too short; the geocoding API needs at least 2 characters")`, mirroring the upstream matching rules.
- [ ] The same message family is appended with the accepted forms so the error is self-explanatory: `accepted forms: Beijing | :Beijing | ~Tsinghua | @39.9042,116.4074` (single constant `USAGE_FORMS` in `src/geo/mod.rs`, reused by the usage error text and by `cirrocast --help`).
- [ ] `pub trait Geocoder { fn search(&self, q: &str, limit: u8) -> Result<Vec<Location>>; }` — synchronous, `&self`, no interior mutability, matching the provider contract's style; network access only through the step-05 `HttpClient` handed to each implementation's constructor.
- [ ] `pub fn resolve(results: Vec<Location>, spec: &LocationSpec, limit: u8) -> Result<(Location, Resolution)>` in `src/geo/mod.rs` with `pub enum Resolution { Only, Fuzzy { candidates: usize }, Exact, Coordinates }`: ranking is (1) exact case-insensitive `name` match against the query, (2) `Location::population` descending, (3) upstream order. Zero results ⇒ `Error::Location("no location found for `Beijing`")` (exit 5); `Exact` with no case-insensitive name match ⇒ the same error naming the query. Population is optional ranking metadata and is never rendered.
- [ ] Ambiguity is reported once, on **stderr**, and never in stdout or JSON: `note: 3 candidates for "Beijing"; using Beijing, Beijing, China (population 18960744) — pass `:Beijing` to require an exact name match`; suppressed by `-q`. Emitted only when `candidates > 1`.
- [ ] `pub fn location_line(loc: &Location) -> String` for the shared header/echo format: `<name>, <admin1>, <country> (<lat>, <lon>) <tz>`, with empty/absent parts skipped, two decimals on both coordinates, and `tz` printed as the IANA name; for `LocationSource::Coordinates`/`Osm` with a provisional UTC zone the tail reads `<timezone resolved at fetch time>`. Step 06's plain renderer reuses this function for its `[LOCATION]` header.
- [ ] `src/geo/open_meteo.rs`: `pub struct OpenMeteoGeocoder<'a> { http: &'a HttpClient, cache: &'a Cache, ttl: Duration }` with `const GEOCODE_URL: &str = "https://geocoding-api.open-meteo.com/v1/search"`, requesting exactly `?name=<q>&count=10&language=en&format=json` (percent-encoding and query assembly come from step 05's `HttpRequest`), parsing `{ "results": [ … ] }` into `GeocodeHit { id, name, latitude, longitude, elevation: Option<f64>, timezone: Option<String>, country: Option<String>, country_code: Option<String>, admin1: Option<String>, population: Option<u64> }` (every optional field is `Option` because the API omits empty fields).
- [ ] Response → `Candidate` mapping: `Location { name, admin1, country: country.unwrap_or_default(), country_code, lat, lon, tz: Tz::from_str(timezone)…, elevation_m, source: LocationSource::Geocoder }`; a missing or unparsable `timezone` is **not** silently replaced — it fails with `Error::Upstream("geocoding result `Beijing` has no usable IANA timezone (got \\"Mars/Olympus\\")")`, because day-part aggregation depends on the zone; a missing `results` key or an empty array is a normal no-hit, not an error.
- [ ] Results are cached at `geocode/<sha256>.json` with the key `sha256("open-meteo|" + query.trim().to_lowercase() + "|" + limit + "|en")` and `cache.geocode_ttl_secs`, written through the step-05 cache (so `--offline` can serve a previously resolved name and `--refresh` bypasses a stale one).
- [ ] `Exact` handling with this API: the endpoint has no exact-match parameter, so `:Beijing` sends `name=Beijing` and then filters on `name.eq_ignore_ascii_case(query)`; nothing matches ⇒ `Error::Location`. A name may carry a qualifier (`Beijing, CN`, using the API's documented `<location>[, <country or admin1>]` form) which is passed through verbatim in `Fuzzy` and `Exact` queries.
- [ ] `src/geo/nominatim.rs`: `pub struct Nominatim<'a> { http: &'a HttpClient, cache: &'a Cache, base_url: String }` — the constructor **requires** `&Cache`, so the module cannot be used without step 05's cache layer (response cache `geocode/<sha256>.json` with the same 30-day TTL, plus the throttle state below). Requests `https://nominatim.openstreetmap.org/search?format=jsonv2&q=<query>&limit=<n>&addressdetails=1&extratags=1`.
- [ ] Mandatory, descriptive `User-Agent` on every Nominatim request: step 05's `cirrocast/<version> (+https://github.com/YangtseSu/cirrocast)`; no request is sent without it (non-empty UA asserted in the transport call), and the CLI sends one request per user action only — never an autocomplete-style loop.
- [ ] 1 request/second self-throttling enforced through step 05's `Clock`: the last-request timestamp lives in the cache directory (`ratelimit/nominatim.json`, `{ "last_request_unix_ms": … }`), and before a request the client asks the `Clock` for `now()`, computes the remaining wait and calls `clock.sleep(remaining)`; the throttle file is written *before* the request. Concurrent invocations therefore also respect the limit, and tests inject a recording `Clock` that asserts the requested sleep instead of sleeping.
- [ ] Nominatim response → `Candidate` mapping from `place_id, lat, lon (strings), name, display_name, category, type, address { country, state, city }, extratags { timezone, population }`: `name` falls back to the first comma-separated segment of `display_name`, `admin1` from `address.state`, `country` from `address.country`, `population` from `extratags.population` when present; timezone is taken from `extratags.timezone` when it parses as an IANA name, otherwise `Tz::UTC` **with** `LocationSource::Osm` so step 06 knows to replace it from the forecast response.
- [ ] ODbL compliance in the surface itself: whenever a `~` result is used, `Location data © OpenStreetMap contributors (ODbL)` is printed on stderr, and step 06's plain renderer adds the same line for `LocationSource::Osm` (the contract's attribution requirement, satisfied at the point of display).
- [ ] Service switchability required by the Nominatim policy: `Config.network.nominatim_url` (empty = the public endpoint) plus the `CIRROCAST_NOMINATIM_URL` env override and a `network.nominatim_url` row in step 02's `KEY_TABLE`, so the service can be changed without a software update.
- [ ] CLI: `cirrocast location search <QUERY> [--limit <N>]` in `src/cli.rs` + dispatch in `src/main.rs`, printing `location_line(&loc)` on stdout, the ambiguity note and ODbL attribution on stderr, `-v` additionally listing every candidate in rank order; `--limit` defaults to `10` (the API's default `count`) and is capped at `100` (the API maximum).
- [ ] `tests/geo_spec.rs`: the fixture-driven parse table (every accepted form, every rejection with its message fragment), the ranking rules (exact match beats a larger population, population beats upstream order, upstream order as the last resort), `location_line` for geocoded/coordinate/OSM locations, and `Resolution` reporting. `tests/geo_open_meteo.rs` and `tests/geo_nominatim.rs` replay fixtures through `StubTransport` and never touch the network.
- [ ] Fixture licensing: the recorded geocoding responses derive from GeoNames/Open-Meteo data (`CC-BY-4.0`) and from OpenStreetMap (`ODbL-1.0`), so add `LICENSES/CC-BY-4.0.txt`, `LICENSES/ODbL-1.0.txt`, exact-path `[[annotations]]` entries with those identifiers in `REUSE.toml`, and narrow step 01's blanket `tests/fixtures/**` override so no fixture is silently relicensed to GPL.

## Design notes

* **The parse never guesses and the ranking is total.** `LocationSpec` is a closed enum, so every
  accepted form has exactly one code path; the fuzzy path is the only ambiguous one and it is resolved
  by a deterministic three-key ordering (exact name → population → upstream order), with the winner
  echoed and the candidate count reported. "Never guess twice" is a property of the *output*, not of
  politeness: a script that re-runs the command gets the same location.
* **`@lat,lon` does not call the geocoding API.** The Open-Meteo geocoding endpoint's documented
  parameters are `name`, `count`, `format`, `language`, `apikey`, `countryCode` — there is no reverse
  lookup — so coordinates become a `Location` with `source = Coordinates` and a **provisional** UTC
  zone; step 06 replaces `tz` (and the country/admin names, when the provider reports them) from the
  forecast response's `timezone` field before aggregating day parts. Inventing an offline timezone
  lookup would mean shipping a tz-boundary database, which is out of proportion here.
* **Nominatim needs the cache layer, so this file's network half lands after step 05.** The usage
  policy requires caching and limits the public service to one request per second per application;
  implementing either without a cache would mean a second, throwaway cache inside `geo/`. The
  constructor therefore takes `&Cache` and `&HttpClient` from step 05, and the step-05 README edge
  ("05 depends on 04") is a *file-order* edge: no cycle exists, only two commits from one session.
  This is the reason the "Depends on" line names 05 for the network half.
* The throttle timestamp is written **before** the request: a crashed or cancelled run then still
  counts against the limit, which is the safe direction for a donated service.
* New dependencies: none. Query encoding, transport, retries and caching all come from step 05;
  `chrono_tz::Tz::from_str` comes from step 03. Fixture licensing needs the two licence *texts* in
  `LICENSES/` (REUSE refuses an identifier with no matching file) but no new crate.

## Out of scope

- `src/http.rs` (timeouts, retries, proxy, `Transport`) and `src/cache.rs` (keys, TTLs, offline): step
  05 owns both, and the geocoders are written against them.
- IP-based location (`IpLocator`, `--ip`, `CIRROCAST_IP_SERVICE`): step 05 (`src/geo/ip.rs`); this step
  only reserves `LocationSource::Ip`.
- Using the resolved location to fetch weather, the day-part aggregation that consumes the tz, and the
  plain renderer that prints the attribution line for OSM results: step 06.
- Interactive location picking, a favourite-locations store and shell completion integration: step 08
  (`08-cli-surface-and-formats.md`) and beyond; a saved-locations file is not part of v1's contract.
- Localised place names (`language=<bcp47>` on the geocoding call): step 09.

## Verification

Fixtures: `tests/fixtures/geo/spec-cases.tsv` (arg → expected `LocationSpec` or expected error
fragment), `tests/fixtures/geo/open_meteo_geocode_beijing.json`, `…_beijing_ambiguous.json` (upstream
order contradicts population order), `…_no_hits.json`, `…_bad_timezone.json`,
`tests/fixtures/geo/nominatim_search_tsinghua.json`, `…_tsinghua_ambiguous.json`. Manual smoke run
(network; the second and third invocations are cache hits):

```sh
cargo run -- location search Beijing
# stdout: Beijing, Beijing, China (39.90, 116.40) Asia/Shanghai
# stderr: note: N candidates for "Beijing"; using Beijing, Beijing, China (population …) — pass `:Beijing` to require an exact name match
cargo run -- location search :Beijing                    # same line, no stderr note
cargo run -- location search '~Tsinghua'
# stdout: Tsinghua University, Beijing, China (40.00, 116.33) <timezone resolved at fetch time>
# stderr: Location data © OpenStreetMap contributors (ODbL)
cargo run -- location search '@39.9042,116.4074'
# stdout: 39.9042, 116.4074 <timezone resolved at fetch time>          (source = Coordinates)
cargo run -- location search '@91,0'; echo $?            # error: latitude 91 is out of range -90..=90   (exit 2)
cargo run -- location search 'Beijing, Mars'             # error: no location found for `Beijing, Mars`   (exit 5)
```

## Exit criteria

- [ ] Every string in `spec-cases.tsv` parses or fails exactly as annotated; no `panic!` path exists in
      `src/geo/` for user input.
- [ ] `tests/geo_open_meteo.rs` and `tests/geo_nominatim.rs` pass with `StubTransport` only — no test
      opens a socket (asserted by having no `UreqTransport` instance in the test binary).
- [ ] The throttle test asserts a requested sleep of ≥ 1000 ms between two Nominatim calls and finishes
      without sleeping in wall-clock time.
- [ ] `cargo fmt --check` clean.
- [ ] `cargo clippy --all-targets -- -D warnings` clean.
- [ ] `cargo test` clean (`tests/geo_spec.rs`, `tests/geo_open_meteo.rs`, `tests/geo_nominatim.rs`).
- [ ] `reuse lint` clean, with `CC-BY-4.0` / `ODbL-1.0` texts present in `LICENSES/` and the
      `tests/fixtures/**` override narrowed to first-party fixtures.
- [ ] Smoke run above prints the shown stdout lines; the ambiguity note and the ODbL line appear on
      stderr only, and `@91,0` exits 2 while the no-hit query exits 5.

## Risks

- Nominatim can withdraw access or change its policy; the configurable `network.nominatim_url`, the
  cache and the strict 1 req/s throttle are the mitigations, and `~` remains optional — every other
  location form works without OSM.
- The ambiguity note depends on upstream ordering only as its last tiebreak; if Open-Meteo changes its
  relevance order, ranking results change only for candidates that tie on both name and population.
  The fixture pins the current behaviour so the change is visible in a diff.
- `extratags.timezone` is present only for OSM objects that carry the tag, so `~` results often have a
  provisional zone; the failure mode is a header that shows the provider-resolved zone rather than the
  Nominatim one, and the plain renderer must always prefer the provider's zone for `Osm`/`Coordinates`.
- A one-character query is rejected client-side with a message naming the upstream rule; if the
  geocoding API ever accepts short queries, this becomes a UX annoyance, not a correctness problem.

## Progress log

- 2026-09-30 — plan written.
