<!--
SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Step 05 — http-cache-and-ip-location

Status: not-started
Depends on: `02-config-and-state.md` (`Config` network/cache tables, `atomic_write`, `Settings` timeout), `03-canonical-model-and-units.md` (`Location`, `LocationSource`), `04-geocoding-and-location-syntax.md` (`LocationSpec`, `Geocoder`, the `location search` CLI) — this step in turn satisfies the network half of `04-geocoding-and-location-syntax.md` (see that file's design notes)
Touches: `src/http.rs`, `src/cache.rs`, `src/geo/ip.rs`, `src/cli.rs`, `src/main.rs`, `tests/http_retry.rs`, `tests/cache.rs`, `tests/geo_ip.rs`, `tests/fixtures/http/`, `tests/fixtures/ip/`, `docs/plans/README.md`

## Goal

All outbound traffic goes through one testable HTTP client with retries, backoff, proxy support and a
stable error taxonomy, and every response the tool consumes passes through one on-disk cache with
per-namespace TTLs, atomic writes and a real `--offline` mode. On top of that, `--ip` resolves the
current location from the public address through ipwho.is with an ipapi.co fallback, cached for a day
and disclosed to the user, so "no location configured" produces weather instead of an error.

## Deliverables

- [ ] `src/http.rs`: `pub struct HttpRequest { method: Method, url: String, query: Vec<(String, String)>, headers: Vec<(String, String)>, timeout: Option<Duration> }` with `HttpRequest::get(url)`, `.query(k, v)`, `.header(k, v)`, `.timeout(d)`; `pub fn normalized(&self) -> String` = `"{METHOD} {url}?{query sorted by key, percent-encoded}\n{headers sorted, lower-cased names}"`, the input to cache-key hashing.
- [ ] `pub struct HttpResponse { status: u16, headers: Vec<(String, String)>, body: String, url: String }` with `header(&self, name) -> Option<&str>`, `pub fn json<T: DeserializeOwned>(&self) -> Result<T>` (a body that does not parse is `Error::Upstream` naming the request URL, never a panic) and `fn retry_after(&self) -> Option<Duration>` (delta-seconds or an HTTP-date, negative/past ⇒ `None`).
- [ ] `pub trait Transport: Send + Sync { fn execute(&self, req: &HttpRequest) -> Result<HttpResponse, TransportError>; }` with `pub enum TransportError { Timeout, Connect(String), Reset, Dns(String), Tls(String), Io(String) }` and `fn is_retryable(&self) -> bool` (`Timeout`/`Connect`/`Reset`/`Dns` retry, `Tls`/`Io` do not).
- [ ] `UreqTransport`: `UreqTransport::new(cfg: &Network, timeout: Duration) -> Result<Self>` building the agent once with `ureq::Agent::config_builder().http_status_as_error(false).user_agent(UA).timeout_connect(Some(t)).timeout_recv_response(Some(t)).timeout_recv_body(Some(t)).proxy(proxy).build().new_agent()`. `UA` is `concat!("cirrocast/", env!("CARGO_PKG_VERSION"), " (+https://github.com/YangtseSu/cirrocast)")`; proxy resolution is config `[network].proxy` (via `ureq::Proxy::new`, a parse failure is `Error::Config`) else `ureq::Proxy::try_from_env()`, which covers `HTTPS_PROXY`/`ALL_PROXY`/`NO_PROXY`; `http_status_as_error(false)` is what lets the retry policy read 4xx/5xx bodies.
- [ ] `StubTransport`: `replies: Mutex<VecDeque<StubReply>>` + `calls: Mutex<Vec<HttpRequest>>`, with `StubReply::ok(status, body)`, `::status(status, headers, body)`, `::err(TransportError)`, `::json_file(path)` (reads a `tests/fixtures/**` file) and `calls()` for assertions — the only transport the test suite instantiates.
- [ ] `pub struct HttpClient { transport: Box<dyn Transport>, attempts: u32, timeout: Duration, clock: Arc<dyn Clock>, verbose: u8 }` with `pub fn send(&self, req: &HttpRequest) -> Result<HttpResponse>`: at most `attempts = min(1 + network.retries, 3)` tries (the config default `retries = 3` therefore means the contract's **max 3 attempts**; `--timeout` overrides only the timeout), waiting `0.5 s · 2ⁿ` between them through the injected `Clock` — `0.5 s` then `1 s` in v1, with the schedule's `2 s` step reachable only if the attempt cap is ever raised; retried are transport timeouts, connection resets, DNS failures, `408`, `429` and `500..=599`; everything else is returned immediately.
- [ ] `Retry-After` is honoured for `429`/`503` when present and **replaces** the exponential delay, clamped to `0..=60 s`; sleep durations are requested from `Clock`, so tests assert them without waiting.
- [ ] Error mapping: retryable failures exhausted ⇒ `Error::Network("GET https://api.open-meteo.com/v1/forecast failed after 3 attempts: timeout while connecting")` (exit 3); non-retryable non-2xx ⇒ `Error::Upstream`, appending `reason` when the body is the `{ "error": true, "reason": "…" }` envelope used by Open-Meteo, its geocoding endpoint and ipapi.co, otherwise the first 200 characters of the body; `-v` logs one line per attempt to stderr (`http: GET … attempt 2/3 after timeout; sleeping 1.0 s`).
- [ ] `src/cache.rs`: `pub trait Clock: Send + Sync { fn now(&self) -> SystemTime; fn sleep(&self, d: Duration); }` with `SystemClock` and a test-only `FakeClock` (`advance(d)`, `sleeps() -> Vec<Duration>`); step 04's Nominatim throttle reuses `Cache::clock()` so both layers share one injected clock.
- [ ] `pub struct Cache { root: PathBuf, schema_version: u32, clock: Arc<dyn Clock>, mode: CacheMode }` with `Cache::open(paths: &Paths, mode: CacheMode, clock: Arc<dyn Clock>) -> Result<Self>` (root `$XDG_CACHE_HOME/cirrocast`) and `Cache::with_root(root, …)` for tests; `pub enum CacheMode { Normal, NoCache, Refresh, Offline }` parsed from the mutually exclusive `--no-cache`/`--refresh`/`--offline` clap group (`--no-cache --offline` ⇒ clap usage error, not a silent precedence rule).
- [ ] `pub struct CacheKey(String)` with `CacheKey::hash(namespace: &str, normalised_request: &str)` (sha256 hex, path `geocode/<sha256>.json`), `CacheKey::ip(service)` (`ip/<service>.json`), `CacheKey::weather(provider, lat, lon, days, date)` (`weather/<provider>-<lat.2dp>-<lon.2dp>-<days>-<local-date>.json`), and `fn path(&self) -> PathBuf`; `-v` prints the key path on every lookup.
- [ ] Entry envelope, one JSON object per file: `{ "cache_schema_version": 1, "key": "…", "fetched_at": "2026-09-30T12:00:00Z", "ttl_secs": 600, "status": 200, "body": "…" }` — the response body is stored as **raw text**, so the cache never re-serialises provider payloads and stays readable/debuggable; `cache_schema_version` is independent of `Config::schema_version` and a mismatch is a miss, not an error.
- [ ] `pub fn read(&self, key: &CacheKey) -> Result<Option<CacheEntry>>`: `NoCache`/`Refresh` never read, `Offline` reads but never writes, an entry older than its TTL is a miss (file left for `cache clean`), unparseable JSON or an unknown `cache_schema_version` is a miss plus a `-v` warning.
- [ ] `pub fn write(&self, key: &CacheKey, status: u16, body: &str, ttl: Duration) -> Result<()>`: `atomic_write(path, bytes, 0o644)` from step 02 (temp file + `rename` in the same directory, parent directories created with `0700`), no-op in `NoCache`/`Offline`.
- [ ] `pub fn read_or_fetch_json<T: DeserializeOwned>(&self, key: &CacheKey, ttl: Duration, fetch: impl FnOnce() -> Result<(u16, String)>) -> Result<T>`: hit ⇒ deserialize; a deserialization failure counts as a miss and refetches once (so a provider schema change self-heals instead of failing forever); miss while `Offline` ⇒ `Error::Network("offline mode: no cached entry for weather/open-meteo-39.90-116.40-3-2026-09-30.json")` naming the exact key path; otherwise `fetch()`, write, deserialize.
- [ ] `pub fn stat(&self) -> Result<CacheStat>` (per namespace: entry count, bytes, oldest and newest `fetched_at`) and `pub fn clean(&self, all: bool) -> Result<CleanReport>` (expired entries only, or the whole tree), both reporting a count that the CLI prints.
- [ ] `src/geo/ip.rs`: `pub trait IpLocator { fn locate(&self) -> Result<Location>; }`, `pub enum IpService { IpWhoIs, IpApiCo }`, `pub struct IpLocatorChain<'a> { http: &'a HttpClient, cache: &'a Cache, services: Vec<IpService> }`; order from `CIRROCAST_IP_SERVICE` (`ipwhois` | `ipapi` | `auto`, default `auto`), falling through to the next service only on `Error::Network|Error::Upstream` — the same rule the provider chain uses in step 06.
- [ ] ipwho.is mapping (`https://ipwho.is/`): `{ ip, success, message, city, region, country, country_code, latitude, longitude, timezone: { id, offset, utc } }`; `success: false` ⇒ `Error::Upstream(message)` so a reserved or blocked address falls through; its documented `1,000 requests/day` free limit and `429` + `Retry-After` response are handled by the shared retry policy.
- [ ] ipapi.co mapping (`https://ipapi.co/json/`): `{ ip, city, region, region_code, country, country_code, country_name, latitude, longitude, timezone, utc_offset, error, reason, message }`; `country` there is the 2-letter code while `country_name` is the display name, so `Location.country = country_name` and `country_code = country_code`; `error: true` ⇒ `Error::Upstream(reason)`.
- [ ] Both services return an IANA zone name; `Tz::from_str` failure is `Error::Upstream` (no silent UTC fallback here, unlike Nominatim — a wrong zone would shift every day part). The result is `Location { name: city, admin1: region, country, country_code, lat, lon, tz, population: None, source: LocationSource::Ip }`.
- [ ] Caching: `ip/<service>.json` with `cache.ip_ttl_secs` (24 h default), keyed per service so a fallback result never masquerades as the primary's; the resolved city is echoed by step 06 just like a geocoded one.
- [ ] Privacy and disclosure: the IP lookup runs **only** when `--ip` is given or when no location is configured at all (`location.default` empty and no `[LOCATION]` argument); it never happens as a silent precondition of a name query, no other request carries the address, and a `-v` line names the service used (`ip: located from the public IP via ipwho.is`). Root `README.md` and `cirrocast --help` must state that this sends the public IP to a third-party service.
- [ ] CLI: `--timeout <SECS>` (overrides `network.timeout_secs`), `--no-cache`/`--refresh`/`--offline`, `cache stat`, `cache clean [--all]`, and `location search --ip` (the observable surface for this step until step 06 hands the location to a provider).
- [ ] Tests: `tests/http_retry.rs` (scripted `StubTransport`: timeout,timeout,ok ⇒ 3 calls and sleeps `[500 ms, 1000 ms]`; timeout×3 ⇒ `Error::Network` mentioning 3 attempts; `500` then `200` ⇒ retry; `400` ⇒ no retry + `reason` from the body; `429` + `Retry-After: 7` ⇒ one 7 s sleep; `Retry-After: 3600` clamped to 60 s; config proxy beats `HTTPS_PROXY`; the exact `User-Agent` string), `tests/cache.rs` (TTL boundary at 599/600 s with `FakeClock`, round-trip, version mismatch ⇒ miss, corrupt entry ⇒ miss + refetch, offline hit, offline miss message naming the key path, `--no-cache` writing nothing, `--refresh` refetching, 100-iteration concurrent writer/reader loop never observing a partial entry and leaving no `*.tmp.*` file, `stat`/`clean` counters), `tests/geo_ip.rs` (ipwho.is fixture, ipapi.co fixture, ipwho.is `success:false` falling through to ipapi.co, both failing ⇒ `Error::Upstream`, second call served from cache, entry expired after 24 h ⇒ refetch).

## Design notes

* **`Transport` instead of `httpmock`.** `httpmock` starts a real HTTP server per test: ports, threads, a
  listener and timing-dependent assertion of a *behaviour* we care about precisely — how many attempts a
  retryable failure causes and how long the client waits between them. `StubTransport` answers in-process,
  cannot flake on port reuse or a slow CI loopback, records the exact request list (method, query pairs,
  headers) for assertion, and reproduces what a test server cannot: connect timeouts, resets and DNS
  failures. The one thing it does not cover — real HTTP framing over TLS — is covered by a single
  `#[ignore]`d live test run manually. A local server would also make every cache/offline test depend on
  network syscalls, which the contract forbids.
* `http_status_as_error(false)` is deliberate: the retry policy needs the status *and* the body (the
  `reason` envelope) of a 4xx/5xx response, which ureq's default error mode turns into an opaque
  `Error::StatusCode`.
* **Raw bodies in the cache.** Storing provider payloads as text keeps the cache transparent (`cat` on an
  entry shows what the API said), makes `--offline` able to serve entries written by a different build,
  and removes a whole class of "we re-serialised and lost a field" bugs. The cost is one JSON envelope
  layer, and the schema version on it is what makes a future envelope change safe.
* `cache_schema_version` is separate from `Config::schema_version`: a new config field must not
  invalidate the cache, and an envelope change must not force a config migration.
* Cache keys are hashed (sha256) rather than readable wherever the request can contain a credential:
  geocoding/geocode keys use `CacheKey::hash`, and step 10's BYOK providers **must** use it too, since a
  readable filename would leak the API key into `$XDG_CACHE_HOME`. The weather key stays readable
  because it contains only provider, coordinates, day count and date — values a user expects to see.
* An offline miss is `Error::Network`, not a new variant: exit code 3 already means "could not obtain a
  network resource", and the message names the exact key path so the user can delete it or drop
  `--offline` — a distinct exit code would break the contract's stable code table for no gain.
* New dependencies, each with its reason: `ureq = { version = "3", features = ["rustls"] }` (blocking,
  rustls-only, no async runtime — the contract's HTTP stack; default features already include gzip),
  `serde_json = "1"` (envelope + provider payloads; it lands here, so step 08's JSON renderer and step 06's
  response parsing reuse it rather than adding it again), `sha2 = "0.11"` (cache keys),
  `percent-encoding = "2"` (query encoding for arbitrary user input). No test-server or mock-HTTP crate.

## Out of scope

- Provider selection, `FetchRequest`/`Env`, and everything that turns a `Location` into a `Report`:
  `06-open-meteo-provider.md`, which also consumes `--ip` end to end.
- The Nominatim throttle's caller side and `~` results (`04-geocoding-and-location-syntax.md`); this step
  supplies the `Cache`, the `Clock` and the shared client it needs.
- `cache verify`/`cache prune --older-than` and cache size limits: no v1 requirement; `clean --all` is the
  escape hatch, and a size cap is a `12-quality-hardening.md` candidate.
- HTTP/2, cookie jars, and per-host rate limiting beyond the Nominatim throttle: unnecessary for the
  handful of endpoints in this project.
- Any retry of *upstream* errors from the chain's perspective: the fallback rule (`Error::Network|Upstream`
  ⇒ next provider) is implemented in `06-open-meteo-provider.md`.

## Verification

Fixtures: `tests/fixtures/http/open_meteo_error_invalid_param.json` (the raw 400 `{error, reason}`
envelope), `tests/fixtures/ip/ipwho_is_beijing.json`, `tests/fixtures/ip/ipwho_is_failure.json`,
`tests/fixtures/ip/ipapi_co_beijing.json`, `tests/fixtures/ip/ipapi_co_error.json` — recorded once and
then minimised: the queried address is replaced by the RFC 5737 documentation address `203.0.113.7` and
the `connection`/`asn`/`org` blocks are dropped, so the fixtures contain no third-party creative content.
Manual smoke run (network on the first command only):

```sh
tmp=$(mktemp -d); export XDG_CACHE_HOME=$tmp
cargo run -- location search --ip
# stdout: Beijing, Beijing, China (39.90, 116.40) Asia/Shanghai
# stderr: ip: located from the public IP via ipwho.is
cargo run -- location search --ip --offline      # identical stdout, zero network calls
cargo run -- location search Beijing --timeout 2 && cargo run -- cache stat
# weather      0 entries       0 B
# geocode      1 entry       1.6 kB   oldest 2026-09-30T…Z   newest 2026-09-30T…Z
# ip           1 entry       1.4 kB   oldest 2026-09-30T…Z   newest 2026-09-30T…Z
cargo run -- cache clean                          # removed 0 expired entries
cargo run -- cache clean --all                    # removed 2 entries
cargo run -- location search --ip --offline; echo $?   # error: offline mode: no cached entry for ip/ipwho-is.json   (exit 3)
cargo run -- cache clean --all --offline; echo $?      # error: offline mode: cache writes are disabled   (exit 2)
```

## Exit criteria

- [ ] No test constructs a `UreqTransport`; `cargo test` passes with the network unplugged.
- [ ] Retry counts and requested sleep durations are asserted exactly from `StubTransport` + `FakeClock`;
      no test sleeps for real (total suite wall time unchanged by the retry tests).
- [ ] `cargo fmt --check` clean.
- [ ] `cargo clippy --all-targets -- -D warnings` clean.
- [ ] `cargo test` clean (`tests/http_retry.rs`, `tests/cache.rs`, `tests/geo_ip.rs`).
- [ ] `reuse lint` clean (IP fixtures stay under the project licence; no new `REUSE.toml` entry needed
      because they are minimised first-party files, and the `tests/fixtures/**` rule already matches).
- [ ] Smoke run above reproduces the shown output shapes, including the `--offline` cache hit, the
      `cache stat` layout, and exit code 3 for the offline miss.

## Risks

- ipwho.is is free but quota-limited (1,000/day per address) and ipapi.co's free tier is rate limited;
  the 24 h cache, the fallback order and `CIRROCAST_IP_SERVICE` are the mitigation, and `--ip` is never
  automatic for name queries.
- A cached IP location can be hours stale after travel; 24 h is a deliberate compromise, and
  `--refresh` forces a fresh lookup.
- `Retry-After` in HTTP-date form depends on clock skew; an unparsable or past value falls back to the
  exponential backoff, and the value is clamped so a hostile `Retry-After: 86400` cannot hang the CLI.
- Hashing geocode keys hides which query an entry belongs to; `-v` prints the key path and the cache
  envelope stores the normalised request string in its `key` field, which restores debuggability without
  putting a credential in a filename.
- A partially written entry can only be observed if a reader ignores the temp-file naming scheme; the
  concurrent reader/writer loop test is what keeps `atomic_write` honest.

## Progress log

- 2026-09-30 — plan written.
