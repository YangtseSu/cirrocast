#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
# SPDX-License-Identifier: GPL-3.0-or-later
#
# The manual cold-run harness: an empty cache, `--refresh`, three timed runs, the median.
#
# This is never run in CI. A cold number depends on the link, the provider and the city, so gating
# it would produce a flaky job that blocks unrelated work; instead the number is recorded in
# `perf/baseline.json`'s `cold` object together with the link it was measured on, and the release
# checklist (backlog B02) re-runs it. See docs/performance.md → "The cold run".
#
# Usage: scripts/bench/cold.sh [LOCATION]
#   LOCATION   the city to fetch (default: Beijing)
#   BIN        the binary to measure (default: target/release/cirrocast)
#   NET        a one-line description of the link, recorded verbatim in the printed summary

set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
cd "$root"

location=${1:-Beijing}
bin=${BIN:-$root/target/release/cirrocast}
net=${NET:-"unspecified — describe the link when recording the baseline"}

if [[ ! -x $bin ]]; then
    echo "no binary at $bin; run scripts/bench/run.sh or cargo build --release" >&2
    exit 1
fi

sandbox=$(mktemp -d)
trap 'rm -rf "$sandbox"' EXIT
export XDG_CACHE_HOME="$sandbox/cache"
export XDG_CONFIG_HOME="$sandbox/config"
export XDG_DATA_HOME="$sandbox/data"
export XDG_CONFIG_DIRS="$sandbox/system"
export HOME="$sandbox/home"
export TZ=UTC
unset COLUMNS
mkdir -p "$XDG_CONFIG_HOME/cirrocast" "$XDG_DATA_HOME" "$XDG_CONFIG_DIRS" "$HOME"

# The same bench configuration `run.sh` uses: alerts are a separate fan-out (a default-config run
# additionally fetches the WMO index and one CAP document per FPAS area — measured at ~8.5 s here),
# and the cold budget is the promise the core run makes: resolve the name, fetch the forecast,
# render. docs/performance.md records both figures.
mkdir -p "$XDG_CONFIG_HOME/cirrocast"
cat > "$XDG_CONFIG_HOME/cirrocast/config.toml" <<'TOML'
schema_version = 2

[alerts]
enabled = false
TOML

# A cold run must reach the network: the caller is expected to have a working link, and the guard
# would otherwise turn every run into an error instead of a measurement.
unset CIRROCAST_FORBID_NETWORK

timings=()
for run in 1 2 3; do
    rm -rf "$XDG_CACHE_HOME/cirrocast"
    start=$(date +%s%N)
    if ! "$bin" --refresh "$location" -f plain >/dev/null 2>"$sandbox/run$run.err"; then
        echo "run $run failed:" >&2
        cat "$sandbox/run$run.err" >&2
        exit 1
    fi
    end=$(date +%s%N)
    ms=$(( (end - start) / 1000000 ))
    timings+=("$ms")
    echo "run $run: ${ms} ms"
done

python3 - "${timings[@]}" <<'PY'
import statistics, sys

timings = sorted(int(value) for value in sys.argv[1:])
print(f"median: {statistics.median(timings):.0f} ms (min {timings[0]}, max {timings[-1]})")
PY

echo
echo "Record in perf/baseline.json → cold: {median_ms, conditions: \"$net\"}"
echo "and re-state the machine in docs/performance.md if it changed."
