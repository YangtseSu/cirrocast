#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
# SPDX-License-Identifier: GPL-3.0-or-later
"""Compare a fresh measurement (`scripts/bench/run.sh`) against the committed baseline.

Prints one row per metric with its budget, the baseline, the fresh median and the delta, and exits
1 when a gated metric fails. A metric fails when

* its fresh median exceeds the baseline by more than [`TOLERANCE`] (the ratio gate: shared CI
  runners fluctuate more than absolute numbers can absorb, so the committed baseline is the
  reference and the allowance is the noise floor), or
* its fresh median exceeds the hard budget **and** the baseline was inside that budget — a budget
  the baseline already misses is not a regression, it is a budget to re-derive, and it is reported
  as such instead of failing every run.

`--raw-only` prints the fresh numbers without a baseline; `scripts/bench/run.sh` uses it for the
human-readable summary at the end of a measurement.

Usage: scripts/bench/compare.py [--raw-only] [perf/baseline.json] target/bench/raw.json
"""

import argparse
import json
import pathlib
import sys

# The hard budgets of step 21's goal, as re-derived in docs/performance.md. `--version` 20 ms, a
# cached run 60 ms and `--help` 200 lines are the goal's numbers, met by the recorded baseline.
# Two budgets are re-derived from measurement:
# * the binary: the goal's 5 MiB predates the offline city table, and the bundled GeoNames members
#   (3.28 MiB) plus the decoder, TLS and the catalogs put the floor near 15 MiB, so the enforced
#   budget is 17 MiB — the measured size plus headroom for one more dependency;
# * RSS: the goal's 15 MiB is the budget of a run that never materialises the city index, which is
#   what `--version` measures; the cached run decodes the name index (the step-18 table) and its
#   budget is 32 MiB, the measured peak plus headroom.
BUDGETS = {
    "version_ms": 20.0,
    "help_ms": None,
    "offline_plain_ms": 60.0,
    "warm_plain_ms": 60.0,
    "version_rss_kib": 15 * 1024.0,
    "plain_rss_kib": 32 * 1024.0,
    "binary_bytes": 17 * 1024 * 1024.0,
    "help_lines": 200.0,
}

# The ratio a fresh median may exceed the baseline by before it counts as a regression.
TOLERANCE = 1.20

# Metrics whose value is a plain number rather than a distribution; the baseline's own shape is
# `{"median": …}` for every metric, so the only difference is how the row prints.
UNITS = {
    "version_ms": "ms",
    "help_ms": "ms",
    "offline_plain_ms": "ms",
    "warm_plain_ms": "ms",
    "version_rss_kib": "KiB",
    "plain_rss_kib": "KiB",
    "binary_bytes": "B",
    "help_lines": "lines",
}


def load(path: pathlib.Path) -> dict:
    try:
        return json.loads(path.read_text())
    except FileNotFoundError:
        print(f"missing {path}; run scripts/bench/run.sh first", file=sys.stderr)
        raise SystemExit(2) from None
    except json.JSONDecodeError as error:
        print(f"{path} is not valid JSON: {error}", file=sys.stderr)
        raise SystemExit(2) from None


def median(entry: dict | float) -> float:
    return float(entry["median"] if isinstance(entry, dict) else entry)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--raw-only", action="store_true", help="print the fresh numbers alone")
    parser.add_argument("baseline", nargs="?", default="perf/baseline.json")
    parser.add_argument("raw", nargs="?", default="target/bench/raw.json")
    args = parser.parse_args()

    raw = load(pathlib.Path(args.raw))
    baseline_path = pathlib.Path(args.baseline)
    baseline = None if args.raw_only else load(baseline_path)
    base_metrics = {} if baseline is None else baseline.get("metrics", {})

    header = f"{'metric':<18} {'budget':>12} {'baseline':>12} {'fresh':>12} {'delta':>8}  verdict"
    print(header)
    print("-" * len(header))

    failed = False
    for name in sorted(raw):
        fresh = median(raw[name])
        budget = BUDGETS.get(name)
        base = median(base_metrics[name]) if name in base_metrics else None
        unit = UNITS.get(name, "")

        if base is None:
            delta = "-"
            verdict = "fresh" if args.raw_only else "FAIL no baseline"
            if not args.raw_only:
                failed = True
        else:
            ratio = fresh / base if base else 1.0
            delta = f"{(ratio - 1) * 100:+.1f}%"
            if ratio > TOLERANCE:
                verdict = f"FAIL +{(ratio - 1) * 100:.0f}% vs baseline"
                failed = True
            elif budget is not None and fresh > budget and base <= budget:
                verdict = "FAIL over budget"
                failed = True
            elif budget is not None and fresh > budget:
                verdict = "warn: budget re-derivation due"
            else:
                verdict = "ok"

        budget_text = "-" if budget is None else f"{budget:,.0f} {unit}".strip()
        base_text = "-" if base is None else f"{base:,.0f} {unit}".strip()
        print(
            f"{name:<18} {budget_text:>12} {base_text:>12} "
            f"{fresh:,.0f} {unit:<5} {delta:>8}  {verdict}"
        )

    if baseline is not None:
        missing = sorted(set(base_metrics) - set(raw))
        if missing:
            print(f"\nFAIL the harness no longer measures: {', '.join(missing)}", file=sys.stderr)
            failed = True
        cold = baseline.get("cold")
        if cold:
            print(
                f"\ncold run (not gated): {cold['median_ms']:.0f} ms — {cold['conditions']}"
            )
        if not failed:
            print("\nno regression: every gated metric is inside its budget and the baseline")

    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
