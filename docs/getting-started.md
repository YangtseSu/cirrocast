<!--
SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Getting started

`cirrocast` prints a weather report to the terminal: the place, its current conditions and a
forecast table, with no account and no API key for the default backend. This guide goes from an
install to a configured, keyless setup; the flag reference is `--help` itself, and each setting is
documented once in the topic file linked from the section that mentions it.

> The command blocks below are real output, captured on 2026-10-06 against `cirrocast 1.3.0`. The
> weather blocks were captured with `--color never --width 80 --lang en-US`; a table row elided in
> the middle is marked with `…`.

## Install

### Arch Linux (AUR)

The package is `cirrocast`:

```console
$ paru -S cirrocast
```

`yay -S cirrocast` works too, as does `git clone https://aur.archlinux.org/cirrocast.git` followed
by `makepkg -si`. The package installs `/usr/bin/cirrocast`, the generated man page as
`/usr/share/man/man1/cirrocast.1`, the bash/zsh/fish completions under `/usr/share/`, and the
licence as `/usr/share/licenses/cirrocast/LICENSE`. It builds from the tagged source tarball
against the system's own glibc, for `x86_64` and `aarch64`.

### Prebuilt release archives

Every release publishes one archive per target:

| Target | Runs on |
|---|---|
| `x86_64-unknown-linux-gnu` | Linux, x86-64 |
| `aarch64-unknown-linux-gnu` | Linux, arm64 |
| `aarch64-apple-darwin` | macOS, Apple silicon |

```console
$ version=v1.3.0 target=x86_64-unknown-linux-gnu
$ curl -LO "https://github.com/YangtseSu/cirrocast/releases/download/$version/cirrocast-$version-$target.tar.gz"
$ curl -LO "https://github.com/YangtseSu/cirrocast/releases/download/$version/cirrocast-$version-$target.tar.gz.sha256"
$ sha256sum -c "cirrocast-$version-$target.tar.gz.sha256"
cirrocast-v1.3.0-x86_64-unknown-linux-gnu.tar.gz: OK
$ tar xzf "cirrocast-$version-$target.tar.gz"
$ install -Dm755 "cirrocast-$version-$target/cirrocast" ~/.local/bin/cirrocast
$ cirrocast --version
cirrocast 1.3.0
```

Each archive contains the `cirrocast` binary plus `README.md`, `LICENSE`, `CHANGELOG.md`, the
generated `cirrocast.1` and `completions/{bash,zsh,fish}` — nothing is installed for you. On macOS
verify the checksum with `shasum -a 256 -c`.

### From crates.io or the checkout

```console
$ cargo install --locked cirrocast             # 1.3.0 is on crates.io
$ cargo install --locked --path .              # from this checkout
```

Neither writes the man page or the completions; the binary generates them itself, so they cannot
drift from the flags it has:

```console
$ cirrocast man > ~/.local/share/man/man1/cirrocast.1
$ cirrocast completion bash > ~/.local/share/bash-completion/completions/cirrocast
$ cirrocast completion zsh  > ~/.local/share/zsh/site-functions/_cirrocast
$ cirrocast completion fish > ~/.config/fish/completions/cirrocast.fish
```

`completion <SHELL>` accepts `bash`, `elvish`, `fish`, `powershell` or `zsh`, and installs nothing
by itself. For a renamed or vendored copy, `--bin-name` names the program the script — or the man
page's `.TH` line — is generated for: `cirrocast completion bash --bin-name myweather` defines
`_myweather`, and `cirrocast man --bin-name myweather` writes `.TH myweather 1`. On both
subcommands it defaults to `cirrocast`.

### Not packaged yet

Nix, Homebrew, `.deb`/`.rpm`, a static musl archive and Windows are the packaging matrix of backlog
**B02** and do not ship.

## First run

### A named place

```console
$ cirrocast Beijing
Weather report: Beijing, CN (39.91, 116.40)

  · * · Clear sky
   (●)  +20°C (+16°C)
  * · * ↙ 11km/h SW
        29% 1021hPa 18km 0.0mm

┌───────────────────────┬───────────────────────┬───────────────────────┐
│ Today, Oct 06         │ Wed 07 Oct            │ Thu 08 Oct            │
├───────────────────────┼───────────────────────┼───────────────────────┤
│    \│/  Morning       │    \│/  Morning       │    \│/  Morning       │
│   ─(●)─ +18°C (+14°C) │   ─(●)─ +17°C (+16°C) │   ─(●)─ +17°C (+16°C) │
│    /│\  ↑ 7.0km/h N   │    /│\  ↗ 1.5km/h NE  │    /│\  ↗ 3.1km/h NNE │
│         0.0mm 0%      │         0.0mm 0%      │         0.0mm 0%      │
… the Noon, Evening and Night bands of each day …
└───────────────────────┴───────────────────────┴───────────────────────┘

Location data by GeoNames (CC BY 4.0) — https://www.geonames.org/
Data: Open-Meteo.com (CC BY 4.0) — https://open-meteo.com/
```

That is the default `art-table` format:

* the header names the resolved place and its coordinates;
* the current block is the condition art and text, the temperature with the feels-like value in
  parentheses, the wind arrow and speed, then humidity, pressure, visibility and precipitation;
* the grid is one column per day — `[defaults] days = 3` by default, clamped to what the backend
  serves — each split into Morning, Noon, Evening and Night bands;
* the closing lines are the attribution the data licences require.

`Beijing` was ambiguous, so the run also printed to **stderr**:

```console
note: 3 candidates for `Beijing`; using Beijing, CN (population 18960744) — pass `:Beijing` to require an exact name match, `--pick` to choose one, or `--yes` to keep the winner
```

stdout stays pipeable; `-q`/`--quiet` silences the note. `:Beijing` demands an exact name, `--pick` prompts,
and `--yes` accepts the ranked winner. The full syntax (`~Tsinghua`, `@39.9,116.4`, `@home`) and
the ranking rules are in [location.md](location.md).

Other views of the same report: `-f plain`, `-f one-line --template @short`, or `-f json`; see
[formats.md](formats.md) for all of them.

### No location argument

An empty location uses `[location] default`; when that is empty too, the public IP decides:

```console
$ cirrocast -f one-line
Zhengzhou: *o* Clear sky +18°C (+17°C), ↓ 6.2km/h S, 58%, 0.0mm, 1022hPa, 11km
```

The IP lookup is visible on stderr, and it is the only way the address leaves the machine — never
without an empty location or an explicit `--ip`:

```console
ip: located from the public IP via ipwho.is
```

The lookup sends the address to `ipwho.is` (with `ipapi.co` and `IP.SB` as fallbacks) and caches
the answer for 24 hours. To avoid it, pin a place once:

```console
$ cirrocast config set location.default Beijing
$ cirrocast
Beijing: *o* Clear sky +20°C (+16°C), ↙ 11km/h SW, 30%, 0.0mm, 1021hPa, 18km
```

(`cirrocast` with no `-f` prints the art table; the line above was captured with `-f one-line`.)

### Several places at once

Passing more than one location is one run: they are fetched at most four at a time and printed in
argument order, whatever order the network answers in.

```console
$ cirrocast :Beijing :Shanghai :Tokyo -f one-line
Beijing: *o* Clear sky +20°C (+16°C), ↙ 11km/h SW, 29%, 0.0mm, 1021hPa, 18km
Shanghai: *o* Clear sky +17°C (+17°C), ↖ 3.2km/h NW, 73%, 0.0mm, 1023hPa, 20km
Tokyo: *o_ Mainly clear +20°C (+20°C), ↑ 7.8km/h NNW, 64%, 0.0mm, 1008hPa, 35km
```

A location that fails keeps its slot and the run continues:

```console
$ cirrocast :Beijing Nope-9x -f one-line
Beijing: *o* Clear sky +20°C (+16°C), ↙ 11km/h SW, 29%, 0.0mm, 1021hPa, 18km
error: Nope-9x: location not found: no location found for `Nope-9x` (no offline match)
$ echo $?
5
```

The process exits with the numerically largest mapped code among the failures. Above one location
`json` becomes an array, and `art-table` draws a combined summary for up to four — a fifth falls
back to the full per-location tables. The output rules are spelled out in
[formats.md](formats.md#multiple-locations).

## The `--help` tour

```console
$ cirrocast --help
Terminal weather client with pluggable backends and wttr.in-style output

Usage: cirrocast [OPTIONS] [LOCATION]... [COMMAND]

Commands:
  config      Inspect and edit the configuration file
  key         Manage API keys for the key-requiring providers
  provider    Inspect the supported weather providers
  location    Resolve a location argument without fetching weather
  cache       Inspect and maintain the on-disk cache
  status      Print one line for a status bar: never aborts on a network failure
  completion  Print a shell completion script
  man         Print the manual page as roff
  help        Print this message or the help of the given subcommand(s)
…
```

`-h` prints a one-screen summary; `--help` adds the flag descriptions. `cirrocast <command> --help`
documents one subcommand. The long form ends with four epilogue sections — `CONFIG PRECEDENCE`,
`EXIT CODES`, `ONE-LINE TOKENS`, `MULTI-LOCATION RUNS` — that summarise the cross-cutting rules.

The options fall into four groups, each documented in one place:

* **Output** — `-f/--format`, `--template`, `--template-file`, `--width`, `--color`,
  `--lang`: [formats.md](formats.md).
* **Location** — the `[LOCATION]...` arguments, `--lat`/`--lon`, `--ip`, `--station`,
  `--pick`/`--yes`: [location.md](location.md).
* **Fetching and caching** — `-p/--provider`, `-d/--days`, `-u/--units`, `--timeout`,
  `--no-cache`, `--refresh`, `--offline`: [configuration.md](configuration.md) for the
  corresponding keys, [providers.md](providers.md) for the backends.
* **Extra panels** — `--alerts`/`--no-alerts`/`--alerts-from`/`--severity`, `--aqi`/`--aqi-index`,
  `--moon`, `--normals`, `--marine`, `--date`/`--history`: described in `--help`, with the
  provider support in [providers.md](providers.md).

## Configuration

The file is `$XDG_CONFIG_HOME/cirrocast/config.toml` (`~/.config/cirrocast/config.toml`). The
blocks below ran with `XDG_CONFIG_HOME=/tmp/demo`. Precedence is **flag > `CIRROCAST_*` environment
variable > this file > built-in default**; [configuration.md](configuration.md) lists every key with
its default.

```console
$ cirrocast config path
/tmp/demo/config/cirrocast/config.toml

$ cirrocast config init
wrote /tmp/demo/config/cirrocast/config.toml

$ cirrocast config get defaults.days
3

$ cirrocast config set defaults.days 5

$ cirrocast config get defaults.days
5

$ CIRROCAST_DAYS=9 cirrocast config get defaults.days
9

$ cirrocast config validate
ok: /tmp/demo/config/cirrocast/config.toml
```

* `config init` writes a **commented** default file; `--force` overwrites an existing one. Every
  key is optional — a missing key falls back to the built-in default.
* `config get <KEY>` reads one key, and an environment override wins over the file (the
  `CIRROCAST_DAYS=9` line above).
* `config set <KEY> <VALUE>` rewrites the file in canonical form, dropping hand-written comments.
* `config validate` parses and checks the file without touching the network.
* `config edit` opens the file in `$VISUAL`/`$EDITOR` and validates it afterwards:
  `ok: /tmp/demo/config/cirrocast/config.toml`.
* `config show` prints the effective configuration as TOML regardless of whether a file exists;
  the default it prints on a fresh machine begins:

  ```console
  schema_version = 2

  [defaults]
  provider = "open-meteo"
  format = "art-table"
  units = "metric"
  days = 3
  language = "auto"
  ```

## Backends without a key

`provider list` marks who is keyless in its `NET` column; those `free` rows are usable immediately:

```console
$ cirrocast provider list
ID                  NAME                  KEY                               NET      OBS  FCST  MAXDAYS
open-meteo          Open-Meteo            none                              free     yes  yes       16
met-no              MET Norway            none                              free     yes  yes        9
open-meteo-archive  Open-Meteo Archive    none                              free     no   yes        0
open-meteo-marine   Open-Meteo Marine     none                              free     yes  yes        8
visualcrossing      Visual Crossing       CIRROCAST_VISUALCROSSING_KEY      nonfree  yes  yes       15
openweathermap      OpenWeatherMap        CIRROCAST_OPENWEATHERMAP_KEY      nonfree  yes  yes        5
weatherapi          WeatherAPI            CIRROCAST_WEATHERAPI_KEY          nonfree  yes  yes        3
worldweatheronline  World Weather Online  CIRROCAST_WORLDWEATHERONLINE_KEY  nonfree  yes  yes        5
pirateweather       Pirate Weather        CIRROCAST_PIRATEWEATHER_KEY       nonfree  yes  yes        7
qweather            QWeather              CIRROCAST_QWEATHER_KEY            nonfree  yes  yes       10
smhi                SMHI                  none                              free     yes  yes       10
brightsky           Bright Sky            none                              free     yes  yes       10
metar               METAR                 none                              free     yes  no         0
nws                 NWS                   none                              free     no   yes        7
```

`-p auto` ranks those keyless backends by how well they cover the resolved place and tries them in
order — for Beijing, `-v` reports `provider: auto for Beijing (39.91, 116.40, CN): open-meteo,
met-no`. Naming a provider explicitly (`-p smhi`) works too, and a comma-separated chain
(`-p met-no,open-meteo`) is tried left to right. What each backend covers, its rate limits and its
licence duties are in [providers.md](providers.md); `cirrocast provider info <id>` prints the same
facts for one backend.

## Setting a key

A `nonfree` row needs an API key. `key set` reads the secret from **stdin**, never from the
command line, so it cannot leak through `ps` or the shell history:

```console
$ printf '%s' "$MY_KEY" | cirrocast key set openweathermap
stored openweathermap API key in /tmp/demo/config/cirrocast/keys.toml

$ cirrocast key list
openweathermap  exam…00  (file)
```

`key list` shows only a masked form — the first four and last two characters, or just `…` for
anything shorter than eight — and the source: `file` for the `0600` `keys.toml`, or `env` when the
value came from the environment. A JWT credential instead shows its identifiers, and when both are
stored the API key gains an `api key` label beside the `jwt` one.
On a terminal, `cirrocast key set openweathermap` asks for the secret with echo off; with a pipe
(as above) it reads one line from stdin, and `--stdin` forces that even on a terminal.
`key rm openweathermap` removes every stored credential for that provider.

QWeather can also authenticate with a **JWT** — an Ed25519 private key — instead of an API key.
`--jwt` switches `key set` to that mode and needs four more flags: `--key-file` for the path to the
PKCS#8 key (or `-` to read it from stdin, like `--stdin` for an API key), then the three
identifiers the console issued for the uploaded public key — `--credential-id` (`kid`),
`--developer-id` (`iss`) and `--project-id` (`sub`):

```console
$ cirrocast key set qweather --jwt --key-file ~/.config/cirrocast/qweather.pem \
    --credential-id cred000001 --developer-id Qtestdev12 --project-id proj000001
stored qweather JWT credential in /tmp/demo/config/cirrocast/keys.toml

$ cirrocast key list
qweather  jwt (kid cred000001, iss Qtestdev12, sub proj000001)  (file)
```

`key list` prints the identifiers but never the key itself, which lives in the same `0600`
`keys.toml` and is never logged. QWeather accepts an ordinary API key too (`CIRROCAST_QWEATHER_KEY`);
the JWT mechanics, the `CIRROCAST_QWEATHER_JWT_*` environment quartet and the console steps are in
[providers.md](providers.md#qweather).

The environment is the alternative — the variable name is the `KEY` column above:

```console
$ CIRROCAST_OPENWEATHERMAP_KEY="$MY_KEY" cirrocast :Beijing -p openweathermap
```

An environment key is used for that run and nothing is written to disk. Without either, a keyed
provider fails fast with exit 6:

```console
$ cirrocast :Beijing -p weatherapi
error: missing API key for weatherapi: run `cirrocast key set weatherapi` or set CIRROCAST_WEATHERAPI_KEY in the environment
$ echo $?
6
```

Never paste a real secret into a config file, a shell history or a bug report; `-v` redacts keys in
its output.

## Offline mode

`--offline` takes an optional scope:

| Flag | Weather | Name resolution |
|---|---|---|
| `--offline` / `--offline=all` | cache only | bundled city table only — no socket at all |
| `--offline=weather` | cache only | the normal geocoders, still online |
| `--offline=geo` | normal, still online | bundled city table only |

With a cold cache there is nothing to serve:

```console
$ cirrocast --offline :Beijing
error: all providers failed: open-meteo (network: offline: no cached open-meteo forecast for Beijing (39.91, 116.40) at weather/open-meteo-39.91-116.40-3-2026-10-06.json; rerun without `--offline` to fetch it)
$ echo $?
3
```

After a normal run has cached the place, the same command renders from the cache, however old the
entry is. Name resolution keeps working offline because the released binary embeds a GeoNames
`cities15000` snapshot:

```console
$ cirrocast location search --offline Beijing
Beijing, CN (39.91, 116.40) Asia/Shanghai
```

`--offline=geo` is the partial case — names come from the bundled table while the weather is still
fetched live:

```console
$ cirrocast --offline=geo :Beijing -f one-line
Beijing: *o* Clear sky +20°C (+16°C), ↙ 11km/h SW, 30%, 0.0mm, 1021hPa, 18km
```

The subcommands that never touch the network — `config`, `key`, `provider`, `cache`, `completion`
and `man` — work as usual. `location search --offline` resolves from the bundled table, and
`location update-data --from <PATH>` reads a local dump (`--offline` is refused there: the update
fetches by definition). To make one scope the standing default, set the key (values `off`,
`weather`, `geo`, `all`) instead of passing the flag each time:

```console
$ cirrocast config set network.offline all
$ cirrocast :Beijing            # now behaves like --offline
error: all providers failed: open-meteo (network: offline: no cached open-meteo forecast for Beijing (39.91, 116.40) at weather/open-meteo-39.91-116.40-3-2026-10-06.json; rerun without `--offline` to fetch it)
```

A command-line `--offline=…` overrides the file for that run. The probe in
[ecosystem.md](ecosystem.md) has its own, softer offline rule for status bars — a stale cached line
beats no line.

## Where to go next

* [configuration.md](configuration.md) — every key, its type, its default and its effect, plus the
  precedence ladder.
* [location.md](location.md) — fuzzy vs `:exact` vs `~osm` vs `@lat,lon` vs alias, and the ranking.
* [formats.md](formats.md) — every output format, the `%`-token table and the multi-location rules.
* [providers.md](providers.md) — the backends, their auth, coverage, rate limits and attribution.
* [schema.md](schema.md) — the `json` and configuration schemas with the key index.
* [ecosystem.md](ecosystem.md) — the `status` probe, the frozen output contracts and status-bar
  recipes.
* [performance.md](performance.md) — the enforced time and memory numbers.
* [troubleshooting.md](troubleshooting.md) — network, keys, cache corruption and bug reports.
* [architecture.md](architecture.md) — the module map and the request data flow.
* [i18n.md](i18n.md) — adding a language.
* [CONTRIBUTING.md](../CONTRIBUTING.md) — the release checklist and the packaging detail behind the
  install paths above.
