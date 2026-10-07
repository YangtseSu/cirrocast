<!--
SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Troubleshooting

Every failure `cirrocast` can hit maps to one of the exit codes below, prints one `error: …` line
on stderr, and usually names its own fix. This file walks the common failure modes by symptom and
shows the command that diagnoses each one; the quoted text is what the shipped binary prints. It
owns the *diagnosis*, not the settings: the config keys, their types and defaults are in
[configuration.md](configuration.md), the provider quotas and the chain contract in
[providers.md](providers.md), the location syntax and ranking in [location.md](location.md), and
where state lives on disk in [architecture.md](architecture.md).

Diagnose in this order — it costs one run and narrows the cause fast:

```console
$ cirrocast --version           # which build
$ cirrocast -vv Beijing         # settings, resolution, every request and cache decision
$ cirrocast config validate     # the config file, without a request
$ cirrocast key list            # credentials, masked
$ cirrocast provider info open-meteo   # one backend's registry row
```

## Exit codes

The values are the CLI's public contract; `cirrocast --help` prints the same table.

| code | meaning |
|---|---|
| `0` | success |
| `1` | generic failure: an unexpected error outside the classes below |
| `2` | usage: a flag or value the command line rejects, or mutually exclusive flags |
| `3` | network or upstream failure: no connection, a retryable status, an unusable body |
| `4` | configuration or state on disk: `config.toml`, `keys.toml`, the cache |
| `5` | location not found: an unresolvable name or station |
| `6` | missing or invalid API key: `cirrocast key set <id>` (or `CIRROCAST_<ID>_KEY`) fixes it |

A multi-location run keeps each failed slot on stdout (`error: <query>: <message>`) and exits with
the largest mapped code among the failures.

## Network, TLS and proxy

### The host is unreachable or the connection is refused

A single-provider run reports the request it tried and the transport error. This is a refused
connection behind a dead proxy (`HTTPS_PROXY=http://127.0.0.1:9`), the shape every transport failure
takes:

```console
$ HTTPS_PROXY=http://127.0.0.1:9 cirrocast -p open-meteo Beijing
error: all providers failed: open-meteo (network: GET https://api.open-meteo.com/v1/forecast?… failed after 3 attempts: connection failed: Connection refused (os error 111))
$ echo $?
3
```

The URL is the cache key's request, printed in full. `-v` prints it once, `-vv` prints one line per
retry with the reason and the wait (the URL is redacted when it carries a credential — see
[API keys](#api-keys)):

```console
$ HTTPS_PROXY=http://127.0.0.1:9 cirrocast -vv -p open-meteo --yes Beijing
http: GET https://api.open-meteo.com/v1/forecast?… attempt 2/3 after connection failed: Connection refused (os error 111); sleeping 0.5 s
http: GET https://api.open-meteo.com/v1/forecast?… attempt 3/3 after connection failed: Connection refused (os error 111); sleeping 1.0 s
```

Retries are bounded: `network.retries` (default `3`) is the number of *extra* attempts, and the
total is clamped to `min(1 + retries, 3)`. Backoff is `0.5 s`, `1 s`, `…`; a `429`/`503` carrying
`Retry-After` uses that instead (clamped to a minute). `-vv` is the only place the retry schedule is
visible. An error that says `failed after N attempts` was retried; one that says only `failed` was
not (a single attempt, or a permanent transport error such as a TLS failure).

### A timeout

`--timeout <SECS>` (or `network.timeout_secs`, default `15`) bounds the connect, the response header
and the body separately as well as the whole request. A host that accepts the socket and then says
nothing produces:

```console
$ CIRROCAST_NOMINATIM_URL=http://10.255.255.1 cirrocast --timeout 2 '~Beijing'
error: network error: GET http://10.255.255.1/search?format=jsonv2&q=Beijing&limit=10&addressdetails=1&extratags=1 failed: timeout while connecting
$ echo $?
3
```

The message distinguishes `timeout while connecting` (the socket never opened) from a timeout on the
response or body; raise `--timeout` only if the upstream is genuinely slow, since a large value also
delays the error for a dead host.

### A proxy

`[network] proxy` takes an `http://` or `https://` URL (or `host:port`), and when it is empty the
environment is consulted: `HTTPS_PROXY`/`https_proxy`, `HTTP_PROXY`/`http_proxy`, `ALL_PROXY`/`all_proxy`,
with `NO_PROXY` honoured by the HTTP stack. `NO_PROXY` lets a direct host leak through a broken
proxy, which is the fastest way to prove the proxy is the problem:

```console
$ HTTPS_PROXY=http://127.0.0.1:9 NO_PROXY=api.open-meteo.com cirrocast -q -f one-line Beijing
Beijing: *o* Clear sky +20°C (+16°C), , 11km/h SW, 30%, 0.0mm, 1021hPa, 18km
```

A value the binary cannot use is a configuration error, not a silent fallback — `config set` refuses
it up front with the same wording a run would print:

```console
$ cirrocast config set network.proxy 'not a url %'
error: config error: network.proxy: `not a url %` is not an `http://` or `https://` proxy URL
$ echo $?
4
```

### SOCKS is refused on purpose

`ureq` accepts a `socks5://` URL and then ignores it without a warning (no logger is installed), so
the request would go out direct with no trace. Any SOCKS scheme in `ALL_PROXY`, `HTTPS_PROXY` or
`HTTP_PROXY` (upper- or lowercase) is refused before a socket opens:

```console
$ ALL_PROXY=socks5://127.0.0.1:1080 cirrocast -p open-meteo Beijing
error: config error: ALL_PROXY `socks5://127.0.0.1:1080` is a SOCKS proxy, which cirrocast does not support; use an `http://` or `https://` proxy (in `[network] proxy` or the environment)
$ echo $?
4
```

Use an HTTP(S) proxy, or a SOCKS-to-HTTP bridge, instead.

### `CIRROCAST_FORBID_NETWORK` (developers and CI)

Setting `CIRROCAST_FORBID_NETWORK` to a non-empty value other than `0` disables every outbound
connection before DNS is attempted, so a test run can prove it never touched the network. It is read
once per process, and loopback URLs stay reachable (an in-process stub can still answer). A guarded
run fails with the guard's own message:

```console
$ CIRROCAST_FORBID_NETWORK=1 cirrocast -p open-meteo Beijing
error: all providers failed: open-meteo (network: GET https://api.open-meteo.com/v1/forecast?… failed: outbound network access is disabled by CIRROCAST_FORBID_NETWORK)
```

Unset it (or set it to `0`/empty) for live runs.

## API keys

Keys are resolved separately from `config.toml`: `CIRROCAST_<PROVIDER>_KEY` first, then
`keys.toml`. There is no third tier, and credentials are never written to `config.toml` (which is
shipped in bug reports). [configuration.md](configuration.md#credentials-are-resolved-separately)
owns the precedence table; this section is the failure half.

### Missing key — exit 6

A keyed backend with no key fails with the fix in the message and exit `6`:

```console
$ cirrocast -p openweathermap Beijing
error: missing API key for openweathermap: run `cirrocast key set openweathermap` or set CIRROCAST_OPENWEATHERMAP_KEY in the environment
$ echo $?
6
```

The environment alternative takes the same value inline, without touching the store:

```console
$ CIRROCAST_OPENWEATHERMAP_KEY=… cirrocast -p openweathermap Beijing
```

A key the upstream rejects is also exit `6` (the message differs so you know to replace it, not
resupply it):

```console
$ CIRROCAST_OPENWEATHERMAP_KEY=wrong cirrocast -p openweathermap Beijing
error: provider openweathermap rejected the API key (HTTP 401): replace it with `cirrocast key set openweathermap`
```

### `keys.toml` is group/world-readable

The store must be `0600`. A looser mode is refused on load, before any request, with the exact mode
and the fix (the file it names is the one it read, under
`$XDG_CONFIG_HOME/cirrocast/keys.toml`):

```console
$ chmod 0644 "$XDG_CONFIG_HOME/cirrocast/keys.toml"
$ cirrocast -p openweathermap Beijing
error: config error: /home/you/.config/cirrocast/keys.toml is readable by group/other (mode 0644); run `chmod 600 /home/you/.config/cirrocast/keys.toml`
$ echo $?
4
```

This is exit `4` (state on disk), and it also blocks `key list` and `key rm`, because the file is not
read at all until it is fixed. `key set` writes the file `0600`, so a fresh store never hits this.

### Storing a key — stdin only

`key set` never takes the secret on the command line (`ps` and the shell history would see it); it
reads stdin, or prompts. An empty input is a usage error:

```console
$ printf '' | cirrocast key set openweathermap
error: no API key given; pipe it in or answer the prompt
$ echo $?
2
```

`--stdin` forces the stdin read even when stdin is a terminal. The store masks on read back:

```console
$ printf 'super-secret-key-ABCDEF0123456789\n' | cirrocast key set openweathermap
stored openweathermap API key in /home/you/.config/cirrocast/keys.toml
$ cirrocast key list
openweathermap  supe…89  (file)
```

The mask keeps the first and last few characters and joins them with a single `…`; the full value is
never printed. `(file)` is the source — a key taken from the environment instead shows the
environment. `key rm <id>` removes every stored credential of a provider (`no openweathermap API key
stored` when there was none).

### QWeather JWT credentials

QWeather accepts a JWT (Ed25519) credential instead of a query key. `key set <id> --jwt` needs four
flags, all required together; omitting them is a usage error (`2`):

```console
$ printf 'x' | cirrocast key set qweather --jwt
error: the following required arguments were not provided:
  --key-file <PATH>
  --credential-id <ID>
  --developer-id <ID>
  --project-id <ID>
```

`--key-file <PATH>` is the PKCS#8 Ed25519 private key (PEM text; `-` reads it from stdin, as for the
console-generated file). `--credential-id` is the console's `kid`, `--developer-id` its `iss` (ten
characters starting with `Q`), `--project-id` its `sub`. A stored JWT is listed by its identifiers
only — no key material ever leaves the file:

```console
$ cirrocast key set qweather --jwt --key-file ~/ed25519.pem \
    --credential-id kid123 --developer-id Q123456789 --project-id proj42
stored qweather JWT credential in /home/you/.config/cirrocast/keys.toml
$ cirrocast key list
qweather  jwt (kid kid123, iss Q123456789, sub proj42)  (file)
```

QWeather's JWT also needs its account host; without it the run is a configuration error (exit `4`) and
names the setting:

```console
$ CIRROCAST_QWEATHER_KEY=dummy cirrocast -p qweather Beijing
error: config error: provider `qweather` needs its account API host: set providers.qweather.host (see https://console.qweather.com/setting, or `cirrocast provider info qweather`)
```

### Never paste a key into a bug report

`keys.toml` is never part of a report, and neither is any `CIRROCAST_*_KEY` value. The binary
redacts a secret wherever it logs a request — the `-vv` line shows `appid=***` for a query
credential and `***` for a header one — but a key you pasted once is compromised and must be rotated
at the provider. `config.toml` is safe to paste (it holds no credentials); `config show` and
`provider info` never print a secret. See [Producing a bug report](#producing-a-bug-report).

## Cache

The cache lives under `$XDG_CACHE_HOME/cirrocast/`, one JSON file per entry in a namespace
directory (`weather/`, `alerts/`, `geocode/`, …); [architecture.md](architecture.md#cache-layout-and-namespaces)
owns the layout. `cache stat` shows what each namespace holds:

```console
$ cirrocast cache stat
weather      1 entry      6.6 kB   oldest 2026-10-06T13:55:52Z   newest 2026-10-06T13:55:52Z
geocode      0 entries       0 B
ip           0 entries       0 B
station      0 entries       0 B
alerts       10 entries  115.3 kB   oldest 2026-10-06T13:55:53Z   newest 2026-10-06T13:56:01Z
grid         0 entries       0 B
normals      0 entries       0 B
ratelimit    0 entries       0 B
geo          0 entries       0 B
```

### A corrupted entry

A truncated or hand-edited file does not abort the run: `-vv` reports it, the entry is ignored, and
the request is made again (the fresh body replaces it):

```console
$ cirrocast -vv --yes -p open-meteo Beijing
cache: $XDG_CACHE_HOME/cirrocast/weather/open-meteo-39.91-116.40-3-2026-10-06.json: unreadable (expected ident at line 1 column 2), ignoring
cache: $XDG_CACHE_HOME/cirrocast/weather/open-meteo-39.91-116.40-3-2026-10-06.json: wrote 6594 bytes
```

The parenthesised part is the JSON parser's own complaint; the rest is the same for any bad entry.
Nothing needs clearing for this case.

### Clearing the cache

The subcommand is `clean`, not `clear` (`clear` is an unknown-subcommand usage error that suggests
it). `clean` removes only expired entries; `--all` removes everything:

```console
$ cirrocast cache clean
removed 0 expired entries
$ cirrocast cache clean --all
removed 11 entries
```

`cache clean --offline` is refused on purpose, since offline mode writes (and deletes) nothing:

```console
$ cirrocast cache clean --offline
error: offline mode: cache writes are disabled
$ echo $?
2
```

To sidestep the cache for a single run without deleting anything, use `--no-cache` (ignore it and
store nothing) or `--refresh` (replace entries with fresh answers). `-vv` shows the cache decision
per request — `hit`, `miss`, or `wrote N bytes` — and `cache no-cache` when `--no-cache` is in
effect.

### TTLs

How long an entry stays valid is per namespace and configurable; the defaults and their exact
ranges are in [configuration.md](configuration.md#cache) (`cache.weather_ttl_secs`,
`cache.ip_ttl_secs`, `cache.geocode_ttl_secs`, `alerts.cache_ttl_secs`). To make the whole cache
irrelevant without deleting it, `--no-cache` or `--refresh` is the flag-level control.

## Providers

### Rate limits and fallthrough

A `429` or `5xx` is retried (see [retries](#the-host-is-unreachable-or-the-connection-is-refused))
and, when it survives, the chain moves to the next backend; a `401` that means "bad credential"
stops the chain at exit `6`. The per-upstream mapping, with each provider's documented quota, is in
[providers.md](providers.md#status-code-behaviour-the-chain-contract-applied-per-upstream). A chain
in which every backend failed prints one row per attempt, in chain order:

```console
$ HTTP_PROXY=http://127.0.0.1:9 cirrocast -p met-no,open-meteo Beijing
error: all providers failed: met-no (network: GET https://api.met.no/weatherapi/locationforecast/2.0/compact?lat=39.9075&lon=116.3972 failed after 3 attempts: connection failed: Connection refused (os error 111)); open-meteo (network: …)
```

One `error:` line, semicolon-separated attempts — the last backend's message is often the one that
matters, and each row names its own provider. `-v` prints the chain `auto` chose before the request.

### No key configured for the named backend

Naming a keyed backend in `--provider` is what triggers the missing-key failure; `auto` only ever
selects keyless entries, so a default run cannot hit it. The message names that backend and its
environment variable (see [Missing key](#missing-key--exit-6)). `provider info <id>` prints the
`key:` row, so you can confirm the variable name without guessing:

```console
$ cirrocast provider info openweathermap
key:         CIRROCAST_OPENWEATHERMAP_KEY
store key:   cirrocast key set openweathermap
```

An unknown id is a usage error that lists every known backend:

```console
$ cirrocast provider info nosuch
error: unknown provider `nosuch`; known providers: open-meteo, met-no, open-meteo-archive, open-meteo-marine, visualcrossing, openweathermap, weatherapi, worldweatheronline, pirateweather, qweather, smhi, brightsky, metar, nws
```

### Archive, marine and normals

These are opt-in surfaces with their own preconditions, and a bare run stays quiet when they are
absent.

`--date`/`--history` need a backend with a history span:

```console
$ cirrocast --history 3d -p brightsky Beijing
error: `--date`/`--history` need a backend with a history span; add `open-meteo` or `open-meteo-archive` to `--provider`
$ echo $?
2
```

A backend with a span can still refuse a date outside it:

```console
$ cirrocast --date 2020-01-01 -p open-meteo Beijing
error: provider open-meteo can answer for 92 days back at most; the requested window starts 2020-01-01 (earliest 2026-07-06)
$ echo $?
2
```

`--marine` and `--normals` degrade with a warning instead of failing the run when the source has
nothing for the point. `--normals` fetches the nearest NOAA NCEI station within `normals.max_distance_km`
(default `60`); an inland or remote point with no station in range prints:

```console
$ cirrocast --normals -f normals '@0,-40'
climate normals unavailable
```

`--marine` appends the wave block and warns when the marine model has no reading:

```console
$ cirrocast --marine -f one-line Ulaanbaatar
warning: marine data unavailable: provider open-meteo-marine failed: the response carries no marine reading for Ulan Bator (47.91, 106.88)
Ulan Bator: ~~~ Overcast +6°C (+3°C), > 8.3km/h E, 63%, 0.0mm, 1014hPa, 65km
```

The table and one-line output ignore the missing block. `--normals` appends the comparison and
`-f normals` prints it standalone; `--marine` appends the wave block to the table and `plain` output.
[formats.md](formats.md) owns the format list. The provider quotas those extra requests count
against are in [providers.md](providers.md).

## Locations

The argument syntax, the ranking rules and the picker are owned by
[location.md](location.md); these are the failure shapes.

### A name that matches several places

The ranked winner is used and a note on stderr says so — it is not an error:

```console
$ cirrocast Beijing
note: 3 candidates for `Beijing`; using Beijing, CN (population 18960744) — pass `:Beijing` to require an exact name match, `--pick` to choose one, or `--yes` to keep the winner
```

The three fixes in the note, in order of determinism:

* `:Beijing` requires the name to match exactly (folded), skipping fuzzy candidates;
* `--pick` prompts on stderr and reads one line from stdin — even when stdin is a pipe;
* `--yes` keeps the winner with no prompt (already the default on a non-terminal run).

`--pick` with a piped answer works, and each candidate prints its coordinates so you can paste them
back as an unambiguous `@lat,lon` next time:

```console
$ printf '2\n' | cirrocast --pick -f one-line Springfield
[1] * Springfield, US (37.22, -93.30) America/Chicago (pop. 170188)
[2]   Springfield, US (42.10, -72.59) America/New_York (pop. 154341)
…
choose a location [1-10, Enter=1, q=quit]: selected: Springfield, US — use @42.10148,-72.58981 to skip the prompt
```

`q` (or end of input) aborts as a location failure, and a bad answer re-prompts:

```console
$ echo q | cirrocast --pick Beijing
choose a location [1-3, Enter=1, q=quit]: error: location not found: no location selected for Beijing
```

### Unresolvable or malformed

A name no source knows is exit `5`:

```console
$ cirrocast Zzzqqqxyz
error: location not found: no location found for `Zzzqqqxyz` (no offline match)
$ echo $?
5
```

An out-of-range coordinate is a usage error (`2`), because it is a command-line mistake rather than
an upstream refusal:

```console
$ cirrocast '@999,999'
error: unknown location alias `@999,999` (`999,999` is not inside the world: latitude -90..=90, longitude -180..=180); no `[locations]` aliases are configured (accepted forms: Beijing | :Beijing | ~Tsinghua | @39.9042,116.4074 | @name (an alias from [locations]))
```

Use `~Tsinghua` for an OpenStreetMap place and `@lat,lon` for a coordinate; `location search
[--all] <QUERY>` resolves a name without fetching weather, which is the quickest way to see *which*
place an argument means before blaming the forecast.

### Stations

`--station <ICAO>` takes exactly four characters beginning with a letter; a malformed value is
rejected by the argument parser (exit `2`), and an identifier no station table knows is exit `5`:

```console
$ cirrocast --station AB -f one-line
error: invalid value 'AB' for '--station <ICAO>': `AB` is not an ICAO station identifier; expected four characters starting with a letter, e.g. `--station EGLL`
$ cirrocast --station ZZZZ -f one-line
error: location not found: unknown station `ZZZZ`; check the identifier or pass coordinates (`cirrocast -p metar @lat,lon`) to use the nearest station
```

A station that reports only some fields renders the missing ones literally rather than inventing a
value — `-.-` for an absent condition, `n/a` for an absent feels-like:

```console
$ cirrocast --station PAFA -f one-line
Fairbanks Intl: -.- Mist +1°C (n/a), / 5.6km/h NE, 86%, 0.0mm, 1000hPa, 9.7km
```

Give coordinates instead of an identifier to use the nearest station that does report. A station that
exists in the table but has no current report is an upstream failure for that backend (exit `3`), not
an empty table — a `metar`-headed chain can fall through to a forecast backend, and the message names
the station and the empty body.

## Weather data that is simply absent

These are not failures — the run succeeds (exit `0`) and the surfaces say what is missing.

* **No alerts.** `-f alerts` prints the contract line, and one-line output simply omits the alert
  line:
  ```console
  $ cirrocast -f alerts Beijing
  no active weather alerts
  ```
  Alerts are on by default; `--no-alerts` turns them off, and `[alerts] enabled = false` does so
  permanently. A source you name that does not cover the point is a usage error naming the covered
  set:
  ```console
  $ cirrocast --alerts-from nws Beijing
  error: alert source `nws` does not cover 39.91,116.40; covered here: qweather, wmoswic, fpas, visualcrossing
  ```
  `visualcrossing`'s warnings travel inside its weather payload, so it needs its provider:
  ```console
  $ cirrocast --alerts-from visualcrossing Beijing
  error: alert source `visualcrossing` travels with its provider's payload; add `--provider visualcrossing`
  ```
* **AQI fields a source does not carry.** The panel prints the missing row instead of a zero, e.g.
  `Pollen: not covered at this location`. If the panel was requested but the air fetch returned no
  reading at all, the surface says so in place of the block rather than fabricating values.
* **No normals.** Covered by `climate normals unavailable` under
  [Archive, marine and normals](#archive-marine-and-normals).

## Producing a bug report

A good report lets a maintainer reproduce the failure without the network state you had. It opens
with the build and platform:

```console
$ cirrocast --version
cirrocast 1.4.0
$ uname -srm
Linux 7.2.9-1-cachyos x86_64
```

and then attaches, verbatim:

* the `-vv` transcript of the failing command (also `-vv` on a working run of the same command, if
  the failure is intermittent). It prints the settings in effect, the location resolution, the chain
  chosen, and every request and cache decision, and it is **redacted**: a credential is shown as
  `***` (e.g. `appid=***`), never in full. Read it once before pasting anyway;
* the `provider info <id>` row for the failing backend — its `verified` date, `limits`, `key` and
  `network` class say which upstream and which terms were in play;
* `config show`, the *effective* configuration as TOML with every `CIRROCAST_*` override applied. It
  is safe to paste because it holds no credentials — keys live in `keys.toml`, which is never
  included.

Never paste:

* the contents of `keys.toml`, or any `CIRROCAST_*_KEY` value, even partially (a masked
  `key list` line is fine);
* a raw `-vv` capture you have not read — if you exported a key variable inline into the command,
  rotate it anyway;
* your public IP if you ran `--ip` and would rather not disclose it (the location section of `-vv`
  names the resolved place).

The issue template asks for exactly these fields. For a security problem, follow
[SECURITY.md](../SECURITY.md) instead of opening a public issue.
