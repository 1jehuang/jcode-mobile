#!/usr/bin/env python3
"""Fail-closed parser for saved Android instrumentation output, without adb."""
import argparse
from pathlib import Path
import re


def passed(text):
    results = re.findall(r"^INSTRUMENTATION_RESULT: result=(.*)$", text, re.MULTILINE)
    codes = re.findall(r"^INSTRUMENTATION_CODE: (.*)$", text, re.MULTILINE)
    failed = re.search(r"^INSTRUMENTATION_(?:RESULT|STATUS): (?:failure|Error|shortMsg)=", text, re.MULTILINE)
    return [value.strip() for value in results] == ["PASS"] and [value.strip() for value in codes] == ["-1"] and not failed


def cleanup_args(text):
    results = re.findall(r"^INSTRUMENTATION_RESULT: result=(.*)$", text, re.MULTILINE)
    codes = re.findall(r"^INSTRUMENTATION_CODE: (.*)$", text, re.MULTILINE)
    states = re.findall(r"^INSTRUMENTATION_RESULT: cleanup_expected_state=(.*)$", text, re.MULTILINE)
    hashes = re.findall(r"^INSTRUMENTATION_RESULT: cleanup_expected_preferences=(.*)$", text, re.MULTILINE)
    states = [value.strip() for value in states]
    hashes = [value.strip() for value in hashes]
    failed = re.search(r"^INSTRUMENTATION_(?:RESULT|STATUS): (?:failure|Error|shortMsg)=", text, re.MULTILINE)
    if [v.strip() for v in results] == ["NEEDS_CLEANUP_PROBE"] and [v.strip() for v in codes] == ["-1"] and len(states) == len(hashes) == 1 and states[0] in ("true", "false") and re.fullmatch(r"[0-9a-f]{64}", hashes[0]) and not failed:
        return states[0] + " " + hashes[0]
    return None


def self_test():
    good = "INSTRUMENTATION_RESULT: result=PASS\nINSTRUMENTATION_CODE: -1\n"
    pending = good.replace("result=PASS", "result=NEEDS_CLEANUP_PROBE") + "INSTRUMENTATION_RESULT: cleanup_expected_state=false\nINSTRUMENTATION_RESULT: cleanup_expected_preferences=" + "a" * 64 + "\n"
    assert not passed(pending)
    assert cleanup_args(pending) == "false " + "a" * 64
    assert cleanup_args(pending + good) is None
    assert cleanup_args(pending.replace("a" * 64, "bad")) is None
    assert cleanup_args(pending + "INSTRUMENTATION_RESULT: failure=bad\n") is None
    assert cleanup_args(good) is None
    assert cleanup_args(pending + "INSTRUMENTATION_RESULT: cleanup_expected_state=bad\n") is None
    assert cleanup_args(pending + "INSTRUMENTATION_RESULT: cleanup_expected_preferences=bad\n") is None
    assert cleanup_args(pending.replace("\n", "\r\n")) == "false " + "a" * 64
    assert passed(good)
    assert passed(good.replace("\n", "\r\n"))
    assert not passed("")
    assert not passed("PASS: summary only\n")
    assert not passed(good + "INSTRUMENTATION_RESULT: result=FAIL\nINSTRUMENTATION_CODE: 0\n")
    assert not passed(good.replace("-1", "0"))
    assert not passed(good + "INSTRUMENTATION_RESULT: failure=BadParcelableException\n")
    assert not passed(good + good)
    assert not passed(good.replace("result=PASS", "result=PASS-ish"))
    assert not passed(good.replace("INSTRUMENTATION_CODE: -1", ""))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("log", type=Path, nargs="?")
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--cleanup-args", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        self_test()
        print("PASS: instrumentation parser rejects mixed, missing and failed terminal results")
    if args.log is not None:
        text = args.log.read_text(errors="replace")
        if args.cleanup_args:
            value = cleanup_args(text)
            if value is None:
                parser.exit(1, "FAIL: invalid pending cleanup result\n")
            print(value)
        elif not passed(text):
            parser.exit(1, "FAIL: instrumentation log lacks a unique successful terminal result\n")
    elif not args.self_test:
        parser.error("provide a saved log or --self-test")


if __name__ == "__main__":
    main()
