<!--
SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Step 27 — QWeather JWT authentication

Status: ✅ done
Depends on: `02-config-and-state.md` (key store), `10-additional-providers.md` (`qweather.rs`), `15-alerts-and-severity.md` (`src/alerts/qweather.rs` adopts the same credential resolver)
Touches: `src/auth/{mod,jwt}.rs`, `src/config/keys.rs`, `src/provider/{mod,qweather}.rs`, `src/alerts/{mod,qweather}.rs`, `src/cli.rs`, `src/error.rs`, `src/status.rs`, `Cargo.toml`, `tests/qweather_jwt.rs`, `tests/{live,keys,alerts,multi_location}.rs`, `tests/fixtures/qweather/`, `REUSE.toml`, `docs/providers.md`, `README.md`, `docs/plans/README.md`, `CHANGELOG.md`

## Goal

QWeather's second authentication method becomes usable: instead of (or beside) an API key, the user
registers an Ed25519 public key in the QWeather console and configures the private key plus the three
identifiers, and every weather and alert request then carries a short-lived `Authorization: Bearer`
JWT minted in-process. The API-key path keeps working unchanged; with neither credential configured
the same exit-6 message names both `cirrocast key set qweather` forms.

Facts this step is built on, read from the vendor documentation on 2026-10-03
(<https://dev.qweather.com/en/docs/configuration/authentication/>):

* QWeather accepts **API KEY** (`X-QW-Api-Key`, what step 10 implements) and **JWT**
  (`Authorization: Bearer <token>`); the vendor recommends JWT.
* The signature is **Ed25519** (EdDSA): the user generates a PKCS#8 PEM private key locally
  (`openssl genpkey -algorithm ED25519`), uploads the public key to
  *Console → Project → Add Credential → JSON Web Token*, and receives a **Credential ID**.
* Header `{"alg":"EdDSA","kid":"<credential id>"}`; payload
  `{"iss":"<developer id, 10 chars starting with Q>","sub":"<project id>","iat":<unix>,
  "exp":<unix>}`; `exp − iat` must not exceed 24 h (86 400 s). `typ`, `aud` and `nbf` are reserved
  and must not be added (`typ`, if present, must be `JWT`).
* The three identifiers are not secrets; the private key is. The API host stays the per-account
  `[providers.qweather].host` step 10 already requires.

## Deliverables

- ✅ `src/config/keys.rs`: `keys.toml` gains a `[jwt.<provider>]` table beside `[keys]`, with
  `credential_id`, `developer_id`, `project_id` and the PEM in a TOML multi-line string
  (`private_key`); `pub enum Credential { ApiKey(String), QWeatherJwt(JwtCredential) }` and
  `KeyStore::credential(id) -> Result<Option<Credential>>`. Resolution order for `qweather`
  (first complete set wins): the JWT environment quartet →
  `keys.toml [jwt.qweather]` → `CIRROCAST_QWEATHER_KEY` → `keys.toml [keys].qweather` → none.
  A *partial* JWT set (some but not all of the four `CIRROCAST_QWEATHER_JWT_*` variables, or a
  `[jwt.qweather]` table missing a field) is `Error::Config` (exit 4) naming the missing items —
  never a silent fall-through to the API key.
- ✅ `src/cli.rs`: `key set qweather --jwt --key-file <PATH|-> --credential-id <ID>
  --developer-id <ID> --project-id <ID>` — the private key is read from a file or stdin, **never
  from argv**; the PEM is validated at set time (`ring` refuses a key that is not an Ed25519
  PKCS#8 document) and stored inline in `keys.toml`, which stays `0600` under the existing mode
  check. Without `--jwt` the command keeps its current meaning. `key rm qweather` removes both
  forms; `key list` prints `qweather  jwt (kid ABCDE12345, iss Q12345ABCD, sub ABC2345DEF)` plus
  `api key` when that form is also stored, and never the PEM.
- ✅ `src/auth/jwt.rs`: `pub fn qweather_token(cred: &JwtCredential, now: SystemTime) ->
  Result<String>` — base64url (unpadded) header and payload from `serde_json`, header
  `{"alg":"EdDSA","kid":…}` only, payload `{"iss","sub","iat","exp"}` with `iat = now − 30 s` and
  `exp = iat + 900 s`, signature over `header.payload` with `ring`'s `Ed25519KeyPair` (v2 through
  `from_pkcs8`, v1 through `from_pkcs8_maybe_unchecked`; see the design note). PEM decoding strips
  the armour and base64-decodes the DER before the key is parsed. Signing is deterministic, so a
  pinned key plus frozen `now` yields a byte-exact token in the test.
- ✅ `src/provider/qweather.rs`: the request builder takes the resolved `Credential` and sets either
  `X-QW-Api-Key` or `Authorization: Bearer`; the bearer value is registered with
  `HttpRequest::secret` so `normalized()`/`redacted_normalized()`, the cache envelope, `-v` output
  and every error message show a placeholder (the cache key must be identical for two mints of the
  same request at different instants — asserted). The token is minted per `fetch_report` call
  (the provider contract forbids interior mutability) and never written to the cache. A `401` keeps
  the step-10 exit code (6) through a two-mode variant, `Error::InvalidCredential`, whose message
  names both remedies: replace the key, or validate the token in *Console → JWT Validation* (the
  single-mode `Error::InvalidKey` text cannot say both; deviation recorded in the log).
- ✅ `src/alerts/qweather.rs` (landed by step 15) resolves its credential through the same store and
  header helper, so alert requests follow the configured auth mode with no second code path.
- ✅ `Cargo.toml`: `ring` and `base64` become direct dependencies. Both are *already* resolved in
  `Cargo.lock` (`rustls → ring 0.17.14`, `ureq → base64 0.23.1`), so no new crate enters the
  dependency graph or the release build; see the design note for the rejected alternatives.
- ✅ Tests (`tests/qweather_jwt.rs`, offline): the pinned token (fixed key, fixed `now`) matches the
  recorded string byte for byte; the header decodes to exactly `{"alg":"EdDSA","kid":…}` and the
  payload to `iss/sub/iat/exp` with `exp − iat == 900`; `StubTransport` sees `Authorization: Bearer
  …` and no `X-QW-Api-Key` in JWT mode and the reverse in key mode; a partial env quartet is exit 4
  naming the missing variables; a non-Ed25519 or truncated PEM is exit 4 at `key set`; a missing
  credential is exit 6 with both hints; two fetches at different instants produce the same cache key
  and two different bearer values; no test log, error string or fixture contains the PEM or a full
  token (the token fixture lives as a fixture *key* + expected string, and the expected token's
  `exp` is in the past — it is a test vector, not a live credential).
- ✅ `docs/providers.md`: the qweather section documents both auth modes, the console steps for the
  credential, the 24 h ceiling and the 15 min lifetime chosen here, and the note that the API host
  requirement is unchanged; `provider info qweather` prints `auth: API key or JWT (Ed25519)`.
  README's key section gains the JWT env quartet and the `key set qweather --jwt` example.
- ✅ `#[ignore]`d live smoke (enabled by `CIRROCAST_LIVE_TESTS=1` with a real credential) that mints
  a token, fetches `/weather/v1/current/<lat>/<lon>` and prints real data; the token is not echoed.

## Design notes

* **Why not `jsonwebtoken`.** `jsonwebtoken 11.1.0` (MIT, MSRV 1.88) is an algorithm framework whose
  two backends are `aws_lc_rs` (a C build) and `rust_crypto` (which pulls `ed25519-dalek` plus a
  second JWT layer); the token here is two base64url segments and one signature, ~40 lines with
  `ring`, and `ring` is already compiled into this binary by `rustls`. `ed25519-dalek 3.0.0`
  (BSD-3-Clause, MSRV 1.85, `pkcs8`/`pem` features) is rejected as a second Ed25519 implementation
  beside ring's. Measured 2026-10-03 on crates.io (through the local proxy) and read from
  `Cargo.lock`. **Corrected 2026-10-06 while implementing:** `ring 0.17.14`'s
  `Ed25519KeyPair::from_pkcs8` accepts only PKCS#8 **v2** (with the public key); `openssl genpkey
  -algorithm ED25519` writes **v1**, which only `from_pkcs8_maybe_unchecked` accepts — ring's own
  documentation says exactly that. The code therefore tries `from_pkcs8` first (so a v2 document
  still gets the public/private consistency check) and falls back to the unchecked parser for v1;
  `ed25519-dalek` was not needed, and the recorded fallback was not taken.
* **Storage: inline PEM, not a path.** `keys.toml` is already the single `0600` credential store and
  a second file would mean a second permission check and a silent breakage when the path moves. The
  PEM never leaves the file except into the signer.
* **Freshness over caching.** Ed25519 signing costs microseconds, so a token is minted per fetch with
  a 15-minute life (well inside the 24 h ceiling); nothing is persisted, and a long-running consumer
  gets a token no older than the request that used it. The `iat` is backdated 30 s to absorb small
  clock skew, and a clock far enough ahead to fail validation surfaces as the console-validator
  remedy in the 401 message.
* **Secrets discipline** (§10 of `AGENTS.md`): the private key is never an argv value (only a path or
  stdin), never logged, never in the cache; the token is marked `secret` on the request so even the
  redacted forms cannot leak it; `key list` prints identifiers only.

## Out of scope

Other providers' auth modes (API keys and step 15's optional MeteoAlarm token stay as they are),
keyring storage (excluded since step 10), JWT for QWeather's GeoAPI (not called), and any change to
the v1 API paths, units or limits step 10 pinned.

## Verification

```sh
cargo fmt --check && cargo clippy --workspace --all-targets --locked -- -D warnings && \
    cargo test --workspace --locked && reuse lint
cargo test qweather_jwt                 # pinned token, mode matrix, exit codes, masking
cargo run -q -- key set qweather --jwt --key-file /tmp/ed25519-private.pem \
    --credential-id ABCDE12345 --developer-id Q12345ABCD --project-id ABC2345DEF
cargo run -q -- key list                # qweather  jwt (kid ABCDE12345, iss Q12345ABCD, sub ABC2345DEF)
cargo run -q -- provider info qweather  # auth: API key or JWT (Ed25519) + store jwt:
cargo run -q -- -p qweather Beijing; echo $?   # exit 6 + both `key set` forms when unset
# `-v` on a configured run (offline, with a warm cache): the note names the mode, never the token
cargo run -q -- -p qweather --offline -v Beijing
#   qweather: host …; auth: jwt (kid ABCDE12345); v1 metric-only measures; …
# optional, with a real credential (the account host is this harness's own variable):
CIRROCAST_LIVE_TESTS=1 CIRROCAST_QWEATHER_HOST=https://<account>.re.qweatherapi.com \
    CIRROCAST_QWEATHER_JWT_{CREDENTIAL_ID,DEVELOPER_ID,PROJECT_ID,PRIVATE_KEY}=… \
    cargo test --test live -- --ignored --nocapture live_qweather_jwt
```

Observable result: a byte-exact token vector passes, a JWT-configured run needs no API key, the
API-key path still works, and no token or PEM byte appears in `-v` output, errors or cache files
(`grep -r "$(head -c 20 /tmp/ed25519-private.pem | tail -1)" "$XDG_CACHE_HOME/cirrocast"` finds
nothing). Two commands of the original list were corrected while implementing: `--lat/--lon` is a
provisional zone, so `qweather` refuses it (exit 2) before any credential is read — a name is
required; and the live smoke lives in `tests/live.rs` (the one file allowed to open sockets), where
its harness and the `CIRROCAST_LIVE_TESTS` gate already are.

## Exit criteria

- ✅ `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`, `reuse lint` clean.
- ✅ The pinned token test fails if the header field set, the base64url alphabet, the claim set or the
      signature input changes.
- ✅ Both auth modes are exercised through `StubTransport`; a partial or malformed credential never
      silently falls back to another mode.
- ✅ `provider info qweather` and `key list` name both modes; no test or fixture carries a live
      secret.
- ✅ Live smoke (opted in) fetches real data with a JWT credential and prints no token.

## Risks

* The console workflow (uploading the public key, copying the Credential ID) is easy to get wrong;
  the error messages name the exact console location, and `key set --jwt` validates the PEM before
  storing it.
* Clock skew beyond the 30 s backdating breaks tokens; the 401 message points at the console's JWT
  validator, and the token is minted per fetch so a wrong clock cannot be cached.
* The vendor could change the reserved-field set or key length limits; the header/payload shape is
  pinned by the byte-exact test, so a change is visible as one test failure.
* A user may configure both modes; precedence is documented and JWT wins, so an expired API plan does
  not shadow a working JWT.

## Progress log

- 2026-10-03 — step opened. Vendor documentation read through the local proxy; the EdDSA header and
  claim shape, the 86 400 s ceiling and the console flow are as recorded above. Dependency facts
  measured on crates.io and against `Cargo.lock` (`ring 0.17.14` and `base64 0.23.1` already in the
  graph). Raised as step 27 rather than an edit of step 10 because step 10 is shipped and its
  behaviour (API-key auth) stays valid; this step adds a mode beside it.
- 2026-10-04 — renumbered from 25 to 27 by the plan reorganization; dependencies (02, 10, 15) unchanged.
- 2026-10-06 — step implemented, all deliverables and exit criteria ticked. Deviations recorded:
  (a) `ring 0.17.14`'s `from_pkcs8` is v2-only, so OpenSSL's v1 key goes through
  `from_pkcs8_maybe_unchecked` (v2 first, so the consistency check still runs); `ed25519-dalek` was
  not needed — see the corrected design note. (b) `ProviderId::jwt_env()` (in `src/provider/mod.rs`)
  is the single source of truth for the quartet and for `[jwt.<provider>]` field names; the chain's
  missing-credential precheck had to switch from `KeyStore::get` to `KeyStore::credential`, or a
  JWT-configured run would have been rejected before the backend ran. (c) The 401 remedy needed a
  second exit-6 variant, `Error::InvalidCredential` (and `Error::MissingCredential` for the missing
  case): `Error::InvalidKey`'s fixed message cannot name two `key set` forms, and `error.rs` must not
  consult the provider registry. `src/status.rs`'s exhaustive `degradable` match lists both.
  (d) `key list` gained `KeyForm`/`KeySummary { forms }` so one row can carry both forms
  (`jwt (kid …, iss …, sub …)` first, `api key <masked>` beside it); `key rm` now reports which forms
  it removed, and the nothing-stored wording for `qweather` became `no qweather credentials stored`.
  (e) The live smoke lives in `tests/live.rs` as `live_qweather_jwt` (the one file allowed to open
  sockets; it reuses that harness and its `CIRROCAST_LIVE_TESTS` gate) rather than in
  `tests/qweather_jwt.rs`, and the verification commands for `--lat/--lon` and that filter were
  corrected accordingly. (f) The recorded test vector was computed independently with Python
  `cryptography` and cross-checked with `openssl pkeyutl -sign -rawin` before being pinned.
