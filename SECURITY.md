<!--
SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Security policy

`cirrocast` is a local terminal weather client. It has no server component, no account system and no
telemetry, so the interesting security surface is small: credential handling, the files it writes,
and how it talks to upstream APIs. This page says what is in scope, how to report a problem, and what
promise the project makes about your data.

## Reporting a vulnerability

Report privately through GitHub's security advisory form:

**<https://github.com/YangtseSu/cirrocast/security/advisories/new>**

(repository → **Security** tab → **Report a vulnerability**)

The project has **no security email address** — none is published, and none will be answered. The
GitHub private advisory is the only private channel; do not open a public issue for a suspected
vulnerability, and do not include exploit details in an ordinary issue.

Please include:

* the `cirrocast --version` output and the platform (`uname -srm`, or the release archive/packaging
  source you installed from);
* what you did, what happened, and what you expected;
* the impact you believe it has — in particular whether it involves API keys, `keys.toml`,
  `config.toml`, the cache under `$XDG_CACHE_HOME/cirrocast/`, or the network path;
* a minimal reproduction, with secrets redacted.

The maintainer is a single person and works on this in their own time: expect an acknowledgement as
soon as possible, an assessment, and a coordinated disclosure date once a fix is ready. Credit is
given in the release notes on request.

Only the most recent release receives fixes; there are no maintenance branches and no backports, so a
report should be checked against the latest tag before it is filed.

## No telemetry and no analytics

`cirrocast` contains no telemetry, no analytics, no crash reporting and no phone-home of any kind. It
contacts a remote host only to answer the request you made:

* the weather/geocoding/alert API selected for your location;
* the public-IP service, and only when you ask with `--ip`, or when no location is configured
  anywhere (the flag and the README document this; the answer is cached for 24 hours);
* nothing else, ever.

There is no identifier, no install ping and no usage counter in the binary or the packaging.

## Key handling

API keys and other credentials never live in `config.toml` (which is `0644`, meant to be pasted into
bug reports and dumped by `cirrocast config show`). They are resolved from, first hit wins:

* an environment variable — `CIRROCAST_<PROVIDER>_KEY`, provider id upper-cased with `-` → `_`
  (e.g. `CIRROCAST_OPENWEATHERMAP_KEY`);
* `$XDG_CONFIG_HOME/cirrocast/keys.toml`, a **`0600`** document written by
  `cirrocast key set <provider>`, which reads the secret from **stdin, never argv** (a terminal, a
  pipe or a here-doc — never a shell history entry or a process listing).

A `keys.toml` that carries any group or other permission bit is **refused** with a configuration
error rather than used, so a stray `chmod 644` fails loudly instead of leaking silently. There is no
OS-keyring tier in v1: a key in neither place is simply reported as missing.

Providers with a second authentication mode (QWeather) keep a `[jwt.<provider>]` table in the same
`0600` file, or the `CIRROCAST_<PROVIDER>_JWT_*` environment quartet; the PEM private key is read
from a file or stdin at `key set` time, never from argv. Requests are signed in memory and the minted
token is never written to disk.

In all cases:

* secrets are never written to `config.toml` and never to the cache (the cache stores upstream
  responses, and every cache key is derived from the request, not from a credential);
* secrets are never logged and never included in error messages, panic messages or `--verbose`
  output — `-vv` records the request, not its credential;
* `cirrocast key list` prints **masked** values only (a JWT credential shows its three non-secret
  identifiers, never the private key).

If you believe a key has been exposed by `cirrocast` itself, that is a security report: file an
advisory, rotate the key with the upstream provider, and — if it was committed to a repository —
treat it as compromised rather than trying to erase the history.

## Scope

In scope:

* **the binary** — the CLI, the location/geocoding path, the provider and renderer code, the template
  engine, HTTP handling and TLS verification;
* **packaging** — the AUR `PKGBUILD`/`.SRCINFO` and the release archives published from this
  repository (a mismatch between what the tree says and what a package installs is a packaging bug);
* **config and cache handling** — the XDG paths, file modes, `keys.toml` permission enforcement, the
  atomic `tmp` + `rename` writes, cache validation and any path taken with attacker-influenced input
  (a location string, a response body, a file under the config or cache directory).

Out of scope:

* **upstream providers' own services** — availability, accuracy, pricing, quota enforcement, their
  data or their TLS configuration. Report those to the provider; the ones this project uses are
  listed in the README and `docs/providers.md`.
* **your own misconfigured proxy** — a wrong `HTTPS_PROXY`, `[network] proxy` setting, or a
  local/enterprise TLS-intercepting proxy that you installed. `cirrocast` performs normal certificate
  verification; a proxy you configured to break that is not a `cirrocast` vulnerability.
* the security of the user's machine, shell, editor or terminal, and anything requiring an attacker
  to already have local code execution or read access to the user's `$XDG_CONFIG_HOME`;
* third-party package repositories and mirrors other than the AUR package and the release archives
  this repository publishes.

## No bug bounty

This project offers **no bug bounty** and cannot pay for reports. It is a volunteer, GPL-3.0-or-later
project; the reward is a fix, a release note and credit if you want it.
