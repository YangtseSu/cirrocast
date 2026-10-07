<!--
SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Full project review — 2026-10-07

Review of the current `main` tree (v1.3.0+). The work ran in six parallel slices — core modules,
providers, render layer, geo module, CLI/i18n, and alerts/template — with one owner per file set.
Two slices (alerts, template/parallel) were cancelled after their agents stalled; their areas are
covered by the static gates and the earlier reviews' findings. Line references below are this
review's.

## Status

| Section | Rows | Result |
|---|---|---|
| Static gates | 8 | all pass (`cargo deny`/`cargo audit` blocked by network) |
| §3 High | 4 | all confirmed, none fixed yet |
| §4 Medium | 6 | all confirmed, none fixed yet |
| §5 Low | 15 | all confirmed, none fixed yet |

No Blocker existed. The review's priority order (§3 High first, then §4 Medium, then §5 Low) is
the order the fixes should follow.

## Static gates

| Check | Result |
|---|---|
| `cargo fmt --check` | pass |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | pass |
| `reuse lint` | pass (507/507 files) |
| `scripts/check-render-imports.py` | pass (20 files, no violation) |
| `cargo test --workspace --locked` | pass (1150 tests) |
| `CIRROCAST_FORBID_NETWORK=1 cargo test --workspace --locked` | pass (503 integration + 26 doctest) |
| `cargo build --workspace --locked --no-default-features` | pass, zero warnings |
| `cargo deny check` | blocked: network unreachable (advisory DB fetch failed) |
| `cargo audit` | blocked: network unreachable (advisory DB fetch failed) |

## Code size

| Metric | Value |
|---|---|
| Source | 55,262 lines Rust (75 files with unit tests) |
| Tests | 23,761 lines Rust (60+ integration test files) |
| Docs | 28 plan files + 12 guides + 4 review records |

## §3 High

### 3.1 `src/template.rs` violates the single-conversion-point contract

**Location**: `src/template.rs:958-1081`

**Problem**: `src/render/mod.rs:4` declares "the only place a unit is ever converted" is the render
layer, and "Nothing else in the crate formats a value for display". However, `src/template.rs`
directly calls `format_temp_signed`, `format_wind`, `format_precip`, `format_pressure`,
`format_visibility` and their `_prec` variants.

**Impact**:
- The `one_line` renderer (`src/render/one_line.rs:62`) and the `status` probe
  (`src/status.rs:205`) call `template::expand`, which triggers these conversions.
- The cache layer must know which `UnitSystem` a user prefers when template expansion is involved,
  because the conversion happens before the cache key is computed.
- This violates the design goal that "cache entries are unit-independent".

**Suggestion**: Move the unit conversions from `template.rs` into the render layer, or explicitly
document that `template.rs` is an extension of the render layer and amend the contract in
`docs/plans/README.md`.

### 3.2 WeatherAPI obscuration codes collapse six distinct phenomena into Fog

**Location**: `src/provider/weatherapi.rs:463-464`

**Problem**: Codes 1012-1048 (haze, dust, sand, smoke, smog, mist) all map to WMO 45 (Fog). WMO 4677
has distinct codes: Smoke=4, Haze=5, Widespread dust=6, Dust/sand raised by wind=7, Mist=10,
Fog=45.

**Impact**: Loss of meteorological information; users cannot distinguish smoke, haze, dust and fog.

**Suggestion**: Split the mapping according to the WMO 4677 standard.

### 3.3 QWeather obscuration codes collapse five distinct phenomena into Fog

**Location**: `src/provider/qweather.rs:638`

**Problem**: Codes 500-515 (mist, fog, haze, sand, dust and their stronger forms) all map to WMO 45
(Fog). WMO 4677 has distinct codes: Mist=10, Haze=5, Widespread dust=6, Dust/sand=7, Fog=45.

**Impact**: Same as §3.2.

**Suggestion**: Split the mapping according to the WMO 4677 standard.

### 3.4 WWO mist and smoky haze map to Fog instead of their own WMO codes

**Location**: `src/provider/worldweatheronline.rs:468-469`

**Problem**: Code 143 (mist) maps to 45 (Fog) but should be 10 (Mist). Code 149 (smoky haze) maps
to 45 (Fog) but should be 4 (Smoke) or 5 (Haze). Both are distinct WMO 4677 phenomena.

**Impact**: Loss of meteorological information.

**Suggestion**: Correct the mapping. The WWO official condition code list
(`https://www.worldweatheronline.com/feed/wwoConditionCodes.xml`) confirms 143="Mist" and
149="Smoky haze".

## §4 Medium

### 4.1 `CIRROCAST_LOCATION` cannot express multi-location runs

**Location**: `src/cli.rs:204`

**Problem**: The `location` field is `Vec<String>` with `env = "CIRROCAST_LOCATION"`. In clap 4,
the `env` value for a positional is provided as a single string, which becomes a single element in
the `Vec<String>`. So `CIRROCAST_LOCATION="Beijing,Shanghai"` produces `vec!["Beijing,Shanghai"]` —
a single element containing a comma — rather than two separate locations.

**Verification**:
```console
$ CIRROCAST_LOCATION="Beijing,Shanghai" cargo run -q -- -f one-line
note: 3 candidates for `Beijing,Shanghai`; using Beijing Goldsun Hotel, Shanghai, China — pass
`:Beijing,Shanghai` to require an exact name match, `--pick` to choose one, or `--yes` to keep
the winner
```

**Suggestion**: Either document that `CIRROCAST_LOCATION` accepts only a single location, or split
the env value on commas in `Sources::read` or `location_targets`.

### 4.2 `i18n.format_temp` is a dead code path in production

**Location**: `src/i18n.rs:1111-1134`

**Problem**: `I18n::format_temp` wraps `format_temp_signed`/`format_temp` with i18n formatting, but
grep shows it is only called in tests. All production renderers call `crate::model::units`
directly.

**Impact**: Maintenance hazard — a developer might use `i18n.format_temp` expecting it to be the
conversion path, but it is not wired into any renderer.

**Suggestion**: Delete it or mark it `#[cfg(test)]`, or wire it into the renderers.

### 4.3 SMHI provider has zero test coverage

**Location**: `src/provider/smhi.rs:409-436`

**Problem**: The entire `#[cfg(test)]` mod tests block is absent. The `condition_of` function maps
SMHI symbol codes 1-27 to WMO 4677 codes but has zero test coverage. This is the only national
provider with no unit tests whatsoever.

**Impact**: Mapping errors cannot be caught.

**Suggestion**: Add exhaustive condition-mapping tests, including the non-obvious decisions (e.g.
code 5 maps to WMO 3 despite being absent from SMHI's own symbol page, codes 12|22 map to 66,
codes 13|14|23|24 map to 67).

### 4.4 NWS condition mapping functions lack test coverage

**Location**: `src/provider/nws.rs:634-680`

**Problem**: The `condition_of`, `icon_condition`, and `text_condition` functions have no test
coverage. The test module (lines 706-735) only tests `wind_kmh` and `temperature_c`.

**Impact**: Mapping errors cannot be caught.

**Suggestion**: Add condition-mapping tests, comparing with `met_no.rs` and `brightsky.rs` which
both have exhaustive tests.

### 4.5 Multiple providers lack fixture-based tests

| Provider | Location | Problem |
|---|---|---|
| OpenWeatherMap | `src/provider/openweathermap.rs:494-551` | Only `condition_of` unit tests; missing full response parsing tests |
| WeatherAPI | `src/provider/weatherapi.rs:499-566` | Only `condition_of` and `current_of` unit tests |
| VisualCrossing | `src/provider/visualcrossing.rs:890-1063` | Only `icon_condition`, `alert_of`, `parse_offset`, `window_request` unit tests |
| Pirate Weather | `src/provider/pirateweather.rs:464-554` | Missing full response parsing tests |

**Suggestion**: Add fixture-based tests for each provider, following the pattern of `met_no.rs` and
`brightsky.rs`.

### 4.6 `--severity` validation gap

**Location**: `src/cli.rs:1848-1869`

**Problem**: `validate_surfaces` checks that `--aqi-index` needs `--aqi` or `--format aqi`, but
does not check that `--severity` needs `--alerts` or `--format alerts`. When those flags are
absent, `--severity` is silently ignored.

**Verification**:
```console
$ cargo run -q -- --severity severe -f plain     # output identical to no --severity
$ cargo run -q -- --severity severe --alerts -f plain  # same
```

**Suggestion**: Add a check in `validate_surfaces` that `--severity` requires `--alerts` or
`--format alerts`, similar to the `--aqi-index` check.

## §5 Low

### 5.1 `QueryArgs` has 20+ boolean/option fields

**Location**: `src/cli.rs:198`

The `#[allow(clippy::struct_excessive_bools)]` attribute suppresses the lint. The doc comment
justifies this. Consider grouping panel-related flags (`aqi`, `moon`, `marine`, `normals`) into a
sub-struct.

### 5.2 `validate_query` checks multi-location conflicts before single-location conflicts

**Location**: `src/cli.rs:1038`

Both error messages are correct, but the ordering means the multi-location message is shown even
when the user might have intended a single location with a typo.

### 5.3 `I18n::load` uses `std::env::var` directly

**Location**: `src/cli.rs:2551`

Tests that need to control the locale environment must mutate the process environment, which is
unsafe in parallel tests.

### 5.4 `language_chain` includes requested language even when no catalog exists

**Location**: `src/i18n.rs:798`

The `-v` output shows the full chain including e.g. `de-DE`, which could be misleading.

### 5.5 `catalog_for` maps all `zh` variants to `zh-CN` without warning

**Location**: `src/i18n.rs:1403`

Documented design choice. The `-v` output shows the substitution.

### 5.6 `one_line` renderer does not use `ctx.width`

**Location**: `src/render/one_line.rs:46-69`

Likely intentional, but not documented. The `plain`, `json`, and `alerts` formats all document
their width-ignoring behavior explicitly.

### 5.7 Summary layout uses first slot's width and depth without checking consistency

**Location**: `src/render/art_table.rs:357-368`

In practice the CLI resolves these once per run, but the code does not enforce or document this
assumption.

### 5.8 Unreachable match arm in `paint` function

**Location**: `src/render/color.rs:141-144`

The `Mono` pattern is unreachable because `Mono` is handled by the early return on line 138.

### 5.9 `art_table.rs` is 1812 lines — candidate for splitting

**Location**: `src/render/art_table.rs:1-1812`

Contains the ArtTable renderer, the Table buffer, summary layout, all metric writers,
border/fit/display helpers, and 465 lines of tests.

### 5.10 `json.rs` is 1290 lines — candidate for splitting

**Location**: `src/render/json.rs:1-1290`

Contains the Json renderer, all JSON structs, and 365 lines of tests.

### 5.11 `resolve_candidates` rejects Default/Alias with Config error

**Location**: `src/geo/mod.rs:350-354`

The error message is internal-facing and could confuse users if it ever leaks to the CLI.

### 5.12 `location_line` uses string comparison to detect coordinate names

**Location**: `src/geo/mod.rs:460`

Implicit coupling: if the format of `coordinate_name` changes, both places must be updated together.

### 5.13 `Error::Other` is a catch-all that may hide specific failures

**Location**: `src/error.rs:141`

The `chain_reason` function returns the full error string for `Other` variants, which could make
`-v` output harder to read.

### 5.14 `main` uses `ExitCode::from(u8)` which truncates on Windows

**Location**: `src/main.rs:18`

Not a practical issue given the current exit code table (0-6).

### 5.15 `worst_exit_code` takes the maximum, which may mask the first failure

**Location**: `src/lib.rs:72`

Documented design choice. The maximum exit code is the "most actionable" problem.

## Positive findings

- **Error handling**: All `unwrap`/`expect`/`panic` are inside `#[cfg(test)]` modules.
- **XDG compliance**: Correctly resolved through `etcetera`.
- **Secret handling**: API keys never enter `config.toml`, never appear in argv.
- **Determinism**: Same inputs produce same output; tests never touch the network.
- **Offline-first**: Bundled GeoNames table with lazy decoding, user table support with eager
  validation, graceful fallback from user to bundled table.
- **Candidate ranking**: Correct and deterministic (match tier → population → name → ascii_name).
- **Alias expansion**: Cycle detection with chain reporting, depth cap, edit-distance suggestions.
- **Timezone handling**: No silent UTC fallback for geocoded names.
- **CI configuration**: Thorough, all actions pinned to commit SHA.
- **REUSE compliance**: 507/507 files, all third-party fixtures correctly annotated.

## Suggested fix priority

1. **High**: Fix `src/template.rs` single-conversion-point violation (architecture).
2. **High**: Fix WeatherAPI/QWeather/WWO WMO condition mapping bugs (data correctness).
3. **Medium**: Fix `CIRROCAST_LOCATION` multi-location issue (functional gap).
4. **Medium**: Clean up `i18n.format_temp` dead code (maintenance risk).
5. **Medium**: Add SMHI/NWS condition-mapping tests (test coverage).
6. **Medium**: Add fixture-based tests for all providers (test coverage).
7. **Low**: Add `--severity` validation (UX consistency).
8. **Low**: Consider splitting large files (maintainability).

## Verification

Commands run on the integrated tree:

```console
$ cargo fmt --check
$ cargo clippy --workspace --all-targets --locked -- -D warnings
$ reuse lint
$ python3 scripts/check-render-imports.py
$ cargo test --workspace --locked
$ CIRROCAST_FORBID_NETWORK=1 cargo test --workspace --locked
$ cargo build --workspace --locked --no-default-features
```

All pass. `cargo deny check` and `cargo audit` could not complete because the advisory database
fetch requires network access (github.com unreachable from this machine).

Live smoke runs of the changed surfaces: `CIRROCAST_LOCATION` with comma-separated values (§4.1),
`--severity` with and without `--alerts` (§4.6), WWO provider output (§3.4).

## Follow-up

- The alerts and template/parallel slices were cancelled; their areas should be reviewed in a
  follow-up pass.
- The WMO condition mapping fixes (§3.2-3.4) should include a test that pins every published code
  to its expected WMO code, so a future edit cannot silently collapse distinct phenomena again.
- The `src/template.rs` fix (§3.1) is an architecture decision: either move the conversions into
  the render layer or amend the contract in `docs/plans/README.md` to name `template.rs` as part of
  the render surface.
