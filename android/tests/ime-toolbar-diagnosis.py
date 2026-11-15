#!/usr/bin/env python3
"""Read-only IME geometry diagnosis, deliberately NOT an acceptance gate.

No device access, mutation, OCR interpretation, or input-content output.
"""
import argparse
import json
from pathlib import Path
import re
import unittest


def diagnose(windows, input_method):
    blocks = re.split(r"(?m)^\s*Window #\d+ ", windows)
    blocks = [b for b in blocks if re.match(r"Window\{[^\n]*\bu0 InputMethod\}", b)]
    result = {"classification": "UNDETERMINED", "docked_content_height": None}
    if len(blocks) != 1:
        return result
    block = blocks[0]
    frame = re.search(r"\bframe=\[(-?\d+),(-?\d+)\]\[(-?\d+),(-?\d+)\]", block)
    inset = re.search(r"mGivenContentInsets=\[-?\d+,(-?\d+)\]", block)
    service = input_method.split("Input method service state for ", 1)
    if len(service) != 2 or not frame or not inset:
        return result
    # Limit state fields to the IME service rather than historical client entries.
    service_state = service[1].split("Dump Time", 1)[0]
    computed = re.search(r"contentTopInsets=(\d+)\b", service_state)
    top, bottom = int(frame[2]), int(frame[4])
    given_top = int(inset[1])
    if not computed or int(computed[1]) != given_top or not 0 <= given_top <= bottom - top:
        return result
    height = bottom - top - given_top
    shown = "mIsInputViewShown=true" in service_state
    accessory = "close_accessory_floating_keyboard" in input_method and "widget_keyboard" in input_method
    result.update(docked_content_height=height, input_view_shown=shown,
                  accessory_controls_present=accessory)
    if shown and height == 0 and accessory:
        result["classification"] = "IME_NON_DOCKED_ACCESSORY_TOOLBAR"
    elif shown and height > 0:
        # Geometry only, not proof of visible keys, occlusion, or successful input.
        result["classification"] = "DOCKED_IME_CONTENT_GEOMETRY"
    return result


class DiagnosisTests(unittest.TestCase):
    def fixture(self, inset=1857, markers=True):
        windows = ("  Window #7 Window{abc u0 InputMethod}:\n"
                   f"mGivenContentInsets=[0,{inset}][0,0]\n"
                   "Frames: parent=[0,63][1080,1920] frame=[0,63][1080,1920]\n")
        service = ("Input method service state for LatinIME:\n"
                   "mIsInputViewShown=true\n"
                   f"contentTopInsets={inset} visibleTopInsets={inset}\nDump Time\n")
        if markers:
            service += "close_accessory_floating_keyboard widget_keyboard"
        return windows, service

    def test_toolbar_is_not_docked_keyboard(self):
        result = diagnose(*self.fixture())
        self.assertEqual(result["classification"], "IME_NON_DOCKED_ACCESSORY_TOOLBAR")
        self.assertEqual(result["docked_content_height"], 0)

    def test_positive_geometry_not_acceptance(self):
        self.assertEqual(diagnose(*self.fixture(985))["classification"], "DOCKED_IME_CONTENT_GEOMETRY")

    def test_no_markers_no_toolbar_claim(self):
        self.assertEqual(diagnose(*self.fixture(markers=False))["classification"], "UNDETERMINED")

    def test_missing_or_ambiguous_evidence(self):
        windows, service = self.fixture()
        self.assertEqual(diagnose("", service)["classification"], "UNDETERMINED")
        self.assertEqual(diagnose(windows + windows, service)["classification"], "UNDETERMINED")
        self.assertEqual(diagnose(windows, "")["classification"], "UNDETERMINED")
        self.assertEqual(diagnose(windows, service.replace("contentTopInsets=1857", "contentTopInsets=100"))["classification"], "UNDETERMINED")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--windows", type=Path)
    parser.add_argument("--input-method", type=Path)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        unittest.main(argv=[__file__])
    elif args.windows and args.input_method:
        print(json.dumps(diagnose(args.windows.read_text(), args.input_method.read_text()), sort_keys=True))
    else:
        parser.error("provide both evidence files or --self-test")
