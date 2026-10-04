#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Refreshes the embedded GeoNames city table (`src/geo/data`) from a `cities15000` dump, or checks
# whether the committed snapshot is still what that dump produces.
#
#   scripts/refresh-city-data.sh                       # refresh from the official dump
#   scripts/refresh-city-data.sh --check               # only report whether it differs
#   scripts/refresh-city-data.sh --from <path-or-url>  # a local `.txt`, or a `.zip` (path or URL)
#
# `--check` builds into a temporary directory and compares the three files byte for byte; it never
# touches the working tree and exits 1 when the dump differs (0 when the committed snapshot is
# current, 2 on a usage error).
#
# A refresh runs the canary tests that pin rows of the committed snapshot but never edits them: when
# the dump moves a pinned value the tests fail and the operator updates the expectation
# deliberately, which is what keeps a data refresh reviewable. The `SNAPSHOT` record (dump date,
# input SHA-256, row and key counts) is rewritten by the builder itself; the size and timing numbers
# in `docs/plans/21-perf-and-resource-budget.md` are re-recorded by hand.
#
# The official host is reachable directly (a China IP and a US egress both work; verified
# 2026-10-04), so a timeout is a transient routing problem rather than an access restriction —
# retry, or pass `--from` with a local copy or a mirror of the dump.
#
# Requirements: `curl` and `unzip` for the URL/zip path (nothing else beyond the Rust toolchain).

set -euo pipefail

usage() {
    sed -n '5,26p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
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

case "$source" in
    *.txt)
        dump=$source
        [ -f "$dump" ] || { echo "error: $dump does not exist" >&2; exit 1; }
        ;;
    *.zip)
        archive=$source
        if [[ $source == http://* || $source == https://* ]]; then
            command -v curl >/dev/null || { echo "error: curl is required to download a dump" >&2; exit 1; }
            archive=$workdir/cities15000.zip
            echo "downloading $source"
            curl --fail --location --silent --show-error --output "$archive" "$source"
        fi
        command -v unzip >/dev/null || { echo "error: unzip is required to extract a dump" >&2; exit 1; }
        dump=$workdir/cities15000.txt
        unzip -p "$archive" cities15000.txt >"$dump"
        ;;
    *)
        echo "error: --from expects a .txt or a .zip, got \`$source\`" >&2
        exit 2
        ;;
esac

if [ "$mode" = check ]; then
    built=$workdir/out
    cargo run -q -p geo-table -- "$dump" "$built" >/dev/null
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

cargo run -q -p geo-table -- "$dump" src/geo/data

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
