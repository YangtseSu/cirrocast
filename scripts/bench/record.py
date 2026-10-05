#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
# SPDX-License-Identifier: GPL-3.0-or-later
"""Compose `perf/baseline.json` from a fresh `target/bench/raw.json` and this machine's spec.

The same script records the baseline locally (docs/performance.md) and, behind the CI job's
explicit `record_baseline` dispatch input, on a runner — so the two paths cannot produce two
shapes. The cold figure is passed in (`--cold-ms`/`--cold-conditions`) because it is measured by
hand with `scripts/bench/cold.sh`, never in CI.

Usage: scripts/bench/record.py [--raw target/bench/raw.json] [--out perf/baseline.json]
                               [--cold-ms 845 --cold-conditions "..."]
"""

import argparse
import datetime
import json
import os
import pathlib
import subprocess
import sys


def first_line(command: list[str], pattern: str | None = None) -> str | None:
    """The first line of `command`'s output, optionally the one matching `pattern`."""
    try:
        completed = subprocess.run(command, capture_output=True, text=True, check=True)
    except (OSError, subprocess.CalledProcessError):
        return None
    for line in completed.stdout.splitlines():
        if pattern is None or pattern in line:
            return line.strip()
    return None


def after_colon(line: str) -> str:
    return line.split(":", 1)[1].strip() if ":" in line else line.strip()


def machine_spec() -> dict:
    cpu = first_line(["lscpu"], "Model name") or first_line(["uname", "-m"]) or "unknown"
    cpu = after_colon(cpu)
    cores = first_line(["nproc"])
    kernel = first_line(["uname", "-r"]) or "unknown"
    rustc = first_line(["rustc", "--version"]) or "unknown"
    cargo = first_line(["cargo", "--version"]) or "unknown"
    hyperfine = first_line(["hyperfine", "--version"]) or "unknown"
    rss = "gnu-time" if os.path.exists("/usr/bin/time") else "hyperfine-wait4"
    return {
        "cpu": cpu,
        "cores": cores,
        "os": f"{os.uname().sysname} {os.uname().machine}",
        "kernel": kernel,
        "rustc": rustc,
        "cargo": cargo,
        "hyperfine": hyperfine,
        "python": sys.version.split()[0],
        "rss_method": rss,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--raw", default="target/bench/raw.json")
    parser.add_argument("--out", default="perf/baseline.json")
    parser.add_argument("--cold-ms", type=float, default=None)
    parser.add_argument("--cold-conditions", default=None)
    args = parser.parse_args()

    raw = pathlib.Path(args.raw)
    if not raw.exists():
        print(f"missing {raw}; run scripts/bench/run.sh first", file=sys.stderr)
        return 2
    metrics = json.loads(raw.read_text())

    commit = first_line(["git", "rev-parse", "--short", "HEAD"]) or "unknown"
    baseline = {
        "schema": 1,
        "recorded": datetime.date.today().isoformat(),
        "commit": commit,
        "machine": machine_spec(),
        "metrics": metrics,
    }
    if args.cold_ms is not None:
        baseline["cold"] = {
            "median_ms": args.cold_ms,
            "conditions": args.cold_conditions or "unspecified",
        }
    else:
        # A re-record on a runner does not re-measure the cold run (it is manual by design), so the
        # committed figure and its link description carry over unchanged.
        out = pathlib.Path(args.out)
        if out.exists():
            try:
                previous = json.loads(out.read_text())
            except json.JSONDecodeError:
                previous = {}
            if "cold" in previous:
                baseline["cold"] = previous["cold"]

    out = pathlib.Path(args.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(baseline, indent=2, sort_keys=True) + "\n")
    print(f"wrote {out} (commit {commit}, {len(metrics)} metrics)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
