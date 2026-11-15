#!/usr/bin/env python3
"""Pure tests only. Never invoke adb, OCR processes or device operations."""
import importlib.util
import json
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("font_acceptance", Path(__file__).with_name("android-foreground-font-scale-acceptance.py"))
font = importlib.util.module_from_spec(spec)
spec.loader.exec_module(font)


def fixture(scale=1, replacements=None):
    rows = ["level\tleft\ttop\twidth\theight\tconf\ttext"]
    for index, (name, label) in enumerate(font.LABELS.items()):
        x = 40
        for word in label.split():
            values = {"level": 5, "left": x, "top": 100 + index * 130,
                      "width": int(12 * len(word) * scale), "height": int(20 * scale), "conf": 95, "text": word}
            values.update((replacements or {}).get(name, {}))
            rows.append("\t".join(str(values[k]) for k in ("level", "left", "top", "width", "height", "conf", "text")))
            x += int(12 * len(word) * scale) + 10
    return "\n".join(rows) + "\n"


class FontTests(unittest.TestCase):
    def test_raw_row_preserves_exact_encoding_and_missing(self):
        self.assertIsNone(font.font_row("other=1\n"))
        for value in ("1", "1.000", ".75", "1."):
            self.assertEqual(font.font_row("other=3\nfont_scale=" + value), value)

    def test_raw_row_rejects_duplicates_and_unsafe_encodings(self):
        for text in ("font_scale=1\nfont_scale=1", "font_scale=null", "font_scale=NaN", "font_scale=1;id", "font_scale=0", "font_scale=9", "font_scale="):
            with self.subTest(text=text), self.assertRaises(AssertionError):
                font.font_row(text)

    def test_effective_configuration_is_unique(self):
        prefix = "topResumedActivity=ActivityRecord{abc u0 org.jcode.mobile/.JcodeActivity t1}\n * Hist #0: ActivityRecord{abc u0 org.jcode.mobile/.JcodeActivity t1}\n"
        self.assertEqual(font.configuration_scale(prefix + " CurrentConfiguration={1.5 310mcc}"), 1.5)
        self.assertEqual(font.configuration_scale(" CurrentConfiguration={2.0 other}\n" + prefix + " CurrentConfiguration={1.5 310mcc}"), 1.5)
        for text in ("", prefix + " CurrentConfiguration={1 0}\n CurrentConfiguration={1.5 0}", prefix + " CurrentConfiguration={? 0}"):
            with self.assertRaises(AssertionError):
                font.configuration_scale(text)

    def test_actual_matching_label_bounds_and_growth(self):
        small = font.measure(fixture(), 1080, 1920)
        large = font.measure(fixture(1.5), 1080, 1920)
        self.assertEqual(set(small), set(font.LABELS))
        self.assertTrue(all(r == 1.5 for values in font.require_growth(small, large).values() for r in values))

    def test_non_scaling_layout_is_not_font_acceptance(self):
        small = font.measure(fixture(), 1080, 1920)
        with self.assertRaises(AssertionError):
            font.require_growth(small, small)

    def test_missing_low_confidence_clipped_and_overlapping_labels_fail(self):
        cases = ({"email": {"text": "missing"}}, {"google": {"conf": 44}},
                 {"submit": {"left": 0}}, {"submit": {"top": 1900}},
                 {"submit": {"top": 230}})
        for replacements in cases:
            with self.subTest(replacements=replacements), self.assertRaises(AssertionError):
                font.measure(fixture(replacements=replacements), 1080, 1920)

    def test_duplicate_labels_are_not_accepted(self):
        tsv = fixture()
        duplicate = "\n".join(tsv.splitlines()[1:6])
        with self.assertRaises(AssertionError):
            font.measure(tsv + duplicate + "\n", 1080, 1920)

    def test_token_alignment_and_invalid_baseline_fail(self):
        a = font.measure(fixture(), 1080, 1920)
        for heights in ([], [0], [20]):
            b = font.measure(fixture(1.5), 1080, 1920)
            b["heading"]["word_heights"] = heights
            with self.assertRaises(AssertionError):
                font.require_growth(a, b)

    def test_exact_restoration_put_or_delete_and_effective_check(self):
        for value in (None, "1.000"):
            with self.subTest(value=value), tempfile.TemporaryDirectory() as directory:
                harness = object.__new__(font.Acceptance)
                harness.out = Path(directory)
                harness.args = SimpleNamespace(settle_seconds=.5)
                harness.original = {"row": value, "effective": 1.0}
                harness.changed = True
                calls = []
                harness.shell = lambda *args: calls.append(args)
                harness.row = lambda: value
                harness.continuity = lambda name, expected: expected
                harness.capture_font = lambda name, expected: calls.append((name, expected))
                with patch.object(font.time, "sleep"):
                    harness.restore()
                expected = ("settings", "--user", "0", "delete", "system", "font_scale") if value is None else ("settings", "--user", "0", "put", "system", "font_scale", value)
                self.assertEqual(calls[0], expected)
                self.assertEqual(json.loads((harness.out / "restored-settings.json").read_text()), {"row": value, "effective": 1.0})

    def test_restore_mismatch_fails(self):
        with tempfile.TemporaryDirectory() as directory:
            harness = object.__new__(font.Acceptance)
            harness.out = Path(directory)
            harness.args = SimpleNamespace(settle_seconds=.5)
            harness.original = {"row": None, "effective": 1.0}
            harness.changed = True
            harness.shell = lambda *args: None
            harness.row = lambda: "1.0"
            with patch.object(font.time, "sleep"), self.assertRaises(AssertionError):
                harness.restore()

    def test_exception_and_interrupt_always_restore_and_fail_closed(self):
        for failure in (RuntimeError("synthetic failure"), KeyboardInterrupt("synthetic interrupt")):
            with self.subTest(failure=failure), tempfile.TemporaryDirectory() as directory:
                harness = object.__new__(font.Acceptance)
                harness.out = Path(directory)
                harness.args = SimpleNamespace(run="emulator-5554")
                harness.changed = True
                harness.metadata = {}
                harness.observations = []
                restored = []
                def exercise():
                    raise failure
                harness.exercise = exercise
                harness.restore = lambda: restored.append(True)
                with self.assertRaises(SystemExit):
                    harness.run()
                self.assertEqual(restored, [True])
                result = json.loads((harness.out / "result.json").read_text())
                self.assertFalse(result["passed"])
                self.assertFalse(result["font_scale_accepted"])
                self.assertFalse(result["reduce_motion_accepted"])


if __name__ == "__main__":
    unittest.main()
