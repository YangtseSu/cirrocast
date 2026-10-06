<!--
SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Step 25 — location sources, second generation

Status: ✅ done
Depends on: `04-geocoding-and-location-syntax.md` (spec parsing, ranking, attribution lines, Nominatim throttle), `05-http-cache-and-ip-location.md` (IP chain and cache), `18-offline-city-database.md` (bundled city index), `20-location-candidate-selection.md` (the picker that consumes the candidate lists this step produces)
Touches: `src/geo/{mod,ip,reverse,chain,merge}.rs`, `src/geo/data/`, `build/geo-table/`, `src/config/mod.rs`, `src/cli.rs`, `src/main.rs`, `tests/{geo_ip,geo_geonames,geo_reverse,geo_merge}.rs`, `tests/fixtures/geo/`, `REUSE.toml`, `LICENSES/`, `docs/providers.md`, `README.md`, `CHANGELOG.md`

## Goal

Location resolution gains a second generation of sources, screened from the breezy-weather audit and
each verified against its own documentation: **IP.SB** as a third keyless IP locator (worldwide, no
key), **GeoNames `searchJSON`** as a BYOK city search with the fuzzy matching the offline table
cannot do, **Nominatim `/reverse`** for naming a coordinate, and **Natural Earth country polygons**
plus step 18's city index for fully offline naming (`@lat,lon` → "Xianghe, Hebei, China 12 km away").
Name searches merge candidates across sources, de-duplicate them (same folded name + country + within
5 km), keep step 04's ranking, and hand several hits to step 20's picker. No source here needs a
bundled credential: GeoNames is BYOK.

## Deliverables

- ✅ `src/geo/ip.rs`: `IpService::IpSb` — `GET https://api.ip.sb/geoip`, keyless, worldwide;
  consumes `latitude`, `longitude`, `city`, `region`, `country`, `country_code`, `timezone`; rejects
  a null or `0,0` answer and a missing IANA zone exactly as the existing services do (no silent
  UTC); cached as `ip/ip.sb.json` under the 24 h cap; `CIRROCAST_IP_SERVICE` gains the `ipsb`
  spelling and `auto` becomes `ipwhois → ipapi → ipsb`, every attempt still reported by the
  exhausted-chain message. The `--ip` disclosure line keeps naming the service that answered.
  Measured 2026-10-03 (direct connection; the local proxy's TLS handshake to this host fails):
  HTTP 200 with `{"city":"Xinxiang","region":"Henan","country":"China","country_code":"CN",
  "latitude":35.1874,"longitude":113.8025,"timezone":"Asia/Shanghai",…}` — city-level, keyless, and
  reachable from a mainland-China network without a proxy, which is exactly the gap the two shipped
  services leave.
- ✅ `src/geo/geonames.rs`: `GET https://secure.geonames.org/searchJSON` with
  `q`, `fuzzy=0.8`, `maxRows=<limit>`, `style=FULL`, `username=<key>`; BYOK through
  `CIRROCAST_GEONAMES_USER` → `keys.toml [keys].geonames` (free registration, no card); response
  `geonames[].{name, lat, lng, countryCode, countryName, adminName1, population, geonameId,
  timezone.timeZoneId}` mapped to `Location` with `LocationSource::Geocoder`; drops entries with
  `lat == 0 && lng == 0` or a missing country code; a `{"status":{"value":…}}` body is
  `Error::Upstream` naming the quota message (values 18/19/20 are the documented limits), never a
  silent empty result. Measured 2026-10-03: with no username the endpoint answers **HTTP 401** with
  `{"status":{"message":"Please add a username …","value":10}}` — that specific case is
  `Error::MissingKey` (exit 6) naming `CIRROCAST_GEONAMES_USER`, because the shared helper would
  otherwise call it an invalid key. Cached under `geocode/<sha256>.json` (the query, limit and
  source in the hash) with `cache.geocode_ttl_secs`; credit
  `Location data by GeoNames (CC BY 4.0) — https://www.geonames.org/` through `geo::attribution_line`.
- ✅ `src/geo/chain.rs`: `pub enum GeoSource { OpenMeteo, GeoNames, Nominatim }` and a search chain
  mirroring the provider chain: explicit selection `[geo] search = "auto" | "open-meteo" | "geonames"
  | "nominatim"` (+ `CIRROCAST_GEO_SEARCH`), `auto` = Open-Meteo, then GeoNames when a username is
  configured, then Nominatim `/search` as the last resort (still 1 req/s and cached, still not
  autocomplete); only `Error::Upstream`/`Network`/no-hit continues down the chain, a missing
  GeoNames credential is not an error under `auto`. `~query` remains Nominatim-only, `:query` keeps
  its exact-name filter, and the winner's credit is the source's own (the existing three
  attribution strings, plus GeoNames and ODbL for Nominatim search).
- ✅ `src/geo/merge.rs`: `pub fn merge(sources: &[(GeoSource, Vec<Location>)]) -> Vec<Location>` —
  order-preserving de-duplication: candidates with the same folded name, the same country code and
  within 5 km of each other collapse to one (the earlier source's record wins, its population and
  zone retained), invalid country codes (not two ASCII letters) are dropped, and the merged list is
  ranked with step 04's keys (exact name, population, then source order). Unit tests cover the
  Beijing overlap (Open-Meteo vs GeoNames), a 5 km boundary pair, a `TW`/`-99` country-code
  disagreement, and two different Springfields staying separate.
- ✅ `src/geo/reverse.rs`: coordinate → name, in this order: (1) the **bundled** city index
  (step 18) scanned for cities within 25 km, ranked by distance then population; (2) when that finds
  nothing and the run is online, Nominatim `/reverse?lat=&lon=&format=jsonv2&zoom=10&addressdetails=1`
  (same base URL, UA, 1 req/s throttle and `geocode/<sha256>.json` cache as the search path, 30-day
  TTL) mapping through the step-04 helper. `[geo] reverse = "auto" | "offline" | "off"` (+
  `CIRROCAST_GEO_REVERSE`, default `auto`); `--offline=geo` and `--offline` imply `offline`, which
  never opens a socket. Several offline candidates inside the radius go through step 20's picker
  (nearest first); the chosen name is a **display attribute only** — a coordinate location keeps its
  `source = Coordinates` and its full-precision `lat`/`lon` as the request key, and `-v` prints which
  source named it and how far away the place is.
- ✅ `src/geo/country.rs` + `build/geo-table/`: Natural Earth 1:50m countries (public domain),
  quantised and stripped to `ISO_A2` + English name, embedded gzip like the city table (fall back to
  the 110m dataset if the compressed member exceeds 1 MiB; measure and record the size);
  point-in-polygon gives the country name and code for any coordinate, offline; the four `ISO_A2`
  values Natural Earth leaves as `-99` (Taiwan, Northern Cyprus, Kosovo, Somaliland) map through an
  explicit name table so no candidate ends up with an invalid code; `REUSE.toml` carries the Natural
  Earth annotation and `LICENSES/CC0-1.0.txt` the licence text.
- ✅ `--ip` naming: when the IP service's own city is empty (allowed by the ipwho.is schema) the
  coordinate now gets a name through `reverse.rs` instead of being printed bare; with several
  nearby places the picker asks; the disclosure line and README/`--help` privacy text list all three
  IP services. `location search @lat,lon` prints the coordinate line plus the named candidates under
  `--all` (step 20's flag); `location search --ip --all` does the same for the IP answer.
- ✅ `docs/providers.md`: a new "Location services" table — service, endpoint, auth, licence/credit,
  privacy-policy URL, cache ceiling, `verified: <date>` — covering the Open-Meteo geocoder,
  Nominatim search and reverse, GeoNames search, IP.SB, ipwho.is, ipapi.co and the bundled Natural
  Earth/GeoNames data sets (which get no network row but a data section), with the per-service
  caching and rate-limit obligations quoted from the live pages at the recorded date.
- ✅ Tests (`tests/geo_ip.rs` extended, `tests/geo_geonames.rs`, `tests/geo_reverse.rs`,
  `tests/geo_merge.rs`, all offline): ip.sb fixture plus its fall-through; a GeoNames hit, a quota
  body, a `0,0` row and an invalid country row; Nominatim reverse fixture sharing the search
  mapping; offline naming from the bundled table (Beijing coordinates → a Hebei city within 25 km),
  the no-candidate case with `[geo] reverse = "off"` and with `--offline`; merge/dedup ordering;
  `--offline=geo` opening no socket (the existing `strace` pattern).

## Design notes

* **Screened from the breezy-weather audit (2026-10-03).** What this step takes and why: IP.SB is
  keyless and worldwide, where the two shipped services are quota-limited; GeoNames `searchJSON` is
  the only free service in the audit with `fuzzy` matching over alternate names; Nominatim's
  `/reverse` is the standard coordinate-naming path and already deployed here under its policy; the
  Natural Earth country layer is public domain and 2 MB *as shipped by breezy* before
  simplification, so a quantised extract is cheap. Explicitly **rejected**: **Baidu IP**
  (`api.map.baidu.com/location/ip`) needs an AK key, is China-only, and answers in GCJ-02, so mixing
  it would put the point hundreds of metres off in a data model that is WGS-84 throughout; a
  licensed offset conversion is out of proportion for a city-level CLI. **Offline IP databases**
  (GeoLite2 and friends) need an account, forbid redistribution of the database and would add tens
  of megabytes to the binary. **Xiaomi/Caiyun** (the `china` module in breezy) is reverse-engineered
  with a hard-coded app key and signature and is not adopted under any circumstances; **BMD
  (Bangladesh)** is a third-party aggregator rather than the agency's API; **Météo-France's**
  location service mints a JWT from a secret shipped inside the app, the opposite of the BYOK model
  this project follows.
* **GeoNames is BYOK, never bundled.** Breezy ships a `BuildConfig.GEO_NAMES_KEY`; this project
  cannot (a bundled credential in a GPL distribution is exactly the pattern `AGENTS.md` §10
  forbids). Without a username the chain simply skips GeoNames.
* **Reverse naming is display-only and offline-first.** The coordinate is the user's own input and
  the request key; naming it must not change what is fetched, and doing it offline first keeps `--ip`
  from leaking the coordinate to a second service unless the bundled tables cannot name the place
  and the user is online.
* **25 km and 5 km** are the audit's own constants (`REVERSE_GEOCODING_DISTANCE_LIMIT`,
  `CLOSE_DISTANCE` in breezy); they are documented here so a later tuning is a deliberate change.
* **`[geo] search` is orthogonal to step 18's `[geo] strategy`.** `strategy = "auto"` consults the
  bundled table first and the network backends only when it yields no hit; `--offline=geo` /
  `[network] offline` restrict geocoding to the bundle entirely, in which case `search` is never
  reached. This step names the network backends behind that table; it changes nothing about the
  offline layering.
* **The picker is the single selection surface.** This step only produces candidate lists; it never
  grows a second chooser, and non-interactive runs keep step 20's rules (ranked winner + note).

## Out of scope

Postal codes, admin-2/admin-3 hierarchies, place-name search over OSM *objects* (`~` already covers
that case), geocoding in non-English locales beyond what each API returns, user-supplied city files,
offline IP databases, and reverse geocoding of multiple coordinates in one run (step 19's
multi-location form will reuse the same single-point path).

## Verification

```sh
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && reuse lint

cargo run -q -- location search Beijing --all        # ranked list, GeoNames/Open-Meteo merge, credit
CIRROCAST_GEONAMES_USER=… cargo run -q -- location search 'Springfiel' -v   # fuzzy hit under -v
cargo run -q -- location search @39.9042,116.4074    # coordinate line + nearest place within 25 km
cargo run -q -- location search @0,-140 -v           # mid-Pacific: no name, verbose says why
cargo run -q -- --ip -f plain -v                     # service named; ip.sb answers when the others fail
cargo run -q -- --offline=geo -f plain @39.9042,116.4074   # offline naming, no socket
CIRROCAST_IP_SERVICE=ipsb cargo run -q -- location search --ip
```

Observable result: three IP services are individually selectable and the chain still aggregates
failures; a GeoNames username adds fuzzy candidates with the CC-BY-4.0 credit; a coordinate gets a
name offline; `--offline=geo` names it without a socket; the merged candidate list has no duplicate
for the same place and keeps step 04's order.

## Exit criteria

- ✅ `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`, `reuse lint` clean
      (with the Natural Earth annotation and licence text in place).
- ✅ `CIRROCAST_IP_SERVICE=ipsb` answers with a real location; the chain message still names every
      attempted service when all three fail.
- ✅ GeoNames without a username is skipped silently under `auto` and named as the missing piece
      under `-v`; with one, a fuzzy query returns merged candidates (fixture-verified: no GeoNames
      account exists for this repository, so the merge is pinned by `tests/geo_merge.rs` and the
      live half by the skip note).
- ✅ `[geo] reverse = "off"` and `--offline` never name a coordinate from the network; `auto` names it
      offline-first (asserted by fixture and by the no-socket `strace` check).
- ✅ The merged list for Beijing contains no duplicate place; the 5 km boundary and `-99` country
      fixtures pass.
- ✅ `docs/providers.md` gains the location-services table with live-verified lines, and README's
      `--ip` privacy note lists all three services.

## Risks

* Third-party geo services change terms or endpoints (GeoNames quotas, Nominatim policy, ip.sb
  availability); each row carries a `verified` date, every service is switchable
  (`nominatim_url`, `CIRROCAST_GEO_SEARCH`, `CIRROCAST_IP_SERVICE`), and all of them are cached.
* Coordinate naming can be confidently wrong near a border or between two close towns; it is
  display-only, the distance is printed under `-v`, and `--all` shows the other candidates, so the
  user can always fall back to `@lat,lon` (which never changes what is fetched).
* The Natural Earth extract adds binary size; the fallback to 110m and the size record keep it
  inside step 21's budget, and the data stays credited.
* GeoNames free usernames are rate-limited per day; a 429/status-code body becomes an exit-3 error
  naming the quota, never a silently empty search, and the geocode cache absorbs repeat queries.

## Progress log

- 2026-10-03 — step opened after the breezy-weather source audit (IP.SB, GeoNames, Nominatim reverse,
  Natural Earth all traced to their modules there) and a local-verification pass on the services this
  project will actually call. Shipping is deliberately arranged so that no source is a prerequisite
  for another: IP.SB and GeoNames can land without the reverse path, and the reverse path degrades to
  the bundled tables alone.
- 2026-10-03 — measurements this file is built on: `api.ip.sb/geoip` answered HTTP 200 JSON directly
  from this network (the local proxy fails its TLS handshake) with `city`/`region`/`country`/
  `country_code`/`latitude`/`longitude`/`timezone` populated; `secure.geonames.org/searchJSON`
  without a `username` answers HTTP 401 `{"status":{"value":10,"message":"Please add a username …"}}`,
  which is why the no-username case is mapped to `MissingKey` rather than the shared helper's
  invalid-key path; both recorded in the deliverables above.
- 2026-10-04 — renumbered from 27 to 25 by the plan reorganization; dependencies are now 18 and 20 (`20-location-candidate-selection.md` was 26).

- 2026-10-06 — the two decisions the recon note left open, taken before the first line of code:
  (1) **credit for a named coordinate** goes through a new `Location::named_by:
  Option<LocationSource>` (serde-defaulted, not part of the `json` projection), not a new
  `LocationSource`: a coordinate keeps `source = Coordinates` because that is what drives
  `provisional_zone` and the providers' zone-correction allowlist, while `attribution_line` falls
  back to `named_by` so a name attached by the bundled tables or Nominatim still carries its
  credit; a bare coordinate keeps carrying none. A *direct* GeoNames search gets the new
  `LocationSource::Geonames` variant the contract already words. The README attribution paragraph
  is amended in the commit that lands `named_by`.
  (2) **the Natural Earth layer** is a second mode of the one builder, not a second crate:
  `cargo run -p geo-table -- --countries <path-or-url> [output-dir] [--check]` writes
  `countries.bin.gz` plus a `COUNTRIES` provenance record beside the city members, with the same
  compare/install discipline, one more `[[annotations]]` entry and `LICENSES/CC0-1.0.txt`.
  Measured on the 2026-10-06 download (`ne_50m_admin_0_countries.geojson`, 242 features, 99 613
  points): 408 KiB gzipped at 1e-3 quantisation, inside the 1 MiB budget, so 1:50m ships.
- 2026-10-06 — `IpService::IpSb` landed: third service, `ipsb` spelling, `auto` = ipwhois → ipapi →
  ipsb, `0,0` answers refused as the service's own sentinel. Live run (`CIRROCAST_IP_SERVICE=ipsb
  cirrocast location search --ip`) answered `Xinxiang, Henan, China (35.19, 113.80) Asia/Shanghai`
  through `api.ip.sb/geoip`; the fixtures are minimised recordings of that answer and of the
  sentinel, and the chain failure test now names all three attempts.
- 2026-10-06 — recon for the next session (no code yet). The subsystem this step extends is fully
  mapped: `src/geo/mod.rs` holds `LocationSpec`/`parse_arg`, the `Geocoder` trait
  (`fn search(&self, query: &str, limit: u8) -> Result<Vec<Location>>`), `resolve_candidates`,
  `attribution_line` and `provisional_zone`; `open_meteo.rs`/`nominatim.rs` are the two geocoders;
  `ip.rs` holds `IpService::{IpWhoIs, IpApiCo}` with `chain()`/`locate_with_service()`/`CacheKey::ip`
  and the `--ip` disclosure line; `offline.rs`+`table.rs`+`build/geo-table/` are the bundled city
  index and its builder; `rank.rs` owns `Candidate`/`rank`, `pick.rs` the step-20 prompt, `fold.rs`
  the NFKD folding, `tz.rs` the offline coordinate → zone lookup. `[geo]` today has
  `strategy`/`data`/`update`/`update_interval_days`/`update_url`; `check_known_keys` in
  `src/config/mod.rs` rejects `search`/`reverse`, so adding them touches `allowed_keys`,
  `GeoConfig`, its `Default`, `DEFAULT_DOCUMENT`, `KEY_TABLE` (getter/setter/validator) and
  `validate_geo` — the file has a test that keeps `allowed_keys` and `KEY_TABLE` in step.
  `cli.rs` resolves names in `resolve_location`/`name_location`/`local_lookup` and enforces
  `--offline=geo` there.
  Two decisions to make in the first commit, both recorded here so they are deliberate:
  (1) **reverse naming and the credit.** The step's letter keeps `source = Coordinates` for a
  name that reverse geocoding attached, but the rendering contract says attribution travels with
  displayed data and `attribution_line` keys off `LocationSource` alone — so a named coordinate
  should either carry a new source (`LocationSource::Geonames` for a direct GeoNames search, which
  the contract already words) or a `named_by` field the line falls back to. Pick one, amend
  `docs/plans/README.md`'s attribution paragraph in the same commit, and keep `coordinates → no
  credit` true for the user's own input.
  (2) **the Natural Earth layer.** `build/geo-table/` takes exactly one positional source and
  emits `cities.bin.gz`+`keys.bin.gz`+`SNAPSHOT`; the country polygons need either a second
  member in that same builder (same `--check`/install/compare discipline, one more
  `[[annotations]]` entry and `LICENSES/CC0-1.0.txt`) or a separate builder. The 1:50m extract must
  stay under the 1 MiB compressed fallback budget named in the step, and the four `ISO_A2 = -99`
  shapes (Taiwan, Northern Cyprus, Kosovo, Somaliland) map through the explicit name table.

- 2026-10-06 — the second-generation search path landed as one change: `src/geo/geonames.rs` (the
  BYOK `searchJSON` geocoder), `src/geo/chain.rs` (the `[geo] search` chain) and `src/geo/merge.rs`
  (the cross-source de-duplication), wired into `name_location`. `[geo] search` +
  `CIRROCAST_GEO_SEARCH` are in `GeoConfig`, `allowed_keys`, `KEY_TABLE`, `validate_geo` and the
  contract block. The GeoNames account name is a *named credential* in `keys.toml`
  (`KeyStore::canonical`/`get`/`set`/`remove`/`list` learned non-provider names, so `key set
  geonames` writes it from stdin and `key list` shows it masked) with `CIRROCAST_GEONAMES_USER` as
  the environment tier. Measured live (2026-10-06): with no account, `auto` answers from Open-Meteo
  and narrates `geonames: skipped (no account name; …)` and `nominatim: not asked (an earlier source
  answered)` under `-v`; `CIRROCAST_GEO_SEARCH=geonames` without an account exits 6 with
  `missing API key for geonames: run \`cirrocast key set geonames\` or set CIRROCAST_GEONAMES_USER`.
  Two environment facts recorded for later sessions: `api.geonames.org` fails TLS certificate
  validation (`no alternative certificate subject name matches target hostname`), which is why the
  endpoint is `secure.geonames.org`; and `nominatim.openstreetmap.org` is unreachable from this
  network (connect timeout, measured 2026-10-06), so the Nominatim path is verified through its
  recorded fixture (`tests/geo_nominatim.rs`, `tests/geo_merge.rs`) rather than live.

- 2026-10-06 — the country layer landed: `src/geo/country.rs` (the Natural Earth parser, the member
  codec, the point-in-polygon lookup and the build path) plus the builder's `--countries` mode
  (`cargo run -p geo-table -- --countries <path-or-url> [output-dir] [--check]`, writing
  `countries.bin.gz` + `COUNTRIES` with the same compare discipline as the city table).
  `src/geo/table.rs`'s byte reader, gzip helper and 64 MiB inflate cap are shared rather than
  copied; `LICENSES/CC0-1.0.txt` and the `REUSE.toml` annotation carry the public-domain dedication.
  Measured on the pinned `v5.1.2` GeoJSON (identical to `master` on 2026-10-06, 3 083 490 bytes):
  242 countries, 99 613 points, **416 965 bytes (407 KiB)** compressed at 1e-3 quantisation — inside
  the step's 1 MiB budget, so 1:50m ships and no 110m fallback is needed; the builder refuses a
  member over the budget instead of letting a refresh regress it silently. The code field prefers
  Natural Earth's corrected `ISO_A2_EH` (which fills France, Norway, the Indian Ocean Territories,
  Ashmore and Cartier, and carries `TW`/`XK`), falls back to `ISO_A2`, then to the explicit
  `DISPUTED_CODES` table (Taiwan `TW`, Kosovo `XK`, N. Cyprus `CY`, Somaliland `SO`), and leaves the
  code *empty* when neither answers (Siachen Glacier), so no `-99` can reach `Location::country_code`.
  Canaries pin Beijing → `CN`/China, Maseru → `LS` (the Lesotho hole inside South Africa's shape),
  the mid-Pacific → no country, and the four disputed codes.

- 2026-10-06 — coordinate naming landed: `src/geo/reverse.rs` (`Policy`, `Nearby`, `from_table`,
  `name`), `Nominatim::reverse` (the `/reverse` request, the refusal mapping, the five-decimal
  cache key), `OfflineTable::nearby`/`Cities::nearby` (one pass over the row section, ordered by
  distance then population), `[geo] reverse` + `CIRROCAST_GEO_REVERSE` in the config schema, and
  the CLI's `name_coordinate`. `Location` gained `named_by: Option<LocationSource>` (serde-default,
  outside the `json` projection) so the credit travels with a name attached to someone else's
  location; `attribution_line` falls back to it and `location_line` now shows the coordinates of a
  named coordinate. Two decisions recorded because they are easy to lose:
  (1) **the automatic picker does not fire for a coordinate.** The step's letter says several
  nearby names "go through step 20's picker"; taken literally that would prompt on every terminal
  `@lat,lon` run whose coordinate has two neighbours within 25 km, asking the user to choose
  something that cannot change what is fetched (the coordinate is the request key) — and the
  picker's own advice ("use @lat,lon to skip the prompt") would be self-contradictory. So `--pick`
  asks, `--all` lists, and the automatic policy stays with the name searches it was written for;
  the README contract paragraph was amended in the same commit. (2) **offline naming carries no
  admin1**: the city table has no admin-1 column, so a bundled name is "Xianghe, China", not the
  step's illustrative "Xianghe, Hebei, China" — the country *name* comes from the new layer, the
  division only from Nominatim. Measured live 2026-10-06: `@39.9042,116.4074` is named "Beijing,
  China (39.90, 116.41)" 0.9 km away with the GeoNames credit; `--offline=geo` names it with no
  socket; `@0,-140` under `reverse = "offline"` prints the bare coordinate and the note `no city
  within 25 km`; under `auto` the unreachable Nominatim is a `-v` note, not a failure.

- 2026-10-06 — `--ip` naming landed: a city the IP service omits is no longer an error in any of
  the three decoders (`Location.name` comes back empty), and the CLI's `name_ip_answer` gives such
  an answer the same treatment a typed coordinate gets — the bundled tables, else Nominatim, else
  the coordinate pair itself, so the header is never blank. `place()` now skips an empty name part
  and `location_line` omits the parenthesised pair when the name *is* the pair
  (`geo::coordinate_name`, one spelling shared by `from_coordinates` and the IP fallback). The
  automatic picker stays out of this path too, for the same reason as the coordinate's: the name is
  display-only and the answer's coordinates are the request key. The privacy disclosure already
  named whichever service answered, and the README/`--help` text lists all three. Observed live
  (2026-10-06, a seeded cityless answer): `location search --ip -v` prints
  `location: named by the bundled tables … Beijing, Beijing, China` with the GeoNames credit and
  the three nearby candidates, `--all` lists them, and `CIRROCAST_GEO_REVERSE=off` falls back to
  `39.907503, 116.397228, Beijing, China`.

- 2026-10-06 — the documentation pass: `docs/providers.md` gained the "Location services at a
  glance" table (endpoint, auth, licence/credit, policy, cache ceiling, `verified`) plus the bundled
  data table, a `### GeoNames searchJSON (BYOK)` section with the measured 401/quota behaviour, an
  `### IP.SB` section with the caller-address-only finding, and a `/reverse` paragraph in the
  Nominatim section; the re-verification log carries the 2026-10-06 entry. The README documents the
  two new `[geo]` keys, the named credential, the search chain, coordinate naming, the refreshed
  location examples and two new data-sources rows; `CHANGELOG.md` opens with the step's entry.

- 2026-10-06 — step closed. Every deliverable and exit criterion is ticked; the four gates
  (`cargo fmt --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`,
  `cargo test --workspace --locked`, `reuse lint`) are clean, and the changed surface was run, not
  only tested: `location search Beijing --all` from the bundled table; `location search
  @39.9042,116.4074` → `Beijing, China (39.90, 116.41)` with the GeoNames credit;
  `location search @0,-140 -v` → the bare pair plus `no city within 25 km` (and, under `auto`, the
  unreachable Nominatim as a `-v` note rather than a failure); `--offline=geo -f plain
  @39.9042,116.4074` → the report headed with the name, `strace -f -e trace=network` counting **0**
  `socket(` calls; `--ip` with a seeded cityless answer named from the bundled tables;
  `CIRROCAST_IP_SERVICE=ipsb location search --ip` → a real answer via `api.ip.sb/geoip`.
  One deviation from the letter of the deliverable was recorded earlier and is now in the README
  contract as well: the automatic picker never fires for a *display-only* name (a coordinate's or
  an IP answer's); `--pick` asks and `--all` lists. One more rule was added while closing, because
  it would have been a latency regression: the `status` probe names a coordinate from the bundled
  tables at most (`GeoRequest::online_naming`), so a status bar never waits on a donated service
  for a display name — pinned by `tests/status_contract.rs`.
  Not verified live, and recorded here so a later session can close it: a GeoNames *account* query
  (`CIRROCAST_GEONAMES_USER=… location search 'Springfiel' -v`) — no account exists for this
  repository, so the fuzzy hit, the quota error and the merge are pinned by `tests/geo_geonames.rs`
  and `tests/geo_merge.rs` against hand-authored/recorded fixtures instead; and a live Nominatim
  answer, since the public instance is unreachable from this network (timeouts), which is why the
  `/reverse` fixture is hand-authored and `docs/providers.md` marks the endpoint `Unverified`.
