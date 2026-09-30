<!--
SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Step 06 — open-meteo-provider

Status: ✅ done
Depends on: `03-canonical-model-and-units.md` (canonical model, units, `DayPartKind`, `severity_rank`, `resolve_local`), `04-geocoding-and-location-syntax.md` (location resolution), `05-http-cache-and-ip-location.md` (`HttpClient`, `Cache`, `KeyStore` consumers, IP location)
Touches: `src/provider/mod.rs`, `src/provider/open_meteo.rs`, `src/render/mod.rs`, `src/render/plain.rs`, `src/cli.rs`, `src/main.rs`, `tests/provider_open_meteo.rs`, `tests/render_plain.rs`, `tests/live.rs`, `tests/fixtures/open_meteo/`, `REUSE.toml`, `LICENSES/`, root `README.md`, `docs/plans/README.md`

## Goal

`cirrocast Beijing --format plain` finally prints weather. A resolved `Location` goes through an
ordered provider chain (today: one keyless Open-Meteo implementation) that returns a canonical
`Report`, day parts are aggregated from hourly data in the location's own timezone, and the plain
renderer prints a location header, one current-conditions line and four day-part lines per day with
no colour and no art — the first end-to-end run of the tool, and the contract every later provider
and renderer extends.

## Deliverables

- ✅ `src/provider/mod.rs`: `pub struct ProviderId(&'static str)` with `as_str()`, `FromStr`, `ALL`; `Capabilities { current, hourly, daily, alerts: bool, max_days: u8, requires_key: bool, key_env: Option<&'static str>, location_kinds: LocationKinds }` and `pub struct LocationKinds { pub city: bool, pub station: bool, pub lat_lon: bool }`, exactly as the contract's shape.
- ✅ `pub trait Provider { fn id(&self) -> ProviderId; fn capabilities(&self) -> Capabilities; fn fetch(&self, loc: &Location, req: &FetchRequest, env: &Env<'_>) -> Result<Report>; }` — synchronous, `&self`, no interior mutability; `pub struct FetchRequest { pub days: u8, pub hourly_resolution: HourlyResolution }` with `pub enum HourlyResolution { Hourly, ThreeHourly, Daily }`; `pub struct Env<'a> { pub http: &'a HttpClient, pub cache: &'a Cache, pub config: &'a Config, pub keys: &'a KeyStore, pub quiet: bool, pub verbose: u8 }` (the contract lists HTTP + cache + config; `keys` is added because every BYOK provider in step 10 must read its own credential, and the chain-level gate below only decides *whether* a key is needed; `quiet`/`verbose` carry the run's output flags, because the chain's fallback warning and a provider's clamp warning both have to respect `-q`).
- ✅ `ProviderMeta` (already the registry table from step 01) gains `implemented: bool`, `alerts: bool` and `licence: Option<&'static str>`, and `ProviderMeta::capabilities()` projects a row into the contract's `Capabilities` — so `provider list`/`provider info` and the chain can never disagree about what a backend offers, and step 10 flips a row's `implemented` flag when its module lands. An unimplemented row keeps `licence: None`: a credit line for a backend that does not exist yet would be an unverified claim (asserted by a unit test).
- ✅ `pub fn select(spec: &str) -> Result<Vec<ProviderId>>`: `--provider a,b,c` is an explicit ordered chain (unknown id ⇒ `Error::Usage` listing `provider list` ids; a known but not-yet-implemented id in step 10+ ⇒ `Error::Usage("provider `smhi` is not implemented yet")`); `auto` expands to the implemented keyless chain (`open-meteo` here, `open-meteo,smhi` from step 10, `+metar` when a station is configured); the bare default comes from `defaults.provider`.
- ✅ `pub fn fetch_chain(ids: &[ProviderId], loc: &Location, req: &FetchRequest, env: &Env<'_>) -> Result<Report>`: for each id, gate on `capabilities().requires_key && env.keys.get(id.as_str())?.is_none()` ⇒ `Error::MissingKey("provider `openweathermap` needs an API key: set CIRROCAST_OPENWEATHERMAP_KEY or run `cirrocast key set openweathermap`")`; then `fetch`; fall through to the next entry **only** for `Error::Network|Error::Upstream` (a usage, location, config, missing-key or decode error stops the chain immediately), printing `-v` lines per attempt and one stderr warning `warning: open-meteo failed (upstream: …); falling back to smhi` unless `-q`. An empty chain (nothing implemented) is `Error::Config`.
- ✅ `src/provider/open_meteo.rs`: `pub struct OpenMeteo` implementing `Provider`, `Capabilities { current: true, hourly: true, daily: true, alerts: false, max_days: 16, requires_key: false, key_env: None, location_kinds: LocationKinds { city: true, station: false, lat_lon: true } }`, `const BASE: &str = "https://api.open-meteo.com/v1/forecast"`.
- ✅ Request assembly, exactly: `latitude`, `longitude`, `current=temperature_2m,relative_humidity_2m,apparent_temperature,is_day,precipitation,weather_code,cloud_cover,pressure_msl,surface_pressure,wind_speed_10m,wind_direction_10m,wind_gusts_10m,visibility`, `hourly=temperature_2m,apparent_temperature,precipitation_probability,precipitation,weather_code,wind_speed_10m,wind_direction_10m,relative_humidity_2m,visibility`, `daily=weather_code,temperature_2m_max,temperature_2m_min,sunrise,sunset`, `timezone=auto`, `forecast_days=<1..=16>`, `temperature_unit=celsius`, `wind_speed_unit=kmh`, `precipitation_unit=mm`. Metric is requested explicitly on every call (never inherited from a default) so a unit-less cache entry can never appear; `daily` is only requested when `days > 0`, because the endpoint requires `timezone` for daily variables and we want `days = 0` to stay a two-parameter current-only call.
- ✅ Response types: `ForecastResponse { latitude, longitude, elevation: Option<f64>, utc_offset_seconds: i32, timezone: String, timezone_abbreviation: String, current: Option<CurrentBlock>, hourly: Option<HourlyBlock>, daily: Option<DailyBlock> }` with every variable array typed `Vec<Option<f32>>` (the API can return `null` for `visibility` and `precipitation_probability` depending on the model) and `time: Vec<String>` in local `YYYY-MM-DDTHH:MM` wall-clock. `HourSample { at: DateTime<Tz>, temp_c, feels_like_c, precip_mm, precip_prob_pct: Option<u8>, weather: Condition, wind_kmh, wind_dir_deg: Option<u16>, humidity_pct: Option<u8>, visibility_km: Option<f32> }`.
- ✅ Mapping rules: `time` strings are resolved with step 03's `resolve_local(tz, naive)` (so a DST gap/ambiguity has one documented answer); `visibility` is **metres** upstream and is divided by `1000` into `visibility_km`; `is_day` (`0|1`) becomes `bool`; `pressure_msl` fills `Current.pressure_hpa`; `current.time` becomes `observed_at` (`DateTime<FixedOffset>`); `weather_code` becomes `Condition::from_u8(code)`; the response's `timezone` replaces a provisional `tz` when `loc.source` is `Coordinates` or `Osm`, and the corrected zone is what the header prints.
- ✅ `Days`/`Report`: `days == 0` ⇒ current only, `days: vec![]`; otherwise `forecast_days = days.clamp(1, max_days)` with a once-per-run stderr warning when clamped. `Report.location` carries the (possibly corrected) location, `Report.days` is ordered oldest → newest starting at the location-local today, and `Attribution { provider: "open-meteo".into(), url: <the full request URL>, fetched_at: Utc::now() via env.cache.clock(), raw: verbose.then(|| …) }` keeps the raw local-time/weather-code pairs only under `-v`.
- ✅ `pub fn aggregate_day(hours: &[HourSample], daily: &DailyBlock, date: NaiveDate, tz: Tz) -> Result<DayForecast>` — pure, network-free, unit-tested. **Written rules:** parts are cut in location-local time as `Morning 06:00–11:59`, `Noon 12:00–17:59`, `Evening 18:00–23:59`, `Night 00:00–05:59` **of the same local date** (the night row is the small hours of that day, not the following night); within a part, `temp_c`, `feels_like_c`, `humidity_pct`, `wind_kmh`, `wind_dir_deg` and `visibility_km` come from the single hourly sample **closest to the part midpoint** (09:00, 15:00, 21:00, 03:00; a tie takes the earlier hour); `precip_mm` is the part **sum**, `precip_prob_pct` the part **maximum**; `condition` is the highest `severity_rank()` code present in the part (step 03's total order), ties broken by the higher frequency among the tied codes and then by the earlier hour, and a part whose codes are all unknown falls back to the representative sample's code; `temp_min_c`/`temp_max_c` come from the `daily` arrays (authoritative for the local calendar day); `sunrise`/`sunset` are parsed as local times and carry the location offset, and are `None` when upstream returns `null` **or** answers the same instant for both — the polar-night placeholder the recording actually returned is `00:00`/`00:00`, not `null`; a part with **zero** hourly samples is `Error::Upstream("no hourly data for 2026-10-05 night (Europe/Berlin)")` while a part with fewer than six is aggregated normally.
- ✅ Caching: the response body is fetched through `env.cache.read_or_fetch_json::<ForecastResponse>` under `CacheKey::weather("open-meteo", lat, lon, days, local_today)`, TTL `cache.weather_ttl_secs` (600 s default), where `local_today` comes from `env.cache.clock()` plus the location tz — a day rollover therefore invalidates yesterday's entry by key, not by TTL luck.
- ✅ `src/render/mod.rs`: introduces only what a working `--format plain` needs, and step 07 **extends** it rather than redefining it — `pub trait Renderer { fn render(&self, report: &Report, ctx: &RenderContext<'_>) -> Result<String>; }`; `pub struct RenderContext { pub units: ResolvedUnits, pub color: ColorMode, pub width: usize, pub term: TermCaps, pub now: DateTime<FixedOffset>, pub tz: Tz }`; `pub enum ColorMode { Auto, Always, Never }`; `pub struct TermCaps { pub is_tty: bool, pub color: bool, pub dumb: bool }`; `pub enum Format { Plain }` (`clap::ValueEnum`) with `format.renderer() -> Result<Box<dyn Renderer>>`. Step 07 adds the contract's remaining context members (`lang`, `i18n`, the full `TermCaps`, `resolve_width`/`resolve_color`) and the remaining `Format` variants, and step 09 fills the catalogs behind `i18n`; nothing delivered by this step reads them, so no placeholder i18n type appears here.
- ✅ `src/render/plain.rs`: box-free, colour-free, pipe-friendly output that never exceeds `ctx.width`, built entirely from step 03's `format_*` functions, step 04's `location_line` and `DayPartKind::label()`; it prints the provider registry's `licence` line as `Data: …` and the location credit from `geo::attribution_line` whenever that returns one — the ODbL line for `~` results, the GeoNames line for a geocoded name; the contract makes the credit part of the output, so the step's original "only when the source is `Osm`" reading was narrowed to the one call that decides the text.
- ✅ The pipeline — in `src/cli.rs`, because `AGENTS.md` keeps `main.rs` to argv/dispatch/exit-code: parse argv → `Config::load`/`Settings::resolve` → `LocationSpec::parse_arg` → resolve (config `location.default` → geocoder → `--ip`) → `Cache::open` with the `CacheMode` from the flags → `UreqTransport`/`HttpClient` → `KeyStore` → `Env` → `select`/`fetch_chain` → timezone correction → renderer → stdout; every error path prints `error: …` to stderr and returns the contract's exit code, and no user-triggerable path panics.
- ✅ Minimal CLI wiring for this step (the full matrix stays in step 08): positional `[LOCATION]`, `-p/--provider`, `-d/--days <0..=14>`, `-f/--format plain`, plus the flags already owned by 02/05 (`--ip`, `--no-cache`, `--refresh`, `--offline`, `--timeout`, `-q`, `-v`). `-u/--units`, `--lang`, `--color`, `--width`, `--lat/--lon`, `--station` and the non-plain formats are step 08's.
- ✅ Tests in `tests/provider_open_meteo.rs`: the recorded request URL asserted from `StubTransport::calls()` against the exact parameter list above; `days = 0` producing a current-only request; the clamp path as a unit test of `requested_days` (the CLI caps `--days` at 14, below Open-Meteo's 16, so with this backend the warning cannot be reached through the command line; step 10's shorter-horizon backends will exercise it end to end); day-part aggregation on all four fixtures (midpoint sampling, sums, maxima, severity pick with a frequency tiebreak, night grouped into its own date); the DST-transition fixture proving the spring-forward day still groups into four parts without shifting them (upstream returns 24 local rows including a 02:00 that the gap policy moves forward — the 23-hour expectation was wrong for this API); min/max from the daily arrays and `sunrise`/`sunset` becoming `None` in the polar-winter fixture; timezone correction for a `Coordinates` location; chain behaviour (network/upstream falls through, usage does not, missing key ⇒ `Error::MissingKey`, `auto` expansion) with `#[cfg(test)] mod tests` stub providers inside `src/provider/mod.rs`.
- ✅ Tests in `tests/render_plain.rs`: the exact line set for a fixture-backed report in all three unit systems, no ANSI escape in the output, `--days 0` printing only the header and current line, and the ODbL attribution line for an `Osm` location.
- ✅ `tests/live.rs`: `#[ignore]`d `live_open_meteo_beijing` and `live_geocode_plus_forecast_at_coordinates` for manual runs; no test in the default suite reaches the network.
- ✅ Attribution and docs: the plain renderer prints the Open-Meteo licence line (its data is CC BY 4.0 and attribution is required), `README.md` gains a usage section with the sample output and the data-source/attribution note, and the recorded responses under `tests/fixtures/open_meteo/` are annotated in `REUSE.toml` with `CC-BY-4.0` plus a `LICENSES/CC-BY-4.0.txt` text file (narrowing step 01's blanket `tests/fixtures/**` rule so the fixtures are not silently relicensed).

## Design notes

* **Day-part aggregation lives in the provider module, not the renderer** (contract rule): the parts are
  a property of the data and its timezone, so every renderer prints the same four rows, and a future
  provider that only offers 3-hourly data aggregates into the same shape. The four rules above are written
  down because they are the only place where a defensible-looking alternative exists (night belonging to
  the previous evening, precipitation averaged instead of summed, "most frequent" instead of "most
  severe"); the fixtures pin the chosen reading.
* The severity comparison uses step 03's single rank table so the aggregation never has to know code
  families, and the frequency tiebreak keeps a one-hour thunderstorm from losing to six hours of drizzle
  only through rank ordering, which is what we want most of the time and what the tiebreak handles when
  ranks are equal.
* Metric is requested explicitly rather than relied upon: `temperature_unit`, `wind_speed_unit` and
  `precipitation_unit` are always in the query, so the cache can never hold a Fahrenheit response, and
  `visibility` is the one field that *has* to be converted because the API only offers metres.
* `Env` carries `quiet`/`verbose` and `keys` in addition to the contract's three members: the chain uses
  the key store to decide whether a provider can run at all, each provider needs the value, and both the
  chain's fallback warning and a provider's clamp warning are output the `-q`/`-v` flags own. Providers
  still never touch the filesystem directly — they go through `KeyStore`.
* The render context is deliberately introduced with fewer members than the contract's final shape: step 07
  is what completes `RenderContext` (adding `lang`, `i18n`, the full `TermCaps` and the width/colour
  resolution) because it is the step that first needs them, and step 09 then fills the catalogs behind
  `i18n`. Neither step has to carry a placeholder i18n type, and the additions are struct fields plus
  their construction in `cli.rs`'s query pipeline.
* No new dependencies: rendering is formatting of step 03 types, JSON parsing uses the step-05
  `serde_json`, and `clap::ValueEnum` comes from the existing `clap`.

## Out of scope

- The art table, `art.rs`, `color.rs`, the `dumb` format and ioctl width probing: `07-art-table-renderer.md`,
  which adds `Format::ArtTable`/`Format::OneLine`/`Format::Dumb` and the palette against the renderer trait
  frozen here.
- `--format json` and its stable schema, the full flag matrix (`-u`, `--lang`, `--color`, `--width`,
  `--lat/--lon`, `--station`), `provider list|info`, `completion` and `man`: `08-cli-surface-and-formats.md`.
  The stable JSON document is a public contract with its own versioning and is therefore explicitly **not**
  delivered here.
- Localised condition strings: the plain renderer prints `Condition::description_en()`; Fluent catalogs and
  `I18n` are `09-localization.md`.
- Every other provider (`smhi`, `metar`, the BYOK ones): `10-additional-providers.md` and
  `11-metar-and-aviation.md`, which only append registry rows and implement `Provider`.

## Verification

Fixtures: `tests/fixtures/open_meteo/forecast_beijing_2026-07-15.json` (hot and humid),
`…/forecast_longyearbyen_2026-01-12.json` (polar winter; sunrise/sunset come back as `00:00`, not
`null`), `…/forecast_berlin_2026-03-29.json` (DST spring-forward day; upstream returns 24 local rows,
including a 02:00 that does not exist in `Europe/Berlin`), `…/forecast_lisbon_2026-05-04.json`
(all-clear day) — recorded responses, `CC-BY-4.0`, annotated per path in `REUSE.toml`. The 400
envelope is step 05's `tests/fixtures/http/open_meteo_error_invalid_param.json`; a duplicate file
under `open_meteo/` would be the same bytes twice.

Provenance, stated exactly: direct TLS to `api.open-meteo.com`, `historical-forecast-api.open-meteo.com`
and `archive-api.open-meteo.com` times out from the recording network (the TLS client hello is sent,
then nothing — the same failure step 05 hit with `ipwho.is`), so each body was fetched through the
`r.jina.ai` text proxy with only its header block stripped, byte-for-byte otherwise unchanged. The four
dates are in the past, so the **Historical Forecast API** served them: same parameter names, same
response schema, same variables as the forecast endpoint the provider calls. Sizes: 5.9 kB each.

Manual smoke run — the fetch half could not be exercised from this network, so the run below is the
CLI over a cache entry seeded with the recorded Beijing response (no socket, `--offline`), which
exercises everything except the transport itself:

```sh
tmp=$(mktemp -d); export XDG_CONFIG_HOME=$tmp/config XDG_CACHE_HOME=$tmp/cache XDG_DATA_HOME=$tmp/data
# seeded: $XDG_CACHE_HOME/cirrocast/weather/open-meteo-39.90-116.41-3-2026-09-30.json (the fixture body)
COLUMNS=120 cargo run -q -- '@39.9042,116.4074' --format plain --offline
# 39.9042, 116.4074 Asia/Shanghai
# Now: 18°C (feels 12°C), Overcast, wind 19 km/h NW, humidity 14%, pressure 1021 hPa, visibility 17 km, 0.0 mm
# 2026-07-15  min 25°C  max 35°C  sunrise 04:58  sunset 19:42
#   Morning  29°C  Clear sky       precip 0.0 mm (0%)   wind 2.5 km/h N
#   Noon     35°C  Clear sky       precip 0.0 mm (0%)   wind 4.7 km/h SW
#   Evening  30°C  Overcast        precip 0.0 mm (0%)   wind 13 km/h SW
#   Night    26°C  Overcast        precip 0.0 mm (0%)   wind 6.0 km/h SW
# 2026-07-16 …                                          # two more days, same four-part shape
# Data: Open-Meteo.com (CC BY 4.0)                      # exit 0
cargo run -q -- '@39.9042,116.4074' --format plain --offline --days 0
# header, Now line, Data line only                      # exit 0
cargo run -q -- -p open-meteo,does-not-exist '@39.9042,116.4074' --format plain; echo $?
# error: unknown provider `does-not-exist`; known providers: …   (exit 2)
cargo run -q -- -p smhi '@39.9042,116.4074' --format plain; echo $?
# error: provider `smhi` is not implemented yet                 (exit 2)
```

The live path was attempted as well, and fails at this network rather than in the code: the geocoding
half resolves (the geocoding host is reachable), then the forecast request reports
`network error: GET https://api.open-meteo.com/v1/forecast?… failed after 3 attempts: timeout while
connecting` and exits 3. The first run on an unblocked network has to confirm the live body; the wire
contract, the decoding, the aggregation and the rendered shape are pinned by the recorded responses and
the seeded-cache run above.

## Exit criteria

- ✅ `cargo test` passes with the network unplugged; the only network-touching tests are `#[ignore]`d.
- ✅ The five aggregation rules are each covered by a named test in `tests/provider_open_meteo.rs`, and
      the fixture set covers hot/humid, polar winter, a DST-transition day and an all-clear day.
- ✅ `cargo fmt --check` clean.
- ✅ `cargo clippy --all-targets -- -D warnings` clean.
- ✅ `cargo test` clean (`tests/provider_open_meteo.rs`, `tests/render_plain.rs`, all unit tests).
- ✅ `reuse lint` clean, with the Open-Meteo fixtures annotated `CC-BY-4.0` and `LICENSES/CC-BY-4.0.txt`
      present.
- ✅ The smoke run reproduces the shown shape — location header, one `Now:` line, one date line with
      min/max/sunrise/sunset, four part lines per day, the attribution line, and the `--offline` rerun
      printing the same body from `weather/open-meteo-39.90-116.41-3-2026-09-30.json` — over a seeded
      cache entry, because the live fetch cannot reach `api.open-meteo.com` from this network (see
      Verification: the TLS handshake times out, exit code 3). The live body stays unverified until a
      run on an unblocked network; every other half of the path is covered by the recorded-response
      tests.

## Risks

- Some Open-Meteo models return `null` for `visibility` or `precipitation_probability` at a given hour;
  the arrays are `Vec<Option<f32>>` and the aggregation treats a missing representative value as `None`
  rather than as `0`, so the plain renderer omits the field instead of printing a fake zero.
- The grid cell Open-Meteo selects can be a few kilometres from the requested coordinate and its elevation
  differs; the header therefore echoes what the geocoder resolved, and the response's `lat`/`lon` are kept
  in the raw cache entry for debugging rather than being shown as the location.
- `forecast_days` is capped at 16 upstream and step 08 caps `--days` at 14 per the contract, so the clamp
  warning is unlikely to fire in practice; it exists for providers with a smaller horizon (step 10).
- Sunrise/sunset absence in polar regions is real behaviour, but Open-Meteo expresses it as
  `00:00`/`00:00` rather than `null` on the recorded days; both forms become `None`, and a renderer that
  assumed the fields exist would print `00:00`, which the fixture test prevents.
- `api.open-meteo.com` is unreachable from the development network used for this step (TLS handshake
  timeout; the geocoding host is reachable), so the live fetch, `--refresh` on a live body and the
  upstream's current schema were not observed here. The recorded fixtures, the request assertions and
  the seeded-cache smoke run cover everything else; a live run on an unblocked network is the first
  thing to do if a decoding error ever appears.
- The plain renderer clips at the resolved width, and step 06 resolves that width from `[render] width`
  → `COLUMNS` → 80 (no tty probing yet). A wide `Now:` line therefore loses its tail on a narrow
  terminal until step 07 adds `--width` and the terminal query.
- The `Attribution.raw` blob exists only under `-v`; if a future step wants it for the JSON renderer, it
  is already on the model, and step 08 decides whether to expose it.

## Progress log

- 2026-09-30 — plan written.
- 2026-09-30 — provider layer landed: `Capabilities`, `Provider`, `FetchRequest`, `HourlyResolution`,
  `Env`, `select`, `fetch_chain`/`fetch_chain_with`, `provider_for`, `licence_line`, and the registry
  extension (`ProviderMeta` gains `implemented`/`alerts`/`licence`, `capabilities()` projects a row).
  Four interface corrections against this file, all recorded in the deliverables above: the registry is
  the step-01 table extended, not a new `PROVIDERS` const; `Env` also carries `quiet`/`verbose`; the
  chain's key gate is `capabilities().requires_key` **before** the call rather than a `fetch` error; and
  `auto` filters on `location_kinds.city` so a station-only backend can never enter it.
- 2026-09-30 — Open-Meteo backend landed: request assembly in a fixed parameter order, `ForecastResponse`
  decoding with every array `Vec<Option<f32>>`, mapping rules (`visibility` metres → km, `is_day`, island
  zone correction for `Coordinates`/`Osm`), `aggregate_day` with the five written rules, and caching
  under `CacheKey::weather` keyed by the location-local date. A part whose hours all carry `null` in a
  required field is reported as `Error::Upstream` rather than filled with zeros.
- 2026-09-30 — render layer and pipeline landed: `Renderer`/`RenderContext`/`ColorMode`/`TermCaps`/
  `Format` (plain only) and `src/render/plain.rs`; `QueryArgs` on the top-level command line, the query
  branch in `Cli::run`, and `run_query` in `src/cli.rs` (not `main.rs`, which `AGENTS.md` keeps thin).
  `RenderContext.units` is a `ResolvedUnits` — the `[units]` per-quantity overrides must be resolved
  before the renderer sees them — and the context carries no lifetime yet, because the `i18n` member it
  would borrow arrives in step 09.
- 2026-09-30 — fixtures and tests landed: four recorded responses under `tests/fixtures/open_meteo/`
  (via the `r.jina.ai` text proxy and the Historical Forecast API; direct TLS to the forecast hosts is
  blocked here), `REUSE.toml` annotations `CC-BY-4.0`, `tests/provider_open_meteo.rs` (13 cases),
  `tests/render_plain.rs` (5), `tests/live.rs` (2 `#[ignore]`d), plus the shared provider harness in
  `tests/common/mod.rs` and four new CLI cases including a seeded-cache end-to-end run.
- 2026-09-30 — step done. Gates: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
  `cargo test` (all binaries green, 2 ignored) and `reuse lint` are clean; the smoke run above was
  executed and observed. The live fetch could not be exercised from this network (`timeout while
  connecting`, exit 3) — recorded under Verification and Risks, and the one thing left for a machine
  with access to `api.open-meteo.com`.
