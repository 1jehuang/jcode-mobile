#!/usr/bin/env python3
"""Fail-closed camera instrumentation log verdict, usable without Android tools."""
import argparse
import json
from pathlib import Path
import re
import sys


def passed(text):
    lines = [line.strip() for line in text.splitlines() if line.strip()]
    # Intermediate PASS fields never override a later failure or a canceled run.
    if re.search(r"\b(?:FAIL|FAILED|FAILURE|INSTRUMENTATION_FAILED|INSTRUMENTATION_ABORTED)\b", text, re.I):
        return False
    results = [line for line in lines if line.startswith("INSTRUMENTATION_RESULT: result=")]
    codes = [line for line in lines if line.startswith("INSTRUMENTATION_CODE:")]
    return (results == ["INSTRUMENTATION_RESULT: result=PASS"]
            and codes == ["INSTRUMENTATION_CODE: -1"]
            and lines[-1:] == codes
            and lines.index(results[0]) < lines.index(codes[0]))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("log", nargs="?", type=Path)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        fixtures = json.loads(Path(__file__).with_name("camera-grant-log-fixtures.json").read_text())
        for fixture in fixtures:
            actual = passed(fixture["log"])
            if actual != fixture["pass"]:
                raise AssertionError(f"{fixture['name']}: expected {fixture['pass']}, got {actual}")
        print(f"PASS: {len(fixtures)} device-free camera log parser regression fixtures")
        return 0
    if args.log is None:
        parser.error("provide a log or --self-test")
    try:
        text = args.log.read_text()
    except (OSError, UnicodeError) as error:
        print(f"FAIL: cannot read camera instrumentation log: {error}", file=sys.stderr)
        return 1
    if not passed(text):
        print("FAIL: camera instrumentation needs a single terminal result=PASS and code -1, with no failures", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
