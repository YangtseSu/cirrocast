<!--
SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Step 20 — wttr-compatible local service

Status: not-started
Depends on: 08 (CLI surface and formats), 10 (provider chain), 14 (v1 acceptance)
Touches: `src/serve/{mod,request,response,query,token,help}.rs`, `src/cli.rs`, `src/lib.rs`,
`src/error.rs`, `Cargo.toml`, `AGENTS.md`, `docs/wttr-compat.md`, `tests/serve_compat.rs`,
`tests/fixtures/serve/`, `REUSE.toml`

## Goal

`cirrocast serve --bind 127.0.0.1:8642` runs a foreground HTTP/1.1 service whose request surface
mirrors wttr.in, so an existing script that does `curl wttr.in/Beijing` keeps working after the host
string is changed to `http://127.0.0.1:8642/Beijing`. The service owns no weather logic: every
request is parsed into the same invocation the CLI builds and rendered by the same renderer, so a
fixed query returns bytes identical to the equivalent `cirrocast` command line. Loopback by default,
`--allow-remote` plus a bearer token for anything else, no directory serving, one access-log line per
request on stderr.

## Deliverables

- [ ] `Cargo.toml`: add `ctrlc = { version = "3.5", features = ["termination"] }` (MIT OR Apache-2.0);
      no other new dependency — the HTTP layer is hand-written on `std::net`.
- [ ] `src/cli.rs` + `src/serve/mod.rs`: `cirrocast serve --bind <ADDR:PORT> [--allow-remote]
      [--token-env <VAR>]`; default bind `127.0.0.1:8642`; `serve` rejects a non-loopback `--bind`
      without `--allow-remote` and rejects `--allow-remote` without `--token-env` (`Error::Config`,
      exit 4) before it binds anything.
- [ ] `src/serve/request.rs`: hand-rolled HTTP/1.1 request parser — `GET`/`HEAD` only (405 with
      `Allow: GET, HEAD` otherwise), request line ≤ 2048 B, header block ≤ 8192 B, ≤ 64 header
      lines, no request bodies, `HTTP/1.0` accepted and always answered with `Connection: close`,
      malformed bytes → 400, oversize → 431.
- [ ] `src/serve/response.rs`: response writer — status line, `Content-Type: text/plain;
      charset=utf-8`, `Content-Length`, `Cache-Control`, `Connection`, `Allow`, and the
      informational `X-Cirrocast-Format` / `X-Cirrocast-Ignored` headers; `HEAD` sends headers with
      an empty body.
- [ ] `src/serve/query.rs`: query string → internal invocation: `%`-decoding, `+` → space,
      location path decoding, the letter-option cluster (`m u M 0 1 2 3 A d F n q Q T`), the long
      options `format= lang= period=`, unknown option → 400 listing the supported set.
- [ ] `src/serve/token.rs`: bearer check (`Authorization: Bearer <t>` or `?token=<t>`), constant-time
      byte comparison, `--token-env` read once at startup, empty/missing variable refused.
- [ ] `src/serve/help.rs`: the `/?` and `/:help` page, generated from the same option table the
      parser consults, so page and parser cannot drift.
- [ ] `src/serve/mod.rs`: bounded worker-thread pool (default `min(available_parallelism, 4)`), accept
      backlog 32, excess connections answered 503 then closed, 5 s header-read timeout, 10 s
      whole-request timeout, `SIGINT`/`SIGTERM` → stop accepting, drain ≤ 5 s, exit 0; one access-log
      line per request on stderr (`-q` silences it), `-v` adds cache hit/miss and upstream time.
- [ ] `tests/serve_compat.rs`: spawns the real server on port 0 in-process (ephemeral port read back
      from `local_addr()`), requests it with `ureq`, and asserts byte-identity against the equivalent
      CLI render call for the frozen-clock/`StubTransport` fixture path.
- [ ] `docs/wttr-compat.md`: the option table enumerated from wttr.in's own current documentation
      (`wttr.in/:help` and its repository README) and re-authored in our own words, the mapping onto
      our flags, and the deliberate non-port list with per-item reasons.
- [ ] `AGENTS.md`: amend the non-goal "no daemon or server mode" to "no background daemon; `serve` is
      a foreground, loopback-by-default compatibility service".
- [ ] `REUSE.toml`: annotation for every new non-commentable file (the `tests/fixtures/serve/` HTTP
      request/response transcripts).

## Design notes

* **Hand-written HTTP/1.1, not `tiny_http`.** `tiny_http` 0.12 (MIT OR Apache-2.0, edition 2018,
  MSRV 1.57, deps `ascii`+`chunked_transfer`+`httpdate`+`log`) is licensed fine and is synchronous,
  but its `ServerConfig` exposes only `addr` and `ssl`: there is no header-size limit and no
  per-request read timeout to configure, and it runs its own internal task pool, which we would have
  to wrap rather than own. The security rules this step must enforce (request-line and header
  budgets, read timeouts, bounded workers, graceful drain, no directory serving) are therefore
  exactly the code we would still have to write, plus a dependency. Rejected alternatives:
  `hyper`/`axum` (async runtime, forbidden by the contract), `rouille` (pulls `tiny_http` plus its
  own multipart/logger stack), `std::net` + `tiny_http` hybrid (two parsers for one job).
* **Signals need a crate.** `std` exposes no signal API and `unsafe_code = "forbid"` means we cannot
  register a handler ourselves, so `ctrlc` (MIT OR Apache-2.0, MSRV 1.69, `nix`/`windows-sys`, no
  async) with the `termination` feature is the minimal way to get SIGINT+SIGTERM. `signal-hook` was
  the other candidate; it is a larger surface for a use we do not have (no signal mask juggling).
* **Byte-identity is a rendering property, not a promise about the network.** Both paths share
  `Env` (client, cache, config), the provider chain, the renderer and `RenderContext`. The header
  timestamp is the report's observation time (cache fetch time when upstream omits one), never the
  request clock, so two runs against the same cache entry inside the same minute agree byte for byte;
  the test freezes the clock on both paths and therefore compares exactly.
* **`Env` must be `Send + Sync`.** The contract already requires `Provider::fetch(&self)` with no
  interior mutability and `ureq::Agent` is `Send + Sync`; the cache uses tmp+rename, so concurrent
  workers are safe. This is a compile-time assertion (`fn assert_sync<T: Send + Sync>()`), not a
  redesign. Do not add a per-worker `Env` clone beyond the `Arc`.
* **`format=j1` is a documented shape change, not an alias.** wttr.in's `j1` is a WorldWeatherOnline
  shaped document; we serve our own versioned JSON (schema shared with step 23). Scripts that parse
  individual WWO fields must be changed; the option table says so explicitly. `j2` is not served at
  all.
* **`?A` and `?T` map onto the colour decision**, because a plain-text service cannot sniff a
  terminal: the compat surface defaults to no colour, `?A` sets `--color always`, `?T` keeps
  `--color never`, and both are overridden by `NO_COLOR` only if the user exported it for the server
  process (documented, surprising otherwise).
* **`?F` is accepted and ignored on purpose.** wttr.in prints a "Follow" line; we never do. Ignoring
  it silently would hide the difference from the user, so the response carries
  `X-Cirrocast-Ignored: F` and the option table lists it as a no-op with the reason.
* **Non-port list (each with its reason), mirrored in `docs/wttr-compat.md`:**
  * `*.png`, `?p`, `?t`, `transparency=`, `background=`, `format=p1` (PNG) — needs font rasterisation
    and text shaping: a bundled font (≥ 1 MB) plus a rasteriser crate (≥ 300 KB of code), which alone
    eats the step 22 binary budget of 5 MB; also uncomparable to CLI output.
  * `format=p1` (Prometheus) — a new renderer plus a metric-name stability contract; scripts can read
    `format=j1` instead.
  * `v2`/`v3` host aliases, `format=v2|v2d|v2n|v3`, `.sxl` map suffixes — a second and third layout
    engine plus geometry; the data-rich fields already appear in our `art-table` and `json`.
  * HTML output (browser User-Agent) — no HTML renderer, and re-authoring one is a separate product.
  * `/moon`, `/moon@YYYY-MM-DD` — we expose the moon as the `%m`/`%M` tokens of step 17, not as a
    location sub-resource; `/moon` would need a second URL namespace for one field.
  * `@<domain>` locations, ZIP/area codes (`/94107`), IATA 3-letter codes (`/muc`) — no DNS+reverse
    geocoding path, no postal-code dataset, no IATA table (we ship ICAO stations for METAR);
    coordinate, city, `:exact`, `~osm` and alias forms cover the same intent explicitly.
  * `de.wttr.in`-style DNS-prefix language selection — no host-based negotiation; `?lang=` and
    `Accept-Language` are served.
  * TLS — termination belongs to a reverse proxy (`--allow-remote` is documented as requiring one).
  * Geolocation of the requester's IP — privacy rule 11 of `AGENTS.md`; an empty location is resolved
    from the *server operator's* IP only if configured, otherwise 400.
* **Status mapping.** 200 render, 400 bad option/location syntax, 401 missing/incorrect token,
  404 location not found, 405 wrong method, 413/431 oversize, 500 internal, 502 upstream failure
  (`Error::Upstream`/`Network`), 503 overloaded/queue full. Every error body is one plain-text line
  starting `error:`, matching the CLI's stderr wording.

## Out of scope

PNG/SVG/HTML rendering, TLS, upstream (`wttr.in`) proxying, requester-IP geolocation, rate limiting
(job for the reverse proxy that also terminates TLS), a systemd unit (step 13 packaging covers the
binary only), and any wttr.in *data* reuse: only the documented option surface is implemented, from
its published documentation and from this repo's renderers.

## Verification

```bash
cargo run -q -- --refresh Beijing                              # prime the cache
cargo run -q -- serve --bind 127.0.0.1:8642 &                  # foreground service
curl -s  'http://127.0.0.1:8642/Beijing'                       # art-table, no colour
curl -s  'http://127.0.0.1:8642/Beijing?format=%l:+%c+%t'      # one-line template
curl -s  'http://127.0.0.1:8642/Beijing?m0T'                   # metric, current only, plain
curl -si 'http://127.0.0.1:8642/?T' | head -n 5                # Content-Type + Cache-Control
diff <(cargo run -q -- Beijing -f art-table --color never --width 80) \
     <(curl -s 'http://127.0.0.1:8642/Beijing?T')              # byte-identical, same minute
curl -s  'http://127.0.0.1:8642/Beijing?z9' ; echo $?          # unsupported option: 400 + list
cargo run -q -- serve --bind 0.0.0.0:8642 ; echo $?            # refused, exit 4, no bind
cargo run -q -- serve --bind 0.0.0.0:8642 --allow-remote ; echo $?   # refused (no token), exit 4
```

Observable result: the four data requests return 200 with `text/plain; charset=utf-8`, the `diff`
prints nothing, the unsupported option returns a 400 whose body lists the supported options, and both
non-loopback invocations exit 4 without opening a socket.

## Exit criteria

- [ ] `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`, `reuse lint` clean.
- [ ] The smoke block above runs as written; the `diff` is empty and `curl -si` shows
      `Content-Type: text/plain; charset=utf-8` and a `Cache-Control` whose `max-age` equals the
      remaining cache TTL.
- [ ] `tests/serve_compat.rs` proves byte-identity for `?T`, `?m0T`, `?format=3`, `?lang=zh`,
      `?n`, `?q`, `?Q` and rejects a remote bind without a token.
- [ ] `kill -TERM` on the running service leaves exit code 0 and closes the listening socket within
      5 s.
- [ ] `docs/wttr-compat.md` lists every served option, its mapping, and the non-port list with reasons.
- [ ] No unsupported option is accepted silently: it is 400, or a no-op declared in
      `X-Cirrocast-Ignored` and in the option table.

## Risks

* Loopback-only usefulness: many wttr.in scripts live in CI or remote shells; the answer is a
  documented reverse proxy, not an open bind, because the service has no TLS.
* wttr.in's own option surface drifts (`:help` is edited); the table is re-enumerated at
  implementation time and the drift is a doc-only change, since the parser is driven by our table.
* Scripts parsing `format=j1` field names break by design (shape change); mitigated by documenting it
  as a shape change in the option table and by shipping the step 23 JSON Schema.
* Port 8642 may be taken; mitigated by failing fast with `Error::Network` on bind and by documenting
  `--bind 127.0.0.1:0` (ephemeral, printed on startup) for tests and sandboxes.

## Progress log

- 2026-09-30 — step opened: surface, security model and non-port list fixed; hand-written HTTP layer
  chosen over `tiny_http` after reviewing its `ServerConfig` (no header limit, no read timeout, own
  internal task pool).
