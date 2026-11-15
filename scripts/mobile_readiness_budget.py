#!/usr/bin/env python3
"""Offline acceptance gate for real mobile connection milestones, not cached paint.

Reads only fixed-label durations. Never prints account, session or run IDs.
A pass applies to the supplied samples, not all devices or future connections.
"""
import argparse
import json
import math
from pathlib import Path
import sys

from mobile_latency_report import samples

# These are independent cumulative milestones from the connection attempt.
# Do not add them together. UI/cache markers deliberately cannot satisfy them.
REQUIRED = ("session_ready", "session_history")
FAILED_ATTEMPT = "session_connect_failed"
MAX_INPUT = 16 * 1024 * 1024


def evaluate(text, budget_ms=1000.0, minimum=1, log=False):
    if not math.isfinite(budget_ms) or budget_ms <= 0 or minimum < 1:
        raise ValueError("budget and minimum must be positive")
    groups = {name: [] for name in (*REQUIRED, FAILED_ATTEMPT)}
    for name, duration in samples(text, log):
        if name not in groups:
            continue
        if (type(duration) not in (int, float) or not math.isfinite(duration)
                or duration < 0 or duration > 604800000):
            raise ValueError("invalid required milestone duration")
        groups[name].append(duration)
    failed_attempts = len(groups.pop(FAILED_ATTEMPT))
    milestones = []
    for name, values in groups.items():
        violations = sum(value >= budget_ms for value in values)
        milestones.append({
            "name": name, "samples": len(values),
            "max_ms": max(values) if values else None,
            "at_or_above_budget": violations,
            "passed": len(values) >= minimum and violations == 0,
        })
    return {"budget_ms_exclusive": budget_ms, "minimum_samples_per_milestone": minimum,
            "passed": failed_attempts == 0 and all(row["passed"] for row in milestones),
            "failed_attempt_samples": failed_attempts,
            "milestones": milestones,
            "scope": "supplied connection-attempt milestones only; not cached paint, command execution, or device-wide SLA"}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("input")
    parser.add_argument("--log", action="store_true")
    parser.add_argument("--budget-ms", type=float, default=1000.0)
    parser.add_argument("--minimum-samples", type=int, default=1)
    args = parser.parse_args()
    try:
        path = Path(args.input)
        if path.stat().st_size > MAX_INPUT:
            raise ValueError("input too large")
        # Bound the read too in case the file grows after stat.
        with path.open("rb") as source:
            raw = source.read(MAX_INPUT + 1)
        if len(raw) > MAX_INPUT:
            raise ValueError("input too large")
        result = evaluate(raw.decode("utf-8"), args.budget_ms, args.minimum_samples, args.log)
    except (OSError, UnicodeError, ValueError, TypeError):
        print("Invalid or unreadable bounded timing input", file=sys.stderr)
        return 2
    print(json.dumps(result, indent=2, allow_nan=False))
    return 0 if result["passed"] else 1


if __name__ == "__main__":
    sys.exit(main())
