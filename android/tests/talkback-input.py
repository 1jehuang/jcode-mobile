#!/usr/bin/env python3
"""Pure TalkBack emulator input calibration, readiness and raw-event checks. No adb."""
import argparse
from pathlib import Path
import re
import sys
import unittest


class NotReady(ValueError):
    pass


def readiness(text, nonce):
    if not re.fullmatch(r"[0-9a-f]{32}", nonce):
        raise ValueError("invalid nonce")
    records = re.findall(r"^INSTRUMENTATION_STATUS: talkback_ready=(.*)\r?$", text, re.MULTILINE)
    if not records:
        raise NotReady("no readiness record")
    if len(records) != 1:
        raise ValueError("duplicate readiness records")
    match = re.fullmatch(re.escape(nonce) + r",([0-9]+),([0-9]+),([0-9]+),([0-9]+),([0-9]+),([0-9]+)", records[0].strip())
    if not match:
        raise ValueError("malformed or stale readiness")
    x1, y1, x2, y2, width, height = map(int, match.groups())
    if not (1 < width <= 20000 and 1 < height <= 20000 and 0 <= x1 < x2 < width and 0 <= y1 == y2 < height):
        raise ValueError("coordinates outside actual display")
    return x1, y1, x2, y2, width, height


def touch_profile(text):
    candidates = []
    for block in re.split(r"(?=^add device )", text, flags=re.MULTILINE):
        path = re.search(r"^add device \d+: (/dev/input/event\d+)\r?$", block, re.MULTILINE)
        name = re.search(r'name:\s+"([^"\r\n]+)"', block)
        if not path or not name:
            continue
        axes = {}
        for axis, low, high in re.findall(r"\b(ABS_[A-Z0-9_]+)\s*:\s*value -?\d+, min (-?\d+), max (-?\d+)", block):
            if axis in axes:
                raise ValueError("duplicate axis")
            axes[axis] = (int(low), int(high))
        needed = {"ABS_MT_POSITION_X", "ABS_MT_POSITION_Y", "ABS_MT_SLOT", "ABS_MT_TRACKING_ID", "ABS_MT_PRESSURE", "ABS_MT_TOUCH_MAJOR", "ABS_MT_TOOL_TYPE"}
        if needed <= axes.keys():
            candidates.append((name.group(1), path.group(1), axes))
    primary = [item for item in candidates if item[0] == "virtio_input_multi_touch_1"]
    selected = primary if primary else candidates
    if len(selected) != 1:
        raise ValueError("no unambiguous primary emulator multitouch device")
    name, path, axes = selected[0]
    for axis in ("ABS_MT_POSITION_X", "ABS_MT_POSITION_Y", "ABS_MT_PRESSURE"):
        low, high = axes[axis]
        if not (0 <= low < high <= 2147483647):
            raise ValueError("invalid axis range")
    if not (axes["ABS_MT_SLOT"][0] <= 0 <= axes["ABS_MT_SLOT"][1]
            and axes["ABS_MT_TRACKING_ID"][0] <= 1 <= 12 <= axes["ABS_MT_TRACKING_ID"][1]
            and axes["ABS_MT_TOOL_TYPE"][0] <= 0 <= axes["ABS_MT_TOOL_TYPE"][1]
            and axes["ABS_MT_TOUCH_MAJOR"][0] <= 1 <= axes["ABS_MT_TOUCH_MAJOR"][1]
            and axes["ABS_MT_PRESSURE"][0] == 0):
        raise ValueError("unsupported slot, tracking, finger, contact or pressure range")
    return path, axes


def scale(pixel, pixels, bounds):
    if not (pixels > 1 and 0 <= pixel < pixels and bounds[0] < bounds[1]):
        raise ValueError("invalid scaling inputs")
    return bounds[0] + pixel * (bounds[1] - bounds[0]) // (pixels - 1)


def calibrate(ready, profile):
    x1, y1, x2, y2, width, height = ready
    path, axes = profile
    pressure = max(1, axes["ABS_MT_PRESSURE"][1] // 2)
    return (path, scale(x1, width, axes["ABS_MT_POSITION_X"]), scale(y1, height, axes["ABS_MT_POSITION_Y"]),
            scale(x2, width, axes["ABS_MT_POSITION_X"]), scale(y2, height, axes["ABS_MT_POSITION_Y"]), pressure)


def verify_events(text, calibrated):
    _, x1, y1, x2, _, pressure = calibrated
    events = re.findall(r"\b(EV_ABS|EV_SYN)\s+(ABS_[A-Z0-9_]+|SYN_REPORT)\s+([0-9a-fA-F]{8})\b", text)
    pairs = {(code, int(value, 16)) for _, code, value in events}
    required = {("ABS_MT_POSITION_X", x1), ("ABS_MT_POSITION_X", x2), ("ABS_MT_POSITION_Y", y1),
                ("ABS_MT_PRESSURE", pressure), ("ABS_MT_PRESSURE", 0), ("ABS_MT_TRACKING_ID", 0xffffffff), ("SYN_REPORT", 0)}
    if not required <= pairs:
        missing = sorted(required - pairs)
        raise ValueError("raw InputReader events missing calibrated contact/move/release: " + repr(missing))


FIXTURE = '''add device 1: /dev/input/event1
  name: "keyboard"
add device 2: /dev/input/event2
  name: "virtio_input_multi_touch_1"
    ABS_MT_POSITION_X : value 0, min 0, max 32767, fuzz 0
    ABS_MT_POSITION_Y : value 0, min 0, max 32767, fuzz 0
    ABS_MT_SLOT : value 0, min 0, max 10, fuzz 0
    ABS_MT_TRACKING_ID : value 0, min 0, max 65535, fuzz 0
    ABS_MT_PRESSURE : value 0, min 0, max 1024, fuzz 0
    ABS_MT_TOUCH_MAJOR : value 0, min 0, max 2147483647, fuzz 0
    ABS_MT_TOOL_TYPE : value 0, min 0, max 15, fuzz 0
'''


class PureTests(unittest.TestCase):
    def test_real_capture_calibration(self):
        result = calibrate((270, 960, 810, 960, 1080, 1920), touch_profile(FIXTURE))
        self.assertEqual(result, ("/dev/input/event2", 8199, 16392, 24598, 16392, 512))
        self.assertEqual(scale(0, 1080, (0, 32767)), 0)
        self.assertEqual(scale(1079, 1080, (0, 32767)), 32767)
        self.assertEqual(scale(2, 5, (10, 110)), 60)

    def test_readiness_strict(self):
        nonce = "a" * 32
        good = "INSTRUMENTATION_STATUS: talkback_ready=" + nonce + ",270,960,810,960,1080,1920\n"
        self.assertEqual(readiness(good, nonce), (270, 960, 810, 960, 1080, 1920))
        self.assertEqual(readiness(good.replace("\n", "\r\n"), nonce), (270, 960, 810, 960, 1080, 1920))
        for bad in (good + good, good.replace(nonce, "b" * 32), good.replace(",810,", ",1080,"), good.replace(",1920", ",0")):
            with self.subTest(bad=bad), self.assertRaises(ValueError):
                readiness(bad, nonce)
        with self.assertRaises(NotReady):
            readiness("", nonce)

    def test_profiles_fail_closed(self):
        for bad in ("", FIXTURE.replace("max 32767", "max 0"), FIXTURE.replace("ABS_MT_PRESSURE", "ABS_OTHER"), FIXTURE + FIXTURE):
            with self.subTest(bad=bad), self.assertRaises(ValueError):
                touch_profile(bad)
        second = FIXTURE.replace("multi_touch_1", "multi_touch_2").replace("/event2", "/event3")
        self.assertEqual(touch_profile(FIXTURE + second)[0], "/dev/input/event2")

    def test_raw_contact_required(self):
        calibration = calibrate((270, 960, 810, 960, 1080, 1920), touch_profile(FIXTURE))
        text = "\n".join("EV_ABS %s %08x" % item for item in [("ABS_MT_POSITION_X", 8199), ("ABS_MT_POSITION_X", 24598), ("ABS_MT_POSITION_Y", 16392), ("ABS_MT_PRESSURE", 512), ("ABS_MT_PRESSURE", 0), ("ABS_MT_TRACKING_ID", 0xffffffff)]) + "\nEV_SYN SYN_REPORT 00000000\n"
        verify_events(text, calibration)
        for bad in ("OK\n", text.replace("00000200", "00000000"), text.replace("00006016", "0000032a"), text.replace("ffffffff", "00000000")):
            with self.subTest(bad=bad), self.assertRaises(ValueError):
                verify_events(bad, calibration)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--ready", type=Path)
    parser.add_argument("--nonce")
    parser.add_argument("--capabilities", type=Path)
    parser.add_argument("--verify-events", type=Path)
    args = parser.parse_args()
    if args.self_test:
        result = unittest.main(argv=[sys.argv[0]], exit=False)
        sys.exit(0 if result.result.wasSuccessful() else 1)
    try:
        ready = readiness(args.ready.read_text(errors="replace"), args.nonce)
        if args.capabilities:
            calibrated = calibrate(ready, touch_profile(args.capabilities.read_text(errors="replace")))
            if args.verify_events:
                verify_events(args.verify_events.read_text(errors="replace"), calibrated)
            print(*calibrated)
        else:
            print("none", *ready[:4], 0)
    except NotReady:
        sys.exit(1)
    except (ValueError, OSError, AttributeError) as error:
        print("FAIL: " + str(error), file=sys.stderr)
        sys.exit(2)


if __name__ == "__main__":
    main()
