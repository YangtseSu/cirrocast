#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Refreshes the embedded GeoNames city table (`src/geo/data`) from a `cities15000` dump, or checks
# whether the committed snapshot is still what that dump produces.
#
#   scripts/refresh-city-data.sh                       # the official dump
#   scripts/refresh-city-data.sh --check               # only report whether it differs
#   scripts/refresh-city-data.sh --from <path-or-url>  # a local `.txt`/`.zip`, or a URL of either
#
# The source is read, fetched, extracted and built by `cargo run -p geo-table` — the same
# `geo::update::build_candidate` that `cirrocast location update-data` runs — so this script only
# decides where the files go, runs the canary tests and shows the diff. A URL goes through the
# shared HTTP client, so `CIRROCAST_FORBID_NETWORK`, the proxy and the retry policy apply here too.
#
# `--check` builds into a temporary directory and compares the three files byte for byte; it never
# touches the working tree and exits 0 when the committed snapshot is current, 1 when the dump
# differs (or the source cannot be read), 2 on a usage error.
#
# A refresh runs the canary tests that pin rows of the committed snapshot but never edits them: when
# the dump moves a pinned value the tests fail and the operator updates the expectation
# deliberately, which is what keeps a data refresh reviewable. The `SNAPSHOT` record (dump date,
# input SHA-256, row and key counts) is rewritten by the builder itself; the size and timing numbers
# in `docs/plans/21-perf-and-resource-budget.md` are re-recorded by hand.
#
# Requirements: nothing beyond the Rust toolchain — the URL and ZIP handling live in the binary.

set -euo pipefail

usage() {
    sed -n '5,25p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
}

mode=refresh
source=${CIRROCAST_GEONAMES_DUMP:-https://download.geonames.org/export/dump/cities15000.zip}
while [ $# -gt 0 ]; do
    case "$1" in
        --check)
            mode=check
            shift
            ;;
        --from)
            [ $# -ge 2 ] || { echo "error: --from needs a path or URL" >&2; exit 2; }
            source=$2
            shift 2
            ;;
        -h | --help)
            usage
            exit 0
            ;;
        *)
            echo "error: unknown argument \`$1\`" >&2
            usage >&2
            exit 2
            ;;
    esac
done

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$repo_root"

workdir=$(mktemp -d)
trap 'rm -rf "$workdir"' EXIT

# One build entry point: `geo-table` classifies the source (`.txt`/`.zip`, path or URL), fetches it
# through the shared HTTP client and writes the three files into the given directory.
build() {
    cargo run -q -p geo-table -- "$source" "$1"
}

if [ "$mode" = check ]; then
    built=$workdir/out
    build "$built" >/dev/null
    echo "== committed snapshot vs this dump =="
    changed=0
    for file in cities.bin.gz keys.bin.gz SNAPSHOT; do
        if cmp --silent "src/geo/data/$file" "$built/$file"; then
            echo "  unchanged  $file"
        else
            echo "  CHANGED    $file"
            changed=1
        fi
    done
    if [ "$changed" -eq 0 ]; then
        echo
        echo "the committed snapshot is current for this dump ($(sed -n 's/^dump-date = //p' src/geo/data/SNAPSHOT))."
        exit 0
    fi
    echo
    diff --unified "src/geo/data/SNAPSHOT" "$built/SNAPSHOT" || true
    echo
    echo "the dump differs; run the script without --check to refresh, then move the pinned" >&2
    echo "expectations the canaries name (src/geo/offline.rs, tests/offline_geo.rs) and re-record" >&2
    echo "the size/timing numbers in docs/plans/21-perf-and-resource-budget.md." >&2
    exit 1
fi

build src/geo/data

echo
echo "== src/geo/data/SNAPSHOT =="
cat src/geo/data/SNAPSHOT
echo
echo "== working tree =="
git diff --stat -- src/geo/data
echo
echo "== canary tests (they pin rows of the committed snapshot) =="
if cargo test -q --lib geo::offline && cargo test -q --test offline_geo; then
    echo
    echo "canaries pass."
    echo "Next: re-record the blob sizes and timings in docs/plans/21-perf-and-resource-budget.md,"
    echo "then commit src/geo/data with the SNAPSHOT."
else
    echo
    echo "the dump moved a pinned expectation; update src/geo/offline.rs / tests/offline_geo.rs" >&2
    echo "deliberately (the failure names the row), then run the canaries again." >&2
    exit 1
fi
