<!--
SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Step 03 — canonical-model-and-units

Status: ⬜ not-started
Depends on: `01-project-scaffold.md` (error taxonomy), `02-config-and-state.md` (`Settings`/`Config` carrying `units`, serde in the dependency set)
Touches: `src/model/mod.rs`, `src/model/condition.rs`, `src/model/units.rs`, `src/lib.rs`, `examples/units_demo.rs`, `tests/model_condition.rs`, `tests/units.rs`, `tests/fixtures/model/`, `docs/plans/README.md`

## Goal

Every weather value the project ever holds is defined here, once: a `Report` tree of
`Location`/`Current`/`DayPart`/`DayForecast` in canonical metric/SI units, a `Condition` newtype over
the WMO 4677 code that never loses an unknown code, and the single conversion/formatting module that
turns canonical values into display strings for the `metric`, `us` and `uk` unit systems. Providers
fill these types, renderers only read them, and no renderer ever converts or branches on a
provider-specific code — the compile-time contract that keeps the rest of the rewrite small.

## Deliverables

- ⬜ `src/model/mod.rs`: `Location { name: String, admin1: Option<String>, country: String, country_code: Option<String>, lat: f64, lon: f64, tz: chrono_tz::Tz, elevation_m: Option<f64>, population: Option<u64>, source: LocationSource }` with `LocationSource = Geocoder | Osm | Coordinates | Ip | Config` (step 04/05 set it; step 06 corrects `tz` from the forecast response when the source is `Coordinates` or `Osm`). `population` is optional ranking metadata for step 04's ambiguous-match ordering and is never rendered.
- ⬜ `Current { observed_at: DateTime<FixedOffset>, temp_c: f32, feels_like_c: f32, humidity_pct: u8, precip_mm: f32, weather: Condition, cloud_cover_pct: u8, pressure_hpa: f32, wind_kmh: f32, wind_dir_deg: u16, wind_gust_kmh: Option<f32>, visibility_km: Option<f32>, is_day: bool }`.
- ⬜ `DayPartKind = Morning | Noon | Evening | Night` with `DayPartKind::ALL: [DayPartKind; 4]`, `index() -> usize`, `Range<u8>` local hours (`Morning 06..12`, `Noon 12..18`, `Evening 18..24`, `Night 00..06`), `midpoint_hour() -> u8`, `label()` for the plain renderer; `DayPart { kind, temp_c, feels_like_c: Option<f32>, precip_mm: f32, precip_prob_pct: Option<u8>, weather: Condition, wind_kmh: f32, wind_dir_deg: Option<u16>, humidity_pct: Option<u8>, visibility_km: Option<f32> }`.
- ⬜ `DayForecast { date: NaiveDate, parts: [DayPart; 4], temp_min_c: f32, temp_max_c: f32, sunrise: Option<DateTime<FixedOffset>>, sunset: Option<DateTime<FixedOffset>> }` — the array makes a missing part unrepresentable; `Report { location: Location, current: Option<Current>, days: Vec<DayForecast>, attribution: Attribution }`; `Attribution { provider: String, url: String, fetched_at: DateTime<Utc>, raw: Option<String> }`, where `provider` is the step-06 registry id (`open-meteo`, …) and `raw` holds the provider's raw code list only when `--verbose` is set (debug only, never read by rendering).
- ⬜ All model types derive `Debug, Clone, PartialEq` + serde `Serialize`/`Deserialize`, and every numeric field is documented with its canonical unit (`// km/h`, `// hPa`, …) so no future provider invents a second convention.
- ⬜ Helpers used by later steps: `pub fn compass_16(deg: u16) -> &'static str` (`N`, `NNE`, … with the 11.25° half-sector, wrapping and `348.75..=360` handled) and `pub fn resolve_local(tz: chrono_tz::Tz, naive: NaiveDateTime) -> Result<DateTime<chrono_tz::Tz>>` implementing the DST policy: `LocalResult::Single` used as-is, `Ambiguous` takes the earliest offset, `None` (spring-forward gap) retries `naive + 1h` and otherwise fails with `Error::Upstream("local time <naive> does not exist in <tz>")`.
- ⬜ `src/model/condition.rs`: `#[serde(transparent)] pub struct Condition(u8)` with `pub fn from_u8(code: u8) -> Self` (total, never fails, keeps the raw code in unknown cases), `code() -> u8`, `is_known() -> bool`, `i18n_key() -> &'static str`, `description_en() -> &'static str`, `is_precipitation()`, `is_fog()`, `is_thunder()`, `is_clear()`, `art_key() -> &'static str`, `severity_rank() -> u8`.
- ⬜ The WMO 4677 table as a single `const CODES: [(u8, &str, &str, u8, &str); 28]` (`code`, description key, English description, severity rank, art key) covering 0, 1, 2, 3, 45, 48, 51, 53, 55, 56, 57, 61, 63, 65, 66, 67, 71, 73, 75, 77, 80, 81, 82, 85, 86, 95, 96, 97, 99 — descriptions re-authored from the WMO code names published with the Open-Meteo docs, e.g. `0 → "Clear sky"`, `48 → "Depositing rime fog"`, `82 → "Violent rain showers"`, `99 → "Thunderstorm with heavy hail"`; never copied from wego or wttr.in data files.
- ⬜ `i18n_key()` returns `cond.<code>` (`cond.95`) so step 09 can add Fluent messages per code; `description_en()` returns the English table entry; for codes without an entry both return `cond.unknown` / `"Unknown"`, `is_known()` is `false`, every classification predicate is `false`, `art_key()` is `"unknown"` and `severity_rank()` is `0` (below clear sky's `1`, so a real code always wins an aggregation tie).
- ⬜ `art_key()` vocabulary fixed here so step 07 maps keys, not codes: the method lives on `Condition` in `model/condition.rs` (the contract's model section places it there, and `render/art.rs` consumes it) and returns `&'static str` from this list — `clear`, `mainly-clear`, `partly-cloudy`, `overcast`, `fog`, `rime-fog`, `drizzle-light`, `drizzle`, `drizzle-dense`, `freezing-drizzle-light`, `freezing-drizzle-dense`, `rain-light`, `rain`, `rain-heavy`, `freezing-rain-light`, `freezing-rain-heavy`, `snow-light`, `snow`, `snow-heavy`, `snow-grains`, `showers-rain-light`, `showers-rain`, `showers-rain-violent`, `showers-snow-light`, `showers-snow-heavy`, `thunderstorm`, `thunderstorm-hail-light`, `thunderstorm-heavy`, `thunderstorm-hail-heavy`, `unknown`.
- ⬜ `severity_rank()` is the total order used by step 06's day-part aggregation: `0/1 → 1`, `2 → 2`, `3 → 3`, `45 → 4`, `48 → 5`, `51 → 6`, `53 → 7`, `55 → 8`, `56 → 9`, `57 → 10`, `61 → 11`, `63 → 12`, `65 → 13`, `66 → 14`, `67 → 15`, `71 → 16`, `73 → 17`, `75 → 18`, `77 → 19`, `80 → 20`, `81 → 21`, `82 → 22`, `85 → 23`, `86 → 24`, `95 → 25`, `96 → 26`, `97 → 27`, `99 → 28`; documented as a project ordering (not a WMO one) whose only promise is "higher = more significant weather".
- ⬜ `src/model/units.rs`: `UnitSystem = Metric | Us | Uk` with `FromStr` and the contract's defaults per system — Metric `(C, kmh, hpa, km, mm)`, Us `(F, mph, inhg, mi, in)`, Uk `(C, mph, hpa, mi, mm)` — plus `UnitOverrides` from the `[units]` config table, resolved into `ResolvedUnits { temp: TempUnit, wind: WindUnit, pressure: PressureUnit, distance: DistanceUnit, precip: PrecipUnit }` by `UnitSystem::resolve(&UnitOverrides) -> Result<ResolvedUnits>` (each override `None` = take the system default; an invalid override is already rejected by step 02's `validate`).
- ⬜ Conversions, each a pure function with a doc-tested constant: `c_to_f`, `kmh_to_mph`, `kmh_to_knots`, `kmh_to_mps`, `hpa_to_inhg`, `hpa_to_mmhg`, `km_to_mi`, `mm_to_in`.
- ⬜ Formatters with wttr.in-like rounding, all taking canonical input and the resolved unit: `format_temp(c, TempUnit)` → integer °C/°F (`23°C`, `73°F`); `format_wind(kmh, WindUnit)` → integer with one decimal only below 10 **after** rounding (`12 km/h`, `8.3 km/h`, `5.8 mph`, and `9.95 km/h` → `10 km/h`); `format_pressure(hpa, PressureUnit)` → integer hPa / 2-decimal inHg / integer mmHg; `format_distance(km, DistanceUnit)` and `format_visibility(km, DistanceUnit)` → one decimal below 10 by the same rule (`4.2 km`, `14 km`, `3.1 mi`); `format_precip(mm, PrecipUnit)` → always one decimal in mm and two in inches (`0.0 mm`, `0.2 mm`, `0.01 in`).
- ⬜ Rounding is one shared helper `fn round_half_away_from_zero(v: f32) -> f32` plus `fn fmt_int(v: f32) -> String` that maps a rounded `-0` to `0`, so `-0.4 °C` prints `0°C` and `-0.6 °C` prints `-1°C`, and the 0 °C → 32 °F boundary is exactly `32°F` (no `31°F`/`33°F`).
- ⬜ `examples/units_demo.rs`: prints the same set of canonical values through Metric/Us/Uk in aligned columns; the developer eyeball for formatting before any renderer exists (excluded from the release binary).
- ⬜ Tests in `tests/units.rs`: the conversion truth table loaded from fixture, the rounding edges (`-0.4`, `-0.6`, `0.0`, `0.4`, `-0.0`, and every temperature spanning the 32 °F boundary), the `< 10` one-decimal rule at `9.94`/`10.0`/`9.95`, `ResolvedUnits` overrides for all five quantities, and `compass_16` at `0/11.24/11.25/348.74/348.75/360`.
- ⬜ Tests in `tests/model_condition.rs`: every row of the known-code fixture (key, English description, predicates, art key, rank), a sweep of `0..=255` proving `from_u8` is total and that codes outside the table report `is_known() == false` with all predicates `false`, the unknown-code list fixture, and `resolve_local` against the spring-forward gap and the fall-back ambiguity of `Europe/Berlin` on 2026-03-29 and 2026-10-25.

## Design notes

* **chrono + chrono_tz instead of jiff.** `jiff` is the more modern API (typed `Zoned`, better civil-time
  ergonomics, `jiff-tzdb` for a bundled database), and it was the alternative seriously considered.
  It loses on two concrete points for this project. First, the IANA database story: `chrono-tz`
  compiles a pinned tzdata release into the crate, so `Tz::from_str("Asia/Shanghai")` works identically
  in a minimal container, under an unusual `TZ`, and on Windows without system tzdata — exactly the
  property the geocoding layer needs when it turns an upstream `timezone` string into a usable zone.
  Second, day-part aggregation: we need `Tz::from_local_datetime(&naive) -> LocalResult` with explicit
  `Single`/`Ambiguous`/`None` arms, because the Open-Meteo hourly arrays are local wall-clock timestamps
  and a DST day has 23 or 25 hours; chrono states that policy in three lines of code, and the same
  `DateTime<Tz>` values then feed `NaiveDate` day grouping and `FixedOffset` artefacts. chrono is also
  already the ecosystem default for serde-friendly timestamps.
* All model fields are canonical metric/SI, and conversion happens **only** in the render layer.
  `ResolvedUnits` is the single object a renderer holds, so a second conversion point cannot appear
  by accident. Cache entries stay unit-independent as a consequence.
* `Condition` is a newtype instead of an enum: providers legitimately return codes we have no
  description for, and dropping or clamping them would hide upstream behaviour. Unknown codes are
  visible (`is_known()`), sort lowest, and render as "Unknown" rather than as clear sky.
* `severity_rank` is deliberately a project-local total order: the aggregation rule "most severe code
  in the part" needs a tiebreak-free comparison, and WMO 4677 has families (rain/drizzle/showers) with
  no canonical ordering between them. Publishing the table here means step 06 tests can assert it.
* `Attribution.provider` is a `String` rather than `provider::ProviderId` to keep the dependency
  direction `provider → model` (step 06 owns `ProviderId`, and a registry-id round-trip test in step 06
  keeps the two honest).

## Out of scope

- Day-part aggregation itself (midpoint sampling, sums, severity pick) — implemented and tested in
  `06-open-meteo-provider.md`, which consumes `DayPartKind`, `severity_rank()` and `resolve_local()`.
- Localised descriptions: only `i18n_key()` and the English table exist here; Fluent catalogs,
  locale negotiation and `I18n` live in `09-localization.md`.
- Unit flags on the CLI and formatted output of any kind: `--units` parsing is wired in
  `08-cli-surface-and-formats.md`; the strings produced here are first printed by the plain renderer
  in `06-open-meteo-provider.md`.
- JSON wire schema: `Report` derives serde here, but the stable, documented JSON document (field
  names, versioning) belongs to `08-cli-surface-and-formats.md`.

## Verification

Fixtures: `tests/fixtures/model/units-cases.tsv` (`quantity  value  system  expected` rows for every
conversion and rounding edge, `#`-commented SPDX header), `tests/fixtures/model/wmo4677-known.tsv`
(the 28-code table: code, key, English text, precip, fog, thunder, art key, rank),
`tests/fixtures/model/wmo4677-unknown.txt` (codes without a description: `4`, `20`, `46`, `52`, `64`,
`70`, `78`, `87`, `90`, `100`, `200`, `255`). Manual smoke run:

```sh
cargo test --test units --test model_condition       # all fixture rows + the 0..=255 sweep pass
cargo run --example units_demo
# expected shape (aligned columns, one row per quantity):
# quantity    metric      us          uk
# temp        23°C        73°F        23°C
# temp        0°C         32°F        0°C
# wind        5.4 km/h    3.4 mph     3.4 mph
# pressure    1013.25 hPa 29.92 inHg  1013.25 hPa
# precip      0.2 mm      0.01 in     0.2 mm
# distance    4.2 km      2.6 mi      2.6 mi
# visibility  8.0 km      5.0 mi      5.0 mi
```

## Exit criteria

- ⬜ `src/model/` contains no provider-specific code, no unit conversion outside `units.rs`, and no
      `panic!`/`unwrap` outside tests.
- ⬜ Every conversion in `tests/fixtures/model/units-cases.tsv` and every code in
      `tests/fixtures/model/wmo4677-known.tsv` is asserted; the unknown sweep covers `0..=255`.
- ⬜ `cargo fmt --check` clean.
- ⬜ `cargo clippy --all-targets -- -D warnings` clean.
- ⬜ `cargo test` clean (`--lib`, `tests/units.rs`, `tests/model_condition.rs`, doctests).
- ⬜ `reuse lint` clean (both fixtures are `.tsv`/`.txt` with `#` SPDX headers; nothing new for
      `REUSE.toml`).
- ⬜ `cargo run --example units_demo` reproduces the table above, including `-0.4 °C → 0°C` and
      `0 °C → 32°F` in the US column.

## Risks

- The `units-cases.tsv` expectations are hand-authored: a wrong expectation would enshrine a wrong
  format. Mitigation: the table is generated from the fixture through `examples/units_demo` and
  reviewed against wttr.in output side by side before the first commit.
- `severity_rank` is a judgement call; if step 06's fixtures disagree (e.g. a part with 1 h thunder and
  5 h rain), the rank table changes and both steps' fixtures move together — the rank lives in one
  place for exactly that reason.
- `chrono-tz` pins a tzdata release, so a newly created zone name (or a changed DST rule) needs a
  `chrono-tz` bump; noted for `13-packaging-and-release.md` dependency review.
- `f32` for weather values keeps `Report` small and matches the JSON wire types; the formatters round
  before printing, so no `23.000000001`-style output can escape. Aggregation sums in `f32` as well —
  acceptable for millimetre-scale precipitation.

## Progress log

- 2026-09-30 — plan written.
