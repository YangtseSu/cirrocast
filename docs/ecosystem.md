<!--
SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Ecosystem and output contracts

`cirrocast` is meant to be read by other programs, not only by people: a status bar asks it for one
line every few minutes, a script reads a field out of `plain`, a dashboard parses the `json`
document. Those three surfaces are **contracts** — this file is where they are written down, and
the tests that enforce them are named beside each one.

The install-path half of this document (Nix, Homebrew, `.deb`/`.rpm`, the static musl archive,
macOS and Windows archives) is step 13's AUR package and release archives today, and arrives as a
separate section with backlog B02.

## The `status` probe

```text
cirrocast status [--format <TEMPLATE>] [--template <TEMPLATE>] [--location <SPEC>]
                 [--max-age <SECS>] [--offline] [--placeholder <TEXT>]
                 [--color never|always]
```

`-f/--format` on this subcommand names a **template**, not one of the query's formats:
`--template` is the same flag under its literal name, the default is `%c %t`, and `--format json`
would print the four characters `json`. Every `%` token of the one-line vocabulary is available
(`docs/formats.md`), plus `@NAME` for a `[templates]` entry.

### The line

* stdout is **exactly one line plus `\n`** — a newline in the template becomes one space and the
  result is trimmed, so a module never has to guess how many lines arrived;
* colour is **`never`** unless `--color always` is given. The probe does not detect: a bar's
  `TERM`, `NO_COLOR` and `CLICOLOR_FORCE` say nothing about whether the bytes reach a terminal.
  (No token emits an ANSI escape today, so the flag currently changes nothing visible; the policy
  is fixed so a future coloured token cannot silently inherit the environment.)
* the credits the data licences require go to **stderr**, like every one-line output; `-q`
  silences them. A bar that shows nothing but stdout therefore needs no filtering — and a wrapper
  that runs the probe in a prompt should pass `-q` or redirect stderr.

### Exit codes

| code | when | what the bar sees |
|---|---|---|
| 0 | a reading | the rendered line |
| 0 | a **transient or data failure**: `Network`, `Upstream`, `LocationNotFound`, `MissingKey`, `InvalidKey`, `InvalidToken`, the provider `Chain` | the placeholder on stdout, one `error: …` line on stderr |
| 2 | a **usage mistake**: an unknown template token, an unknown value, `--format` next to `--template` | nothing on stdout |
| 4 | a **configuration problem**: an unreadable `config.toml`, no location configured | nothing on stdout |

"Never exits non-zero because the network was unavailable" is the promise a bar is written
against: a timer that runs this every 15 minutes renders the placeholder instead of a crashed
module. Only problems the user has to fix — a typo, a broken file, a missing location — fail.

The classification is `status::degradable` (`src/status.rs`), an exhaustive match: a new `Error`
variant cannot join the contract without a decision there. `tests/status_contract.rs` drives the
upstream-failure, offline-miss, missing-key and placeholder cases end to end.

### Location and privacy

* `--location <SPEC>` takes any location argument (`Beijing`, `:Berlin`, `~Tsinghua`,
  `@39.9,116.4`, `@home`), resolved by the same sources and the same ranking as a query.
* With no `--location`, `CIRROCAST_LOCATION` supplies it, then `[location] default` — the same
  precedence ladder as everywhere else (flag, environment, configuration). With none of the three,
  the probe exits 4: it **never performs the public-IP lookup**, because a status bar has no way to
  consent to one.
* An ambiguous name takes the ranked winner: the probe **never prompts** (no `--pick`, no stdin).
  Use `location search --all` when the choice matters.

### Freshness: `--max-age` versus the cache TTL

`[cache] weather_ttl_secs` (600 s) decides whether a cached answer is **valid**; `--max-age`
decides whether it is **fresh enough to skip the network for**. A bar that refreshes every 15
minutes against a 10-minute TTL would otherwise revalidate on every refresh; `--max-age 900` makes
the entry servable for 15 minutes. The window is the wider of the two, the default is the weather
TTL, and `0` means "follow the TTL" — the knob can only widen, never shorten.

`--offline` is stronger: no socket is opened at all (the name scope is offline too), and **any**
cached answer is served, however old, because a stale reading beats no reading in a bar. On a cold
cache the placeholder takes over, still with exit 0.

### What the probe fetches

The forecast comes from the configured provider chain, exactly as a query's does. The two tokens
backed by a *second* upstream request are fetched only when the template shows them — the alert set
for `%A`, the air-quality reading for `%q` — and a failure in either is a warning, never a
placeholder: the reading the bar already has must survive it. The astro tokens (`%m`, `%M`, `%S`,
`%s`) are computed locally and always attached.

## Status-bar recipes

The runnable examples live in [`contrib/statusbar/`](../contrib/statusbar/): waybar, polybar,
i3blocks, tmux, starship, and bash/zsh prompt snippets. Each file is a real configuration fragment
for that tool, and each carries a machine-readable marker naming **the exact command it runs**:

```text
# cirrocast-example: cirrocast status -q --format '%c %t'
```

[`contrib/statusbar/verify.sh`](../contrib/statusbar/verify.sh) extracts that command from every
example, runs it **with `--offline`** against a throwaway XDG tree seeded from
`tests/fixtures/cache/weather/open-meteo-39.90-116.40-3-2026-10-05.json`, and asserts exit 0 with
exactly one non-empty line carrying the fixture's temperature. The examples themselves never pass
`--offline` — a bar wants live weather — and none of them hardcodes a location: the tree's
`config.toml` sets `[location] default`. The CI job `statusbar` runs the same script, so a recipe
that stops working fails the build instead of rotting.

The two prompt snippets cache the rendered line under `${XDG_RUNTIME_DIR:-${TMPDIR:-/tmp}}/
cirrocast/status` and re-render only when it is older than `CIRROCAST_STATUS_INTERVAL` (default
900 s): a prompt must never block on the network. `verify.sh` does not source them (they are for an
interactive shell); it runs the probe command their marker names.

Versions exercised (recorded 2026-10-06, the releases Arch Linux `extra` shipped at that date):
waybar 0.15, polybar 3.7.2, tmux 3.7c and starship 1.26 for the configuration formats, plus
i3blocks' block-file format and the bash/zsh/POSIX shells of `ubuntu-26.04`. The CI job runs the
committed files, not the upstream documentation, so a key renamed upstream is a documentation
problem here, not a break in the probe.

## Output contracts

Three surfaces are frozen. A change to any of them **is a release event, not a commit**: a minor
version bump, one release of dual emission where feasible, a `CHANGELOG.md` entry under
`### Breaking` naming the old and the new shape, and a `docs/schema.md` note for the versions.

### `json` — the machine-readable document

* `schema_version: 2` is the current document; the key set, the types and the nullability rules are
  listed in [`docs/schema.md`](schema.md) and are *machine-checked* there (`tests/render_json.rs`
  reads the key index and fails if the renderer disagrees with it).
* Additive changes — a new key, a value that becomes nullable — stay within `schema_version` 2: a
  consumer must ignore keys it does not know and handle `null` for every key. A removal, a rename,
  a retype or a unit change bumps the version.
* The shape is `oneOf`: one location is a plain object, several are an array of at least two
  entries in argument order, where a failed slot is a three-key error object.
* [`docs/schema/json-v2.json`](schema/json-v2.json) is the same contract as a JSON Schema (Draft
  2020-12). Step 28 adds the frozen v1 schema beside it and the `jsonschema`-crate test that
  validates live, fixture-backed output against both.

### `one-line` — the `%`-token vocabulary

The meaning of every token is frozen (`docs/formats.md`, and `template::TOKENS` is the single
table the tests walk): `%L` may not silently become something else again — step 19's change from
"coordinates" to "today's low" is the worked example of what a breaking token change costs. Adding
a *new* letter is additive and does not break a consumer; the compat surface of backlog B01 keeps
unknown tokens literal for exactly this reason.

### `plain` — the record order

The label-per-record form is frozen in two dimensions, both pinned by `tests/plain_order.rs`:

* the **order of records**: `location`, the `alert` records, `updated`, `current`, one `day <date>`
  record per forecast day, the air panel (`air_quality`, its pollutants, `pollen`, `uv`), the
  `moon`/`sun` records, the credit lines, and `attribution` last;
* the **order of the fields inside `current`**: condition, temperature, `(feels …)`, `wind`,
  `humidity`, `precip`, `pressure`, `visibility` — a field that disappears breaks a script, which
  is why the test fails rather than tolerating it.

The labels are catalog strings, so a translated run prints translated record keys; the record
*order* and the `label: value` shape are the contract.

### `art-table` and `dumb` are not contracts

They are the default because they are the reason the tool exists — a table a person reads. Their
layout may be refined in a minor release (a column, a glyph, the stacked fallback below 60
columns), and nothing here promises otherwise. A script parses `json`, `plain` or `one-line`.

## Enforcing the contracts

| contract | what fails when it moves |
|---|---|
| JSON key set and types | `tests/render_json.rs` against the `docs/schema.md` key index; `docs/schema/json-v2.json` for the schema |
| `%`-token meanings | `tests/templates.rs` (the `TOKENS` table), `tests/render_one_line.rs` snapshots, the `--help` epilogue copy |
| `plain` record and field order | `tests/plain_order.rs` |
| the `status` line, exit codes and freshness | `tests/status_contract.rs`, `contrib/statusbar/verify.sh` (CI job `statusbar`) |
| the config keys | `src/config/mod.rs` (`check_known_keys`, the built-in-defaults test), `docs/schema.md` |
| `--help` itself | `tests/cli.rs` (the line budget) and step 28's `help_snapshot` |

See also: [`docs/formats.md`](formats.md) for every format and the full token table,
[`docs/schema.md`](schema.md) for the JSON and configuration schemas and the compatibility rule,
[`docs/providers.md`](providers.md) for the backends behind the values.
