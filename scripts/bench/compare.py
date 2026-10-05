#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
# SPDX-License-Identifier: GPL-3.0-or-later
"""Compare a fresh measurement (`scripts/bench/run.sh`) against the committed baseline.

Prints one row per metric with its budget, the baseline, the fresh median and the delta, and exits
1 when a gated metric fails. A metric fails when

* its fresh median exceeds the hard budget **and** the baseline was inside that budget — a budget
  the baseline already misses is not a regression, it is a budget to re-derive, and it is reported
  as such instead of failing every run; or
* it is not in [`BUDGET_ONLY`], and it exceeds the baseline by more than [`TOLERANCE`] **and** the
  delta reaches the metric's absolute floor (the ratio gate: the size and count metrics are stable
  across machines, so a ratio means something there, and the floor keeps it on costs a user can
  feel).

The metrics in [`BUDGET_ONLY`] are the timing ones: a shared runner's host-to-host spread is the
size of the allowance itself (`--offline` 48.6 → 57.3 ms between hosts on the same commit), so a
ratio there measures the host, not the change. Their ratio is still printed as a warning, and the
budget — the promise — is what fails them.

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

# The metrics a ratio may not fail: the timing ones. Their host dependence on a shared runner is
# the size of the allowance itself — the same commit measured 48.6 ms and 57.3 ms `--offline` on two
# `ubuntu-26.04` hosts minutes apart (±18 %) and 2.03 → 2.7 ms `--version` (±30 %) — so the ratio
# would report the host, not the change. They are judged by the budget they promise; the ratio is
# printed as a warning so a drift stays visible to the reviewer.
BUDGET_ONLY = {"version_ms", "help_ms", "offline_plain_ms", "warm_plain_ms"}

# The absolute delta a ratio-gated metric must also reach before it counts as a regression, in the
# metric's own unit: the size and count metrics are stable enough for a ratio, and these floors keep
# the test on costs a user can feel. A ratio-gated metric without its own row falls back to the
# unit's floor (`ms` → [`DEFAULT_MS_FLOOR`], everything else ratio-only).
FLOORS = {
    "version_rss_kib": 1024.0,
    "plain_rss_kib": 1024.0,
    "binary_bytes": 512.0 * 1024.0,
    "help_lines": 5.0,
}

# The floor a timing metric without a row of its own falls back to, in milliseconds.
DEFAULT_MS_FLOOR = 5.0

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
            delta_value = fresh - base
            floor = FLOORS.get(
                name, DEFAULT_MS_FLOOR if UNITS.get(name) == "ms" else 0.0
            )
            delta = f"{(ratio - 1) * 100:+.1f}%"
            over_budget = budget is not None and fresh > budget
            if over_budget and base <= budget:
                verdict = "FAIL over budget"
                failed = True
            elif name in BUDGET_ONLY:
                # The host dependence of these metrics is the size of the ratio allowance itself
                # (see `BUDGET_ONLY`), so only the promised budget fails them; a ratio over the
                # allowance is still printed, for the reviewer.
                verdict = (
                    f"warn +{(ratio - 1) * 100:.0f}% vs baseline (budget-only metric)"
                    if ratio > TOLERANCE
                    else "warn: budget re-derivation due"
                    if over_budget
                    else "ok"
                )
            elif ratio > TOLERANCE and delta_value >= floor:
                verdict = f"FAIL +{(ratio - 1) * 100:.0f}% vs baseline"
                failed = True
            elif ratio > TOLERANCE:
                verdict = f"ok (under the {floor:,.0f} {unit} floor)".strip()
            elif over_budget:
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
