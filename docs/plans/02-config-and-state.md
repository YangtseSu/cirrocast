<!--
SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Step 02 — config-and-state

Status: ✅ done
Depends on: `01-project-scaffold.md` (error taxonomy, `Paths`, `src/cli.rs` skeleton, root `README.md`)
Touches: `src/config/mod.rs`, `src/config/keys.rs`, `src/cli.rs`, `src/lib.rs`, `src/paths.rs`,
`README.md`, `tests/config.rs`, `tests/keys.rs`, `tests/common/mod.rs`, `tests/cli.rs`,
`tests/fixtures/config/`, `docs/plans/README.md`

## Goal

`cirrocast` reads and writes its own configuration state: `$XDG_CONFIG_HOME/cirrocast/config.toml`
is parsed into a typed `Config`, missing keys fall back to built-in defaults, environment variables
and CLI flags override the file in a documented order, and BYOK API keys live in a separate,
`0600`-only `keys.toml` addressed through `cirrocast key set/rm/list`. Everything the later steps
need (timeouts, TTLs, units, provider choice, location default) now exists as one resolved
structure, and the user has a full `cirrocast config …` surface to inspect and edit it.

## Deliverables

- ✅ `src/config/mod.rs`: `Config` mirroring the contract's TOML schema exactly — `schema_version: u32`, `defaults: Defaults { provider, format, units, days, language }`, `location: LocationDefaults { default }`, `units: UnitOverrides { temp, wind, pressure, distance, precip }`, `network: Network { timeout_secs, retries, proxy }`, `cache: CacheConfig { enabled, weather_ttl_secs, ip_ttl_secs, geocode_ttl_secs }`, `render: RenderConfig { color, width }`, `providers: Providers { metar: MetarConfig { station }, qweather: QWeatherConfig { host } }`; `#[serde(default)]` on every field and table; `impl Default for Config` = the built-in values (`open-meteo`, `art-table`, `metric`, `3`, `auto`, `""`, five *unset* unit overrides (`Option<String>` = `None`), `15`, `3`, `""`, `true`, `600`, `86400`, `2592000`, `auto`, `0`, `""`, `""`).
- ✅ `Config::load(paths: &Paths) -> Result<Config>`: search `$XDG_CONFIG_HOME/cirrocast/config.toml` then each `$XDG_CONFIG_DIRS` entry in order (the primary directory comes from `etcetera`; `etcetera` has no `config_dirs()` accessor, so the `$XDG_CONFIG_DIRS` entries are read from the environment directly, default `/etc/xdg`), parse the first hit; no file anywhere ⇒ `Config::default()`; malformed TOML ⇒ `Error::Config` whose message is `<path>:<line>:<col>: <cause>`, position taken from `toml::de::Error::span()` computed against the source text (no span ⇒ path + cause only); `schema_version` above `CURRENT_SCHEMA_VERSION` ⇒ `Error::Config("config written by a newer cirrocast (schema_version 99, supported 1)")`.
- ✅ `fn migrate(schema_version: u32, doc: &mut toml::Value) -> Result<u32>` hook called by `load` before deserialisation: accepts `1` unchanged, rejects `0` and `>1` with an actionable `Error::Config`. It is the single place a future schema change transforms an old document; unit-tested for both directions in `#[cfg(test)] mod tests` of `src/config/mod.rs`.
- ✅ `Config::validate(&self) -> Result<()>`: enum fields against the same value sets clap uses, `timeout_secs` 1..=300, `retries` 0..=10, three TTLs `> 0`, `render.width` 0 or 40..=500, `days` 0..=14, `network.proxy` syntactically checked as `scheme://host[:port]` or `host:port` here (step 05 re-checks it with `ureq::Proxy::new` once `ureq` is a dependency); every failure names the dotted key (`defaults.days: 99 is out of range 0..=14`).
- ✅ `Config::write_default(paths: &Paths, force: bool) -> Result<PathBuf>`: refuses to overwrite unless `force` (`Error::Config("… exists; pass --force")`), otherwise writes the commented default document atomically with mode `0644` and returns the path. Also `Config::save(&self, paths: &Paths) -> Result<PathBuf>` writing the canonical serialisation (`toml::to_string_pretty`) with the same helper.
- ✅ `pub(crate) fn atomic_write(path: &Path, data: &[u8], mode: u32) -> Result<()>` in `src/config/mod.rs`: sibling `.<file>.tmp.<pid>` created with `OpenOptions::new().write(true).create_new(true).mode(mode)` (`#[cfg(unix)]`; documented platform shim elsewhere), `write_all` + `sync_all` + `rename` over the target, temp file removed on error. Step 05 reuses this for cache entries and step 02's key store reuses it for `keys.toml`.
- ✅ Dotted-key accessors `Config::get_key(&self, key: &str) -> Result<String>` and `Config::set_key(&mut self, key: &str, value: &str) -> Result<()>` driven by one `KEY_TABLE: &[KeySpec]` in `src/config/mod.rs` (`KeySpec { name: &'static str, kind: KeyKind, doc: &'static str, env: Option<&'static str> }`, `KeyKind = Str | Enum(&'static [&'static str]) | U32 | Bool`; the `env` column is what lets `config get` honour `CIRROCAST_*`, since e.g. `CIRROCAST_DAYS` cannot be derived from `defaults.days`), with the table seeded from the contract's schema; setting a value runs `KeyKind` parsing and `Config::validate()`, and the caller (`config set`) persists it with `Config::save()` — `set_key` receives no `Paths` to write to; unknown key ⇒ `Error::Usage("unknown config key `defaults.pvd`")` plus the list of known keys. Later steps append rows here (04 `network.nominatim_url`, 05 nothing new).
- ✅ `Settings` resolved view in `src/config/mod.rs`: `Settings { provider: String, format: String, units: String, days: u8, lang: String, location: Option<String>, timeout_secs: u32, proxy: Option<String>, no_cache: bool, refresh: bool, offline: bool }` built by `Settings::resolve(&Config, &CliOverrides) -> Result<Settings>` implementing the precedence CLI flag > `CIRROCAST_*` env > config file > built-in default (`CliOverrides` is the seam step 08 fills from the clap flags, whose `env = "CIRROCAST_*"` attributes have already folded flag-over-environment in). The four enum-ish fields stay strings here on purpose: step 03 owns `UnitSystem`, step 05 `CacheMode` and step 06 `Format`, so the consumers parse them with their own `FromStr` and this step never has to know their types; validity is guaranteed upstream because `Config::validate()`/`set_key` reject values outside the contract's sets. CLI flags carry clap `env = "CIRROCAST_PROVIDER" | "CIRROCAST_FORMAT" | "CIRROCAST_UNITS" | "CIRROCAST_DAYS" | "CIRROCAST_LANG" | "CIRROCAST_LOCATION" | "CIRROCAST_TIMEOUT"` attributes so clap already resolves flag-over-env; `resolve` only fills what is still `None` from the file.
- ✅ `src/cli.rs`: `Command::Config(ConfigCmd)` and `Command::Key(KeyCmd)` with clap subcommands `config path|init [--force]|show|get <KEY>|set <KEY> <VALUE>|edit|validate` and `key set <PROVIDER> [--stdin]|rm <PROVIDER>|list`; dispatch lives in `Cli::run` exactly as in step 01 (`src/main.rs` still only turns `Error` into an exit code), keeping that mapping (usage ⇒ 2, config/state ⇒ 4). `config path` prints the absolute resolved `config.toml` path and creates nothing; `config show` prints the effective config as canonical TOML; `config edit` runs `$VISUAL`/`$EDITOR`/`vi` on the file and re-validates afterwards, exiting non-zero with the validation error if the edited file is invalid.
- ✅ `src/config/keys.rs` key store: `KeyStore::new(paths: &Paths)`, `get(&self, provider: &str) -> Result<Option<String>>`, `set(&self, provider: &str, value: &str) -> Result<()>`, `remove(&self, provider: &str) -> Result<bool>`, `list(&self) -> Result<Vec<KeySummary>>` (`KeySummary { provider, masked, source: KeySource }`, `KeySource = Env | File`), file format `[keys]` with one `provider = "value"` entry per provider. Precedence first hit wins: `CIRROCAST_<PROVIDER>_KEY` (provider id uppercased, `-` → `_`, e.g. `CIRROCAST_OPENWEATHERMAP_KEY`) → `keys.toml` in the config dir → OS keyring, feature-gated and reached through `KeyBackend::OsKeyring` only when `cfg(feature = "keyring")` (added to the enum in step 10).
- ✅ Key file discipline: `set` writes `keys.toml` through `atomic_write(..., 0o600)`; `get`/`list`/`set` re-`metadata()` the file and refuse to read it when `mode & 0o077 != 0` (`Error::Config("keys.toml is readable by group/other (mode 0644); run `chmod 600 <path>`")`); `KeyStore::mask(value)` renders `abcd…yz` (first 4 chars, `…` U+2026, last 2; values shorter than 8 chars print as `…`); `list` never prints a full secret and marks `source = Env` for environment-provided keys.
- ✅ `key set` reads the secret from stdin (`--stdin`, or a piped stdin), never from argv; when stdin is a tty and `--stdin` was not given it prompts on the terminal with echo disabled via `rpassword`, so the value stays out of the process list, shell history and `ps` output.
- ✅ `Config::load` + `KeyStore` + `Settings::resolve` tests in `tests/config.rs` and `tests/keys.rs` (assert_cmd CLI level, temp dirs via `tempfile`, `XDG_CONFIG_HOME` set per invocation): defaults when no file exists, malformed-file message contains `config.toml:5:` (the fixture is installed as `config.toml`, so the position names that path), future-schema rejection, `config set` round-trip through `config get`, a probe round trip over every `KEY_TABLE` row, precedence (env beats file beats default through the CLI, plus a unit test for override beats file beats default, because the flags themselves arrive in step 08), `config init` twice without `--force`, unknown-key error, key masking, `0644` refusal, env-over-file key precedence, and `atomic_write` leaving no `*.tmp.*` behind.
- ✅ Documentation: configuration and key sections in root `README.md` (schema table, the precedence order, `keys.toml` vs `config.toml` split, `CIRROCAST_*` list) and step row 02 plus the step-01 leftovers updated in `docs/plans/README.md`.

## Design notes

* **`config set` rewrites the file, so comments do not survive.** `set` deserialises `config.toml`
  into `Config`, mutates one field, and serialises the whole struct back through `toml::to_string_pretty`
  (atomic write). The file therefore always has exactly one canonical shape, one source of truth for
  types and validation, and `get`/`set`/`load` cannot disagree about defaults. Keeping user comments
  would require `toml_edit` and a second, untyped code path for every key — twice the surface for a
  17-key file. Comments that must survive belong in the docs and `--help`; a user who wants the
  annotated file back runs `cirrocast config init --force`, which writes the fully commented default.
* **Keys never live in `config.toml`.** That file is `0644`, is meant to be pasted into issue reports,
  is dumped wholesale by `config show`, and is edited by hand; a secret there leaks through all four.
  Keys get their own file, their own mode, their own precedence chain and their own commands, and
  `keys.*` is deliberately absent from `KEY_TABLE`, so `config set keys.openweathermap …` fails.
* `keys.toml` is checked on every read instead of on write only: a file the user `chmod 644`-ed by hand
  must fail loudly (with the fix in the message) rather than be used silently.
* `rpassword` is the one new dependency taken for the key prompt: it disables terminal echo without
  `unsafe` in our crate (`unsafe_code = "forbid"` in `Cargo.toml`), and the alternative — argv or an
  echoed prompt — puts secrets into `ps` and shell history.
* Dependencies added by this step, each with its reason: `serde = { version = "1", features = ["derive"] }`
  (typed config + `Settings`), `toml = "1"` (parse/serialise, gives `span()` for error positions),
  `rpassword = "7"` (no-echo secret input). `tempfile` already exists as a dev-dependency; no release
  dependency is added for the atomic write.
* Unknown keys in `config.toml` are ignored rather than rejected, so a config written by a newer
  cirrocast (with `schema_version` unchanged) keeps working for the fields we know. Typos are caught
  by `config validate` for known keys only; a strict unknown-key check is a step-12 hardening candidate,
  not a blocker.

## Out of scope

- OS keyring backend: `KeyBackend::OsKeyring` and the `keyring` cargo feature land in step 10 together
  with the first key-requiring provider (`10-additional-providers.md`); this step only defines the
  enum seam.
- Anything that *consumes* `render.color`, `render.width`, `units.*` or `defaults.format`: they are
  stored, validated and printed here; the model consumes units in `03-canonical-model-and-units.md`,
  the renderer consumes colour/width in `07-art-table-renderer.md`, and the full flag matrix in
  `08-cli-surface-and-formats.md`.
- Real schema migrations: schema `1` is the first released schema, so `migrate()` only gates versions;
  a mapping is written when a `2` exists.
- Localised config error messages: `09-localization.md`.

## Verification

Fixtures: `tests/fixtures/config/bad-syntax.toml` (unclosed table header on a known line),
`tests/fixtures/config/future-schema.toml` (`schema_version = 99`), `tests/fixtures/config/expected-after-set.toml`
(canonical output expected from one `config set` round-trip). Manual smoke run:

```sh
tmp=$(mktemp -d); export XDG_CONFIG_HOME=$tmp
cargo run -- config path                       # /tmp/…/cirrocast/config.toml  (file not created)
cargo run -- config init                       # wrote /tmp/…/cirrocast/config.toml
cargo run -- config get defaults.provider      # open-meteo
cargo run -- config set defaults.days 5 && cargo run -- config show | grep '^days'   # days = 5
cargo run -- config set defaults.days 99; echo $?   # error: defaults.days: 99 is out of range 0..=14   (exit 4)
CIRROCAST_DAYS=7 cargo run -- config get defaults.days    # 7  (env beats file)
cargo run -- config validate                   # ok: /tmp/…/cirrocast/config.toml
printf %s 'sk-test-abcdef123456' | cargo run -- key set openweathermap   # key stored in keys.toml
cargo run -- key list                          # openweathermap  sk-t…56  (file)
stat -c %a $tmp/cirrocast/keys.toml            # 600
chmod 644 $tmp/cirrocast/keys.toml; cargo run -- key list; echo $?   # error: … mode 0644 … chmod 600   (exit 4)
CIRROCAST_OPENWEATHERMAP_KEY=sk-env-abcdef123456 cargo run -- key list    # openweathermap  sk-e…56  (env)
```

## Exit criteria

- ✅ Every deliverable above implemented; no `TODO`/stub left in `src/config/`.
- ✅ `cargo test` covers: defaults, malformed-position, future schema, precedence matrix, key mode
      refusal, masking, atomic-write cleanliness.
- ✅ `docs/plans/README.md` row 02 marked `done` in the same commit.
- ✅ `cargo fmt --check` clean.
- ✅ `cargo clippy --all-targets -- -D warnings` clean.
- ✅ `cargo test` clean (unit + `tests/config.rs` + `tests/keys.rs`).
- ✅ `reuse lint` clean (fixtures under `tests/fixtures/config/` carry in-file SPDX headers; extend
      `REUSE.toml` only if a non-commentable file type is added).
- ✅ Smoke run above reproduces the shown output shape, including exit code 4 for the invalid value
      and the `0600` mode on `keys.toml`.

## Risks

- `$XDG_CONFIG_DIRS` fallback for reads vs. always writing to `$XDG_CONFIG_HOME`: a stale file in a
  system dir can shadow the user's file only when the user's file is absent. `config path` prints the
  user file (the write target), `config validate` prints the file that was actually read, and
  `config show` is the debugging entry point. Documented in the README.
- Losing user comments on `config set` (see design notes) is a visible behaviour change for anyone
  hand-editing; mitigated by `config init --force` and by documenting it in `config set --help`.
- `toml::de::Error::span()` is `None` for some error kinds; the message then lacks `:line:col`, and the
  test for the malformed fixture asserts the fixture path only if the upstream crate changes behaviour.
- Permission-bit checks are unix-only; the non-unix path is a documented shim, and CI is Linux.

## Progress log

- 2026-09-30 — plan written.
- 2026-09-30 — `src/config/{mod,keys}.rs`, the `config`/`key` subcommands, the fixtures and the
  integration tests landed. Reality-vs-plan fixes made in this commit: the `[units]` overrides are
  `Option<String>` with `None` = "follow `defaults.units`" (the plan's literal `c`/`kmh`/… defaults
  would freeze a metric choice into every document and contradicted step 03's `None` = take the
  system default; the contract block in `docs/plans/README.md` now shows them commented out);
  `KeySpec` gained an `env` column because `CIRROCAST_DAYS` cannot be derived from `defaults.days`;
  `Settings::resolve` takes `CliOverrides` instead of `&Cli` because the weather flags only arrive in
  step 08 (the override-over-file-over-default half is unit-tested, env-over-file is tested through
  the CLI); `network.proxy` is checked locally until step 05 adds `ureq`; `set_key` validates and
  `config set` persists, because `set_key` receives no `Paths`; dispatch stayed in `Cli::run` (the
  step-01 layout), so `src/main.rs` is untouched; `config path` prints the user file instead of
  whichever `$XDG_CONFIG_DIRS` document shadows it — `config validate` reports the file read.
- 2026-09-30 — step done: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo
  test` (26 unit + 5 CLI + 12 config + 7 keys) and `reuse lint` (46/46 files) clean. The verification
  block reproduced: `config path` → `<tmp>/cirrocast/config.toml` without creating anything, `config
  init` → `wrote …`, `config get defaults.provider` → `open-meteo`, `config set defaults.days 5` then
  `config show` shows `days = 5`, `config set defaults.days 99` → exit 4 with the range message,
  `CIRROCAST_DAYS=7 config get defaults.days` → `7`, `config validate` → `ok: …`; a piped `key set`
  wrote `keys.toml` at mode `600` with the secret masked as `sk-t…56` in `key list`, a `chmod 644`
  key file failed with exit 4 and the `chmod 600` hint, and `CIRROCAST_OPENWEATHERMAP_KEY` was listed
  as `(env)`.
