<!--
SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
SPDX-License-Identifier: GPL-3.0-or-later
-->

# cirrocast — implementation plans

Master index and architecture contract for the build-out of `cirrocast`, a terminal weather
client (a rewrite and replacement for `wego`, taking `wttr.in` as the output reference).

Related: [`/AGENTS.md`](../../AGENTS.md) — operating rules for agents and humans working in this repo.

## How these plans are used

* One file per step: `NN-kebab-case-title.md`, ordered by execution.
* A step is worked on alone, top to bottom. Every `- ⬜` item in a step is one committable unit.
* Progress is tracked with emoji, **inside each step file**:
  * a `Status:` header line — one of `⬜ not-started`, `🚧 in-progress`, `⛔ blocked`, `✅ done`
  * `- ⬜` (open) / `- ✅` (done) markers on every task in `## Deliverables` and `## Exit criteria`
  * a `## Progress log` section, appended (never rewritten) with `YYYY-MM-DD — note`
* When a step is finished, its `Status:` becomes `✅ done` and the matching row in the table below is
  updated in the same commit. Never mark a step done while any `- ⬜` item is still open.
* A step file is a living document: if the design changes, the doc changes in the same commit as the
  code. Docs are not written once and abandoned.
* Work discovered after a phase has been planned is appended as a new numbered step and a new phase
  row: the number is the execution order, so an appended step runs after the existing tail. A step
  may be started ahead of its number — while steps with lower numbers are still open — only when
  every entry of its `Depends on` line is done, and the deviation is recorded in its
  `## Progress log`.

## Phases and milestones

| Phase | Steps | Milestone |
|---|---|---|
| A — Foundation | 01–06 | real end-to-end run: `cirrocast Beijing -f plain` prints live data from a keyless backend |
| B — Output parity | 07–11 | wttr.in-style `art-table` plus `one-line`/`plain`/`json`, en-US + zh-CN, all v1 backends |
| C — Quality and release | 12–14 | **v1.0.0 = "basically formed"**, packaged and reproducible |
| D — Parity and reach | 15–19 | v1.1–v1.2: alerts, air quality, moon/astro, offline city database, extra providers |
| E — Integration and ecosystem | 20–24 | v2.0: wttr.in-compatible local service, multi-location, perf budgets, docs, ecosystem packages |
| F — Sources, selection and auth | 25–29 | QWeather JWT, pick-a-candidate location resolution, second-generation geo/IP sources, keyless national backends with coverage-aware `auto`, climate normals |

Phase F was appended on 2026-10-03 after a review of `breezy-weather`'s source catalogue and two
upstream requests (QWeather JWT, selectable location candidates); it is independent of phase E. The
same review added the append-and-pull-forward rule to `AGENTS.md`'s plan-driven workflow (step 1)
and to "How these plans are used" above, because a phase appended after the tail would otherwise
wait on ten unrelated steps.

## Step files

| # | Phase | Step | Status | Depends on |
|---|-------|------|--------|------------|
| 01 | A | [project-scaffold](01-project-scaffold.md) | ✅ done | — |
| 02 | A | [config-and-state](02-config-and-state.md) | ✅ done | 01 |
| 03 | A | [canonical-model-and-units](03-canonical-model-and-units.md) | ✅ done | 01, 02 |
| 04 | A | [geocoding-and-location-syntax](04-geocoding-and-location-syntax.md) | ✅ done | 02, 03 |
| 05 | A | [http-cache-and-ip-location](05-http-cache-and-ip-location.md) | ✅ done | 02, 03, 04 |
| 06 | A | [open-meteo-provider](06-open-meteo-provider.md) | ✅ done | 03, 04, 05 |
| 07 | B | [art-table-renderer](07-art-table-renderer.md) | ✅ done | 03, 06 |
| 08 | B | [cli-surface-and-formats](08-cli-surface-and-formats.md) | ✅ done | 06, 07 |
| 09 | B | [localization](09-localization.md) | ✅ done | 03, 07 |
| 10 | B | [additional-providers](10-additional-providers.md) | ✅ done | 05, 06, 08 |
| 11 | B | [metar-and-aviation](11-metar-and-aviation.md) | ✅ done | 05, 10 |
| 12 | C | [quality-hardening](12-quality-hardening.md) | ✅ done | 08, 09, 10 |
| 13 | C | [packaging-and-release](13-packaging-and-release.md) | ✅ done | 08, 12 |
| 14 | C | [v1-acceptance](14-v1-acceptance.md) | ✅ done | all of A–C |
| 15 | D | [alerts-and-severity](15-alerts-and-severity.md) | ✅ done | 10, 12 |
| 16 | D | [air-quality-and-pollen](16-air-quality-and-pollen.md) | ✅ done | 03, 08 |
| 17 | D | [moon-phase-and-astro](17-moon-phase-and-astro.md) | ✅ done | 03, 08 |
| 18 | D | [offline-city-database](18-offline-city-database.md) | ⬜ not-started | 04, 05 |
| 19 | D | [more-providers](19-more-providers.md) | ⬜ not-started | 10, 15, 16 |
| 20 | E | [wttr-compat-service](20-wttr-compat-service.md) | ⬜ not-started | 08, 10, 14 |
| 21 | E | [multi-location-and-templates](21-multi-location-and-templates.md) | ⬜ not-started | 08, 14 |
| 22 | E | [perf-and-resource-budget](22-perf-and-resource-budget.md) | ⬜ not-started | 12, 21 |
| 23 | E | [docs-and-guides](23-docs-and-guides.md) | ⬜ not-started | 14, 21 |
| 24 | E | [ecosystem-integration](24-ecosystem-integration.md) | ⬜ not-started | 13, 20, 21 |
| 25 | F | [qweather-jwt-auth](25-qweather-jwt-auth.md) | ⬜ not-started | 02, 10, 15 |
| 26 | F | [location-candidate-selection](26-location-candidate-selection.md) | ⬜ not-started | 04, 05, 08, 18 |
| 27 | F | [location-sources-2](27-location-sources-2.md) | ⬜ not-started | 04, 05, 18, 26 |
| 28 | F | [keyless-national-providers](28-keyless-national-providers.md) | ⬜ not-started | 06, 10, 19 |
| 29 | F | [climate-normals](29-climate-normals.md) | ⬜ not-started | 03, 06, 08 |

**v1.0.0 = "basically formed"** (steps 01–14) means, end to end and demonstrated in step 14: eight
backends selectable (three keyless), BYOK keys never touching `config.toml`, city-name, coordinate
and IP location resolution, four text output formats with the wttr.in-style `art-table` as default,
metric/us/uk units with per-quantity overrides, en-US + zh-CN output, XDG-compliant config, cache and
data directories, and a packaged, REUSE-compliant, CI-clean release. Everything in phases D and E is
*planned work with a plan file*, not a roadmap wish and not a stub in `src/`.

## Requirement traceability

| Original requirement | Steps that deliver it |
|---|---|
| 1. Rust with `clap` and friends | 01 (toolchain, dependencies, lint gates) |
| 2. Multiple backends, keyless first | 06 (open-meteo), 10 (owm, weatherapi, wwo, pirateweather, qweather, smhi), 11 (metar), 19 (met.no, visualcrossing, open-meteo archive/marine), 28 (nws, brightsky) |
| 3. BYOK for key-requiring backends | 02 (key store + `key` subcommands), 10 (consumption, `MissingKey`), 12 (secret-handling audit), 25 (JWT credentials) |
| 4. All wttr.in outputs | 07 (`art-table`, `dumb`), 08 (`one-line` templates, `plain`, `json`, completions, man), 17 (astro/moon tokens), 20 (wttr.in-compatible local service incl. the `?` option table), 21 (multi-location output) |
| 5. City name → coordinates | 04 (Open-Meteo geocoding + Nominatim), 18 (offline bundled city database, crate evaluation), 27 (GeoNames search, multi-source merge) |
| 6. IP → city | 05 (ipwho.is + ipapi.co, opt-in, cached, privacy documented), 27 (IP.SB, coordinate naming, coverage) |
| 7. Own CLI design, no wego copying | 01 + `AGENTS.md` (no-copy rule), 08 (documented flag matrix and precedence) |
| 8. Selectable units and output language | 03 (unit system + formatting), 09 (Fluent i18n, en-US + zh-CN) |
| 9. Standard XDG directories | 02 (`etcetera`-based config/cache/data), 12 (XDG audit) |
| 10. Project name | 01 / `AGENTS.md` — `cirrocast`; verified free on crates.io, AUR, Arch, npm, PyPI and GitHub |
| 11. Candidates selectable when a name/IP lookup is ambiguous | 26 (picker, `--pick`/`--yes`, `location search --all`), 27 (multi-source merge, coordinate naming feeds the picker) |
| 12. QWeather JSON Web Token authentication | 25 (Ed25519 credentials, `key set qweather --jwt`, bearer header) |
| 13. Official keyless national backends, chosen by coverage | 28 (NWS, Bright Sky, coverage-aware `auto`), 15 (global alert aggregators WMO SWIC and FPAS, HKO alerts) |
| 14. Climate normals for the location | 29 (NOAA NCEI Global Summary of the Month, 1991–2020, cached 30 days) |

Rows 11–14 were added on 2026-10-03: 11 and 12 are upstream requests, 13 and 14 come from the
`breezy-weather` source audit (proprietary and reverse-engineered sources, including Xiaomi's
`china` API, are explicitly not adopted — see step 19's design notes).

## Definition of "basically formed" (v1.0.0 gate, checked in step 14)

1. `cirrocast` with no arguments prints weather for the configured default location or the IP-derived
   location, in the default `art-table` format, in under a second on a warm cache.
2. Every backend in the v1 matrix can be selected with `--provider` and either works keyless or fails
   with the exact `cirrocast key set <id>` instruction.
3. `--format art-table|one-line|plain|json|dumb`, `--units metric|us|uk`, `--lang`, `--days`,
   `--lat/--lon`, `--ip`, `--station`, the cache-control flags and `--color`/`--width` all behave as
   documented in `--help` and in this file.
4. The `config`/`key`/`provider`/`cache`/`location` subcommands are functional, not decorative.
5. `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test`, `reuse lint`, `cargo deny` and CI
   are green; a clean Arch machine can install from the PKGBUILD and run it.
6. No placeholder code, no dead flags, no undocumented exit codes.

## Architecture contract

Everything below is binding for all steps. Changing it means changing this file first.

### Module map

```
src/
  main.rs            thin entry: parse argv, dispatch, map Error -> exit code
  cli.rs             clap definitions, location argument parsing, subcommand dispatch
  error.rs           Error enum (thiserror), Result alias, exit-code mapping
  paths.rs           XDG resolution (config/cache/data dirs) via etcetera
  config/
    mod.rs           Config struct, load/merge/save, schema_version migration hooks
    keys.rs          BYOK key store (env, keys.toml @0600, optional keyring), masking
  model/
    mod.rs           Location, Current, DayPart, DayForecast, Report, Attribution
    condition.rs     canonical WMO 4677 code type + classification helpers
    units.rs         UnitSystem + conversion + formatting (single conversion point)
    alert.rs         CAP-shaped alert types (step 15)
    air.rs           AirQuality, Pollen, AirSource (step 16)
    astro.rs         Astro, Moon, Sun, MoonPhase, Polar, SunSource (step 17)
  geo/
    mod.rs           Geocoder trait, IpLocator trait, LocationSpec
    open_meteo.rs    Open-Meteo geocoding (no key)
    nominatim.rs     OSM Nominatim fallback (`~query`), 1 req/s + mandatory cache
    ip.rs            ipwho.is primary, ipapi.co fallback
    tz.rs            offline coordinate → IANA zone lookup (`tzf-rs`), for payloads that carry none
  http.rs            shared HTTP client: timeouts, UA, retries/backoff, proxy, error taxonomy
  cache.rs           on-disk cache: keys, TTLs, atomic writes, offline mode
  air/
    mod.rs           air-quality facade (best-effort, one panel per run)
    aqi.rs           the US and European AQI category scales
    open_meteo.rs    Open-Meteo Air Quality API (keyless)
  astro/
    mod.rs           Astro::compute, GMST, the local-day window and the shared rise/set search
    julian.rs        Julian dates and the ΔT seam (Espenak–Meeus fits)
    moon.rs          truncated ELP-2000/82 position, phase, illumination, age, rise/set
    sun.rs           solar position and the local day's sunrise/sunset/polar state
  provider/
    mod.rs           Provider trait, ProviderId, registry metadata, selection + fallback chain
    open_meteo.rs    Open-Meteo (keyless, default)
    openweathermap.rs / weatherapi.rs / worldweatheronline.rs
    pirateweather.rs / qweather.rs / smhi.rs
    metar.rs         aviationweather.gov METAR/TAF (keyless, station based)
  render/
    mod.rs           Renderer trait, RenderContext, terminal capability detection
    art_table.rs     wttr.in-style day-part column table
    one_line.rs      template output (`%c`, `%t`, ... wttr.in-compatible tokens)
    plain.rs         box-free, pipe friendly
    json.rs          stable JSON schema
    alerts.rs        the alert banner, records and listing (step 15)
    air.rs           the air-quality panel, records and standalone view (step 16)
    moon.rs          the moon/sun panel, records and standalone view (step 17)
    art.rs           canonical condition -> unicode art blocks (day/night)
    color.rs         256-color palette, NO_COLOR / CLICOLOR_FORCE handling
  i18n.rs            Fluent bundle loading, locale negotiation, embedded .ftl catalogs
locales/             en-US/main.ftl, zh-CN/main.ftl, ...
tests/               integration tests (CLI level), fixtures/ = recorded API responses
```

### Canonical data model (binding)

* **Condition = WMO 4677 code** (`u8`, 0..=99) wrapped in a newtype with `is_precipitation()`,
  `is_thunder()`, `art_key()`, `i18n_key()`. Every provider maps its native codes to WMO; the
  original code is kept in `Attribution`/`raw` for debugging only. Rendering and text never branch
  on provider-specific codes.
* All numeric fields are stored in **canonical metric/SI** (`temp_c`, `wind_kmh`, `precip_mm`,
  `pressure_hpa`, `visibility_km`). Providers request metric from upstream wherever the API allows
  it; **the render layer is the only place that converts units**. Cache entries are therefore
  unit-independent.
* `Report { location, current: Option<Current>, days: Vec<DayForecast>, alerts, air, astro,
  attribution }`; `alerts` (step 15), `air` (step 16) and `astro` (step 17) are attached after the
  forecast and default to empty/`None`, so a hand-built or older document still parses.
  `days` is ordered oldest → newest and always starts at the location-local today. A backend that
  serves observations only (step 11's `metar`) answers with `current: Some(..)`, `days: []`, and the
  renderers shape their output from the provider's declared capabilities, never from its id.
* `Location` carries `station: Option<String>` (plus `LocationSource::Station`): a station-based
  backend keys its cache, builds its request and names the place from the identifier, which the
  display name cannot carry. It is `#[serde(default)]`, so documents written before the field exist
  still parse, and it is `null` for every non-station location.
* Day parts are exactly `Morning | Noon | Evening | Night` (wttr.in's four rows), each aggregated
  from hourly data by the provider module using the location timezone, never by the renderer.
* Time is `chrono` with `chrono_tz::Tz` for the location; all artifacts carry the location offset.

### Provider contract (binding)

```rust
pub struct Capabilities {
    pub current: bool, pub hourly: bool, pub daily: bool, pub alerts: bool,
    pub max_days: u8, pub requires_key: bool, pub key_env: Option<&'static str>,
    pub location_kinds: LocationKinds, // City | Station | LatLon
}

pub trait Provider {
    fn id(&self) -> ProviderId;
    fn capabilities(&self) -> Capabilities;
    /// Provided: runs `fetch_report`, refuses a report whose readings are not finite
    /// (`Error::Upstream`), then returns it. Callers always use this.
    fn fetch(&self, loc: &Location, req: &FetchRequest, env: &Env<'_>) -> Result<Report>;
    /// Required: the backend's own request and decode.
    fn fetch_report(&self, loc: &Location, req: &FetchRequest, env: &Env<'_>) -> Result<Report>;
}
```

* `fetch(&self)` takes `&self` (no interior mutability), is **synchronous** — the project does not
  use an async runtime; `ureq` is the HTTP stack. Do not add `tokio`/`reqwest` without updating this
  contract and stating why in the step doc.
* Backends implement `fetch_report`; `fetch` is the one choke point that validates the report (no
  reading may be `NaN`/`inf`), so no backend can skip the guard. This split — a provided `fetch` plus
  the required `fetch_report` — is the shipped shape since the 2026-10-02 review fixes.
* `req: FetchRequest { days, hourly_resolution }`; `env: Env` gives access to the shared HTTP client,
  cache and config. Providers never open sockets or read files directly.
* Adding a provider = one file + one `ProviderId` variant + one registry row + fixtures + a
  `provider list`/`provider info` update. No CLI flag is added per provider.
* Selection: `--provider a,b,c` is an explicit ordered chain; bare default comes from config
  (`defaults.provider`), whose built-in value is `open-meteo`. `auto` expands to the keyless chain
  that answers for a resolved place, **ranked by registry coverage** (country match, then bounding
  box, then the global entries; step 28) — until step 28 lands it is the interim fixed list
  `open-meteo,met-no,smhi` (step 19). A station is never part of it — `--station`
  selects `metar` when no provider is given, and prepends it to `auto` when one is. A failure in a chain falls through
  to the next entry only when the error is transport/upstream (`Error::Upstream`/`Network`), never
  when it is a usage, key or location error.
* Alerts are a **separate source registry**, not a provider capability: step 15's sources (NWS,
  MeteoAlarm, HKO, WMO SWIC, FPAS, QWeather, VisualCrossing) declare their own coverage and are
  selected by it (`[alerts] sources = ["auto"]`, `--alerts-from` overrides); a provider's
  `alerts: true` means its *own payload* carries warnings. The global aggregators (WMO SWIC, FPAS)
  are what make `--alerts` meaningful outside the US, the EU and China.

### Rendering contract (binding)

```rust
pub struct RenderContext<'a> {  // built once in main, passed by reference
    pub units: ResolvedUnits, pub lang: LanguageId, pub color: ColorMode, pub width: usize,
    pub term: TermCaps, pub now: DateTime<FixedOffset>, pub tz: chrono_tz::Tz, pub i18n: &'a I18n,
}
pub trait Renderer { fn render(&self, report: &Report, ctx: &RenderContext<'_>) -> Result<String>; }
```

`units` is the *resolved* unit set (step 03's `UnitSystem::resolve`), `color` is already resolved
(never `Auto`) and `width` already clamped, so a renderer never consults the environment; `term`
is the `TermCaps` step 07 describes (`is_tty`, `term`, `utf8`, `depth`, `color_pref`).

* The `json` document carries a `capabilities` object (the registry row of the answering backend:
  `current`, `hourly`, `daily`, `alerts`, `max_days`, `requires_key`, `key_env`, `locations`), so a
  consumer can tell "no `days` because the backend is an observation" from "no `days` because the
  request asked for none". Additive within `schema_version = 2`, which added the `alerts` array
  (the CAP-shaped warning set, strongest first), `alert_credits` (step 15), the `air` object
  (step 16: the best-effort air-quality reading — both raw indices, the six pollutants, the
  nullable pollen block and the `units` pair) and the `astro` object (step 17: the locally
  computed moon block — phase, illuminated fraction, age, rise/set, the next four instants — and
  the sun block with its `source`; `null` unless the run asked). The key sets are listed in
  `docs/schema.md`; the CAP subset actually parsed is the list at the top of
  `docs/plans/15-alerts-and-severity.md`.
* Formats: `art-table` (default, wttr.in's classic four-row coloured columns), `one-line`
  (wttr.in-compatible `%` tokens), `plain`, `json`, `alerts` (the full severe-weather warning
  listing; `no active weather alerts` when there are none), `aqi` (the standalone air-quality
  panel, step 16; `air quality unavailable` when the best-effort fetch failed) and `moon` (the
  standalone moon/sun view, step 17). `dumb` is not a
  fourth layout: it is the art table in the ASCII character set (`+ - |`, ASCII art, no degree
  sign), selected by
  `--format dumb` and automatically for `TERM=dumb` or a non-UTF-8 locale.
* Width handling: `--width` > `COLUMNS` > terminal size > 80, and never below 20 columns (a
  narrower source is raised and reported under `--verbose`). The terminal's own size comes from
  `rustix::termios::tcgetwinsize` (introduced by step 07; the crate forbids `unsafe`, so a raw
  `ioctl` is not an option, and non-Unix targets skip this tier). Below 60 columns the table
  degrades to a stacked layout; the table formats never emit lines wider than the resolved width.
  `plain` and `json` are record formats and **ignore the width**: truncating a record would delete
  the values the format exists to carry, and a pipe wraps or not at its leisure (step 08).
* Colour: honour `NO_COLOR` (present with any value, an empty one included, disables),
  `CLICOLOR_FORCE` (set and not `0` enables, and wins over `NO_COLOR`), `--color auto|always|never`,
  and non-tty stdout ⇒ no colour in `auto`. An explicit `always` emits escapes even into a pipe; the
  palette is folded to the sixteen ANSI colours when `TERM`/`COLORTERM` advertise no more. Palette is
  re-authored 256-colour (temperature ramp, wind, rain), not copied from wego.
* **Attribution is part of the output contract.** Displaying a place or a forecast is displaying
  someone's data, so the credit travels with it: `Location data based on GeoNames (CC-BY-4.0) via
  Open-Meteo — https://open-meteo.com/` for a geocoded name (CC-BY-4.0 asks for credit plus a service
  link next to the data), `Location data © OpenStreetMap contributors (ODbL)` for `~` results and
  `Weather data by Open-Meteo.com (https://open-meteo.com/)` for an Open-Meteo forecast; a direct
  GeoNames search (step 27) carries `Location data by GeoNames (CC BY 4.0) —
  https://www.geonames.org/` instead of the Open-Meteo-via wording. `geo::attribution_line` is the
  single place that decides the location-side text; every renderer
  that shows upstream data — `plain`, `art-table`, `one-line` and the `json` envelope — carries the
  matching credit, on stderr where the format is meant to be piped. Step 08 fixed the three kinds:
  `plain` and the `json` `attribution` object keep the credits in the document, `art-table` keeps
  them in its footer, and `one-line` — one line by contract — prints them to stderr.
* Coordinates and IP answers carry no credit: the first is the user's own input, and none of the IP
  services (`ipwho.is`, `ipapi.co`, IP.SB) asks for one (the `--ip` disclosure already names the
  service).
* Art blocks and translated condition strings are **re-authored** in this repo. Copying wego or
  wttr.in source, data files or art is forbidden (see AGENTS.md).

### Config and state (binding)

`$XDG_CONFIG_HOME/cirrocast/` (`~/.config/cirrocast/`), `$XDG_CACHE_HOME/cirrocast/`,
`$XDG_DATA_HOME/cirrocast/`; resolved with `etcetera` so `XDG_CONFIG_DIRS` is respected for reads.

```toml
schema_version = 1
[defaults]  provider = "open-meteo"  format = "art-table"  units = "metric"  days = 3  language = "auto"
[location]  default = ""            # "Beijing", ":Beijing", "@39.9,116.4", "~Tsinghua"
            pick = "auto"           # auto | never (never = always take the ranked winner; step 26)
[geo]       strategy = "auto"       # bundled table first, network fallback (step 18)
            search = "auto"         # auto | open-meteo | geonames | nominatim (step 27)
            reverse = "auto"        # auto | offline | off — coordinate naming (step 27)
[units]     # per-quantity overrides; an absent (or empty) key follows defaults.units
            # temp = "c"  wind = "kmh"  pressure = "hpa"  distance = "km"  precip = "mm"
[network]   timeout_secs = 15  retries = 3  proxy = ""  nominatim_url = ""
[cache]     enabled = true  weather_ttl_secs = 600  ip_ttl_secs = 86400  geocode_ttl_secs = 2592000
            # ip_ttl_secs is capped at 86400: ipapi.co's terms allow caching an IP answer for at most 24 hours
[render]    color = "auto"  width = 0
[alerts]    enabled = true  severity_threshold = "minor"  sources = ["auto"]  fpas_url = ""  cache_ttl_secs = 300  # step 15
[normals]   period = "1991-2020"  max_distance_km = 60          # step 29
[providers.metar]    station = ""
[providers.qweather] host = ""
```

* These keys are addressed as dotted paths (`cirrocast config get defaults.days`). The ones that
  also exist as command line flags carry a `CIRROCAST_*` override — `PROVIDER`, `FORMAT`, `UNITS`,
  `DAYS`, `LANG`, `LOCATION`, `TIMEOUT` — and `config get` prints the environment value when set.
* API keys are **never** written to `config.toml`. Precedence (first hit wins):
  `CIRROCAST_<PROVIDER>_KEY` env var → `keys.toml` in the config dir with mode `0600`
  (`cirrocast key set/rm/list`). There is no third tier: OS keyring storage is explicitly out of
  scope for v1 (step 10, `## Out of scope`), so a key in neither place is simply missing.
* Providers with a second authentication mode store a `[jwt.<provider>]` table in the same `0600`
  `keys.toml` (step 25, QWeather): the PEM private key plus its non-secret identifiers
  (`credential_id`, `developer_id`, `project_id`). Resolution for such a provider checks the JWT
  environment quartet → `[jwt.<provider>]` → the API-key tiers above; a *partial* JWT set is
  `Error::Config`, never a silent fall-through. A PEM is read from a file or stdin at `key set` time
  and never from argv; tokens are minted per fetch and never written to disk.
* A `keys.toml` with any group/other permission bits is refused with `Error::Config`, not silently used.
* Cache layout: `geocode/<sha256(query)>.json`, `ip/<service>.json`,
  `weather/<provider>-<lat.2dp>-<lon.2dp>-<days>-<local-date>.json`,
  `weather/metar-<ICAO>-{current,taf}.json` for the station resources, `station/<ICAO>.json` for
  30-day station metadata, and `alerts/<source>-<lat.2dp>-<lon.2dp>-<utc-hour>.json` for alert
  responses (a CAP document fetched per identifier hashes the identifier instead of a place);
  writes are `tmp` + `rename`. Steps 28 and 29 add two namespaces with the
  same discipline: `grid/<provider>-<lat.3dp>-<lon.3dp>.json` (a provider's coordinate → grid/point
  mapping, 30 days) and `normals/<station>-<YYYY-MM>.json` (climate normals, 30 days). `cache stat`
  reports every namespace it finds.
* `--no-cache`, `--refresh`, `--offline` (cache-only, never touches the network), `cache stat`,
  `cache clean`.

### Error handling and exit codes (binding)

`Error` (thiserror) variants map to stable exit codes: `0` success, `1` generic, `2` clap usage,
`3` network/upstream, `4` config/state, `5` location not found, `6` missing/invalid API key.
`main.rs` prints `error: <msg>` to stderr plus `--verbose` cause chains, and never panics on
user-triggered conditions. `unwrap`/`expect`/`panic!`/indexing that can fail are not allowed outside
tests; `unsafe` is denied by lint.

### CLI surface (binding, own design — not wego's)

```
cirrocast [OPTIONS] [LOCATION]
  -p, --provider <ID[,ID...]>   open-meteo | openweathermap | weatherapi | worldweatheronline
                                | pirateweather | qweather | smhi | metar | auto
  -f, --format <NAME>           art-table | one-line | plain | json | dumb | alerts
  -d, --days <N>                0..=14 (clamped per provider, warned once)
  -u, --units <metric|us|uk>
      --lang <TAG>              BCP-47, or "auto"
      --lat <DEG> --lon <DEG>
      --ip                      locate from the public IP
      --station <ICAO>          METAR station; selects `metar` when no provider is given
      --alerts                  fetch severe-weather warnings (auto-on from `[alerts] enabled`)
      --no-alerts               do not fetch warnings in this run
      --alerts-from <LIST>      explicit alert sources: nws, meteoalarm, qweather, hko, wmoswic, fpas
      --severity <LEVEL>        lowest alert severity to show (unknown..extreme)
      --no-cache / --refresh / --offline
      --timeout <SECS>
      --color <auto|always|never>
      --width <COLS>
      --pick / --yes            force the candidate picker / take the ranked winner (step 26)
  -q, --quiet   -v, --verbose (repeatable)
  -h, --help    -V, --version

cirrocast config   <path|init|show|get|set|edit|validate>
cirrocast key      <set|rm|list>            # `key set qweather --jwt …` (step 25)
cirrocast provider <list|info>
cirrocast cache    <stat|clean>
cirrocast location <search>                 # `location search --all` lists the ranked candidates (step 26)
cirrocast completion <shell>    cirrocast man
```

Location argument syntax: bare `Beijing` = fuzzy search; `:Beijing` = exact name match; `~Tsinghua` =
OpenStreetMap/Nominatim; `@39.9,116.4` = coordinates; empty = config `location.default`, else public
IP. Fuzzy matches are ranked deterministically (exact-name, then population, then source order) and
the chosen location is echoed in the header. When a lookup yields **more than one candidate** — a
name search, the offline table, a coordinate or IP answer named by several nearby places — the tool
asks: on a terminal (stdin and stderr) the ranked list is printed on stderr and one line is read
(step 26); `--pick` forces the prompt, `--yes`/`[location] pick = "never"` and any non-terminal run
take the ranked winner and print the extended note, so a script is never prompted and never hangs.
Aborting the choice is `Error::Location` (exit 5). Coordinates (`@lat,lon`) remain the only
geocoder-independent spec and are what the selection echo prints back.

### Licensing and REUSE (binding)

* Project licence: **GPL-3.0-or-later**. The full text lives at `./LICENSE` (so GitHub detects it)
  and is exposed to REUSE through the sibling link `LICENSES/GPL-3.0-or-later.txt`.
* The repository is [REUSE](https://reuse.software/) compliant: every file carries SPDX
  file-copyright and licence tags, either in a comment header (native comment syntax of the file) or,
  for files that cannot hold comments (`Cargo.lock`, `tests/fixtures/**/*.json`, binary fixtures), as a
  `[[annotations]]` entry in `REUSE.toml`.
  <!-- REUSE-IgnoreStart -->
  The header form is:

  ```
  SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
  SPDX-License-Identifier: GPL-3.0-or-later
  ```

  <!-- REUSE-IgnoreEnd -->
* `reuse lint` must pass and is part of every step's exit criteria and of CI. Never add a file
  without licensing information; when a new non-commentable file type appears, extend `REUSE.toml`.
* Copied third-party material (art, data, fixtures) keeps its upstream licence and must be listed in
  `REUSE.toml` with its own `SPDX-License-Identifier` — never relicensed silently to GPL.
* `Cargo.toml` declares `license = "GPL-3.0-or-later"`; no `MIT OR Apache-2.0` relicensing of the
  project itself. Dependencies keep their own licences (`cargo deny` review in step 12).

### Conventions

* Rust 2024, MSRV 1.98 (the latest stable toolchain: the project tracks stable rather than holding
  a floor below it), no async runtime, `ureq` + `rustls`, `serde` for all wire formats.
  `tzf-rs` (step 11) is the one coordinate → IANA zone lookup and `quick-xml` (step 15, MIT) the
  one XML reader, used for CAP 1.2 alert documents through a hand-written state machine; the
  manifest accepts any 2.x of the former and 0.42 of the latter, and `Cargo.lock` records the
  resolved release — no dependency is pinned to a version the manifest could not float past.
* New dependencies require a one-line justification in the step doc's design notes; prefer std +
  already-present crates. `cargo deny`/`cargo audit` are introduced in step 12.
* Every provider and renderer ships tests against **recorded fixtures** (`tests/fixtures/`). No test
  may hit the live network; live smoke tests are `#[ignore]`d and run manually.
* Public modules keep doc comments; every step's exit criteria include `cargo fmt --check`,
  `cargo clippy -- -D warnings`, `cargo test`, and a manual smoke run of the changed surface.
* English is used for all code, comments, docs, commit messages and branch names.

### Step file template

```markdown
# Step NN — Title

Status: ⬜ not-started
Depends on: 0X
Touches: src/..., tests/..., docs/...

## Goal
One paragraph: the observable capability added by this step.

## Deliverables
- ⬜ concrete task, with file paths and the API/behaviour it introduces

## Design notes
Decisions, rejected alternatives, dependency justifications.

## Out of scope
Explicitly deferred items and the step that will pick them up.

## Verification
Exact commands and the observable result that proves the step works (smoke run, not only tests).

## Exit criteria
- ⬜ `cargo fmt --check` / `cargo clippy -- -D warnings` / `cargo test` clean
- ⬜ step-specific observable outcome

## Risks
Known unknowns and mitigations.

## Progress log
- YYYY-MM-DD — entry
```
