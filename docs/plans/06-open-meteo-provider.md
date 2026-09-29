<!--
SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Step 06 — open-meteo-provider

Status: ⬜ not-started
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

- ⬜ `src/provider/mod.rs`: `pub struct ProviderId(&'static str)` with `as_str()`, `FromStr`, `ALL`; `Capabilities { current, hourly, daily, alerts: bool, max_days: u8, requires_key: bool, key_env: Option<&'static str>, location_kinds: LocationKinds }` and `pub struct LocationKinds { pub city: bool, pub station: bool, pub lat_lon: bool }`, exactly as the contract's shape.
- ⬜ `pub trait Provider { fn id(&self) -> ProviderId; fn capabilities(&self) -> Capabilities; fn fetch(&self, loc: &Location, req: &FetchRequest, env: &Env<'_>) -> Result<Report>; }` — synchronous, `&self`, no interior mutability; `pub struct FetchRequest { pub days: u8, pub hourly_resolution: HourlyResolution }` with `pub enum HourlyResolution { Hourly, ThreeHourly, Daily }`; `pub struct Env<'a> { pub http: &'a HttpClient, pub cache: &'a Cache, pub config: &'a Config, pub keys: &'a KeyStore }` (the contract lists HTTP + cache + config; `keys` is added because every BYOK provider in step 10 must read its own credential, and the chain-level gate below only decides *whether* a key is needed).
- ⬜ `pub struct ProviderMeta { id: ProviderId, display_name: &'static str, implemented: bool, licence: &'static str, homepage: &'static str }` and `pub const PROVIDERS: &[ProviderMeta]` seeded with `open-meteo` (`implemented: true`, `licence: "CC-BY-4.0 — data by Open-Meteo.com"`, homepage `https://open-meteo.com`) plus placeholder-free rows are **not** added for unimplemented providers; step 10 appends its rows when its implementations exist.
- ⬜ `pub fn select(spec: &str) -> Result<Vec<ProviderId>>`: `--provider a,b,c` is an explicit ordered chain (unknown id ⇒ `Error::Usage` listing `provider list` ids; a known but not-yet-implemented id in step 10+ ⇒ `Error::Usage("provider `smhi` is not implemented yet")`); `auto` expands to the implemented keyless chain (`open-meteo` here, `open-meteo,smhi` from step 10, `+metar` when a station is configured); the bare default comes from `defaults.provider`.
- ⬜ `pub fn fetch_chain(ids: &[ProviderId], loc: &Location, req: &FetchRequest, env: &Env<'_>) -> Result<Report>`: for each id, gate on `capabilities().requires_key && env.keys.get(id.as_str())?.is_none()` ⇒ `Error::MissingKey("provider `openweathermap` needs an API key: set CIRROCAST_OPENWEATHERMAP_KEY or run `cirrocast key set openweathermap`")`; then `fetch`; fall through to the next entry **only** for `Error::Network|Error::Upstream` (a usage, location, config, missing-key or decode error stops the chain immediately), printing `-v` lines per attempt and one stderr warning `warning: open-meteo failed (upstream: …); falling back to smhi` unless `-q`. An empty chain (nothing implemented) is `Error::Config`.
- ⬜ `src/provider/open_meteo.rs`: `pub struct OpenMeteo` implementing `Provider`, `Capabilities { current: true, hourly: true, daily: true, alerts: false, max_days: 16, requires_key: false, key_env: None, location_kinds: LocationKinds { city: true, station: false, lat_lon: true } }`, `const BASE: &str = "https://api.open-meteo.com/v1/forecast"`.
- ⬜ Request assembly, exactly: `latitude`, `longitude`, `current=temperature_2m,relative_humidity_2m,apparent_temperature,is_day,precipitation,weather_code,cloud_cover,pressure_msl,surface_pressure,wind_speed_10m,wind_direction_10m,wind_gusts_10m,visibility`, `hourly=temperature_2m,apparent_temperature,precipitation_probability,precipitation,weather_code,wind_speed_10m,wind_direction_10m,relative_humidity_2m,visibility`, `daily=weather_code,temperature_2m_max,temperature_2m_min,sunrise,sunset`, `timezone=auto`, `forecast_days=<1..=16>`, `temperature_unit=celsius`, `wind_speed_unit=kmh`, `precipitation_unit=mm`. Metric is requested explicitly on every call (never inherited from a default) so a unit-less cache entry can never appear; `daily` is only requested when `days > 0`, because the endpoint requires `timezone` for daily variables and we want `days = 0` to stay a two-parameter current-only call.
- ⬜ Response types: `ForecastResponse { latitude, longitude, elevation: Option<f64>, utc_offset_seconds: i32, timezone: String, timezone_abbreviation: String, current: Option<CurrentBlock>, hourly: Option<HourlyBlock>, daily: Option<DailyBlock> }` with every variable array typed `Vec<Option<f32>>` (the API can return `null` for `visibility` and `precipitation_probability` depending on the model) and `time: Vec<String>` in local `YYYY-MM-DDTHH:MM` wall-clock. `HourSample { at: DateTime<Tz>, temp_c, feels_like_c, precip_mm, precip_prob_pct: Option<u8>, weather: Condition, wind_kmh, wind_dir_deg: Option<u16>, humidity_pct: Option<u8>, visibility_km: Option<f32> }`.
- ⬜ Mapping rules: `time` strings are resolved with step 03's `resolve_local(tz, naive)` (so a DST gap/ambiguity has one documented answer); `visibility` is **metres** upstream and is divided by `1000` into `visibility_km`; `is_day` (`0|1`) becomes `bool`; `pressure_msl` fills `Current.pressure_hpa`; `current.time` becomes `observed_at` (`DateTime<FixedOffset>`); `weather_code` becomes `Condition::from_u8(code)`; the response's `timezone` replaces a provisional `tz` when `loc.source` is `Coordinates` or `Osm`, and the corrected zone is what the header prints.
- ⬜ `Days`/`Report`: `days == 0` ⇒ current only, `days: vec![]`; otherwise `forecast_days = days.clamp(1, max_days)` with a once-per-run stderr warning when clamped. `Report.location` carries the (possibly corrected) location, `Report.days` is ordered oldest → newest starting at the location-local today, and `Attribution { provider: "open-meteo".into(), url: <the full request URL>, fetched_at: Utc::now() via env.cache.clock(), raw: verbose.then(|| …) }` keeps the raw local-time/weather-code pairs only under `-v`.
- ⬜ `pub fn aggregate_day(hours: &[HourSample], daily: &DailyBlock, date: NaiveDate, tz: Tz) -> Result<DayForecast>` — pure, network-free, unit-tested. **Written rules:** parts are cut in location-local time as `Morning 06:00–11:59`, `Noon 12:00–17:59`, `Evening 18:00–23:59`, `Night 00:00–05:59` **of the same local date** (the night row is the small hours of that day, not the following night); within a part, `temp_c`, `feels_like_c`, `humidity_pct`, `wind_kmh`, `wind_dir_deg` and `visibility_km` come from the single hourly sample **closest to the part midpoint** (09:00, 15:00, 21:00, 03:00; a tie takes the earlier hour); `precip_mm` is the part **sum**, `precip_prob_pct` the part **maximum**; `condition` is the highest `severity_rank()` code present in the part (step 03's total order), ties broken by the higher frequency among the tied codes and then by the earlier hour, and a part whose codes are all unknown falls back to the representative sample's code; `temp_min_c`/`temp_max_c` come from the `daily` arrays (authoritative for the local calendar day); `sunrise`/`sunset` are parsed as local times and carry the location offset, `None` when upstream returns null (polar night/day); a part with **zero** hourly samples is `Error::Upstream("no hourly data for 2026-10-05 night (Europe/Berlin)")` while a part with fewer than six is aggregated normally.
- ⬜ Caching: the response body is fetched through `env.cache.read_or_fetch_json::<ForecastResponse>` under `CacheKey::weather("open-meteo", lat, lon, days, local_today)`, TTL `cache.weather_ttl_secs` (600 s default), where `local_today` comes from `env.cache.clock()` plus the location tz — a day rollover therefore invalidates yesterday's entry by key, not by TTL luck.
- ⬜ `src/render/mod.rs`: introduces only what a working `--format plain` needs, and step 07 **extends** it rather than redefining it — `pub trait Renderer { fn render(&self, report: &Report, ctx: &RenderContext<'_>) -> Result<String>; }`; `pub struct RenderContext { pub units: UnitSystem, pub color: ColorMode, pub width: usize, pub term: TermCaps, pub now: DateTime<FixedOffset>, pub tz: Tz }`; `pub enum ColorMode { Auto, Always, Never }`; `pub struct TermCaps { pub is_tty: bool, pub color: bool, pub dumb: bool }`; `pub enum Format { Plain }` (`clap::ValueEnum`) with `format.renderer() -> Result<Box<dyn Renderer>>`. Step 07 adds the contract's remaining context members (`lang`, `i18n`, the full `TermCaps`, `resolve_width`/`resolve_color`) and the remaining `Format` variants, and step 09 fills the catalogs behind `i18n`; nothing delivered by this step reads them, so no placeholder i18n type appears here.
- ⬜ `src/render/plain.rs`: box-free, colour-free, pipe-friendly output that never exceeds `ctx.width`, built entirely from step 03's `format_*` functions, step 04's `location_line` and `DayPartKind::label()`; it prints the provider registry's `licence` line as `Data: …`, and prints `Location data © OpenStreetMap contributors (ODbL)` when `report.location.source == Osm`.
- ⬜ `src/main.rs` pipeline: parse argv → `Config::load`/`Settings::resolve` → `LocationSpec::parse_arg` → resolve (config `location.default` → geocoder → `--ip`) → `Cache::open` with the `CacheMode` from the flags → `UreqTransport`/`HttpClient` → `KeyStore` → `Env` → `select`/`fetch_chain` → timezone correction → renderer → stdout; every error path prints `error: …` to stderr and returns the contract's exit code, and no user-triggerable path panics.
- ⬜ Minimal CLI wiring for this step (the full matrix stays in step 08): positional `[LOCATION]`, `-p/--provider`, `-d/--days <0..=14>`, `-f/--format plain`, plus the flags already owned by 02/05 (`--ip`, `--no-cache`, `--refresh`, `--offline`, `--timeout`, `-q`, `-v`). `-u/--units`, `--lang`, `--color`, `--width`, `--lat/--lon`, `--station` and the non-plain formats are step 08's.
- ⬜ Tests in `tests/provider_open_meteo.rs`: the recorded request URL asserted from `StubTransport::calls()` against the exact parameter list above; `days = 0` producing a current-only request; the clamp warning path via a stub provider with `max_days = 7`; day-part aggregation on all four fixtures (midpoint sampling, sums, maxima, severity pick with a frequency tiebreak, night grouped into its own date); the DST-transition fixture proving the 23-hour local day is grouped correctly and that the missing 02:00 local hour does not shift the parts; min/max from the daily arrays and `sunrise`/`sunset` becoming `None` in the polar-winter fixture; timezone correction for a `Coordinates` location; chain behaviour (network/upstream falls through, usage does not, missing key ⇒ `Error::MissingKey`, `auto` expansion) with `#[cfg(test)] mod tests` stub providers inside `src/provider/mod.rs`.
- ⬜ Tests in `tests/render_plain.rs`: the exact line set for a fixture-backed report in all three unit systems, no ANSI escape in the output, `--days 0` printing only the header and current line, and the ODbL attribution line for an `Osm` location.
- ⬜ `tests/live.rs`: `#[ignore]`d `live_open_meteo_beijing` and `live_geocode_plus_forecast_at_coordinates` for manual runs; no test in the default suite reaches the network.
- ⬜ Attribution and docs: the plain renderer prints the Open-Meteo licence line (its data is CC BY 4.0 and attribution is required), `README.md` gains a usage section with the sample output and the data-source/attribution note, and the recorded responses under `tests/fixtures/open_meteo/` are annotated in `REUSE.toml` with `CC-BY-4.0` plus a `LICENSES/CC-BY-4.0.txt` text file (narrowing step 01's blanket `tests/fixtures/**` rule so the fixtures are not silently relicensed).

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
* `Env` carries `keys` in addition to the contract's three members: the chain uses the key store to decide
  whether a provider can run at all, and each provider needs the value. Providers still never touch the
  filesystem directly — they go through `KeyStore`.
* The render context is deliberately introduced with fewer members than the contract's final shape: step 07
  is what completes `RenderContext` (adding `lang`, `i18n`, the full `TermCaps` and the width/colour
  resolution) because it is the step that first needs them, and step 09 then fills the catalogs behind
  `i18n`. Neither step has to carry a placeholder i18n type, and the additions are struct fields plus their
  construction in `main.rs`.
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
`…/forecast_longyearbyen_2026-01-12.json` (polar winter, null sunrise/sunset),
`…/forecast_berlin_2026-03-29.json` (DST spring-forward day, 23 local hours),
`…/forecast_lisbon_2026-05-04.json` (all-clear day), `tests/fixtures/open_meteo/error_invalid_param.json`
(the 400 envelope) — recorded responses, `CC-BY-4.0`. Manual smoke run:

```sh
cargo run -- Beijing --format plain
# Beijing, Beijing, China (39.90, 116.40) Asia/Shanghai
# Now: 31°C (feels 36°C), Partly cloudy, wind 12 km/h SE, humidity 66%, pressure 1004 hPa, visibility 10 km, 0.0 mm
# 2026-09-30  min 26°C  max 34°C  sunrise 06:08  sunset 17:58
#   Morning  29°C  Partly cloudy  precip 0.0 mm (10%)   wind 8 km/h S
#   Noon     33°C  Overcast       precip 0.2 mm (25%)   wind 15 km/h SE
#   Evening  30°C  Light rain     precip 1.1 mm (60%)   wind 12 km/h E
#   Night    27°C  Clear sky      precip 0.0 mm (5%)    wind 6 km/h N
# Data: Open-Meteo.com (CC BY 4.0)
cargo run -- Beijing --format plain --offline      # identical body, served from weather/open-meteo-39.90-116.40-3-2026-09-30.json
cargo run -- Beijing --format plain --refresh      # refetches, same shape
cargo run -- Beijing --days 0 --format plain       # header + Now line only, no day lines
cargo run -- '@39.9042,116.4074' --format plain    # header shows the provider-resolved zone: 39.90, 116.41 (Asia/Shanghai)
cargo run -- -p open-meteo,does-not-exist --format plain; echo $?   # error: unknown provider `does-not-exist`   (exit 2)
cargo run -- Longyearbyen --days 1 --format plain  # sunrise/sunset omitted when upstream returns null
```

## Exit criteria

- ⬜ `cargo test` passes with the network unplugged; the only network-touching tests are `#[ignore]`d.
- ⬜ The five aggregation rules are each covered by a named test in `tests/provider_open_meteo.rs`, and
      the fixture set covers hot/humid, polar winter, a DST-transition day and an all-clear day.
- ⬜ `cargo fmt --check` clean.
- ⬜ `cargo clippy --all-targets -- -D warnings` clean.
- ⬜ `cargo test` clean (`tests/provider_open_meteo.rs`, `tests/render_plain.rs`, all unit tests).
- ⬜ `reuse lint` clean, with the Open-Meteo fixtures annotated `CC-BY-4.0` and `LICENSES/CC-BY-4.0.txt`
      present.
- ⬜ Smoke run above reproduces the shown shape: location header, one `Now:` line, one date line with
      min/max/sunrise/sunset, four part lines per day, the attribution line, and the `--offline` rerun
      printing the same body without touching the network.

## Risks

- Some Open-Meteo models return `null` for `visibility` or `precipitation_probability` at a given hour;
  the arrays are `Vec<Option<f32>>` and the aggregation treats a missing representative value as `None`
  rather than as `0`, so the plain renderer omits the field instead of printing a fake zero.
- The grid cell Open-Meteo selects can be a few kilometres from the requested coordinate and its elevation
  differs; the header therefore echoes what the geocoder resolved, and the response's `lat`/`lon` are kept
  in the raw cache entry for debugging rather than being shown as the location.
- `forecast_days` is capped at 16 upstream and step 08 caps `--days` at 14 per the contract, so the clamp
  warning is unlikely to fire in practice; it exists for providers with a smaller horizon (step 10).
- Sunrise/sunset nullness in polar regions is real behaviour; a renderer that assumes the fields exist
  would print `None`, which the fixture test prevents.
- The `Attribution.raw` blob exists only under `-v`; if a future step wants it for the JSON renderer, it
  is already on the model, and step 08 decides whether to expose it.

## Progress log

- 2026-09-30 — plan written.
