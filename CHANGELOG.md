<!--
SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Changelog

All notable, user-visible changes to `cirrocast` are documented here. The format follows
[Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/), and the version numbers follow
[Semantic Versioning 2.0.0](https://semver.org/spec/v2.0.0.html). The crate version, the JSON
output schema version and the config schema version move independently; [`docs/schema.md`](docs/schema.md)
records how each of them changes and which changes are breaking.

<!--
The template a breaking output change uses (see `docs/ecosystem.md`, "Output contracts"): a minor
version bump, one release of dual emission where feasible, and an entry that names both shapes so a
consumer can see exactly what moved. It lands under the version it applies to, after `### Changed`:

### Breaking

* **<the surface>** (step NN). `<old shape>` is now `<new shape>`: <what a consumer must do>, and
  <whether the old shape is still emitted, for how long, and behind what flag>. The JSON
  `schema_version` is now <N> (`docs/schema.md`).
-->

## [Unreleased]

### Changed

* The README is user-facing again: its versioning, packaging, publishing and CI sections moved to
  where the contributors need them — the version table with the schema compatibility rules it belongs
  to (`docs/schema.md#versioning`), and the packaging, release, publishing and CI material, including
  the release checklist, to [`CONTRIBUTING.md`](CONTRIBUTING.md#packaging-and-release). The README
  keeps a short Development pointer, and `AGENTS.md`'s release section keeps only the two invariants.
* `CIRROCAST_LOCATION` is documented as what it has always been — **one** location. The `--help`
  epilogue, `docs/configuration.md` and `docs/location.md` now say that the environment tier has no
  second argument, so a comma in the value is part of the spec (a coordinate pair, or a
  `City, Region` name) and a multi-location run belongs on the command line.

### Fixed

* **Obscuration conditions no longer collapse into fog (WeatherAPI, QWeather, World Weather Online,
  NWS).** These backends mapped haze, dust, sand, smoke and mist to WMO 45 (`Fog`), although the
  canonical model describes each phenomenon (WMO 4/5/6/7/10) and the METAR/OpenWeatherMap mappings
  already used them: a `Smoke` reading prints `Smoke`, a sandstorm `Dust or sand raised by wind`,
  and NWS's `Blowing Dust` is no longer `Fog`. Only the members WMO 4677 does not describe in this
  crate's table still take the nearest described family (dust/sand *storms*, smog, and NWS's
  combined `Fog/mist` icon). World Weather Online also gained the eleven published obscuration codes
  it had no arm for (125–161, previously `Unknown`), and each affected provider's published code
  list is now pinned by an exact mapping test — the NWS icon table against the vendor's `/icons`
  list, the three native-code tables against their published feeds.
* The unreachable `Mono` branch in `color::paint` is gone: a monochrome run resolves to "nothing may
  be painted" through the same helper `write_paint` uses, so the two no-colour paths cannot drift
  apart.

### Removed

* `I18n::format_temp` — a public wrapper no renderer called (every format formats temperatures
  through `model::units`), deleted so it can no longer be mistaken for the conversion path. The
  `format-temp-c`/`format-temp-f` catalog messages remain, like the other reserved `format-*`
  spellings.

## [1.3.0] - 2026-10-06

Phases E–G landed together — the enforced performance and resource budget and the `status` probe
with the ecosystem recipes and the frozen output contracts (steps 21–22), the additional backends
with the coverage-aware `auto` chain, the second-generation location sources, climate normals and
QWeather's Ed25519 JWT (23–27), and the documentation set with the frozen JSON Schemas (28). No
release was cut at the phase E boundary, so the planned `1.4.0` is folded into this one. The v1 CLI
contract is unchanged, and the JSON and configuration documents stay at `schema_version` 2. The
bundled `GeoNames` snapshot is refreshed to the official 2026-10-05 `cities15000` dump (the first
refresh since 1.2.0), and the Natural Earth country layer is unchanged against its tagged release
(both re-checked 2026-10-06).

### Added

* **QWeather JWT authentication (step 27).** The second authentication mode the vendor recommends is
  now usable beside the API key. `cirrocast key set qweather --jwt --key-file <PATH|-> --credential-id
  <ID> --developer-id <ID> --project-id <ID>` validates an Ed25519 (PKCS#8) PEM — read from a file or
  stdin, never from argv — and stores it with its three non-secret identifiers in the same `0600`
  `keys.toml`, under `[jwt.qweather]`; the `CIRROCAST_QWEATHER_JWT_{CREDENTIAL_ID,DEVELOPER_ID,PROJECT_ID,PRIVATE_KEY}`
  quartet configures the same credential from the environment. Resolution is first-complete-set-wins
  (quartet → `[jwt.qweather]` → `CIRROCAST_QWEATHER_KEY` → `[keys]`), and a **partial** JWT set is a
  config error naming what is missing rather than a silent fall-back to the API key. Every weather
  and alert request then carries `Authorization: Bearer <token>`, a 15-minute EdDSA token minted per
  fetch (backdated 30 s for clock skew, inside the vendor's 24 h ceiling) and never persisted; the
  token is registered as a request secret, so no `-v` line, error message or cache entry can contain
  it, and two fetches of the same request share one cache key. `key list` prints
  `qweather  jwt (kid …, iss …, sub …)` (plus `api key <masked>` when both are stored, never the
  PEM), `key rm qweather` removes both forms, `provider info qweather` says
  `auth: API key or JWT (Ed25519)`, and a 401 names both remedies — replace the credential, or
  validate the token in the console's JWT Validation. `ring` and `base64` become direct
  dependencies; both were already in the build (`rustls`, `ureq`), so no crate joins the graph. The
  API host requirement is unchanged.

* **Climate normals (step 26).** `--normals` compares the forecast with the climate: the month's
  normal (mean of the calendar month's mean daily temperature, mean high, mean low and monthly
  precipitation) is **computed here** from NOAA NCEI's Global Summary of the Month — the mean over
  `[normals] period`, default the WMO `1991-2020` window — for the station nearest the location
  (`[normals] max_distance_km`, default 60 km; the station, its distance and the contributing years
  are printed, and a month with fewer than 20 usable years yields no comparison). The line lands in
  the `art-table` footer and `plain` (`high 26.4°C (-2.4°C) · low 16.1°C (-2.1°C) · precip
  48.9 mm/mo (-100%)`, signs as text so a colourless terminal keeps them), the typed `normals`
  object in `json` (additive, `schema_version` 2), and `--format normals` prints it standalone;
  `one-line` is unchanged. Two keyless requests, both cached for 30 days under
  `$XDG_CACHE_HOME/cirrocast/normals/`, and **zero** requests without the flag; `[defaults] normals`
  turns it on for every run, `CIRROCAST_NORMALS_PERIOD` and `CIRROCAST_NORMALS_MAX_DISTANCE_KM`
  override the two keys, and a failed fetch is a warning (or a `-v` note for the "no normal exists"
  outcomes) that never changes the exit code. Public-domain NOAA data, credited as `Climate normals
  computed from NOAA NCEI Global Summary of the Month (public domain)`.

* **Second-generation location sources (step 25).** A plain name now runs through `[geo] search`
  (`CIRROCAST_GEO_SEARCH`): `auto` queries Open-Meteo, then **GeoNames `searchJSON`** (BYOK —
  `CIRROCAST_GEONAMES_USER` or `key set geonames`; a free account with fuzzy matching), then
  **Nominatim** as the last resort, and the per-source candidate lists are merged and
  de-duplicated (same folded name, same country code, within 5 km) before the shared ranking picks
  a winner. **`IP.SB`** joins the IP chain as the third keyless service (`auto` = ipwho.is →
  ipapi.co → IP.SB), a `0,0` answer being the service's own sentinel. A coordinate — and an IP
  answer whose service reported no city — is now **named for display**: the bundled city index
  within 25 km, the new **Natural Earth 1:50m country layer** (public domain, 407 KiB, embedded)
  for the country name, and Nominatim `/reverse` when the tables find nothing; `[geo] reverse`
  (`CIRROCAST_GEO_REVERSE`) picks `auto`/`offline`/`off`. A name is display-only — the coordinate
  stays the request key — and its credit travels with it, so a named coordinate carries the
  GeoNames or ODbL line while a bare one carries none. `--all` lists the nearby names, `--pick`
  asks which one to use, and `-v` says which source named the point and how far away it is.
* **Two keyless national backends (step 24).** `nws` (`api.weather.gov`: the US and its
  territories, hourly plus 7-day periods, its coordinate → grid mapping cached for 30 days in the
  new `grid/` cache namespace) and `brightsky` (Bright Sky's DWD open data for Germany, hourly).
  Both are keyless and both declare themselves `free` in the new network class.
* **`--provider auto` is now coverage-ranked (step 24).** Instead of a fixed list it expands per
  resolved location from the registry's machine-readable coverage: an exact country code first
  (a US point starts at `nws`, a German one at `brightsky`), then a containing bounding box
  (a Swedish one at `smhi`), then the global keyless entries (`open-meteo`, `met-no`) — registry
  order within each tier, and `-v` prints the expansion. A multi-location run ranks every slot for
  its own place. Station-only, archive-only and supplementary rows never enter it.
* **Network class and a `NET` column (step 24).** Every registry row now declares whether it sits
  on free infrastructure or a commercial service; `provider list` prints `free`/`nonfree` in a
  `NET` column, `provider info` prints `network:`, `history days:` and `marine:` rows, and
  `docs/providers.md` defines the classification.
* **`grid/` cache namespace (step 24).** A provider's coordinate → grid-cell mapping is neither a
  station nor a forecast, so it has its own directory (30-day TTL); `cache stat` reports it and
  `cache clean` removes it like any other entry namespace.
* **Four more backends (step 23).** `met-no` (MET Norway, keyless, global, 9 days; the first backend
  that speaks `Expires`/`Last-Modified`, so a refresh costs headers instead of a body),
  `visualcrossing` (BYOK `CIRROCAST_VISUALCROSSING_KEY`, 15 days, and the first backend whose own
  payload carries severe-weather warnings), `open-meteo-archive` (keyless ERA5/ERA5-Land/IFS
  reanalysis from 1940-01-01) and `open-meteo-marine` (keyless waves, swell and sea-surface
  temperature for the nearest sea cell), and `--provider auto` gained `met-no` through the interim
  chain step 24 then replaced with the coverage ranking described above.
* **`--date <YYYY-MM-DD>` and `--history <N>d` (step 23)**: ask a history-capable backend for one
  calendar date or the last `N` days (ending yesterday) instead of a forecast. Every format labels
  the answer — the `art-table` header ends `· 2026-09-14 · archive`, `plain` writes an
  `archive: <date>` record, `one-line` prefixes the line and `json` carries `"mode"`. A backend
  without an archive span is a usage error (exit 2) naming the flags, and an archive-only backend
  refuses `--days` rather than silently clamping it.
* **`--marine` (step 23)**: appends the wave block (waves, period, direction, swell, sea-surface
  temperature) to `art-table` and `plain`, and as a `marine` object to `json`, with the sampled sea
  cell named when it lies more than 25 km from the requested point. The marine source is
  supplementary: `--provider open-meteo-marine` alone is a usage error.
* **WMO 4677 codes 68 and 69 (sleet)** join the canonical condition table, with their art blocks,
  one-line glyphs and both catalogs.
* **The JSON document gains four keys** (additive within `schema_version` 2): `mode`
  (`forecast`|`archive`), `marine` (the wave block, `null` unless `--marine` asked), and
  `capabilities.history_days` / `capabilities.marine`. `docs/schema.md` lists them.
* **`--alerts-from visualcrossing`** is now accepted, and `visualcrossing` joins `[alerts] sources`
  under `auto` when its provider is on the chain — its warnings are merged from the forecast
  payload rather than fetched again, so naming it explicitly needs `--provider visualcrossing`.
* **`cirrocast status`, the status-bar probe (step 22).** One line per run: `--format` (and its
  synonym `--template`) take a `%`-template, `%c %t` by default, and stdout is exactly one line —
  a template newline becomes a space and the line is trimmed. Colour is off unless
  `--color always`, because bars strip ANSI inconsistently. A transient or data failure — no
  network, no cached answer, a missing or rejected key — prints the placeholder (`n/a`,
  `--placeholder`, `[status] placeholder`) on stdout with one `error: …` line on stderr and
  **exit 0**, so a bar never shows a crashed module; only a usage mistake (2) and a configuration
  problem (4) fail. The probe never performs the public-IP lookup: with no `--location` and no
  `[location] default` it exits 4 naming the key to set. It never prompts either — an ambiguous
  name takes the ranked winner. `--max-age <SECS>` widens the window in which a cached answer is
  served without revalidating (`0` and the default follow `[cache] weather_ttl_secs`, and the knob
  only widens); `--offline` never opens a socket and serves any cached answer however old. The
  alert set is fetched only when the template shows `%A` and the air reading only when it shows
  `%q`, so the default probe costs one request.
* **Status-bar recipes** in [`contrib/statusbar/`](contrib/statusbar/): waybar, polybar, i3blocks,
  tmux and starship fragments plus bash/zsh prompt snippets that cache the rendered line under
  `${XDG_RUNTIME_DIR:-${TMPDIR:-/tmp}}/cirrocast/status` and re-render only when it is older than
  `CIRROCAST_STATUS_INTERVAL` (900 s). Every example carries the command it runs in a
  machine-readable marker; `contrib/statusbar/verify.sh` extracts it, runs it offline against the
  committed cache fixture `tests/fixtures/cache/weather/open-meteo-39.90-116.40-3-2026-10-05.json`
  and asserts one line with exit 0. The CI job `statusbar` runs the same script.
* **[`docs/ecosystem.md`](docs/ecosystem.md)** records the probe's contract and freezes the three
  machine-readable surfaces — `json` (`schema_version` 2, additive only), the `one-line` token
  meanings and the `plain` record and field order (pinned by `tests/plain_order.rs`) — together
  with the breaking-change policy: a minor bump, one release of dual emission where feasible, and a
  `### Breaking` entry naming both shapes. [`docs/schema/json-v2.json`](docs/schema/json-v2.json)
  is the JSON document as a Draft 2020-12 schema (the frozen v1 beside it, and the validator test,
  arrive with step 28).
* **Configuration**: `[status] placeholder` (`"n/a"` by default) sets what the probe prints when it
  has no reading. It is addressable as `config get/set status.placeholder`.

### Changed

* **Faster, smaller builds (step 21).** The release profile now uses one codegen unit and aborts on
  panic, the renderers compose a report in a single buffer instead of a `String` per line, and each
  report derives the location's clock once instead of per rendered field. The `--help` epilogue was
  compacted to 198 lines (from 200) with the same content. Nothing about the CLI, the output
  formats or the JSON schema changes; the enforced budgets and the measurement harness are recorded
  in `docs/performance.md`, and `cargo build --release --no-default-features` stays the supported
  reduced build (city search through the network geocoder, 3.41 MiB smaller).

* **The Open-Meteo credit line now carries the licence link**, as CC BY 4.0 asks:
  `Data: Open-Meteo.com (CC BY 4.0) — https://open-meteo.com/` (the `Data:` label stays localized).
  The binding attribution clause was amended to the exact rendered line.
* **Four JSON fields may now be `null` where they previously carried a fabricated value**:
  `current.wind_dir_deg` (a variable or calm wind has no direction), `current.humidity_pct` and
  `current.cloud_cover_pct` (a `-999` sentinel, or a field the backend does not report, is
  "not reported" — never a `0`), and the six `air.pollen.*` members. Widening a value to nullable
  is additive within `schema_version` 2.
* **`location.source` no longer emits `config`** — the variant was never constructed, and the
  value is gone from `docs/schema.md` and the model.
* **`[render] width` accepts the same range as `--width`** (0 or 1..=500); the undocumented
  40-column floor is gone, and `--width 12` in the file behaves like the flag.
* **`--format dumb` is colourless by contract** and says so under `--verbose`; `--color always`
  under a `TERM` that advertises no colour now folds down to the sixteen ANSI colours instead of
  being emitted as 256, and is never silently dropped.

### Fixed

* **`gzip`-encoded responses are decoded instead of refused.** QWeather's v1 host began compressing
  every weather response unconditionally (observed 2026-10-06; it ignores `Accept-Encoding:
  identity`), and the shared HTTP client refused any non-identity `Content-Encoding` to keep the
  body cap meaningful — so a `-p qweather` run failed with `upstream: the response is
  `gzip`-encoded…` (exit 3) as soon as its cache went stale. `src/http.rs` now decodes `gzip` (and
  `x-gzip`, concatenated members included) **after** the capped read, with the same
  `MAX_BODY_BYTES` ceiling applied to the decoded size, so a hostile or broken upstream still
  cannot make the process allocate without bound (the concern of the 2026-10-05 review, §3.9);
  every other coding is still refused with the cap named. An error response's body is decoded
  best-effort so the upstream's own `detail` survives into the message, and a decode failure there
  never replaces the status error. `StubReply::bytes` scripts a binary body for tests.
* **The QWeather alert panel says why it is empty on an account without the product.** A host that
  does not serve `/weatheralert/v7/alert/now` answers `404` with an empty body, which read as
  `answered HTTP 404: ` with nothing after it; the note now names the Weather Alert subscription and
  the `[alerts] sources` escape. Best-effort as before: it never changes the exit code.

* **QWeather conditions reported the wrong weather**: `309` (drizzle) is drizzle rather than heavy
  rain, and `515` (extra heavy fog) is fog rather than light freezing drizzle; a drizzle no longer
  outranks the day's real weather in the day-part summary.
* **Decoding**: a `-999` sentinel in Pirate Weather's humidity, cloud cover or wind bearing nulls
  only that field instead of discarding the whole observation; SMHI's in-band `9999` does the
  same; a METAR `TEMPO`/`BECMG` trend block can no longer overwrite the observation; METAR
  `R06L/2000FT` and `VCSH` groups are decoded; an out-of-range Open-Meteo `weather_code` stays
  "unknown" instead of becoming "clear sky"; WeatherAPI's moderate/heavy shower codes, WWO's
  freezing fog and OWM's smoke/haze/dust/sand/squall codes map to their own conditions.
* **A variable wind is no longer a north wind**: the METAR `VRB` flag and an OpenWeatherMap
  observation without `wind.deg` render no direction (JSON `null`, speed kept).
* **An empty or whitespace-only location argument is "absent"**, so `[location] default` applies
  instead of a silent public-IP lookup (a privacy leak); `cirrocast "" --ip` is no longer a
  conflict.
* **Alert configuration**: `[alerts] sources` rejects `auto` mixed with explicit ids and the
  not-yet-wired `visualcrossing`, so a file that validates can no longer fail every run with
  exit 2; `--alerts-from` ids are checked before any location resolution, so a typo costs no
  request; a non-`auto` source list from the file is treated as the promise it is documented to be.
* **Alerts**: a CAP document whose `status` is `Test`, `Exercise` or `Draft` is dropped like its
  NWS `is_test` twin; a CAP aggregation link must be `https` on the querying host; the HKO warning
  summary is cached per language instead of serving Chinese to an English run; an `<expires>` in
  the ISO-8601 basic or fractional spelling parses instead of becoming "live forever"; and warning
  polygons that cross the antimeridian keep their points.
* **Caching**: a cache-write failure never discards a successful fetch (the METAR path included);
  `cache stat` reports the `ratelimit/` and `geo/` state; `cache clean --all` removes them and a
  crashed run's temporaries; concurrent writers to one path cannot unlink each other's temp file.
* **HTTP**: a request that carries a credential never follows a redirect, so the QWeather key and
  the MeteoAlarm token cannot be delivered to another authority, while a header-free request
  follows a bounded chain so FPAS's canonical `/alert/<id>` redirect keeps working; `--timeout`
  bounds the whole request, not only its phases; `Retry-After` is honoured only on `429`/`503`;
  a non-identity `Content-Encoding` and a body over the cap are typed upstream errors; and a
  SOCKS proxy from the environment is refused with the variable named instead of being ignored.
* **Geo and the city table**: a hostile or bit-rotted user table or update download is bounded and
  typed rather than an unbounded allocation; the IP locator's and Open-Meteo geocoder's coordinate
  answers are validated like Nominatim's; the Nominatim 1 req/s throttle holds under concurrency;
  `location update-data --from` rejects a non-http(s) scheme as a usage error; the `geo-table`
  builder's `--help` exits 0 and its download honours the configured proxy.
* **CLI**: multi-location `--pick` resolves the prompts in argument order, so the same command line
  selects the same places; the `selected: …` echo survives `-q`; `--station` prepends `metar` to a
  configured chain; `--template-file -` keeps a one-line template on one line; `-f <name>` follows
  a configured `@`-chain; `location search`/`location update-data` honour `CIRROCAST_TIMEOUT`;
  `--severity` with `--no-alerts` is refused like its siblings; `config set location.default`
  validates only the key being set.
* **Rendering**: the multi-location JSON document keeps each report's key order; `-0.0`
  coordinates normalise like every other float; the standalone air and moon panels fold to ASCII
  before the width applies; the observed line prints the location's own offset; a 1 MB template is
  validated in one pass instead of quadratically; the unknown-token report names the spelling the
  user typed; and an unknown alias that looks like out-of-range coordinates says which range check
  it failed.
* **Text and docs**: the message for a dangling `keys.toml` symlink reports itself instead of
  "No such file or directory"; the `--help` epilogue names the three undocumented `CIRROCAST_*`
  overrides; and the contract, README, `docs/providers.md`, `docs/schema.md` and the plan records
  were brought back in line with the shipped code.

## [1.2.0] - 2026-10-05

Phase D completed — the offline city database and the user-installed table updates (steps 18 and
18b), multi-location output with the shared `%`-token template engine (step 19) and the interactive
location picker (step 20), on the official `GeoNames` 2026-10-04 `cities15000` snapshot. The v1 CLI
contract is unchanged; the configuration document moves to `schema_version` 2 for the `[locations]`
and `[templates]` tables, and the JSON document stays at `schema_version` 2.

### Added

* **Interactive location candidates (step 20).** When a name resolves to several places — the ten
  hits a geocoder returns, or the offline table's near-matches — a terminal
  run now asks which one: the ranked list is printed on stderr (`[1]`…`[N]`, the ranked winner
  marked `*`, population where known) and one line is read from stdin — an index, Enter for the
  winner, `q`/`Q` or EOF to abort with exit 5 (`no location selected for <query>`); three invalid
  answers are a usage error (exit 2). New `--pick` (force the prompt — the scriptable way in) and
  `--yes` (take the ranked winner without asking) are a clap conflict group, `[location] pick =
  "auto" | "never"` and `CIRROCAST_LOCATION_PICK` set the default, and anything non-interactive —
  pipes, cron, CI — is unchanged: the ranked winner on stdout. A selection is echoed as
  `selected: Beijing, Beijing Municipality, China — use @39.9042,116.4074 to skip the prompt`, so
  the choice can be pinned without re-ranking a name. The ambiguous-name note now ends with the
  picker hints (`--pick` to choose one, or `--yes` to keep the winner).
* **Multi-location runs (step 19).** `cirrocast Beijing Shanghai Tokyo` fetches the arguments at
  most four at a time and prints them in **argument order**, whatever order the network answers in
  (results are written into per-argument slots, so a slow request cannot reorder the output). A
  location that fails keeps its place: `error: <query>: <message>` on stdout, the full error on
  stderr, every other location still rendered, and the process exits with the numerically largest
  mapped code among the failures (a missing key 6 outranks a location miss 5). Above one location
  `json` becomes an array, where a failed slot is
  `{"schema_version": 2, "query": …, "error": {"code": …, "message": …}}`; `art-table` (and `dumb`)
  draw a combined 2–4 location summary (one header line and one aligned grid row per location) and
  fall back to the full tables above four with a one-time note. `--lat/--lon`, `--ip` and
  `--station` describe one place and are refused with several.
* **The `%`-token template engine** (`src/template.rs`) is now the single implementation behind
  `one-line`, its presets, the `status` probe (step 22) and the wttr.in compatibility surface
  (B01). New tokens: `%x` (condition art in plain 7-bit text), `%H`/`%L` (today's high/low), `%e`
  (dew point, computed from the reported temperature and humidity) and `%T` (local time). Tokens
  accept `%[-][0][<width>][.<prec>]X` — padding, zero-padding for numeric tokens, text truncation
  and numeric rounding — and `%{c}` writes a token next to text that would glue onto its letter.
  `--template-file <PATH|->` reads the template from a file or stdin, `--format full|minimal` (and
  any `[templates]` key) selects a one-line preset by name, and `--help` enumerates the exported
  `TOKENS` table.
* `[locations]` aliases (`home = "@39.9,116.4"` → `cirrocast @home`) and `[templates]` named
  templates (`compact = "%c%t"` → `-f compact`). Aliases chain, are cycle-checked at load with the
  chain named, and an unknown name lists up to three suggestions within edit distance 2.
  `@39.9,116.4` stays a coordinate pair whenever both sides parse and are in range; anything else
  after `@` is an alias name.

* **Offline city database (step 18).** A `GeoNames` `cities15000` snapshot (CC BY 4.0, dump date and
  input checksum in `src/geo/data/SNAPSHOT`) is embedded in the binary — about 3.3 MiB compressed,
  decoded lazily on the first name lookup and never written — so a plain city name resolves to
  coordinates, a time zone and a country code with no network at all. Folding is NFKD-based, so
  `São Paulo`/`Sao Paulo`, `MÜNCHEN`/`munchen`, `北京`/`Beijing`/`Peking` and `Wien`/`Vienna` all
  reach their city, and the offline and network paths rank through one shared function. New:
  `--offline[=<weather|geo|all>]` (`weather` = cache-only forecast with live geocoding, `geo` =
  bundled names with live weather, bare/`all` = no socket at all) and its default
  `[network] offline`; `[geo] strategy` = `auto` (bundled table first, network on a miss — the new
  default), `bundled` or `network`; `location search --all` prints the ranked candidate table and
  `--exact` is the flag spelling of `:query`. Offline-resolved places print
  `Location data by GeoNames (CC BY 4.0) — https://www.geonames.org/`, and a build with
  `--no-default-features` drops the table (`offline-geo` is a default feature) and falls back to
  the network geocoder.
* **User-installed city tables (step 18b).** `cirrocast location update-data` builds a city table
  from a `GeoNames` `cities15000` dump — the official ZIP, a mirror through `[geo] update_url`, or a
  local `.txt`/`.zip` with `--from` — and installs it under `$XDG_DATA_HOME/cirrocast/geo/`, where
  name resolution prefers it: `[geo] data = "auto"` (the default) uses the user table when one is
  installed and valid, `bundled` ignores it, and `user` requires it. `--check` reports whether a
  source would change anything (exit 1 when it would) without writing. The bundled table stays the
  default and the fallback, and a corrupt user table warns once and falls back. Nothing in a query
  ever fetches city data: `[geo] update = "check"` only prints a once-a-day note when the answering
  table is older than `update_interval_days` (90 by default), and automatic refresh is the user's
  own timer running the same idempotent command (systemd and cron examples ship in the README).
* `[geo] data`, `[geo] update`, `[geo] update_interval_days` and `[geo] update_url` configuration
  keys with `config get`/`set` support.
* `cirrocast`'s workspace gained the dev-only `build/geo-table` builder that produces
  `src/geo/data/*.bin.gz` from a `cities15000.txt` (`cargo run -p geo-table -- <file> src/geo/data`);
  its output is byte-for-byte deterministic and it runs the same fetch/extract/build code as
  `location update-data` (a `.txt`/`.zip` path or URL); `cargo run -p geo-table -- <source> --check`
  reports whether the committed snapshot still matches a dump without writing anything (exit 1 when
  it differs), and the test suite pins rows of the committed data so a refresh cannot pass unnoticed.

### Changed

* **Breaking: `%L` is today's low temperature**, matching wttr.in's documented `H`/`L` pair. It
  used to print the location's coordinates; those remain in `art-table`'s header and in `plain`'s
  `location:` record. `docs/formats.md` documents the token contract.
* **Breaking: an unknown `%X` in `--template`, `--template-file` or a `[templates]` preset is a
  usage error** (exit 2, naming the position and the known tokens) instead of printing literally
  with a `-v` note. The wttr.in compatibility surface keeps the literal passthrough.
* **Breaking: the positional location is variadic.** Scripts passing a single location plus a stray
  argument were already wrong and now get a usage error instead of silently ignoring the extra
  argument.
* `[templates]` keys and the `full`/`minimal` (and `default`/`short`/`uv`/`sun`) presets are valid
  `defaults.format` values as well as `--format` names.
* The configuration document moves to `schema_version = 2` for the `[locations]` and `[templates]`
  tables; a version 1 file is migrated by stamping the version (both tables are optional). A
  malformed alias table is a config error at load, naming the entry.
* The 7-bit condition art table gained `%x` as its charset-independent spelling; `%c` and `%x`
  render identically today and may diverge when a Unicode condition glyph is added.
* A missing user-installed city table no longer prints a warning on every run: `[geo] data =
  "auto"` falls back to the bundled table silently when nothing is installed (the documented
  "user table when present"), and only an installed-but-unusable table warns — once per process,
  so a multi-location run cannot repeat it. `data = "user"` still fails loudly with the fix.

* A plain location name (`Beijing`, `:Beijing`) is now resolved by the bundled city table first and
  only falls back to the Open-Meteo geocoding API when the table has no hit. The table carries the
  ISO country code rather than the country name the geocoder reports, and no admin-1 division, so a
  default run's location line reads `Beijing, CN (…)` instead of `Beijing, Beijing Municipality,
  China (…)`; `[geo] strategy = "network"` restores the previous path.
* `--offline` is no longer a boolean: bare `--offline` still means "no socket at all" (now spelled
  `--offline=all`), and the cache-only-weather behaviour of previous releases is `--offline=weather`.
  `--offline=geo` is new. The empty-cache message now reads `offline: no cached <provider> forecast
  for <place> at <key path>`, followed by the rerun hint.

## [1.1.0] - 2026-10-04

Phase D's first three steps — severe-weather alerts (15), air quality (16) and moon/astro (17) —
on top of the 2026-10-02 review fixes
([`docs/reviews/02-review-01-fixes-2026-10-02.md`](docs/reviews/02-review-01-fixes-2026-10-02.md)).
The v1 CLI contract is unchanged; the JSON document moves to `schema_version` 2 because the
`alerts` array arrived with the alerts work (see below), and the config document stays at
`schema_version` 1 — the `[alerts]` and `[air]` tables are additive.

### Added

* **Severe-weather alerts (step 15).** `cirrocast` now fetches official warnings by default —
  `--no-alerts` opts out, `--alerts` forces them, `--alerts-from nws,meteoalarm,…` names the sources —
  and renders them as a severity-coloured banner above `art-table`/`one-line`, as `alert:` records in
  `plain`, as the new `--format alerts` listing and as the `alerts` array in `json`. Sources are
  selected by coverage: NWS (US and territories), MeteoAlarm (EUMETNET members, optional
  `CIRROCAST_METEOALARM_KEY`), HKO (Hong Kong), QWeather (China, on its provider's chain) and the two
  global aggregators WMO SWIC and FPAS (self-hostable through `[alerts] fpas_url`). Warnings are
  normalised to CAP 1.2, expired ones are dropped (`ends`, else `expires`), duplicates across sources
  are collapsed, and strongest comes first. The threshold is `[alerts] severity_threshold`/`--severity`
  (default `minor`); the cache namespace `alerts/` has a 300 s TTL and `--offline` replays the last
  set. One-line's `%A` expands to the strongest alert's event (empty when none).
* JSON output: `schema_version` is now **2**, adding the `alerts` array and `alert_credits`
  (`docs/schema.md`); version 1 documents still parse. `provider info` gained the alert row
  (`provider info qweather` → `alerts: qweather`, `provider info smhi` → `alerts: none`).
* `[alerts]` configuration table (`enabled`, `severity_threshold`, `sources`, `fpas_url`,
  `cache_ttl_secs`) with `config get`/`set` support.
* **Air quality (step 16).** `--aqi` appends an air-quality panel to `art-table` and `plain` — the
  US and European AQI, the six regulated pollutants in μg/m³ and, inside the CAMS European domain,
  the six pollen species in grains/m³ — `--format aqi` prints it standalone, `one-line` gains the
  `%q` token, and `json` carries an `air` object (additive within `schema_version` 2). The reading
  is one keyless request to Open-Meteo's Air Quality API for the location the run already resolved,
  cached under `weather/open-meteo-air-<lat>-<lon>-<local-date>.json` with `cache.weather_ttl_secs`
  and the usual `--no-cache`/`--refresh`/`--offline` semantics. Categories are computed locally from
  the published breakpoints; `--aqi-index` (`[air] index`, default `us`) picks the scale that drives
  the category colour and `%q`; `--units` leaves the pollutant values alone by design. The fetch is
  best-effort: a failure prints `warning: air quality unavailable: …` on stderr and never changes
  the run's exit code, and a location outside the pollen domain says `not covered at this location`
  instead of inventing a zero.
* **Moon phase and astronomy (step 17).** `--moon` appends a locally computed moon/sun block to
  `art-table` and `plain`, `--format moon` prints the standalone view, `one-line` gains `%m` (the
  phase's art glyph) and `%M` (the phase's name), and `json` carries an `astro` object (additive
  within `schema_version` 2). Nothing is fetched: the phase, the geocentric illuminated fraction,
  the age, moonrise/moonset, the next four phase instants and — when the backend sends no sun
  times — sunrise/sunset/daylight are computed from the truncated Meeus series (ELP-2000/82 and
  solar, ΔT from the Espenak–Meeus fits). The sun block prefers the provider's own times and records
  where they came from in `astro.sun.source`; inside the polar circles the state is named
  (`polar day`/`polar night`) instead of clamping to `00:00`, and an event a day does not have
  prints `—`. `%m`/`%M` are always available; `--moon` is a usage error for the formats that have
  no astro surface (`one-line`, `alerts`, `aqi`).

### Changed

* `network.proxy` accepts only `http://` and `https://` URLs. A SOCKS URL is refused by the config
  validator, naming the key, instead of reaching `ureq` — which is built without a SOCKS connector and
  panicked on a hand-written setting.
* `providers.qweather.host` must be the account's HTTPS host, `https://<account-id>.re.qweatherapi.com`.
  A legacy shared host or a plain-`http://` value now fails validation on every run: the legacy hosts
  answer `403 Invalid Host`, and cleartext would leak the key.
* `config show` prints the values `config get` reports, `CIRROCAST_*` overrides included, and exits 4
  when an override is invalid; it previously ignored the environment.
* `config edit` rejects unknown keys like `config validate`; `config set` validates only the key it
  writes, so an unrelated invalid value no longer blocks it.
* `config init`, and `config edit` on a missing file, seed the new file with the *effective*
  configuration when a system document is shadowed (that one case writes canonical TOML without the
  commented template; with no other source the template is unchanged).
* A whitespace-only `CIRROCAST_*` override counts as unset, like a whitespace-only config value.
* `--lang` accepts POSIX spellings (`zh_CN.UTF-8` → `zh-CN`), and `en-*`/`zh-*` tags resolve through
  their family chain (`en-GB → en-US`) without the fallback warning.
* `art-table`: the `dumb`/ASCII arrows are `,` (SW) and `` ` `` (NW) — the previous keypad digits read
  as part of the speed — and the arrow sector follows the 16-point compass, so arrow and label always
  turn together.
* `art-table` at 20–36 columns: the stacked ladder's rungs keep the precipitation and wind fields
  instead of silently dropping them (at ≥37 columns the output is unchanged).
* `-f dumb` is always plain, escapes included: `--color always` no longer paints the ASCII table.
* `-f one-line`: `%w` prints the speed alone when the direction is absent, instead of `n/a`.
* `-f json`: `-0.0` is written as `0.0`, like every other display path.
* `-v`: the missing-key dump runs after the forecast and reports each missing key once per run; `-vv`
  request logs redact secrets in their percent-encoded spelling too.
* Messages: an unknown location no longer promises a candidate list it does not print, an unknown
  station points at `@lat,lon`, `--lat/--lon` report the command-line source, and the help epilogue
  spells the precedence as `LOCATION CIRROCAST_LOCATION`.

### Fixed

* A reading that is `NaN` or `inf` is refused with `Error::Upstream` in `Provider::fetch` — the one
  path every backend's answer takes — before it can be cached or rendered.
* QWeather: precipitation probability is read as the percent upstream sends (a `40` came out as `100%`
  through the fraction helper), and code 515 maps to WMO 56 (freezing drizzle), not a fog variant.
* Open-Meteo: an absent or truncated `precipitation_probability` array means "no probability", not an
  upstream error.
* Pirate Weather: `-999` sentinels in humidity, cloud cover and probability are missing values, not
  readings.
* WorldWeatherOnline: the `{"data":{"error":[…]}}` envelope is an upstream error carrying the message,
  not a decode failure.
* METAR: an unmapped obscuration falls back to the sky condition, `IC` maps to WMO 79 (ice pellets,
  the nearest described family) instead of failing, and conversions keep the exact value rather than a
  pre-rounded one.
* A value just below a `.5` tie no longer rounds to the wrong side (`fmt_int` on large readings,
  `fmt_small` on a negative reading).
* An extreme timestamp in an upstream payload produces a typed error instead of overflowing.
* A failed cache write logs at `-vv` and still serves the fetched answer.
* Nominatim requests are sent once, without retry, so its 1 req/s policy is never breached by backoff.
* A non-absolute `XDG_CONFIG_DIRS` entry is ignored instead of read.
* A `days` array whose parts are not exactly `[Morning, Noon, Evening, Night]` is rejected at
  deserialisation, so no renderer can show a part under another part's label.
* `defaults.format` accepts `moon`: the validator's list stopped at `aqi` and the template comment
  at `dumb`, so `config set defaults.format moon` failed and a hand-written `format = "moon"` was
  refused on every run although `-f moon` and `CIRROCAST_FORMAT=moon` worked. The three lists are
  now pinned to each other by a test.

## [1.0.0] - 2026-10-02

The v1 acceptance release: the whole A–C surface proved by execution in step 14 — the eight
backends against all five formats, the side-by-side against `wttr.in`, the four failure exit codes,
the packaged install — and `1.0.0` freezes the CLI, the JSON schema v1 and the config schema v1 for
the v1 scope. No new surface; the acceptance run produced three fixes:

### Changed

* `art-table` and its `dumb` twin: the day-cell tail pairs the precipitation amount with the
  precipitation *probability* (`0.0mm 20%`), matching the `plain` document's `0.0mm (0%)` and
  wttr.in's `0.0 mm | 0%`; humidity stays in the current-conditions block above the table (step 07).
* `--help` and the man page describe `-v`/`-vv` as they really behave: `-v` prints the upstream
  request behind the answer, `-vv` adds every HTTP attempt and the cache decisions (step 12).

### Fixed

* `--ip` with both location services failing names every attempt
  (`all IP location services failed: ipwho.is (…); ipapi.co (…)`) instead of only the last service
  (step 05).

## [0.1.0] - 2026-10-01

The first release: the whole v1 surface, shipped as `0.x` while phases A–C of `docs/plans/`
landed. `1.0.0` freezes it.

### Added

* Weather backends behind `--provider` / `CIRROCAST_PROVIDER` / `[defaults].provider`: the keyless
  `open-meteo` (default), `smhi` (Nordics and adjacent seas) and `metar` (station observations,
  selected by `--station` or `@lat,lon`); the BYOK backends `openweathermap`, `weatherapi`,
  `worldweatheronline`, `pirateweather` and `qweather`; `auto` expands to the keyless chain, and a
  chain falls through to the next entry only on a transport or upstream failure. `provider list`
  and `provider info <ID>` print the registry rows.
* Location resolution in four spellings, all documented with the winning place echoed on stderr:
  fuzzy geocoding (`Beijing`), exact-name (`:Beijing`), OpenStreetMap (`~Tsinghua`, one request per
  second, cached 30 days, base URL swappable through `network.nominatim_url`), coordinates
  (`@39.9,116.4`, `--lat`/`--lon`) and a METAR station (`--station`); the public-IP lookup
  (`--ip`, or implicit when nothing is configured) uses ipwho.is with an ipapi.co fallback and
  caches its answer for at most 24 hours. `location search` resolves a name without printing
  weather.
* Output formats behind `-f art-table|one-line|plain|json|dumb` (`art-table` is the default; `dumb`
  is the ASCII art table and engages automatically for `TERM=dumb` or a non-UTF-8 locale);
  `--template` takes a `%`-token string or a preset (`@default`, `@short`, `@full`, `@uv`,
  `@sun`); `plain` and `json` never truncate a record; `-f json` emits the stable document
  documented in [`docs/schema.md`](docs/schema.md) with `schema_version: 1`.
* Unit systems `-u metric|us|uk` with per-quantity overrides under `[units]`; every value is stored
  and cached canonically metric, so switching units never refetches.
* Output languages: `--lang`, `CIRROCAST_LANG` and `defaults.language` with the embedded `en-US`
  and `zh-CN` catalogs, `auto` following the ambient locale, and a documented fallback chain that
  warns instead of failing.
* Configuration and state under the XDG directories: `config init|show|get|set|edit|validate`,
  `cache stat|clean`, and BYOK keys in `keys.toml` (`key set|rm|list`, mode 0600, environment
  variable first) so secrets never enter `config.toml`, argv or any log.
* Cache control with `--no-cache`, `--refresh` and `--offline` (cache-only, never touches the
  network) over the weather, geocoding, IP and station namespaces.
* Terminal integration: `completion bash|zsh|fish|elvish|powershell` and `man` generate the shell
  completion and the manual page from the binary; colour honours `NO_COLOR`, `CLICOLOR_FORCE` and
  `--color auto|always|never` with a 16-colour fold when the terminal advertises no more, and
  `--width`/`COLUMNS` drive a stacked layout below 60 columns.
* Attribution is part of the output: every format carries the credits its data licences require
  (`plain` and `json` in the document, `art-table` in the footer, `one-line` on stderr).
* Exit codes `0`–`6` documented in `--help` and the README, with `error: …` on stderr and the
  cause chain under `-v`.

[Unreleased]: https://github.com/YangtseSu/cirrocast/compare/v1.3.0...HEAD
[1.3.0]: https://github.com/YangtseSu/cirrocast/releases/tag/v1.3.0
[1.2.0]: https://github.com/YangtseSu/cirrocast/releases/tag/v1.2.0
[1.1.0]: https://github.com/YangtseSu/cirrocast/releases/tag/v1.1.0
[1.0.0]: https://github.com/YangtseSu/cirrocast/releases/tag/v1.0.0
[0.1.0]: https://github.com/YangtseSu/cirrocast/releases/tag/v0.1.0
