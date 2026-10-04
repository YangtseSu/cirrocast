<!--
SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Step 12 — Quality hardening

Status: ✅ done
Depends on: 08 (cli-surface-and-formats), 09 (localization), 10 (additional-providers)
Touches: `src/**/*.rs` (error text and logging only), `src/http.rs` (network guard), `src/config/mod.rs`
(validation), `src/cli.rs` (help text), `src/model/mod.rs` + `src/provider/mod.rs` (the attribution mirror the
render-path gate requires), `deny.toml` (new), `.github/workflows/ci.yml` (new), `LICENSES/GPL-3.0-or-later.txt`,
`REUSE.toml`, `tests/{exit_codes,no_network,decoder_robustness,xdg}.rs` (new), `tests/fixtures/malformed/**` (new),
`README.md`, `AGENTS.md` (lint trap only)

## Goal

No new features: make everything already shipped trustworthy and continuously verifiable. Every user-facing
error names the offending value and the next command; `--verbose` explains a failure from cause to symptom
without leaking key material; `--offline` provably never touches the network; contradictory config is rejected
with the key path; `--help` documents precedence and exit codes; dependency licences are checked for
GPL-3.0-or-later compatibility and advisories are tracked; the crate builds on its declared 1.98 MSRV; every
upstream JSON decoder survives truncated and hostile input with `Error::Upstream`, not a panic; all gated by CI.

## Deliverables

- ✅ Error-message audit across every module (`error.rs`, `cli.rs`, `config/`, `geo/`, `http.rs`, `cache.rs`,
  `provider/*`): every message names the offending value plus the accepted forms or the next command — missing
  key → `cirrocast key set openweathermap` and `CIRROCAST_OPENWEATHERMAP_KEY`; unknown provider/format/units/
  language → the accepted values; bad `--days` → the `0..=14` range and the provider's `max_days`; `keys.toml`
  wrong mode → `chmod 600 <path>`; config error → the key path; unknown location → `cirrocast location search
  <q>`; offline miss → provider, location, "rerun without `--offline`".
- ✅ `tests/exit_codes.rs`: one deterministic scenario per binding exit code — 0 (`--version`), 2 (`--days 99`,
  `--format yaml`, `--station 12`), 4 (unreadable config, `keys.toml` at 0644), 6 (`--provider qweather`, no key,
  scratch config dir), 3 (`--refresh` with `CIRROCAST_FORBID_NETWORK=1`), 5 (unknown ICAO, stub stationinfo
  response). Asserts the code and that stderr names the offending value, never a full sentence.
- ✅ `--verbose` review: `-v` prints the settings with their sources, the resolution notes, the upstream
  request behind the answer (secrets redacted) and one indented `caused by:` line per error source; `-vv` adds
  every HTTP attempt (status and backoff delay) and the cache decisions; `--quiet` mutes warnings, never
  errors; no log line ever contains a key or a `keys.toml` body.
- ✅ `--offline` correctness: with the guard on, `--offline` constructs no request at all; a warm cache renders
  identically to a run without the flag; a cold cache fails with exit 3 plus the rerun hint; `--no-cache
  --offline` and `--refresh --offline` are usage errors naming both flags; `cache stat` separates
  expired-but-present from valid entries.
- ✅ `cirrocast config validate` returns 0 on the shipped template and 4 with the key path for: unknown keys,
  unknown enum values, a non-ICAO `[providers.metar] station`, `render.width` below the minimum,
  zero/negative timeouts and TTLs, a `location.default` that fails the location grammar, and
  `cache.enabled = false` combined with `--offline`; the precedence rule `[units] <key> > defaults.units` is
  stated in the message when an override conflicts.
- ✅ Help-text review: `--help`/`--version` read no config, XDG or cache state; a `CONFIG PRECEDENCE` block
  states CLI flag > `CIRROCAST_*` env var > `config.toml` > built-in default; an `EXIT CODES` block lists all
  seven codes with triggers; the man page renders from the same `cli.rs` text.
- ✅ Startup gate recheck: `hyperfine --warmup 10 --runs 50 'target/release/cirrocast --version'` under 50 ms,
  and `strace -f -e trace=network target/release/cirrocast --version` with no `socket(` (enforced budgets with
  CI thresholds are step 21). Measured locally: 1.4 ms ± 0.2 ms mean, `socket(` count 0 for `--version`; the
  guarded fetch path opens no socket either (`CIRROCAST_FORBID_NETWORK=1 … Beijing` also counts 0, which is the
  guard's "before DNS" promise seen from outside).
- ✅ `deny.toml` + `cargo deny check` green: `[licenses]` allow-list (MIT, Apache-2.0, ISC, BSD-2-Clause,
  BSD-3-Clause, Zlib, Unicode-3.0, CDLA-Permissive-2.0, CC0-1.0, MPL-2.0, each with a one-line justification),
  deny list for licences that cannot combine with GPL-3.0-or-later (`GPL-2.0-only`, pre-3.0 `OpenSSL`,
  `SSPL-1.0`, `BUSL-1.1`, `Elastic-2.0`, `Commons-Clause`, `JSON`, `BSD-4-Clause`), `[bans]` denying duplicate
  versions and wildcard dependencies, `[sources]` restricted to crates.io, and `[advisories]` ignore entries
  each carrying a reason and a review date.
- ✅ Dependency licence audit: enumerate every crate with `cargo metadata --all-features` and commit the
  name/version/licence/GPL-3.0-or-later verdict table into this file's Design notes in the same commit as
  `deny.toml`; expected exceptions needing an allow-list entry are `Unicode-3.0` (through `idna`/`url`) and
  `CDLA-Permissive-2.0` (through `webpki-roots`); an incompatible dependency is a blocker with a chosen
  replacement, never a silent allow-list. `cargo audit` runs beside `cargo deny` since the sources disagree.
- ✅ MSRV at 1.98: `cargo +1.98.0 build --all-targets --locked` and `cargo +1.98.0 test --locked` pass locally
  and in a dedicated CI job; `cargo msrv verify` (cargo-msrv, optional) confirms the declared `rust-version`;
  `rust-toolchain.toml` keeps pinning `stable` for development, the `+1.98.0` override wins in the job.
  (The floor tracks the latest stable release rather than lagging behind it: step 11 raised 1.85 → 1.98 in
  one move, which is also what `tzf-rs` 2.x and the crate's let-chains require.) This machine has a single
  system toolchain (1.98.1) and no rustup, so the local verification is `cargo build --all-targets --locked` +
  `cargo test --locked` on 1.98.1 — same minor as the floor, patch above it — while the `1.98.0` matrix leg in
  CI is the exact-pin check; the earlier `1.85.0` placeholders in this file's Verification/Exit-criteria blocks
  were stale and are corrected to `1.98.0`.
- ✅ `reuse lint` reaches 0 problems and stays there. The tree currently reports 1 invalid SPDX expression and
  1 file with no licensing information: `AGENTS.md` demonstrates a header with prose on the same line, so REUSE
  parses the prose as part of the licence expression (the value must sit alone on its line, or the example must
  be wrapped in `REUSE-IgnoreStart`/`REUSE-IgnoreEnd`), and `src/main.rs` needs its header. Fixture licensing
  follows the upstream rule — exact-path annotations with the upstream licence (CC-BY-4.0, ODbL-1.0, a
  public-domain `LicenseRef` for NOAA data) — so no blanket `tests/fixtures/**` override may remain. Windows
  checkout decision (Design notes): replace the `LICENSES/GPL-3.0-or-later.txt` symlink with a real copy of
  `LICENSE` and gate the pair with `cmp -s LICENSE LICENSES/GPL-3.0-or-later.txt` in CI. (Both earlier problems
  were fixed in steps 09–11: `reuse lint` reports 0 problems for 260 files, the symlink is replaced by the copy,
  the `cmp` gate is in the `gates` CI job, and `tests/fixtures/malformed/**` is annotated as first-party.)
- ✅ `tests/no_network.rs`: the `CIRROCAST_FORBID_NETWORK=1` guard is enforced in `src/http.rs` before DNS and
  connect; the test proves the guard intercepts (a cold-cache request to a real upstream exits 3 with the guard
  message) and, on Linux, reruns the CLI under `unshare -rn` to prove no external DNS is required.
- ✅ Decoder robustness: new malformed fixtures plus a sweep over every upstream JSON decoder (open-meteo, the
  six key-requiring providers, metar, geocoding, IP location, config) feeding a truncation sweep at every byte
  offset, empty/`{}`/`[]`/`null` bodies, wrong types (`"temp": "abc"`) and single-byte mutations — each input
  yields `Error::Upstream`/`Error::Config` with a cause chain and never panics, hangs or allocates unboundedly
  (payload cap enforced in `http.rs`). Implemented in `tests/decoder_robustness.rs` with
  `tests/fixtures/malformed/**`; the sweep is exhaustive (every byte offset) for payloads up to 8 KiB and uses
  a bounded stride (at most 2048 offsets per fixture) for the larger recordings, because a debug-build decode
  costs about a millisecond and the exhaustive form of an 84 KiB payload alone took minutes — the deviation, and
  its reason, are recorded in the progress log. `Error` variants carry their full text inline (no `source()`), so "cause chain" means the
  message names the provider and the defect rather than an empty `caused by:` chain; the taxonomy assertions
  check the variant class and the provider name.
- ✅ Render-path audit: `src/render/**` and `src/model/**` import nothing from `http`, `provider` or `cache`,
  enforced by a CI grep gate over `crate::(http|provider|cache)`, grouped `crate::{…}` imports and
  `super::(http|provider|cache)` forms in `src/render`/`src/model` (comment lines excluded, so a rustdoc link
  is not a code edge). Beyond
  `std` and crate-internal modules, the two trees use only `serde`, `serde_json`, `chrono`, `chrono-tz`,
  `clap` (the `ValueEnum` derives on the format/colour enums), `unicode-width` and `fluent-bundle` — no
  transport, no cache, no registry. (The three registry lookups the renderers used — capabilities, display
  name, licence credit — moved into `model::Attribution`: every provider fills them through
  `provider::attribution(...)`, and `the_report_mirror_carries_every_registry_field` proves the copy is
  complete. The hand-written report fixtures carry the same fields, so every renderer snapshot is
  byte-identical.)
- ✅ XDG audit: no write outside `$XDG_{CONFIG,CACHE,DATA}_HOME/cirrocast` (verified by running with all three
  pointed at a temporary tree and diffing it), `XDG_CONFIG_DIRS` honoured for reads, and `--offline`/`--no-cache`
  creating no cache directory.
- ✅ Secret-handling audit: keys never land in `config.toml`, `key list` masks values, `keys.toml` is written
  0600 and refused when wider, and a test greps captured `-vv` stderr for the fake key it exported and finds
  nothing.
- ✅ `.github/workflows/ci.yml`: jobs `fmt`, `clippy --all-targets -- -D warnings`, `test` (matrix
  ubuntu-26.04 + macos-26 × stable + 1.98.0, `CIRROCAST_FORBID_NETWORK=1`, `--locked`), `reuse`
  (`fsfe/reuse-action`), `deny` (`EmbarkStudios/cargo-deny-action`), `audit` (`rustsec/audit-check`), plus the
  layer and `cmp` gates; every third-party action pinned to a commit SHA, `concurrency` cancelling superseded
  runs, no Windows job. README/AGENTS document the matrix, MSRV, deny policy, the no-network rule and the
  resulting CI summary. (Runner labels are explicit — `ubuntu-26.04` and `macos-26`, never `<os>-latest`: the
  `ubuntu-latest` alias still resolves to 24.04 while 26.04 is the current GA image, so the alias would silently
  pin an older distribution than the project targets.)

## Design notes

* `LICENSES/GPL-3.0-or-later.txt` is currently a symlink to `../LICENSE`; a Windows checkout with
  `core.symlinks=false` (default outside Developer Mode) materialises it as a text file containing `../LICENSE`,
  so the lint verdict depends on the platform. **Decision: ship a real copy** beside the root `LICENSE` that
  GitHub detects, with `cmp -s` in CI against drift. Verified while writing this plan: replacing the symlink with
  a copy in a scratch checkout leaves `reuse lint` output identical, so it costs one duplicated 35 KiB file.
  Rejected: lint on Linux/macOS only (Windows contributors keep a red local lint, no documented fix); requiring
  `core.symlinks=true` (an undocumented per-developer setup step); making the root `LICENSE` the pointer (GitHub
  licence detection needs a real root file).
* Guard design: `CIRROCAST_FORBID_NETWORK=1` is read once by `src/http.rs` and blocks every non-loopback
  connection before DNS, so a blocked run cannot even resolve a name. Loopback stays reachable so tests can aim a
  provider's base URL at an in-process stub; the guard's own test asserts blocking against a real upstream,
  because CI has network and a silently broken guard would otherwise pass. The whole `test` job exports the
  variable, so an accidentally network-dependent test fails loudly.
* Robustness approach: deterministic truncation and mutation sweeps instead of a fuzzing dependency —
  `cargo-fuzz`/`honggfuzz` need nightly plus corpus infrastructure that a CLI decoder does not justify, while a
  byte-offset sweep covers exactly the realistic failure class (unchecked slicing, `serde` type assumptions).
  The sweep is driven by fixtures already committed in steps 06/10/11, so it stays reproducible and offline.
* Licence policy: GPL-3.0-or-later admits permissive licences (MIT/Apache-2.0/ISC/BSD/Zlib/CC0), MPL-2.0
  (compatible, with a source-availability duty for modified MPL files) and the permissive data licences
  `Unicode-3.0` and `CDLA-Permissive-2.0`; the denied classes above are GPL-incompatible. The verdict table for
  the full dependency set is committed here together with `deny.toml`.
* `cargo deny` gates licences, advisories, bans and sources; `cargo audit` stays as an independent advisory check
  because mirrored databases can lag behind live RUSTSEC data.
* Dependency licence verdicts, generated 2026-10-01 from `cargo metadata --all-features` (174 crates in the
  resolved graph, dev-dependencies included; `cargo deny check` is the enforced form of this table):

  | expression | crates | verdict |
  |---|---|---|
  | `MIT OR Apache-2.0` (also spelled `Apache-2.0 OR MIT`, `MIT/Apache-2.0`, `Apache-2.0/MIT`) | 147 | compatible — the MIT branch is GPL-compatible |
  | `MIT` | 16 | compatible |
  | `Apache-2.0` | 5 | compatible |
  | `Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT` (`rustix`, `linux-raw-sys`, `wasi`) | 3 | compatible — Apache-2.0 branch |
  | `Unicode-3.0` (`tinystr`, `zerofrom`, `zerovec`) | 3 | compatible — permissive data licence, attribution only |
  | `Unlicense OR MIT` (`aho-corasick`, `memchr`), `0BSD OR MIT OR Apache-2.0` (`adler2`), `MIT OR Zlib OR Apache-2.0` (`miniz_oxide`) | 4 | compatible — MIT branch |
  | `MIT OR Apache-2.0 OR LGPL-2.1-or-later` (`r-efi`, Windows-only) | 1 | compatible — MIT branch |
  | `Apache-2.0 AND ISC` (`ring`) | 1 | compatible — both allowed |
  | `Apache-2.0 OR ISC OR MIT` (`rustls`) | 1 | compatible |
  | `Apache-2.0 OR GPL-2.0-only` (`self_cell`) | 1 | compatible — Apache-2.0 branch, `GPL-2.0-only` stays denied |
  | `BSD-3-Clause` (`subtle`), `ISC` (`rustls-webpki`, `untrusted`), `Zlib` (`zlib-rs`) | 4 | compatible |
  | `(MIT OR Apache-2.0) AND Unicode-3.0` (`unicode-ident`) | 1 | compatible — both terms permissive |
  | `CDLA-Permissive-2.0` (`webpki-roots`) | 1 | compatible — permissive data licence, attribution only |
  | `MIT AND ODbL-1.0` (`tzf-dist`) | 1 | compatible — the code is MIT; the timezone polygons are ODbL, used unmodified with the crate's own notice |

  No dependency is GPL-3.0-or-later-incompatible, so no replacement was needed. The full per-crate list:

  ```
adler2 2.0.1 | 0BSD OR MIT OR Apache-2.0
aho-corasick 1.1.5 | Unlicense OR MIT
android_system_properties 0.1.6 | MIT OR Apache-2.0
anstream 1.0.0 | MIT OR Apache-2.0
anstyle 1.0.14 | MIT OR Apache-2.0
anstyle-parse 1.0.0 | MIT OR Apache-2.0
anstyle-query 1.1.5 | MIT OR Apache-2.0
anstyle-wincon 3.0.11 | MIT OR Apache-2.0
assert_cmd 2.2.2 | MIT OR Apache-2.0
autocfg 1.5.1 | Apache-2.0 OR MIT
base64 0.23.1 | MIT OR Apache-2.0
bitflags 2.13.2 | MIT OR Apache-2.0
block-buffer 0.12.1 | MIT OR Apache-2.0
bstr 1.13.1 | MIT OR Apache-2.0
bumpalo 3.20.3 | MIT OR Apache-2.0
bytes 1.12.1 | MIT
cc 1.5.1 | MIT OR Apache-2.0
cfg-if 1.0.5 | MIT OR Apache-2.0
chrono 0.4.45 | MIT OR Apache-2.0
chrono-tz 0.10.4 | MIT OR Apache-2.0
clap 4.6.7 | MIT OR Apache-2.0
clap_builder 4.6.7 | MIT OR Apache-2.0
clap_complete 4.6.11 | MIT OR Apache-2.0
clap_derive 4.6.7 | MIT OR Apache-2.0
clap_lex 1.1.1 | MIT OR Apache-2.0
clap_mangen 0.3.3 | MIT OR Apache-2.0
colorchoice 1.0.5 | MIT OR Apache-2.0
console 0.16.6 | MIT
const-oid 0.10.2 | Apache-2.0 OR MIT
core-foundation-sys 0.8.7 | MIT OR Apache-2.0
cpufeatures 0.3.1 | MIT OR Apache-2.0
crc32fast 1.5.2 | MIT OR Apache-2.0
crypto-common 0.2.2 | MIT OR Apache-2.0
difflib 0.4.0 | MIT
digest 0.11.3 | MIT OR Apache-2.0
displaydoc 0.2.7 | MIT OR Apache-2.0
encode_unicode 1.0.0 | Apache-2.0 OR MIT
equivalent 1.0.2 | Apache-2.0 OR MIT
errno 0.3.14 | MIT OR Apache-2.0
etcetera 0.11.0 | MIT OR Apache-2.0
fastrand 2.5.0 | Apache-2.0 OR MIT
find-msvc-tools 0.1.14 | MIT OR Apache-2.0
flate2 1.1.10 | MIT OR Apache-2.0
float-cmp 0.10.0 | MIT
fluent-bundle 0.16.0 | Apache-2.0 OR MIT
fluent-langneg 0.13.1 | Apache-2.0 OR MIT
fluent-syntax 0.12.0 | Apache-2.0 OR MIT
futures-core 0.3.34 | MIT OR Apache-2.0
futures-task 0.3.34 | MIT OR Apache-2.0
futures-util 0.3.34 | MIT OR Apache-2.0
geometry-rs 0.5.1 | MIT
getrandom 0.2.17 | MIT OR Apache-2.0
getrandom 0.4.3 | MIT OR Apache-2.0
hashbrown 0.17.1 | MIT OR Apache-2.0
heck 0.5.0 | MIT OR Apache-2.0
http 1.5.0 | MIT OR Apache-2.0
httparse 1.10.1 | MIT OR Apache-2.0
hybrid-array 0.4.15 | MIT OR Apache-2.0
iana-time-zone 0.1.65 | MIT OR Apache-2.0
iana-time-zone-haiku 0.1.2 | MIT OR Apache-2.0
indexmap 2.14.2 | Apache-2.0 OR MIT
insta 1.48.0 | Apache-2.0
intl-memoizer 0.5.3 | Apache-2.0 OR MIT
intl_pluralrules 7.0.2 | Apache-2.0/MIT
is_terminal_polyfill 1.70.2 | MIT OR Apache-2.0
itoa 1.0.18 | MIT OR Apache-2.0
js-sys 0.3.106 | MIT OR Apache-2.0
libc 0.2.189 | MIT OR Apache-2.0
linux-raw-sys 0.12.1 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT
log 0.4.34 | MIT OR Apache-2.0
memchr 2.8.3 | Unlicense OR MIT
miniz_oxide 0.9.1 | MIT OR Zlib OR Apache-2.0
normalize-line-endings 0.3.0 | Apache-2.0
num-traits 0.2.19 | MIT OR Apache-2.0
once_cell 1.21.4 | MIT OR Apache-2.0
once_cell_polyfill 1.70.2 | MIT OR Apache-2.0
percent-encoding 2.3.2 | MIT OR Apache-2.0
phf 0.12.1 | MIT
phf_shared 0.12.1 | MIT
pin-project-lite 0.2.17 | Apache-2.0 OR MIT
pqueue 0.1.0 | MIT
predicates 3.1.4 | MIT OR Apache-2.0
predicates-core 1.0.10 | MIT OR Apache-2.0
predicates-tree 1.0.13 | MIT OR Apache-2.0
proc-macro-hack 0.5.20+deprecated | MIT OR Apache-2.0
proc-macro2 1.0.107 | MIT OR Apache-2.0
quote 1.0.47 | MIT OR Apache-2.0
r-efi 6.0.0 | MIT OR Apache-2.0 OR LGPL-2.1-or-later
regex 1.13.1 | MIT OR Apache-2.0
regex-automata 0.4.18 | MIT OR Apache-2.0
regex-syntax 0.8.11 | MIT OR Apache-2.0
ring 0.17.14 | Apache-2.0 AND ISC
roff 1.1.1 | MIT OR Apache-2.0
rpassword 7.5.4 | Apache-2.0
rtoolbox 0.0.6 | Apache-2.0
rtree_rs 0.1.4 | MIT
rustc-hash 2.1.3 | Apache-2.0 OR MIT
rustix 1.1.5 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT
rustls 0.23.45 | Apache-2.0 OR ISC OR MIT
rustls-pki-types 1.15.1 | MIT OR Apache-2.0
rustls-webpki 0.103.15 | ISC
rustversion 1.0.23 | MIT OR Apache-2.0
self_cell 1.3.0 | Apache-2.0 OR GPL-2.0-only
serde 1.0.229 | MIT OR Apache-2.0
serde_core 1.0.229 | MIT OR Apache-2.0
serde_derive 1.0.229 | MIT OR Apache-2.0
serde_json 1.0.151 | MIT OR Apache-2.0
serde_spanned 1.1.1 | MIT OR Apache-2.0
sha2 0.11.0 | MIT OR Apache-2.0
shlex 2.0.1 | MIT OR Apache-2.0
simd-adler32 0.3.10 | MIT
similar 2.7.0 | Apache-2.0
siphasher 1.0.4 | MIT OR Apache-2.0
slab 0.4.12 | MIT
smallvec 1.16.2 | MIT OR Apache-2.0
strsim 0.11.1 | MIT
subtle 2.6.1 | BSD-3-Clause
syn 2.0.119 | MIT OR Apache-2.0
syn 3.0.6 | MIT OR Apache-2.0
tempfile 3.27.0 | MIT OR Apache-2.0
terminal_size 0.4.4 | MIT OR Apache-2.0
termtree 0.5.1 | MIT
thiserror 2.0.21 | MIT OR Apache-2.0
thiserror-impl 2.0.21 | MIT OR Apache-2.0
tinystr 0.8.4 | Unicode-3.0
toml 1.1.6+spec-1.1.0 | MIT OR Apache-2.0
toml_datetime 1.1.1+spec-1.1.0 | MIT OR Apache-2.0
toml_parser 1.1.3+spec-1.1.0 | MIT OR Apache-2.0
toml_writer 1.1.2+spec-1.1.0 | MIT OR Apache-2.0
type-map 0.5.1 | MIT/Apache-2.0
typenum 1.20.1 | MIT OR Apache-2.0
tzf-dist 0.0.2026-d-fix1 | MIT AND ODbL-1.0
tzf-rs 2.1.2 | MIT
unic-langid 0.9.6 | MIT OR Apache-2.0
unic-langid-impl 0.9.6 | MIT OR Apache-2.0
unic-langid-macros 0.9.6 | MIT OR Apache-2.0
unic-langid-macros-impl 0.9.6 | MIT OR Apache-2.0
unicode-ident 1.0.26 | (MIT OR Apache-2.0) AND Unicode-3.0
unicode-width 0.2.2 | MIT OR Apache-2.0
untrusted 0.9.0 | ISC
ureq 3.4.2 | MIT OR Apache-2.0
ureq-proto 0.6.4 | MIT OR Apache-2.0
utf8-zero 0.8.1 | MIT OR Apache-2.0
utf8parse 0.2.2 | Apache-2.0 OR MIT
wait-timeout 0.2.1 | MIT/Apache-2.0
wasi 0.11.1+wasi-snapshot-preview1 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT
wasm-bindgen 0.2.129 | MIT OR Apache-2.0
wasm-bindgen-macro 0.2.129 | MIT OR Apache-2.0
wasm-bindgen-macro-support 0.2.129 | MIT OR Apache-2.0
wasm-bindgen-shared 0.2.129 | MIT OR Apache-2.0
webpki-roots 1.0.9 | CDLA-Permissive-2.0
windows-core 0.62.2 | MIT OR Apache-2.0
windows-implement 0.60.2 | MIT OR Apache-2.0
windows-interface 0.59.3 | MIT OR Apache-2.0
windows-link 0.2.1 | MIT OR Apache-2.0
windows-result 0.4.1 | MIT OR Apache-2.0
windows-strings 0.5.1 | MIT OR Apache-2.0
windows-sys 0.52.0 | MIT OR Apache-2.0
windows-sys 0.61.2 | MIT OR Apache-2.0
windows-targets 0.52.6 | MIT OR Apache-2.0
windows_aarch64_gnullvm 0.52.6 | MIT OR Apache-2.0
windows_aarch64_msvc 0.52.6 | MIT OR Apache-2.0
windows_i686_gnu 0.52.6 | MIT OR Apache-2.0
windows_i686_gnullvm 0.52.6 | MIT OR Apache-2.0
windows_i686_msvc 0.52.6 | MIT OR Apache-2.0
windows_x86_64_gnu 0.52.6 | MIT OR Apache-2.0
windows_x86_64_gnullvm 0.52.6 | MIT OR Apache-2.0
windows_x86_64_msvc 0.52.6 | MIT OR Apache-2.0
winnow 1.0.4 | MIT
zerofrom 0.1.8 | Unicode-3.0
zeroize 1.9.0 | Apache-2.0 OR MIT
zerovec 0.11.8 | Unicode-3.0
zlib-rs 0.6.8 | Zlib
zmij 1.0.23 | MIT
  ```
* `cargo deny` bans: three duplicate versions exist in the graph and are skipped with a reason each (`syn` 2/3,
  `windows-sys` 0.52/0.61); wildcard requirements and non-crates.io sources are denied outright. `cargo audit`
  is clean; the yanked-crate check needs the crates.io API, which a mirror can refuse (`403`), so the CI job is
  the place that runs it against the real API.
* No Windows CI job in v1: no Windows packaging exists (step 13 ships Linux/macOS archives, ecosystem packaging
  is backlog B02), no Windows-specific code path exists, and the symlink trap would make lint platform-dependent.

## Out of scope

* Feature work, including the error wording of features that do not exist yet: alerts (step 15), air quality and
  pollen (step 16), moon/astro (step 17), offline city database (step 18), more providers (step 23), the
  wttr.in-compatible service (step B01), multi-location output (step 19).
* Enforced performance/resource budgets with CI thresholds: step 21 (here only the startup gate is rechecked).
* Packaging and release mechanics: step 13; ecosystem artefacts and Windows/macOS installers: backlog B02.

## Verification

```
cargo deny check && cargo audit && reuse lint                    # all green, 0 lint problems
cargo +1.98.0 build --all-targets --locked && cargo +1.98.0 test --locked
CIRROCAST_FORBID_NETWORK=1 cargo test                            # suite green with the guard active
strace -f -e trace=network target/release/cirrocast --version 2>&1 | grep -c 'socket('   # expect 0
hyperfine --warmup 10 --runs 50 'target/release/cirrocast --version'                     # under 50 ms
target/release/cirrocast cache clean && target/release/cirrocast Beijing --offline        # exit 3, rerun hint
```

## Exit criteria

- ✅ `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`, `reuse lint` all clean.
- ✅ `cargo deny check` and `cargo audit` green, `cargo +1.98.0 build --all-targets --locked` green, and the
  dependency licence verdict table filled in with no unresolved GPL-3.0-or-later incompatibility. (Locally the
  single system toolchain 1.98.1 ran `cargo build --all-targets --locked` and `cargo test --locked`; the exact
  `1.98.0` pin is the CI matrix leg. `cargo audit` needs `--no-yanked` behind this machine's mirror, which
  answers the crates.io API with 403; CI runs the full check.)
- ✅ `CIRROCAST_FORBID_NETWORK=1 cargo test` passes (206 unit + 43 integration binaries green), the guard test
  shows a blocked request exiting 3 with the guard message, and `hyperfine` reports `--version` at 1.4 ms mean
  with no socket opened.

## Risks

* Advisory-database or mirror flakiness in CI would destabilise red/green: pin actions by SHA and give every
  `[advisories] ignore` entry a reason plus a review date.
* A dependency bump can raise MSRV above 1.98 or add a denied licence: `--locked` in CI forces such a bump into a
  reviewed commit that the licence gate can fail.
* The loopback exception could hide a network-dependent test: the blocking test targets a real host, while the
  layer gate and the `unshare -rn` run give independent signals.
* Message-contract tests could ossify wording: assertions target the offending value and the next-command
  substring only, keeping rewordings cheap.
* macOS runners lack `strace`: the no-network check is Linux-only, while macOS runs `--version` and the timing check.

## Progress log

- 2026-09-30 — step file written (status: not-started); `reuse lint` state (1 invalid expression, 1 headerless
  file) and the symlink-versus-copy behaviour recorded while drafting.
- 2026-10-01 — step opened (status: in-progress). Landed: the `CIRROCAST_FORBID_NETWORK` guard and the 8 MiB
  body cap in `src/http.rs` (blocked before DNS; loopback exempt for in-process stubs); the offline-miss error
  now names provider, place, key path and the rerun hint (`Cache::read_or_fetch_json` gained the `place`
  argument, so `fetch_json` takes the `Location` and metar its station); `cache stat` counts expired entries
  separately; `config validate` got the strict key check (`check_known_keys`), the ICAO station rule
  (`provider::metar::is_icao_station`, shared with `--station`), the `location.default` grammar check, the
  `cache.enabled = false` + `--offline` combination (also enforced on a weather run) and the `[units]` override
  precedence notes; `MissingKey` and `LocationNotFound` name their next command. Two deliverables ticked;
  the verbose review and the message audit are still open.
- 2026-10-01 — second batch: `tests/exit_codes.rs` pins 0/2/3/4/5/6 with the offending value in the message;
  `tests/no_network.rs` proves the guard blocks a cold-cache request and that a guarded `--offline` run works
  inside `unshare -rn` (no DNS at all); the secret audit found and fixed a real leak — five providers put
  `full_url()` (key included) into the report attribution, which `-vv` printed, so all of them now use
  `redacted_url()` and `tests/cli_offline.rs` greps both streams and the cache tree for an exported fake key;
  `-v`/`-vv` are split (request URLs, statuses, retries and cache decisions moved to `-vv`); `cache stat` prints
  the expired count; the `--help` epilog gained `CONFIG PRECEDENCE` plus one-line exit-code triggers, and
  `--help`/`--version` are proven state-free with `man` rendering the same tables. `tests/xdg.rs` walks the whole
  throwaway tree after writes and checks `XDG_CONFIG_DIRS` reads. `deny.toml` lands with the audited allow list
  (the deny list is allow-list-by-omission; `cargo-deny` has no separate deny list) plus the 174-crate verdict
  table above; `cargo deny check` and `cargo audit --no-yanked` are green (the yank check needs the crates.io
  API, which this mirror refuses with 403 — CI runs it against the real API).
- 2026-10-01 — final batch: `tests/decoder_robustness.rs` (every provider, the geocoder, the IP chain and the
  config parser over truncation, empty/`{}`/`[]`/`null`, wrong-typed and byte-flipped bodies, with
  `tests/fixtures/malformed/**`), `tests/xdg.rs`, the render-layer refactor (registry metadata now travels in
  `model::Attribution`), `.github/workflows/ci.yml` on explicit `ubuntu-26.04`/`macos-26` images with
  SHA-pinned actions, README/AGENTS CI documentation, the startup gate (1.4 ms, no `socket(`), the MSRV
  recheck on the system 1.98.1 and the `LICENSES/GPL-3.0-or-later.txt` copy (`cmp` gate in CI).
  Deliberate deviations, both recorded above and in the deliverable text: the decoder sweep is exhaustive for
  payloads up to 8 KiB and strided (≤ 2048 offsets per fixture) beyond it, because the exhaustive form cost
  343 s in a debug build and 33 s after the bound; and the exact `+1.98.0` toolchain leg runs only in CI,
  because this machine has one system toolchain and no rustup. Step marked done: all deliverables and exit
  criteria are ✅.
- 2026-10-02 — defect found by step 14's flag/help review and fixed here: this step's deliverable text and
  `cirrocast --help` both said request URLs appear only at `-vv`, but `-v` has always printed the provider
  request URL as part of `verbose_report` (the credit line a bug report needs). The wording now describes
  what the split really is — `-v`: settings with sources, resolution notes, the upstream request behind the
  answer (secrets redacted) and the error cause chain; `-vv`: every HTTP attempt with its status and backoff
  delay plus the cache decisions — in `src/cli.rs`'s `--verbose` doc comment (and therefore the man page) and
  in the deliverable line above. The historical log line below the split's own entry is left as written; this
  entry is the correction.
- 2026-10-02 — the `audit` job's action pin moved from `rustsec/audit-check` v2.0.0 to the `main` commit
  "Update to use Node 24 (#48)" (`858dc40f`, 2026-03-20): GitHub now runs Node 20 actions on Node 24 and
  annotates every run with a deprecation warning, and the newest *release* still declares `node20`. The commit
  changes only `runs.using` (plus changelog and package metadata), so the pin is safe; the comment above the
  `uses:` line says to switch back to a tag once a release carries the fix.
- 2026-10-02 — Dependabot added (`.github/dependabot.yml`): a daily check of `cargo` (manifest and lockfile)
  and `github-actions` (the pins in `.github/workflows/`, SHA and `# vX.Y.Z` comment together), grouped as one
  pull request per ecosystem for minor and patch bumps with majors left individual — the MSRV, `deny` and
  `audit` gates are what decide whether a bump can land, so the grouped PR is the reviewable unit and a major
  gets its own. The `github-actions` group deliberately omits `update-types`: the reference does not state that
  SemVer-level grouping is supported for that ecosystem, and there are only eight actions. Dependabot alerts
  and automated security updates were switched on in the repository settings (both were off); version updates
  keep Dependabot's default three-day cooldown, which does not apply to security updates. README's development
  section documents it.
- 2026-10-03 — review-01 fix 5.1 split the `Provider` trait (`fetch` provided with the finite-reading
  guard, backends implement `fetch_report`) but only the review record was updated; the binding sketch in
  `docs/plans/README.md` still showed a single required `fetch`, which a step-19 implementor would copy into
  code that cannot compile. The sketch is synced here, together with the guard sentence. No step-12
  deliverable or exit criterion changes.
