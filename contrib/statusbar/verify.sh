#!/bin/sh
# SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Runs every contrib/statusbar example's probe command offline against a
# throwaway XDG tree seeded from the committed fixture cache, and asserts that
# each one exits 0 with exactly one non-empty line containing the fixture
# temperature. POSIX sh only; no network, no jq.
#
# The two prompt snippets (bash-prompt.sh, zsh-prompt.zsh) are for an
# interactive shell and are NOT sourced here: the marker in each file names the
# probe command, which is extracted and run exactly like the six bar examples.

set -eu

CIRROCAST=${CIRROCAST:-target/debug/cirrocast}

# Resolve the repository root from this script's own location so it can be run
# from any directory.
cd "$(dirname "$0")/../.."

# Locate the binary and put its directory first on PATH: the marker command
# spells the program as `cirrocast`.
if [ -x "$CIRROCAST" ]; then
    case "$CIRROCAST" in
        */*) PATH="$(cd "$(dirname "$CIRROCAST")" && pwd):$PATH" ;;
    esac
elif ! command -v "$CIRROCAST" >/dev/null 2>&1; then
    echo "verify.sh: cirrocast binary not found or not executable: $CIRROCAST" >&2
    exit 1
fi

tmp=$(mktemp -d) || {
    echo "verify.sh: mktemp -d failed" >&2
    exit 1
}
# Remove the throwaway tree on exit, including on interrupt.
trap 'rm -rf "$tmp"' 0 1 2 15

# Throwaway XDG tree with a default coordinate location and a cache entry for
# the date the binary keys on: the *location-local* date of the coordinate spec
# (Beijing is +08:00), not the runner's UTC date. Seeding only the UTC date
# missed the entry whenever the two differ — every day from 16:00 UTC on, which
# is when CI runs.
HOME="$tmp/home"
XDG_CONFIG_HOME="$tmp/config"
XDG_CACHE_HOME="$tmp/cache"
XDG_DATA_HOME="$tmp/data"
XDG_CONFIG_DIRS="$tmp/config-dirs"
LC_ALL=C.UTF-8
LANG=C.UTF-8
CIRROCAST_FORBID_NETWORK=1
export HOME XDG_CONFIG_HOME XDG_CACHE_HOME XDG_DATA_HOME XDG_CONFIG_DIRS
export LC_ALL LANG CIRROCAST_FORBID_NETWORK

mkdir -p "$HOME" "$XDG_DATA_HOME" "$XDG_CONFIG_DIRS" "$XDG_CONFIG_HOME/cirrocast"
printf '[location]\ndefault = "@39.9,116.4"\n' >"$XDG_CONFIG_HOME/cirrocast/config.toml"

fixture=tests/fixtures/cache/weather/open-meteo-39.90-116.40-3-2026-10-05.json
cache_dir="$XDG_CACHE_HOME/cirrocast/weather"
mkdir -p "$cache_dir"
# Both spellings of "today" are seeded: the location's zone is the one the
# binary computes, and the UTC date costs nothing while covering a run that
# crosses the local midnight between this seed and the probe.
for zone in Asia/Shanghai UTC; do
    cp "$fixture" "$cache_dir/open-meteo-39.90-116.40-3-$(TZ=$zone date +%F).json"
done

files="waybar.jsonc polybar.ini i3blocks.conf tmux.conf starship.toml bash-prompt.sh zsh-prompt.zsh"

for file in $files; do
    path="contrib/statusbar/$file"

    # The marker is the only source of the command; exactly one must be present.
    command=$(sed -n 's@^[[:space:]]*[#/][#/]*[[:space:]]*cirrocast-example: @@p' "$path")
    marker_count=$(printf '%s\n' "$command" | sed '/^$/d' | wc -l)
    if [ "$marker_count" -ne 1 ]; then
        echo "verify.sh: $path must contain exactly one cirrocast-example marker (found $marker_count)" >&2
        exit 1
    fi

    set +e
    out=$(sh -c "$command --offline" 2>"$tmp/stderr")
    rc=$?
    set -e

    fail() {
        echo "verify.sh: $path: $1" >&2
        echo "--- stdout ---" >&2
        printf '%s\n' "$out" >&2
        echo "--- stderr ---" >&2
        cat "$tmp/stderr" >&2
        exit 1
    }

    [ "$rc" -eq 0 ] || fail "expected exit 0, got $rc"
    [ -n "$out" ] || fail "stdout was empty"
    # A single line has no embedded newline, so stripping the trailing newline
    # makes `wc -l` report 0.
    [ "$(printf '%s' "$out" | wc -l)" -eq 0 ] || fail "stdout was not exactly one line"
    case "$out" in
        *'+18°C'*) ;;
        *) fail "stdout did not contain +18°C" ;;
    esac

    echo "ok: $file"
done

exit 0
