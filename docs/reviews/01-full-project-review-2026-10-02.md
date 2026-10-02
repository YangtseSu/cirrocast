<!--
SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Full-project review — 2026-10-02

A whole-codebase review of `cirrocast` at commit `172ec60` (v1.0.0, phases A–C of
`docs/plans/README.md` complete). Every finding below is either reproduced against the built
binary or quoted from the source with a `file:line` reference. Nothing here is a style opinion.

This file is a review record, not a plan step. It is not referenced by the architecture contract
and nothing in `src/` depends on it. Reviews live in `docs/reviews/`, numbered
`NN-<slug>-<date>.md` in execution order, the same convention `docs/plans/` uses for steps.

## Method

Gates run at the reviewed commit, all green:

| Gate | Result |
|---|---|
| `cargo fmt --check` | clean |
| `cargo clippy --all-targets --locked -- -D warnings` | clean |
| `CIRROCAST_FORBID_NETWORK=1 cargo test --locked` | all suites pass, 0 failed |
| `reuse lint` | 265/265 files, REUSE 3.3 compliant |
| `cargo deny check` | advisories/bans/licenses/sources ok |
| `cargo audit` | clean |

Behavioural evidence came from running `./target/release/cirrocast` against live upstreams
(Open-Meteo geocoding, aviationweather.gov METAR, an OpenWeatherMap 401) and from a seeded
offline cache that exercises the full render path without a network.

The review was split into seven slices: core plumbing, config/keys, geo/i18n/cli, providers,
render, model/units, and CI/tests/docs. The first pass lost the `render` and `model`/`units`
slices to an out-of-memory crash, so both were re-run and their results are in sections 5 and 7
below. Every finding in this document was re-verified by hand against the built binary before it
was written down; nothing is carried over on a subagent's word alone.

---

## 1. Blockers

### 1.1 The documented `socks5://` proxy example panics the process

`Cargo.toml:56` enables `ureq = { version = "3.4.2", features = ["rustls"] }`. The `socks-proxy`
feature is absent, so a manually configured SOCKS proxy reaches ureq's
`WarnOnNoSocksConnector`, which panics (ureq-3.4.2 `src/unversioned/transport/mod.rs:443`).

The shipped configuration template advertises exactly that value in two places —
`src/config/mod.rs:955` and `docs/schema.md:346`:

```toml
proxy = ""               # e.g. "socks5://127.0.0.1:1080"; empty = connect directly
```

and `src/config/mod.rs:1678` pins it as valid in a test. Reproduced:

```console
$ cirrocast config set network.proxy 'socks5://127.0.0.1:1080'
$ cirrocast location search Beijing
thread 'main' (299406) panicked at ureq-3.4.2/src/unversioned/transport/mod.rs:443:29:
Enable feature socks-proxy to use manually configured proxy
RC=101
```

This violates AGENTS.md golden rule 8 (no `panic!` outside `#[cfg(test)]`) and produces exit
code 101, which is not one of the seven documented codes. A user who follows the documented
example crashes on their first outbound request.

**Fix** — either add `"socks-proxy"` to the ureq feature list, or restrict `is_proxy_url`
(`src/config/mod.rs:706`) to `http`/`https` and drop the `socks5://` example from
`DEFAULT_DOCUMENT`, `docs/schema.md` and the test at `src/config/mod.rs:1678`.

### 1.2 QWeather renders every non-zero precipitation probability as 100%

`src/provider/qweather.rs:454` passes `precipitation.probability` through `fraction()`
(`src/provider/qweather.rs:515`), which multiplies by 100 and clamps to 100:

```rust
/// A 0–1 fraction as a whole percent.
fn fraction(value: f32) -> u8 {
    (value * 100.0).round().clamp(0.0, 100.0) as u8
}
```

But the field is a percentage, as the struct's own doc comment states
(`src/provider/qweather.rs:330`, "The probability in percent") and as `docs/providers.md:575`
agrees. Humidity and cloud cover genuinely are 0–1 fractions; only probability is not.

Reproduced by patching the recorded fixture to `probability: 40` for the hours the existing
assertion covers:

```console
left: Some(100)
right: Some(0)
```

Every day part therefore shows "100% chance of precipitation". The bug is invisible to CI
because `tests/fixtures/qweather/hourly.json` contains only `probability: 0`, and
`tests/provider_qweather.rs:150` (`assert_eq!(..., Some(0))`) pins the buggy value.

**Fix** — clamp probability without scaling, keep `fraction` for humidity/cloud cover, and add a
non-zero-probability fixture row plus assertion so the direction is pinned.

### 1.3 A hostile upstream timestamp panics in `resolve_local`

`src/model/mod.rs:380` uses the `Add` implementation for a time shift:

```rust
LocalResult::None => {
    let shifted = naive + TimeDelta::hours(1);
```

`NaiveDateTime + TimeDelta` panics on overflow, and `naive` arrives straight from an upstream
timestamp (`src/provider/open_meteo.rs:489`, `src/provider/metar.rs:383`, `:393`). A body
carrying `"sunrise": "+262142-12-31T23:30"` with `"timezone": "America/Santiago"` — a zone
whose `LocalResult::None` branch is taken at `NaiveDateTime::MAX` — aborts the process.
Reproduced against the release binary with a seeded cache entry:

```console
thread 'main' (330534) panicked at src/model/mod.rs:380:27:
`NaiveDateTime + TimeDelta` overflowed
RC=101
```

This is the second finding that produces an undocumented exit code from user-reachable input,
and it violates the same rule as 1.1. The fallible path is two lines away.

**Fix** — `naive.checked_add_signed(TimeDelta::hours(1))`, mapping `None` to the
`Error::Upstream` already constructed at `src/model/mod.rs:384-388`.

---

## 2. Major

### 2.1 A cache write failure discards a successful fetch and exits 4

`src/cache.rs:524` writes the cache after `fetch()` has already returned a usable body, and
propagates the write error with `?`. `Cache::write` maps every `create_dir_all`/`open`/`rename`
failure to `Error::Config` (exit 4). A read-only or full `$XDG_CACHE_HOME` therefore turns a
working answer into a hard failure:

```console
$ chmod 0555 $XDG_CACHE_HOME/cirrocast
$ cirrocast location search Beijing
error: config error: cannot create .../geocode: Permission denied (os error 13)
RC=4
```

The repository already holds the opposite intent: `src/provider/metar.rs:339` writes with
`let _ = env.cache.write(...)`, treating the cache as best-effort.

**Fix** — log the failure under `-vv` and return the parsed value anyway.

### 2.2 A percent-encoded API key leaks verbatim in error messages and `-vv`

`HttpRequest::full_url` percent-encodes query values with the QUERY set
(`src/http.rs:296`), while `HttpRequest::redact` (`src/http.rs:272-283`) searches the resulting
text for the *raw* secret. A key containing any character outside the RFC 3986 unreserved set
does not match after encoding, so it is printed in full. `KeyStore::set`
(`src/config/keys.rs:110-119`) only trims and rejects empty values, so such a key is accepted.

Reproduced with a key containing `+` against a dead proxy (which forces the request-URL error
path):

```console
$ keys.toml: worldweatheronline = "abc+defSECRETVALUE123"
$ cirrocast --provider worldweatheronline Beijing -f plain
error: all providers failed: worldweatheronline (network: GET https://api.worldweatheronline.com/
  premium/v1/weather.ashx?key=abc%2BdefSECRETVALUE123&q=39.9075%2C116.3972&...)
```

The raw spelling appears 0 times and the encoded spelling once, so the redaction missed it
entirely. Three providers put the key in a query string and are affected:
`src/provider/worldweatheronline.rs:87`, `src/provider/openweathermap.rs:156`,
`src/provider/weatherapi.rs:77`. A key with only `[A-Za-z0-9]` is redacted correctly.

**Fix** — in `redact`, replace both the raw secret and its percent-encoded form, longest first.

### 2.3 `config set` deadlocks when the file already holds two invalid values

`Config::set_key` ends with `self.validate()` (`src/config/mod.rs:1302`), which checks every
field of the loaded document. Any pre-existing invalid value blocks every subsequent `set`,
including the one that would repair it. Reproduced:

```console
# config.toml holds days = 99 and render.width = 12
$ cirrocast config set render.width 80
error: config error: defaults.days: 99 is out of range 0..=14
$ cirrocast config set defaults.days 5
error: config error: render.width: 12 is not 0 or within 40..=500
```

Recovery requires hand-editing the file or `config init --force`, which discards every other
setting. The per-key checks already exist inside each match arm; the five arms that do not
check inline (`location.default`, `network.proxy`, `network.nominatim_url`,
`providers.metar.station`, `providers.qweather.host`) need their own field validated instead of
the whole struct.

**Fix** — validate only the key being set. No test covers `config set` against a document that
already holds an invalid value elsewhere.

### 2.4 `config show` ignores the `CIRROCAST_*` overrides that `config get` applies

`ConfigCommand::Show` serialises `Config::load` directly (`src/cli.rs:1489-1495`), while
`ConfigCommand::Get` routes through `Config::get_key`, which applies the environment override
first. The two subcommands disagree about the same key. `config show --help` promises "the
effective configuration" and `docs/schema.md:317` says it "prints the effective values", so this
is documented-behaviour drift on the command the plan names as the debugging entry point.

**Fix** — apply the `CIRROCAST_*` override per key when rendering the document.

### 2.5 `config init` and `config edit` overwrite an effective system configuration

`write_default` (`src/config/mod.rs:837`) always writes `DEFAULT_DOCUMENT`, ignoring the
configuration currently being read. With a system-wide `$XDG_CONFIG_DIRS/cirrocast/config.toml`
in effect (holding `days = 9`, `format = "json"`, `station = "ZBAA"`) and no user file, both
`config init` and `config edit` create a user file full of built-in defaults, silently changing
the effective configuration with no warning. `config set` does not have this problem because it
loads and re-serialises the effective document.

**Fix** — `init`/`edit` should write the loaded configuration, or refuse while a system document
shadows the user file.

### 2.6 `config edit` accepts an unknown key that `config validate` rejects

`edit_config` (`src/cli.rs:1620-1622`) runs `Config::load` plus `config.validate()` but never
`check_known_keys`, so a typo introduced in the editor is accepted with exit 0, while
`ConfigCommand::Validate` (`src/cli.rs:1515`) rejects the same file with exit 4 and names the
unknown key. `validate` exists to catch exactly this, and `edit` is the command most likely to
produce one.

**Fix** — call `check_known_keys` on the loaded document, as `ConfigCommand::Validate` does.

### 2.7 A relative entry in `XDG_CONFIG_DIRS` is honoured

`src/paths.rs:92` splits `XDG_CONFIG_DIRS` and joins each entry with no absoluteness check, so
`XDG_CONFIG_DIRS=.` yields the candidate `./cirrocast/config.toml`, which `Config::locate`
accepts. Reproduced with no user config file present:

```console
$ XDG_CONFIG_DIRS=. cirrocast config get defaults.days   # ./cirrocast/config.toml has days = 11
11
```

The XDG base directory specification requires that paths in these variables be absolute and that
a relative path be treated as invalid and ignored; the crate's own `etcetera` dependency already
does this for `XDG_CONFIG_HOME`.

**Fix** — filter the split entries with `Path::new(entry).is_absolute()` before joining.

### 2.8 Nominatim's retry loop bypasses its own 1 req/s throttle

`Nominatim::fetch` (`src/geo/nominatim.rs:102-106`) calls `self.throttle()` once and then
`self.http.send(...)`, but `HttpClient::send` (`src/http.rs:660-691`) loops internally over
`attempts = (1 + retries).clamp(1, 3)` — three with the shipped default `network.retries = 3` —
sleeping 500 ms and 1 s. A single `search()` can therefore put two requests on the wire 500 ms
apart, breaking both the module's stated invariant (`src/geo/nominatim.rs:6-7`, "it never sends
more than one request per second") and the Nominatim usage policy the file exists to honour. The
retry is also invisible to the throttle stamp, which is written once before the first attempt
(`src/geo/nominatim.rs:110-129`).

**Fix** — throttle per HTTP attempt, or disable retries on the Nominatim client.

### 2.9 Open-Meteo rejects a valid payload when `precipitation_probability` is absent

`hourly_samples` (`src/provider/open_meteo.rs:367-379`) builds a length guard over eight arrays
but omits `precipitation_probability`, which the per-sample loop then reads with `at(...)`
(`src/provider/open_meteo.rs:407-412`). `at` returns `Err` — not `None` — when the index is out
of range. Open-Meteo omits that field from the hourly block when `precipProbability` is
unavailable for a model or day, and `docs/providers.md:122` explicitly requires treating a
missing hour as `None`, "never as zero". As written, a shorter array turns the whole fetch into
`Error::Upstream`.

**Fix** — read the field with `.get(index).copied().flatten()`, or add it to the guard and treat
a short array as all-`None`.

### 2.10 METAR reports volcanic ash as clear sky

`obscuration_code` returns `0` for `VA` (`src/provider/metar/decode.rs:826-834`) and the caller
returns it as a real `Condition` with rank `RANK_OBSCURATION` (`:723-726`), so
`decoded.condition` becomes WMO 0. Because `state.weather_rank` is then non-zero, the fallback at
`src/provider/metar/decode.rs:303` (`if state.weather_rank == 0 { decoded.condition =
sky_condition(...) }`) never runs and the reported sky layers are discarded. A report such as
`KJFK 302351Z ... VA BKN020 ...` renders "Clear sky" — the opposite of the rule the module
documents at `src/provider/metar/decode.rs:52`.

**Fix** — treat an unmapped obscuration as "no weather", so the sky code wins, as documented.

### 2.11 METAR `IC` maps to a WMO code the model does not describe

`precip_code` returns `76` for ice crystals (`src/provider/metar/decode.rs:815`), but `76` is
absent from the 35-row `CODES` table in `src/model/condition.rs:29-137` and is also absent from
`tests/fixtures/model/wmo4677-unknown.txt`. `Condition::from_u8(76).is_known()` is therefore
false, so the observation renders as "Unknown" and `dominant_condition`
(`src/provider/dayparts.rs:218-238`) filters it out, so it can never win a day part.

**Fix** — map `IC` to a described neighbour (79, ice pellets) or add 76 to `CODES` with its WMO
description and a decode test.

### 2.12 The CI render-import gate can be defeated by ordinary Rust

`.github/workflows/ci.yml:98` runs:

```sh
grep -rn 'use crate::\(http\|provider\|cache\)' src/render src/model
```

Three ordinary forms all pass while breaking the rule the gate claims to enforce:
`use crate::{cache, provider, http};` (a grouped import), `use super::cache;`, and a bare path
in a function body such as `crate::cache::Cache::new()`. `AGENTS.md` states the `gates` job
"enforces the render-layer import rule"; today `src/render` and `src/model` do contain no such
import, so the gate is green correctly — the defect is that it does not enforce what it claims.

**Fix** — enforce the boundary at the compiler level (crate-level lint or a narrow accessor), or
at minimum widen the pattern to `crate::\(http\|provider\|cache\)::` without the `use ` anchor.

### 2.13 `cargo deny check` in CI does not use the committed lockfile

`.github/workflows/ci.yml:119-121` runs `command: check` without `--locked`, so cargo-deny
re-resolves the graph; `cargo metadata` without `--locked` updates `Cargo.lock` when the manifest
and lockfile disagree. A dependency set that the build would refuse to use can still pass the
licence and advisory gate. The `test` job does use `--locked`, but nothing binds the two.

**Fix** — pass `--locked` (or `--frozen`).

### 2.14 REUSE relabels recorded third-party payloads as GPL-3.0-or-later

`REUSE.toml:26-31` places `tests/fixtures/ip/**` and `tests/fixtures/http/**` in the first-party
GPL block, directly under the comment at `REUSE.toml:21-22` promising that "no recorded upstream
payload is ever relicensed to GPL by a blanket rule". The repository's own records contradict
that:

- `README.md:371` records ipwho.is terms as "personal or internal use, no redistribution";
- `README.md:372` records ipapi.co as "internal use, no resale";
- `docs/plans/05-http-cache-and-ip-location.md:104` states `ipapi_co_error.json` "is a verbatim
  recording";
- `docs/plans/06-open-meteo-provider.md:91-92` identifies
  `tests/fixtures/http/open_meteo_error_invalid_param.json` as a verbatim Open-Meteo 400
  envelope, while the same provider's other four payloads are correctly annotated CC-BY-4.0 at
  `REUSE.toml:69-72`.

Declaring GPL over these asserts a redistribution grant the terms do not permit, and gives the
same provider's data two different licences depending only on the directory it was filed under.

**Fix** — annotate `tests/fixtures/ip/**` with `LicenseRef-Proprietary-API-Data` (already used
for other proprietary payloads) and the Open-Meteo 400 envelope as CC-BY-4.0.

---

## 3. Language negotiation and output-message defects

### 3.1 `--lang en` reports that English is unsupported

`catalog_for` (`src/i18n.rs:988-993`) matches the exact catalog tags, then special-cases the `zh`
family at the language level. Nothing does the same for `en`, so any `en-*` tag takes the
`Note::Fallback` branch:

```console
$ cirrocast --lang en Beijing
warning: unsupported language "en", falling back to en-US (available: en-US, zh-CN)
```

The rendered output is the language the user asked for. The same warning appears for `en-GB`,
`EN` and `fr`. Reproduced against the built binary.

**Fix** — give `en` the same language-level fallback `zh` has, so any `en-*` resolves to
`LanguageId::EnUs` without a note.

### 3.2 `--lang` never normalises a POSIX locale spelling

`LanguageRequest::parse` only tries `request.parse::<LanguageIdentifier>()`; `normalize_locale`
(`src/i18n.rs:453`), which strips `.codeset`/`@modifier` and maps `_` to `-`, is used solely on
the ambient environment path. The two paths therefore disagree:

```console
$ LANG=zh_CN.UTF-8 cirrocast Beijing          # selects zh-CN
$ cirrocast --lang zh_CN.UTF-8 Beijing
warning: unsupported language "zh_CN.UTF-8", falling back to en-US
```

The module doc (`src/i18n.rs:16`) and `docs/plans/09-localization.md:30` both state the
normalisation as part of negotiation, whose first tier is `--lang`. It also corrupts the `-v`
line, which reports `i18n: requested zh_CN.UTF-8 → selected en-US (chain zh-CN → en-US)`,
implying zh-CN was consulted when the tag was rejected outright.

**Fix** — normalise inside `parse` before parsing.

### 3.3 A Fluent pattern error returns the half-rendered pattern, not the key

The doc comment for `I18n::format` (`src/i18n.rs:681-709`) states that a pattern error returns
the raw key "so a mistyped argument in a catalog is visible in the output instead of printing
nothing". The code records the note and then returns `rendered`:

```rust
if !errors.is_empty() {
    self.missing(&name);
}
Cow::Owned(rendered.into_owned())
```

`fluent-bundle` writes a literal `{$name}` for an unresolved variable, so a catalog message
referencing a variable its call site does not pass emits `{$x}` into the middle of a table. The
completeness test cannot catch it: `tests/i18n.rs:105-119` supplies all eleven arguments to
every key, so a per-call-site argument shortfall is invisible.

**Fix** — `if !errors.is_empty() { self.missing(&name); return Cow::Owned(name.into_owned()); }`.

### 3.4 Only the first missing catalog key of a run is reported

`reported_missing` is a single `Cell<bool>` (`src/i18n.rs:843-847`), so the first call sets it and
every later call returns early regardless of which key is missing. The per-key
de-duplication immediately below is therefore unreachable. The doc says "at most once per key";
the behaviour is at most once per run, so a catalog missing two distinct keys reports only the
first under `-v`. The unit test (`src/i18n.rs:1277-1290`) looks up the same key twice and cannot
observe it.

**Fix** — drop the `Cell<bool>` and keep the per-key scan.

### 3.5 The `-v` missing-key dump runs before any renderer can miss a key

`render_notes` (`src/cli.rs:844-847`) is called right after `RenderContext` resolution, before
`fetch_chain` (`src/cli.rs:886`) and before the render (`src/cli.rs:919`). The loop at
`src/cli.rs:1144-1148` filters `i18n.notes()` down to `Note::MissingKey`, but at that point the
only notes that can exist are the negotiation ones; renderers are the only callers of
`text`/`format`. The diagnostic is dead code.

**Fix** — print the notes after the render.

### 3.6 16 direction keys escape the catalog completeness test

`RENDERER_KEYS` (`src/i18n.rs:265-337`) is documented as "every static message key a renderer, a
token or the CLI can ask for" and is the input to the completeness test. It enumerates PARTS,
WEEKDAYS, MONTHS and UV_BANDS element-wise but contains no `keys::DIRECTIONS[…]` entry, even
though `direction_key` (`src/i18n.rs:926-938`) is their only producer and three renderers call
`I18n::direction` (`src/render/art_table.rs:850`, `plain.rs:136`, `one_line.rs:479`). The
render half of `every_catalog_resolves_every_key_through_the_bundle`
(`tests/i18n.rs:148-163`) therefore never renders a single `dir-*` message, and only the key-set
equality half covers them.

**Fix** — add the 16 entries to `RENDERER_KEYS`.

### 3.7 `-v` misreports `--lat/--lon` as coming from the config

`Sources::read` (`src/cli.rs:659-661`) derives `location` from the positional argument, whose
only sources are the positional and `CIRROCAST_LOCATION`. `location_arg`
(`src/cli.rs:1003-1009`) folds `--lat/--lon` into `settings.location` afterwards, leaving
`self.location` as `Source::Default`:

```console
$ cirrocast -v --lat 39.9 --lon 116.4 --offline
location: @39.9,116.4 (from the config or the built-in default)
```

**Fix** — derive the source from `query.lat.is_some()`, or record it where `location_arg`
substitutes.

### 3.8 The help epilogue advertises a `--location` flag that does not exist

The CONFIG PRECEDENCE table at `src/cli.rs:129` reads `--format CIRROCAST_FORMAT --units
CIRROCAST_UNITS --location CIRROCAST_LOCATION`, in the same flag-then-variable shape as every
other row. There is no `--location` flag: the location is the positional `[LOCATION]` with
`env = "CIRROCAST_LOCATION"`. Verified: `cirrocast --location Beijing` fails with "unexpected
argument".

**Fix** — spell the row as `LOCATION CIRROCAST_LOCATION`.

### 3.9 Two error messages point at a command that cannot do what they promise

- An unknown location says "check the spelling or run `cirrocast location search <query>` to see
  the candidates", but `location search` prints the single resolved place, not the candidate
  list. Verified: `cirrocast location search Tsinghua` fails with the same exit-5 error and
  shows nothing.
- An unknown METAR station says "find a nearby one with `cirrocast location search <place>`", yet
  that command cannot resolve airports: `cirrocast location search "Beijing Intl"` fails with
  exit 5 even though `--station ZBAA` resolves to exactly that place.

**Fix** — the station message should point at coordinates, which do work
(`cirrocast -p metar @40.08,116.60` resolves ZBAA via the nearest-station table), or the
location error should list the candidates it already has.

### 3.10 An upstream JSON error message is not length-bounded

`error_message` (`src/http.rs:753-762`) truncates to 200 characters only on the non-JSON
fallback. A JSON envelope's `reason`/`message` field is returned verbatim, so an upstream
answering `{"error":true,"reason":"<8 MiB>"}` under the `MAX_BODY_BYTES` cap yields an
`Error::Upstream` whose message is that whole body, and `chain_reason` copies it into every
`Error::Chain` attempt entry.

**Fix** — apply the same bound to the extracted field.

---

## 4. Provider decoding semantics

### 4.1 QWeather collapses freezing drizzle into fog

`condition_of` (`src/provider/qweather.rs:559-560`) maps the whole `500..=515` range to `45`
(fog). Per QWeather's published list the 500 family is mist/fog/haze/sand/dust through 514;
515 is freezing drizzle. Reporting freezing drizzle as "Fog" is a wrong observation rather than
an honest lossy collapse. The test asserts `is_known()`, never the semantic, so the
mislabelling passes.

**Fix** — give 515 its own arm mapping to 56, and assert the semantic in the test.

### 4.2 The QWeather host is not validated, so the key can be sent anywhere

`host()` (`src/provider/qweather.rs:167-171`) checks only that the value starts with `http://` or
`https://`, then `request()` (`:178-186`) sends the credential in the `X-QW-Api-Key` header to
`<host>/weather/v1/...`. A config value of `https://attacker.example` exfiltrates the user's
QWeather key, and an `http://` value sends it in cleartext. `docs/providers.md:563-566` records
that the host is per-account and that the shared legacy domains answer `403 Invalid Host` —
which is the constraint the validation should encode.

**Fix** — require `https` and validate against the documented `<account-id>.re.qweatherapi.com`
shape. Related: `providers.qweather.host` is also accepted as a bare `http://` by
`validate_providers` (`src/config/mod.rs:605-610`), which then builds the host-less URL
`http:///weather/v1/current/…`; `is_service_url` already exists in that module and is applied to
`network.nominatim_url`.

### 4.3 WWO treats an HTTP-200 error envelope as an empty report

`Data` carries `#[serde(default)]` on both `current_condition` and `weather`
(`src/provider/worldweatheronline.rs:123-130`), and WWO's error envelope is
`{"data":{"error":[…]}}` — the shape recorded in `tests/fixtures/wwo/error_401.json`. That body
deserialises successfully into an empty `Data`, so a bad key or an exhausted quota becomes either
`Error::Upstream("the response covers no complete local day")` or, for `-d 0`, a successful
`Report` with `current: None` and no days (`src/provider/worldweatheronline.rs:279-283` skips the
empty-`forecasts` guard when `days == 0`) — exit 0 with a misleading "no forecast days".

**Fix** — detect the `data.error` envelope explicitly.

### 4.4 Pirate Weather's `-999` sentinel is not applied to three fields

`value()` (`src/provider/pirateweather.rs:426-429`) filters the documented `-999` sentinel for
temperature, apparent temperature, pressure, wind, gust, bearing, visibility and UV index, but
`humidity.map(fraction)`, `cloud_cover.map(fraction)` and `precip_probability.map(fraction)`
(`:355`, `:361`, `:375`, `:387`, `:393`) bypass it. A `-999` humidity becomes
`clamp(-99900, 0, 100)` = 0% — a plausible-looking dry reading invented from a missing value,
which contradicts the aggregation rule in `src/provider/dayparts.rs` that a missing reading must
never become a plausible-looking zero.

**Fix** — route all three through `value()`.

---

## 5. Canonical model and unit layer

### 5.1 Non-finite weather readings render as `inf` and break the JSON schema

No formatter guards `f32::INFINITY` or `f32::NAN`, and nothing in `src/` calls `is_finite` on a
weather reading — only coordinates (`src/geo/tz.rs:35`, `src/geo/mod.rs:365`,
`src/geo/nominatim.rs:274`) and CLI flags (`src/cli.rs:294`) are checked. Reproduced with a
seeded Open-Meteo body carrying `"temperature_2m": 1e39`:

```console
$ cirrocast --offline -d 1 -f plain --lang en-US @39.90,116.40
current: Clear sky inf°C (feels 13°C) wind 13km/h NW ...

$ cirrocast --offline -d 1 -f json @39.90,116.40
    "temp_c": null,
```

The two formats disagree about the same report: the text renderer prints `inf°C` while
`serde_json` writes a non-finite `f32` as `null`, and `docs/schema.md:69` declares
`current.temp_c` a non-nullable `number`. `tests/render_json.rs:200` would reject such a fixture.

**Fix** — reject non-finite values in the provider `require`/`sample` path as the existing
`Error::Upstream`, so a stored canonical value is always a real reading.

### 5.2 METAR rounds at decode time and again at format time, so metric wind is wrong

`round_speed` and `round_two` (`src/provider/metar/decode.rs:846-853`) round the converted value
at decode time, and `fmt_small`/`fmt_1dp` (`src/model/units.rs:499-511`) round again at format
time. When the first rounding crosses the second threshold they disagree. 17 kt is exactly
31.484 km/h; the decoder stores 31.5 and `fmt_small` prints 32. Reproduced end to end with a
seeded `29017KT` observation:

```console
$ cirrocast --offline --station ZBAA -f plain --lang en-US
current: Clear sky 16°C wind 32km/h WNW ...      # exact value is 31
```

A sweep of 0–250 kt finds the same divergence at 17, 34, 44, 51, 63, 69, 71, 78, 98, 105 and
132 kt. The same applies to the altimeter (`A2984` → 1011 hPa where 1010.4988 gives 1010) and to
the precipitation remark (`P0112` → 3.1 mm where 3.048 gives 3.0). This breaks the "round once,
at format time" rule the unit module states, and it makes the default metric METAR output wrong
at roughly 4% of reported wind speeds.

**Fix** — drop `round_speed`/`round_two` and let `units` be the only rounding point.

### 5.3 `parts` order is unenforced and two renderers disagree about it

`DayForecast.parts` is `[DayPart; 4]` (`src/model/mod.rs:224`) with a redundant per-element
`kind`, and serde accepts any order. A shuffled array deserialises without complaint, after
which:

- `src/render/json.rs:261-265` indexes **positionally**, so a Night reading is published as
  `morning.temp_c`;
- `src/render/one_line.rs:413` indexes **by `kind.index()`**, so a noon observation reads the
  Morning part;
- `src/render/plain.rs:164` and `art_table.rs:395,509` iterate and use `part.kind`, which is
  correct.

Three renderers, two answers, no error. The defect is latent rather than live today — no
production path deserialises a `Report` (see 5.6) — but it is a trap for the first feature that
does, and nothing in the test suite would catch it.

**Fix** — make the index authoritative (`DayForecast::part(kind)` used everywhere, or `kind`
derived from the index) and reject a mismatched order on deserialisation.

### 5.4 The half-away-from-zero nudge stops being a nudge at 2^22

`round_half_away_from_zero` (`src/model/units.rs:465-467`) is
`(value * (1.0 + f32::EPSILON)).round()`. The nudge is *relative*, so it grows with the
magnitude: at `|value| >= 2^22` one ulp is already `>= 0.5` and the function returns one more
than the nearest integer. Verified: `fmt_int(4194303.0)` = `"4194304"`,
`fmt_int(8388608.0)` = `"8388609"`, and `format_wind(f32::MAX, WindUnit::Knots)` prints a
39-digit number. The doc's claim that the nudge is "far below anything a weather reading carries"
holds for readings but not for a `pub fn` any future caller can reach.

**Fix** — nudge only when the value is genuinely at a tie, or snap after the `*10.0`/`*100.0`
scaling that introduces the representation error rather than scaling the value itself.

### 5.5 `UnitSystem::resolve` rejects an empty override the config layer accepts

`parse_override(Some(""))` returns `Err(Usage "unknown wind unit ``")` at
`src/model/units.rs:319`, while `empty_as_none` (`src/config/mod.rs:274-281`) maps `""` to
`None` before `resolve` sees it. Through the CLI the two agree (`units.wind = ""` renders
`19km/h`, verified), but `resolve` is `pub` and its own doc names "a caller which built the
override set by hand" as the audience that must not smuggle a bad unit in — that caller gets
exit 2 for a spelling the config file accepts.

**Fix** — pick one behaviour and pin it with a test.

### 5.6 "The same types are the cache payload" is false

The module doc at `src/model/mod.rs:15-17` states the canonical types are what the cache stores,
but `CacheEntry` holds the raw upstream body as a `String` (`src/cache.rs:288-302`) and no
`from_str::<Report>` or `from_value::<Report>` exists anywhere in `src/`. The `json` output is a
separate projection in `src/render/json.rs` with renamed keys (`observed_at` → `time`, `tz` →
`timezone`, `temp_min_c` → `min_c`, `fetched_at` → `retrieved_at`, `weather` →
`condition.code`). A new field must be added in three places, and the model's `Deserialize`
impls are exercised only by test fixtures.

This is good news for robustness — the hostile-document probes in section 8 all stay a
fixture-level concern — but the doc comment should say what is actually true.

### 5.7 `#[serde(default)]` on three `Option` fields is dead

`src/model/mod.rs:85`, `:250` and `:253` carry `#[serde(default)]` on `Option` fields, which
serde already maps to `None` when absent. Only `display_name` (a `String`) needs the attribute.
The `station` comment ("so documents written before this field still deserialise") describes a
migration serde performs anyway.

### 5.8 Coverage gaps in the model and unit layer

- **No assertion for any reverse conversion.** The unit module has no inverse function (metric
  is canonical), but the provider-side native → metric conversions are the reverse direction,
  and two are nearly unpinned: `KT_TO_KMH = 1.852` (`src/provider/metar/decode.rs:69`) would
  survive as `1.85` or `1.86` across the whole METAR suite, because every knot fixture (0, 2, 3,
  7) is a speed where both divisors round to the same 0.1 km/h; `SM_TO_KM` likewise survives
  `1.609` because `10SM` → 16.09 against 16.093441, inside `close()`'s 0.005 tolerance. The
  inHg, mm-per-inch and m/s divisors *are* pinned, so the gap is specifically knots → km/h and
  miles → km.
- **`format_temp_signed` has no fixture row.** `tests/fixtures/model/units-cases.tsv` drives
  `format_temp` only. The signed form — the one `art_table.rs:797`, `one_line.rs:470` and
  `i18n.rs:766` actually call — is covered by four doctests, none of them a negative temperature
  in Fahrenheit.
- **`fmt_small`'s negative branch is unreachable.** `rounded < 10.0` is never true for a
  negative value, so `-9.96` prints `-10.0 km/h` where `+9.96` prints `10 km/h`, and the value is
  reachable end to end from `"wind_speed_10m": -9.96`. No fixture row has a negative wind,
  distance or visibility.
- **No hostile or version-shifted `Report` test.** `tests/decoder_robustness.rs` sweeps
  provider payloads; nothing feeds a mutated `Report` to `from_str`, so the shuffled-`parts` case
  in 5.3 and the out-of-range condition codes are unasserted.
- `hpa_to_inhg`'s doctest tolerance admits a `33.863` divisor that no other constant's tolerance
  would; a reciprocal identity (`hpa_to_inhg(33.863_89) ≈ 1.0`, mirroring `km_to_mi`) would
  close it.

### 5.9 Verified clean in the model and unit layer

- **Every conversion constant and formula is correct**: `c * 9/5 + 32`; `/1.609344` (statute
  mile); `/1.852` (knot); `/3.6`; `/33.86389` (inHg); `/1.333224` (mmHg); `/25.4`.
- **Each divisor is pinned by a doctest** against an independent identity at 1e-4 or tighter
  (`kmh_to_mph(16.093_44) ≈ 10`, `km_to_mi(1.609_344) ≈ 1`, `mm_to_in(25.4) ≈ 1`), so a
  fourth-digit typo in any of them fails `cargo test --doc`.
- **`format_temp_signed` is correct in both unit systems**, with sign and magnitude derived from
  the same rounded value so they cannot disagree: `-0.4°C` → `+0°C` (never `-0`), `-5.2°C` →
  `-5°C`, `-5.2°C` in Fahrenheit → `+23°F` (positive in the target unit, which is right),
  `-0.6°C` → `-1°C`, `0.0°F` → `+32°F`. Negative ties match positive ones:
  `round(±2.5) = ±3`, `fmt_int(-23.5) = "-24"`.
- **Rounding otherwise happens exactly once, at format time** — `fmt_int`, `round_1dp`,
  `fmt_1dp` and `fmt_2dp` all funnel through `round_half_away_from_zero` plus `normalise_zero`,
  and no `-0` escapes any formatter.
- **`compass_16` is exact at every boundary and cannot overflow** (`deg % 360` bounds the
  arithmetic at 1481): 11/12, 33/34, 348/349, 359/360, 360, 455 and `u16::MAX` all land in the
  documented sector.
- **`Condition` is genuinely total.** `from_u8` is `const` and cannot fail; an undescribed code
  yields `cond.unknown` / `"Unknown"` / `art_key() == "unknown"` (a real `ART` entry,
  `src/render/art.rs:386`) / rank `0` / all predicates false. `ART` is checked in both
  directions, and `dayparts::dominant_condition` filters on `is_known()` before ranking, so an
  unknown code can never win a day part.
- **The four predicates match `CODES` and `tests/fixtures/model/wmo4677-known.tsv` row for row**
  (35/35, plus a 0..=255 sweep). The ranges do include undescribed codes (52, 54, 58–60, 68–70…),
  which makes the `is_known()` guard load-bearing: dropping it would make code 52 report as
  precipitation.
- **Serde never panics; every hostile document yields a typed error.** `"temp_c": "18"` →
  `invalid type: string`; `null` → `invalid type: null`; a missing field → ``missing field
  `temp_c` ``; a 3-element `parts` array → `invalid length 3, expected an array of length 4`; a
  5-element array → `trailing characters`; `-1`/`256` → `invalid value, expected u8`; `"tz":
  "Mars/Olympus"` → `failed to parse timezone`; `"tz": 0` → `invalid type: integer`; a bad part
  kind → ``unknown variant `dusk` ``; `humidity: 300` → `invalid value`. Extra keys are accepted,
  and a `fetched_at` with `+08:00` is accepted and normalised. No `unwrap` touches user data.
- **Four parts is structural** — the type is `[DayPart; 4]` and serde enforces it on the way in,
  so a renderer can never be handed three or five. Aggregation is provider-side in the location
  zone; no renderer aggregates.
- **`UnitSystem::resolve` cannot half-convert** — all five quantities come from the system
  defaults, each independently overridden, and every `FromStr` rejects the unknown with
  `Error::Usage`. A nonsense override exits 4 naming the key
  (``units.wind: `furlongs` is not one of …``), verified.
- Attribution reaches the output in all four formats (verified live: `Data: Open-Meteo.com
  (CC BY 4.0)` in `plain`, stderr in `one-line`, the footer in `art-table`, `attribution.notice`
  in `json`).

---

## 6. Render layer

### 6.1 Two pairs of 7-bit wind arrows are the same character

`ARROWS` (`src/render/art.rs:464-472`) pairs four Unicode diagonals with two ASCII marks:
`("↗","/")` at index 1 and `("↙","/")` at index 5, `("↘","\\")` at 3 and `("↖","\\")` at 7. Eight
arrows, six distinct ASCII marks. Verified: `wind_arrow(45, Ascii) == wind_arrow(225, Ascii) ==
"/"`, and 135°/315° both give `"\\"`.

This matters because the arrow is not always decoration. In the boxed ASCII table at
`METRICS_W_NARROW = 10` the cardinal label is dropped, so the arrow is the only direction cue,
and the doc at `src/render/art.rs:459-461` claims "the arrow points the way the label beside it
reads" — false for exactly the case where there is no label. A northeast wind and a southwest
wind render identically.

**Fix** — four distinct ASCII marks, and assert `ARROWS.map(|(_, a)| a)` has 8 distinct values.

### 6.2 The `smoke` art block is 5 columns wide where every other block is 7

`src/render/art.rs:316-317` declares `unicode: ["  ∿ ∿", " ∿ ∿", "  ∿ ∿", ""]`. U+223F SINE WAVE
has `East_Asian_Width = N` and measures **1** column, so the rows measure 5, 4 and 5 — under
`ART_W = 7` but unequal to each other, and the ASCII twin `"  ~ ~"` has the same defect. The
second row's metrics therefore start one column left of the first and third:
`"  ∿ ∿   Smoke"` (13) / `" ∿ ∿    +22°C (+24°C)"` (21) / `"  ∿ ∿   ↗ 12km/h NE"` (19). Nothing
overflows — `gap_after` pads to `ART_W` — the block is simply drawn off-centre in its own cell.

**Fix** — pad the rows to a fixed 7 columns, and assert that the unicode and ASCII rows of every
block have equal display width.

### 6.3 `ART_W` is enforced only by a `#[cfg(test)]` test

`gap_after` is `" ".repeat(ART_W.saturating_sub(display_width(art_line)) + GAP)`
(`src/render/art.rs:483-491`). The `saturating_sub` means a 9-column art line yields `GAP = 1`
instead of an error: the metric column shifts left, `cell_row`'s `fit(cell, cell_w)` clips the
metric, and the row stops lining up. `[&'static str; ART_LINES]` pins arity, not width, so
nothing in the build rejects a mis-measured block. All 36 current blocks are correct, so this is
a trap for the next one rather than a live defect.

**Fix** — a `debug_assert!` in the block accessor, or a width-checked constructor.

### 6.4 The stacked layout silently drops wind and precipitation below 37 columns

`src/render/art_table.rs:534-541` documents that the stacked rungs drop fields "before the line is
clipped at all", but the last rung still emits label + temperature + wind + tail behind three
separators, which measures 36 columns. Reproduced with a one-day forecast:

```console
$ cirrocast --offline -d 1 --width 20 @39.90,116.40
  早上    | +29C ...
  中午    | +35C ...

$ cirrocast --offline -d 1 --width 36 @39.90,116.40
  早上    | +29C | ^ 2.5km/h | 0.0mm
```

The width invariant holds — nothing exceeds 20 columns — but a reader at any width from 20 to 36
loses the wind and the precipitation without being told, and there is no rung that fits
`MIN_WIDTH = 20` cleanly.

**Fix** — add rungs that drop the tail first, then the wind, so each fits.

### 6.5 `%U` prints a band that disagrees with the number beside it

`uv_band_key` (`src/i18n.rs:908-921`) cuts on the raw `f32` while `%U` prints `fmt_int(uv)`, which
rounds (`src/render/one_line.rs:499-513`). Measured: `2.9` → `"3 (low)"` but `3.0` →
`"3 (moderate)"`; `5.9` → `"6 (moderate)"` against `6.0` → `"6 (high)"`; `7.9` → `"8 (high)"`
against `8.0` → `"8 (very high)"`. The doc at `src/i18n.rs:740-742` asserts the opposite, and its
worked example is wrong (`fmt_int(2.9) == 3`, not `2`).

**Fix** — cut the band on the rounded value, or print one decimal.

### 6.6 `%w` discards a known wind speed when only the direction is missing

`Token::Wind` (`src/render/one_line.rs:474-482`) matches `(Some(kmh), Some(deg))` and falls back to
`n/a` otherwise. But `wind_dir_deg` is genuinely optional in the model — OpenWeatherMap's
`wind.deg` is absent when calm, and METAR reports `VRB` as `None` — so a report carrying
`wind_kmh: 9.0, wind_dir_deg: None` prints `%w → "n/a"` while the same report renders `9.0km/h`
in `art-table` and `wind 9.0km/h` in `plain`.

**Fix** — match `(Some(kmh), dir)` and print the arrow-free speed alone when `deg` is `None`, as
`art_table::wind_text` already does.

### 6.7 `json` serialises `-0.0` and folds `NaN` into the documented absent-value `null`

`src/render/json.rs:164-184` serialises `f32` fields directly. `serde_json` writes negative zero
as `-0.0`, and `units::normalise_zero` (`src/model/units.rs:487`) — which every display path uses —
is not applied here. More seriously, `serde_json` writes a non-finite `f32` as `null`, which is
indistinguishable from the documented "the provider did not report it" (`src/render/json.rs:12-14`).
That NaN is treated as reachable is consistent with `color::temp_fg`
(`src/render/color.rs:53-56`), which already guards `is_nan()`. This compounds 5.1: the same
upstream value reaches the user as `inf°C` in one format and as a legitimate-looking absent value
in the other.

**Fix** — normalise zero and reject non-finite values before serialising.

### 6.8 `Format::Dumb` is documented as colourless but the renderer paints

`--format dumb` is documented as "the art table in the ASCII character set, with no colour"
(`src/render/mod.rs:428`, `README.md:198`), and `Format::Dumb` is a public variant of a public
enum. But `renderer_for(Format::Dumb, …)` returns `ArtTable::new(Charset::Ascii)` and nothing in
`src/render/` forces `Mono`; the suppression lives in the CLI at `src/cli.rs:1094-1096`. A library
consumer calling the public `renderer_for` with `ctx.color = Always` gets colour escapes in the
"no colour" format.

**Fix** — force `Mono` in the `Dumb` renderer, or move the suppression into `renderer_for`.

### 6.9 The wind arrow and `compass_16` disagree on every sector edge

`wind_arrow` (`src/render/art.rs:477-481`) bins eight sectors with `+22/45`; `compass_16` bins
sixteen with `(*4+45)/90`. Across 22.5°–24.9° the arrow reads north while the label reads NNE, and
so on at every boundary — a wind can render `↗` beside `ENE`. In the narrow ASCII cell, where the
label is dropped (see 6.1), the arrow is the only cue and it is off by up to one 22.5° sector.

**Fix** — derive the arrow index from `compass_16(deg)` so one function owns the edges.

### 6.10 Every table cell `String` is copied three times per row

`columns` builds `per_day: Vec<Vec<String>>`, then clones each row's cells
(`src/render/art_table.rs:370`), and `cell_row` copies again through `fit` — whose
no-truncation path is `line.to_owned()` (`:742`) — and once more in `pad_columns` (`:670`).
Counting global allocator traffic, a 7-day/80-column/mono render performs **2066 allocations
totalling 90 KB to produce 7.5 KB of output**; the stacked layout is worse per output byte, at
2722 allocations for 922 bytes at width 20. Not user-visible today, but it is the hot path and
the amplification is roughly 12×.

**Fix** — pass `&[&str]` into `cell_row` and write into one reused `String`.

### 6.11 Two stale doc comments on `%D`

`src/render/one_line.rs:14` and `:158` document `%D` as `Wed, Sep 30`, but
`date-short = { $weekday } { $day } { $month }` (`locales/en-US/main.ftl:159`) produces
`Wed 30 Sep` — which is what `src/cli.rs:145` and `tests/render_one_line.rs:112` already say.
Only the two comments in the slice are wrong.

### 6.12 A JSON null-vs-absent assertion cannot fail

`tests/render_json.rs:237` asserts `document["current"]["wind_gust_kmh"].is_null()`, which also
passes when the key is *omitted*, because `serde_json::Value` indexing returns `Value::Null` for a
missing key. The same holds for the three location nulls at `:247-249`. The `key_paths` diff at
`tests/render_json.rs:154-192` does catch omission, so this is misleading redundancy rather than a
hole — but a test named "is null and never an omitted key" proves neither half.

**Fix** — assert `as_object().contains_key(...)` alongside `is_null()`.

### 6.13 Coverage gaps in the render layer

- **Colour × width is untested.** Every width test hardcodes `ColorMode::Never`
  (`tests/render_width.rs:55`), and the one colour+width test strips escapes leniently, so it
  cannot see a missing reset. The `debug_assert!` at `src/render/art_table.rs:777` is compiled out
  in release.
- **`json` and `one-line` width-ignoring is untested.** Only `plain` has the two-width
  byte-equality proof (`tests/render_plain.rs:158-180`); the other two hardcode `width: 80`.
- **`plain` with absent values is untested.** The golden at `tests/render_plain.rs:68-72` uses the
  complete forecast fixture, so only the `Some` branches run — a regression printing
  `visibility 0km` for a missing value would pass.
- **`tests/render_width.rs:181-186`** asserts `!contains('┌')`, `lines().count() <= 10` and a
  per-line width loop, all of which the empty string satisfies, so a current-only render
  regressing to `""` passes.
- **The eleven three-cell box-drawing snapshots over-pin** `CELL_W`, `METRICS_W_NARROW`, `PAD` and
  the `│` runs, and `art_gallery.snap` additionally freezes trailing `{:<7}` padding on every row.
  A pure layout change rewrites roughly 6 KB of committed snapshots for no user-visible reason.
  The one-line, json and plain snapshots are well-pinned.
- No assertion covers the `ARROWS` ASCII distinctness (6.1), `uv_band` boundary agreement (6.5),
  or unicode/ASCII art-row width equality (6.2).

### 6.14 Verified clean in the render layer

- **The width invariant holds under colour**, in both languages, at every boundary. Probed across
  widths 20/24/26/30/40/59/60/74/80/120 × en-US and zh-CN × 7 days, counting SGR opens against
  resets: zero unbalanced lines and zero over-width lines. The parity close at
  `src/render/art_table.rs:773-775` is correct.
- **No release overflow hides behind a `debug_assert!`**: the marker-width guard prevents a
  negative budget, `used + cell` cannot exceed the budget, escapes measure zero columns, and
  `columns`' `ctx.width - 1` only runs at `width >= 60`.
- **`border()` and `cell_row()` agree** on `1 + n·(cell_w + 3)`, checked for `cell_w ∈ {18, 21}`
  × `n ∈ 1..3`.
- **No `unwrap`, `expect`, `panic!` or fallible indexing outside `#[cfg(test)]`** in any of the
  seven render files.
- **`plain`, `json` and `one-line` provably ignore `ctx.width`** — byte-identical output at widths
  20 and 200, and none of the three mentions `ctx.width`.
- **The 16-colour fold is total**: `ansi16_from_256` returns from a 16-entry loop so the folded
  index is `0..=15`, `rgb_of` and `cube_level` cannot be asked out of range, and `paint` returns
  `Cow::Borrowed` under `Mono` and for empty text, so no style escapes colour support.
- **No provider-identity branching and no unresolved colour mode read** anywhere in the layer. The
  only capability use is `capabilities.current && !capabilities.daily`
  (`src/render/art_table.rs:141-145`), which the contract sanctions. No `ProviderId` appears in
  `src/render/`.
- **All 36 art blocks have exactly `ART_LINES` lines and none exceeds `ART_W = 7`** — verified by
  walking all 100 WMO codes × day/night × both charsets.
- **The locale key sets are identical** (219 ids each, all 101 `cond-*` present in both), and a
  miss degrades to the key name plus `Note::MissingKey`, never to English text.
- **JSON schema hygiene holds**: `schema_version` comes first, every `Option` serialises as `null`
  with no `skip_serializing_if`, the key set matches `docs/schema.md` in both directions with
  types and nullability, and `--units` provably changes no byte of the output.

---

## 7. Plan and documentation drift

### 7.1 The binding contract states the geocoding ranking order backwards

`docs/plans/README.md:318` says ambiguous fuzzy matches are "resolved by deterministic ranking
(population, then exact-name, then provider order)". The implementation ranks exact-name first and
population second (`src/geo/mod.rs:140-150`), and three other documents agree with the code:
`README.md:325`, `docs/plans/04-geocoding-and-location-syntax.md:18` and `:54`. The order is
observable: for candidates `Beijing City` (pop 9 000 000) and `Beijing` (pop 10) the code picks
`Beijing`, where the documented order would pick `Beijing City`.

**Fix** — correct the binding doc line to match the code.

### 7.2 The contract promises an OS-keyring tier that no step implements

`docs/plans/README.md:271` and `src/config/keys.rs:16-18` both name the OS keyring as the third
key-resolution tier, "only if the `keyring` feature is enabled (step 10+)". Step 10 is
`Status: ✅ done` and its `## Out of scope` (`docs/plans/10-additional-providers.md:200`) lists
"Keyring storage of API keys" as deferred. `Cargo.toml` has no `[features]` table and no
`keyring` dependency, so no code can reach that tier.

**Fix** — amend the contract and the doc comment, and either add a step that owns the backend or
drop the reference.

### 7.3 Step 21 re-purposes a shipped, test-pinned `%L` token without stating the break

`docs/plans/08-cli-surface-and-formats.md:102` ships `%l %L` as "name / 39.90,116.40";
`src/render/one_line.rs:170`, `:199` and the test at `:568` pin it. Step 21
(`docs/plans/21-multi-location-and-templates.md:94`, `:172`) re-assigns `%L` to the day's low
without recording that this breaks a shipped token, and its exit criteria require every token to
have a rendering test — implementing it as written fails the suite in a way the plan does not
predict. Step 21's token list at `:51` also omits `%U`, which step 08's `@uv` preset depends on.

**Fix** — state the breaking change in step 21's Deliverables and add `%U`.

### 7.4 A ticked step-10 exit criterion still describes QWeather as unimplemented

`docs/plans/10-additional-providers.md:217-218` is ticked and asserts "QWeather's `credit:` is
the not-implemented placeholder until its backend lands", while the same file's progress log
records the backend landing on 2026-10-01 and `src/provider/mod.rs:244` now carries
`licence: Some("QWeather — https://www.qweather.com/")`.

**Fix** — drop the parenthetical.

### 7.5 `INSTA_UPDATE=no` is documented in four places and set in none

`docs/plans/07-art-table-renderer.md:119`, `:153` and `:177`, plus `docs/plans/23-docs-and-guides.md:116`
and `tests/render_snapshots.rs:10`, all state that CI sets `INSTA_UPDATE=no` so drift fails
instead of silently rewriting. `.github/workflows/ci.yml` contains no `INSTA` reference. The
behaviour is only accidentally correct: insta's `auto` default writes `.snap.new` and passes
outside CI, and detects `CI` itself inside CI.

**Fix** — export `INSTA_UPDATE=no` in the CI test job.

### 7.6 The module map omits `src/geo/tz.rs`

`docs/plans/11-metar-and-aviation.md` declares the new `src/geo/tz.rs` module and the step is
`✅ done`, but the module map in `docs/plans/README.md:120-132` does not list it.

**Fix** — add the module to the map.

### 7.7 A provider doc comment still describes the pre-step-10 world

`src/provider/mod.rs:765` says "today `open-meteo` alone, `open-meteo,smhi` once step 10 lands",
but `auto_chain()` is filter-driven and `src/provider/mod.rs:1081-1085` asserts
`[OpenMeteo, Smhi]`.

**Fix** — update the comment.

### 7.8 README omits six implemented flags

The README usage block never mentions `-vv` (`src/cli.rs:104`), `cache clean --all`
(`src/cli.rs:431-433`), `cache clean --offline` (`:435-437`), `config validate --offline`
(`src/cli.rs:474-476`), `key set --stdin` (`:551-553`), and `location search --ip`/`--timeout`
(`src/cli.rs:367-371`, `:376-377`). The `-vv` case is self-contradictory: CHANGELOG [1.0.0] says
`--help` documents `v`/`vv` while `README.md:137` lists only `-v`.

### 7.9 `deny.toml`'s duplicate-accounting comment is wrong

`Cargo.lock` contains three duplicated crate names — `getrandom` (0.2.17 via ring, 0.4.3 via
tempfile), `syn` (2.0.119, 3.0.6) and `windows-sys` (0.52.0, 0.61.2) — but the `skip` list at
`deny.toml:60-71` has four entries covering only `syn` and `windows-sys`. `cargo deny check bans`
never flags `getrandom`, because the check ignores a version whose only dependent chain is
reachable solely through a dev-dependency.

**Fix** — correct the comment, or promote `tempfile` so the duplicate is accounted for.

### 7.10 Two stale REUSE provenance claims

- `REUSE.toml:90` asserts "The API key the recording used is replaced by `REDACTED`", but
  `grep -rn 'REDACTED' tests/fixtures/` returns zero matches; the key travels in the query
  string and was never recorded in the body. A false provenance record is worse than none.
- `REUSE.toml:129-133` attributes all of `tests/fixtures/metar/**` and
  `tests/fixtures/stationinfo/**` to NOAA/NWS, but `tests/fixtures/stationinfo/ZZZZ.json`,
  `tests/fixtures/metar/ZZZZ/current.json` and `tests/fixtures/metar/ZZZZ/truncated.json` are
  hand-authored. The repo already carves out the hand-authored SMHI sentinel at `REUSE.toml:97-101`.

---

## 8. Test-suite policy issues

### 8.1 `tests/no_network.rs` performs a real network request

`a_cold_cache_request_is_refused_before_it_reaches_the_network` runs the binary against a real
upstream host, deliberately: the module doc says "there is no stub a separate process could be
pointed at", so a silently broken guard fails on a networked machine. This contradicts the
AGENTS.md rule that no test may open a network connection; the second test in the file
(`unshare -rn`) is the compliant way to prove the same property and is Linux-only.

**Fix** — make the first test `#[ignore]`d with the documented live-test env var, or prove the
same property through the in-process `StubTransport` seam plus the namespace test.

### 8.2 `no_fixture_carries_an_api_key` silently scans nothing on CI

`tests/cli_offline.rs:312-330` reads the developer's own `~/.config/cirrocast/keys.toml` and
returns early when it is absent. On CI (`HOME=/home/runner`, no such file) both `else` arms
return, and the test is green having scanned zero fixtures. It is also machine-dependent.

**Fix** — rewrite it to scan a checked-in deny-list of key prefixes, or gate it on a documented
env var.

### 8.3 `tests/common/mod.rs` never points `HOME` at the sandbox

`tests/xdg.rs:6` documents that "`HOME` is pointed at the same throwaway tree as the three XDG
variables, so a stray write to `~/.config` cannot hide", but `Sandbox::cirrocast`
(`tests/common/mod.rs:88-102`) sets only the XDG variables and never `HOME`.
`every_write_stays_inside_the_xdg_tree` then walks the developer's real home directory. The
product behaviour is currently correct — the binary writes only under `$XDG_*` — but the guard
does not test what it claims.

**Fix** — set `HOME` in `Sandbox::cirrocast`.

### 8.4 A test mutates after the fetch it appears to set up

`tests/provider_open_meteo.rs:316-322`:

```rust
let mut geocoded = fixture_location("lisbon");
let report = run.fetch(&geocoded, 1).expect("the fixture parses");
geocoded.tz = chrono_tz::Tz::Europe__Lisbon;
assert_eq!(report.location.tz, chrono_tz::Tz::Europe__Lisbon, ...);
```

`tests/common/mod.rs:224` already returns `Tz::Europe__Lisbon` for `"lisbon"`, so the assignment
is a no-op applied after the fetch. The assertion would pass identically if the provider
discarded the geocoded zone — the opposite of the stated intent.

**Fix** — set a deliberately different zone before the fetch.

### 8.5 A wiring assertion the snapshots already cover

`tests/render_snapshots.rs:481-487` (`the_fixtures_load_as_reports`) asserts the report fixture
parses, holds three days, and carries `Asia/Shanghai`. `Report` is a hard return type, so a
parse failure would not compile; `render()` at line 88 loads the same fixture for all 24
snapshots; the remaining two assertions restate fixture content. This is the wiring/copy class
AGENTS.md forbids.

**Fix** — delete it, or convert it into the real invariant: iterate over every file in
`tests/fixtures/report/` and assert each loads as a `Report`, so a newly added fixture cannot rot.

---

## 9. Performance and resource notes

- **`src/cli.rs:1287` and `:1294`** deep-copy every geocoder hit (`hits.clone()`, cloning
  `name`/`admin1`/`country`/`country_code`) and rank it twice per geocoded query, purely to
  build a `candidates` vector consumed only under `if cli.verbose > 0` (`:984-992`, `:1210-1219`).
  On a normal run this is a wasted allocation and a second sort. Build it lazily.
- **`src/print_causes` in `src/main.rs:38-41`** is unreachable: no `Error` variant declares
  `#[source]` or `#[from]`, so `error.source()` is always `None` and `-v` never prints a cause
  chain through that path. Harmless, but the documented verbose cause output has no test.
- Warm-cache latency is far inside budget: 2 ms median for `art-table`, `json` and `one-line`
  against a seeded cache, versus the step-14 gate of under one second.

---

## 10. Verified clean

Recorded so a future change does not have to re-derive it.

**Toolchain and policy**

- `cargo fmt --check`, `cargo clippy --all-targets --locked -- -D warnings` and
  `CIRROCAST_FORBID_NETWORK=1 cargo test --locked` are all clean; `reuse lint` reports 265/265
  files compliant; `cargo deny check` and `cargo audit` are green; `cargo publish --dry-run
  --locked` succeeds.
- No `unwrap()`/`expect`/`panic!`/`todo!` outside `#[cfg(test)]` except nine sites that cannot
  fail: constant BCP-47 tag parses (`src/i18n.rs:371`, `:475`, `:480`) and infallible
  `Vec::remove`/`String` helpers (`src/cli.rs:1553`, `src/geo/mod.rs:192`, `src/http.rs:111-114`).
- `unsafe_code` is forbidden crate-wide and no `unsafe` block exists.
- Every third-party CI action is pinned to a commit SHA; runner images are named explicitly; the
  MSRV 1.98.0 is in the test matrix; a concurrency group is present; `LICENSE` is byte-identical
  to `LICENSES/GPL-3.0-or-later.txt`.

**Error taxonomy**

- The nine `Error` variants map to exit codes 0–6 exactly as the contract states
  (`src/error.rs:100-108`, pinned by `exit_codes_follow_the_contract`).
- Verified live: bad coordinates → 2, unknown station → 5, missing key → 6, upstream 401 → 6 with
  the exact `cirrocast key set <id>` instruction, offline cache miss → 3, unknown config key → 2,
  invalid config value → 4.
- `main.rs` sends every diagnostic to stderr and returns `ExitCode::from(error.exit_code())`.

**HTTP and cache**

- Bodies are bounded by `MAX_BODY_BYTES` via ureq's `limit`, mapped to a non-retryable `TooLarge`.
- Retries are GET-only, cover statuses 408/429/5xx, and clamp `Retry-After`.
- `--offline` cannot reach the network: `read_or_fetch_json` returns before `fetch()`, and the
  Nominatim throttle is only reachable from `fetch()`.
- A backwards-running clock is deliberately treated as fresh (`src/cache.rs:311-315`).
- Atomic writes use a sibling temp file, `create_new` + `sync_all` + `rename`, and per-pid names,
  so concurrent processes do not collide.
- `cache stat` reports all four namespaces.

**Privacy and secrets**

- The public-IP lookup fires only from `resolve_location`'s `LocationSpec::Default` arm, reachable
  only via `--ip` or an empty argument with no `location.default`; `--lat/--lon` are folded into
  `@lat,lon` first and `--station` bypasses location resolution entirely. The cache key is
  `ip|ipwho-is` and carries no address.
- `ip_ttl_secs` is capped at 86400 at its single use site; the chain error names every attempted
  service.
- A plain alphanumeric key appears zero times in `-vv` output, including the failing 401 path.
- `keys.toml` with mode 0644 is refused with `Error::Config` and a `chmod 600` hint; `key set`
  refuses the key in argv; `key list` prints only a mask.
- No committed fixture carries a real key; the IP fixtures use the RFC 5737 documentation address
  203.0.113.7.

**Rendering**

- The width ladder never emits an over-wide line. Measured at 20, 24, 30, 40, 55, 100 and 200
  columns with CJK content measured by East Asian width and colour forced via
  `CLICOLOR_FORCE=1`: maximum observed width equals or is below the target in every case, and
  every emitted line carries a balanced number of escape sequences.
- Truncation is genuinely exercised: at 20 columns the header clips to
  `天气报告： 39.9, ...` and the pressure cell to `1021h...`, both width-legal. `fit()`
  (`src/render/art_table.rs:740-782`) copies escapes verbatim and closes an unterminated colour
  before the ellipsis.
- `json` and `one-line` correctly ignore `--width` as record formats.
- Colour precedence is correct: `NO_COLOR=1 --color always` still emits escapes (an explicit
  request wins), and `CLICOLOR_FORCE=1` into a pipe emits escapes.
- Output is byte-identical across runs; no `HashMap`/`HashSet` appears anywhere in `src/`, so no
  iteration order can reach the output. `src/render` and `src/model` contain no `ProviderId`
  reference — renderers cannot branch on provider identity.
- Attribution follows the fixed three-way split: in-document for `plain` and `json`, footer for
  `art-table`, stderr for `one-line`, all via `geo::attribution_line`.

**Providers**

- The fallback chain falls through only on `Network`/`Upstream` errors and aborts on usage, key
  and location errors.
- Day-part aggregation handles DST boundaries and short hourly series; a part with no samples is
  reported as an upstream error rather than a fabricated zero.
- Live end-to-end runs succeeded: `--station ZBAA` returned a real METAR with the correct
  public-domain attribution; `-p metar` resolves the nearest station from coordinates and from a
  city name; `-p smhi` rejects bare coordinates with an actionable exit-2 message.

**Location and configuration**

- The location grammar handles `@91,200`, `@1,181`, `@abc,1`, `@1,2,3`, empty `:`/`~`, and a
  leading-dash argument, all with exit 2 and the accepted-forms hint.
- Ranking is total and input-only (`sort_by` is stable; keys are exact-name desc, then population
  desc, with upstream order as the final tiebreak), and the chosen candidate is echoed.
- Locale negotiation follows the POSIX order `LC_ALL` > `LC_MESSAGES` > `LANG`, skips `C`/`POSIX`
  and unparsable values without stopping the walk, and keeps U+2068/U+2069 out of the output.
- Every catalog has all 100 `cond-*` ids plus `cond-unknown`, and the key sets of the two locales
  are identical.
- A missing catalog key never yields an empty string: `text()` falls back to the key name and
  `format()` falls back to the key.

**Tests**

- All 35 `art_key` values map to an art block, with the coverage asserted at build time
  (`src/render/art.rs:518-552`); no default fallback is reachable for any provider-constructible
  `Condition`.
- All three `#[ignore]`d tests live in `tests/live.rs` and all three read the env var they name.
- No `.snap.new` files are committed, no `--force-update-snapshots` appears anywhere, and every
  snapshot header points at an existing file.

---

## 11. Suggested fix order

Ordered by user-visible severity, not by file. Each item names the finding it closes.

1. **1.1 and 1.3** — the two panics. Both are reachable from input, both exit with the
   undocumented code 101, and 1.1 is reachable by following the shipped configuration template.
   Each fix is a few lines (`socks-proxy` feature or a narrower `is_proxy_url`; `checked_add_signed`).
2. **1.2** QWeather probability — wrong data shown to users, currently masked by a fixture that
   contains only zeros.
3. **2.2** encoded-secret leak — a credential in cleartext on stderr.
4. **5.1** non-finite readings — `inf°C` in text output and `null` in JSON from the same report.
5. **5.2** METAR double rounding — the default metric wind output is wrong at roughly 4% of
   reported speeds.
6. **2.1** cache-write failure — turns a successful fetch into exit 4 on a read-only cache dir.
7. **2.3** `config set` deadlock — leaves a user with no supported way to repair their config.
8. **2.14** REUSE relabelling — a licensing claim the upstream terms do not support.
9. **6.1** colliding 7-bit wind arrows — a northeast and a southwest wind render identically in
   the narrow ASCII cell, and it is a one-line fix with a one-line assertion.
10. Everything else, grouped by module: config/`paths` (2.4–2.7), transport and geocoding
    (2.8, 3.10), providers (2.9–2.11, 4.1–4.4), i18n and CLI messages (3.1–3.9), the model layer
    (5.3–5.7), the render layer (6.2–6.12), CI and supply chain (2.12, 2.13, 7.5, 7.9), plan and
    docs drift (7.1–7.4, 7.6–7.8, 7.10), and the test-policy items (8.1–8.5).

Findings 1.1, 1.2, 2.3 and 5.2 each have a test or fixture that currently pins the buggy
behaviour, so those fixes must land together with the test change. Findings 1.3, 2.1, 2.2, 5.1,
6.1, 6.4 and 6.5 are currently unpinned and need a regression test added with the fix.

## 12. What this review did not cover

- **The live Open-Meteo forecast endpoint was unreachable from the review machine**
  (`api.open-meteo.com` times out; the geocoding host answers). Every Open-Meteo forecast
  rendering claim here comes from a seeded cache, not a live response. The decoder paths were
  reviewed by reading, and the 2.9 finding is an omission visible in the source, but a live
  smoke run of the default backend is still outstanding.
- **Terminal-fidelity claims are approximations.** Width and escape-balance checks used
  `East_Asian_Width` via `unicode_width`/Python, not a real terminal. Emoji, combining marks and
  ambiguous-width characters may render differently than measured.
- **The performance figures in 6.10 and section 9 are allocator counts, not profiles.** No
  `perf`, flamegraph or criterion run was performed.
- **Steps 15–24 of `docs/plans/README.md` were not reviewed** — they are unstarted, so there is
  no code to review, only the plan text, which was checked for internal consistency in section 7.
- **Windows and macOS behaviour was not exercised.** All runs were on Linux; the
  `rustix::termios` terminal-size tier and the `unshare`-based test in `tests/no_network.rs` are
  Linux-specific by construction.
