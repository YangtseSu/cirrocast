#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
# SPDX-License-Identifier: GPL-3.0-or-later
#
# The measurement harness behind step 21's performance budget (docs/performance.md).
#
# It builds the real `release` profile, pins a throwaway XDG tree and a seeded cache, runs
# hyperfine over the four gated commands, measures the peak RSS of the cached run and the binary
# size and help length, and writes every number to `target/bench/raw.json` — the file
# `scripts/bench/compare.py` reads. Nothing here touches the network: `CIRROCAST_FORBID_NETWORK=1`
# is exported for the measured runs, so an accidental request fails loudly instead of quietly
# making the run slower and the number meaningless.
#
# Usage: scripts/bench/run.sh
#   BIN           override the binary under test (default: target/release/cirrocast)
#   RUNS/WARMUP   hyperfine run and warm-up counts (defaults: 30 / 5)

set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
cd "$root"

runs=${RUNS:-30}
warmup=${WARMUP:-5}
out_dir="$root/target/bench"
sandbox="$out_dir/sandbox"
raw="$out_dir/raw.json"

echo "==> building the release binary"
cargo build --release --locked

bin=${BIN:-$root/target/release/cirrocast}
if [[ ! -x $bin ]]; then
    echo "no binary at $bin" >&2
    exit 1
fi

echo "==> preparing the sandbox and the seeded cache"
rm -rf "$sandbox"
mkdir -p "$sandbox"/{cache,config,data,home,system}
export XDG_CACHE_HOME="$sandbox/cache"
export XDG_CONFIG_HOME="$sandbox/config"
export XDG_DATA_HOME="$sandbox/data"
export XDG_CONFIG_DIRS="$sandbox/system"
export HOME="$sandbox/home"
export TZ=UTC
export LANG=C.UTF-8
export LC_ALL=C.UTF-8
unset COLUMNS
# Alerts are on by default and would send the run to a national service; the measured run is the
# deterministic core — resolve the name, read the forecast from the cache, render — so the bench
# config turns them off rather than letting a live fetch decide the number.
mkdir -p "$XDG_CONFIG_HOME/cirrocast"
cat > "$XDG_CONFIG_HOME/cirrocast/config.toml" <<'TOML'
schema_version = 2

[alerts]
enabled = false
TOML

python3 - "$XDG_CACHE_HOME/cirrocast" <<'PY'
import json, pathlib, subprocess, sys

root = pathlib.Path(sys.argv[1])
# The key carries the location-local date, exactly as `CacheKey::weather` builds it.
date = subprocess.run(
    ["date", "+%F"], capture_output=True, text=True, check=True,
    env={"TZ": "Asia/Shanghai", "PATH": "/usr/bin:/bin"},
).stdout.strip()
body = pathlib.Path("tests/fixtures/open_meteo/forecast_beijing_2026-07-15.json").read_text()
entry = {
    "cache_schema_version": 1,
    "key": f"weather|open-meteo|39.91|116.40|3|{date}",
    "fetched_at": "2026-01-01T00:00:00Z",
    "ttl_secs": 315_360_000,
    "status": 200,
    "body": body,
}
target = root / "weather" / f"open-meteo-39.91-116.40-3-{date}.json"
target.parent.mkdir(parents=True, exist_ok=True)
target.write_text(json.dumps(entry))
print(f"seeded {target.relative_to(root.parent)}")
PY

# Timing runs are pinned to one core so a scheduler decision cannot move the median; `taskset` is
# Linux-only, and its absence (macOS) is documented in docs/performance.md rather than papered
# over.
pin=""
if command -v taskset >/dev/null 2>&1; then
    pin="taskset -c 0 "
fi

export CIRROCAST_FORBID_NETWORK=1

measure() { # measure <name> <command>
    local name=$1 command=$2
    echo "==> $name: $command"
    hyperfine --warmup "$warmup" --runs "$runs" -N --style basic \
        --export-json "$out_dir/$name.json" \
        "$pin$command" >/dev/null
}

measure version "$bin --version"
measure help "$bin --help"
measure offline_plain "$bin --offline Beijing -f plain"
measure warm_plain "$bin Beijing -f plain"

echo "==> binary size and help length"
binary_bytes=$(stat -c %s "$bin" 2>/dev/null || stat -f %z "$bin")
help_lines=$("$bin" --help | wc -l | tr -d ' ')

python3 - "$out_dir" "$raw" "$bin" "$binary_bytes" "$help_lines" <<'PY'
import json, os, pathlib, subprocess, sys

out_dir, raw, binary, binary_bytes, help_lines = (
    pathlib.Path(sys.argv[1]),
    pathlib.Path(sys.argv[2]),
    sys.argv[3],
    float(sys.argv[4]),
    float(sys.argv[5]),
)


def payload(name):
    return json.loads((out_dir / f"{name}.json").read_text())["results"][0]


def timing(name):
    result = payload(name)
    return {
        "median": result["median"] * 1000.0,
        "min": result["min"] * 1000.0,
        "max": result["max"] * 1000.0,
        "unit": "ms",
        "runs": len(result["times"]),
    }


def gnu_time_kib(command):
    """GNU time's `Maximum resident set size`, or None when GNU time is not installed."""
    tool = "/usr/bin/time"
    if not os.path.exists(tool):
        return None
    try:
        completed = subprocess.run(
            [tool, "-v", *command],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.PIPE,
            text=True,
            check=True,
        )
    except (OSError, subprocess.CalledProcessError):
        return None
    for line in completed.stderr.splitlines():
        if "Maximum resident set size" in line:
            return float(line.rsplit(":", 1)[1].strip())
    return None


def rss_kib(name, command):
    """Peak RSS of one measured command, in KiB.

    GNU time (`/usr/bin/time -v`) is the exact per-process peak and the primary measurement. It
    is a GNU coreutils extra that is not installed everywhere (this machine included), so the
    fallback is the peak hyperfine recorded for the same command — its own resident set is a few
    megabytes, far below either measured number, and it is a per-child `wait4` figure. A
    `/proc`-sampling fallback is deliberately not used: a 50 ms run is too short to sample, and
    `getrusage(RUSAGE_CHILDREN)` read from a forked parent counts the parent's pages, which
    overstated `--version` by ~6 MiB when it was tried. docs/performance.md records which path a
    baseline was measured with.
    """
    peak = gnu_time_kib(command)
    if peak is not None:
        return peak
    return max(payload(name)["memory_usage_byte"]) / 1024.0


metrics = {
    "version_ms": timing("version"),
    "help_ms": timing("help"),
    "offline_plain_ms": timing("offline_plain"),
    "warm_plain_ms": timing("warm_plain"),
    "version_rss_kib": {
        "median": rss_kib("version", [binary, "--version"]),
        "unit": "KiB",
    },
    "plain_rss_kib": {
        "median": rss_kib("offline_plain", [binary, "--offline", "Beijing", "-f", "plain"]),
        "unit": "KiB",
    },
    "binary_bytes": {"median": binary_bytes, "unit": "B"},
    "help_lines": {"median": help_lines, "unit": "lines"},
}
raw.write_text(json.dumps(metrics, indent=2, sort_keys=True) + "\n")
print(f"wrote {raw}")
PY

python3 "$root/scripts/bench/compare.py" --raw-only "$raw"
