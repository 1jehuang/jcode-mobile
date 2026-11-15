#!/usr/bin/env python3
"""Pure synthetic tests. Never invoke adb or require a running device."""
import argparse
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("release_runtime", Path(__file__).with_name("android-release-runtime.py"))
m = importlib.util.module_from_spec(spec)
spec.loader.exec_module(m)


class Parsing(unittest.TestCase):
    def test_manifest_missing_booleans_defaults_false(self):
        self.assertTrue(m.manifest_is_non_debug('A: package="org.jcode.mobile"'))

    def test_manifest_explicit_false(self):
        for value in ("false", "0", "0x00000000"):
            self.assertTrue(m.manifest_is_non_debug('package="org.jcode.mobile"\nA: android:debuggable(0x0101000f)=' + value))

    def test_manifest_true_or_unrecognized_rejected(self):
        for name in ("debuggable", "testOnly"):
            for value in ("true", "0xffffffff", "unknown", '"false"'):
                self.assertFalse(m.manifest_is_non_debug(f'package="org.jcode.mobile"\nA: android:{name}(0x1)={value}'))
        self.assertFalse(m.manifest_is_non_debug('package="org.jcode.mobile" android:debuggable=true'))
        self.assertFalse(m.manifest_is_non_debug('package="different.app"'))

    def test_package_flags_fail_closed(self):
        self.assertTrue(m.package_is_non_debug('  flags=[ HAS_CODE ALLOW_CLEAR_USER_DATA ]'))
        self.assertTrue(m.package_is_non_debug('  pkgFlags=[ HAS_CODE ]'))
        for text in ('', 'flags=[]\n flags=[DEBUGGABLE HAS_CODE]', 'flags=[TEST_ONLY]', 'DEBUGGABLE absent'):
            self.assertFalse(m.package_is_non_debug(text))

    def test_ime_geometry_actual_frame(self):
        dump = '\n  Window #1 Window{123 u0 InputMethod}:\n    mHasSurface=true isOnScreen=true\n    mWindowFrames: frame=[0,1200][1080,2400] display=[0,0][1080,2400]\n'
        self.assertEqual(m.ime_top(dump), 1200)
        self.assertEqual(m.ime_top(dump.replace('frame=', 'mFrame=')), 1200)

    def test_fullscreen_ime_uses_content_inset_not_transparent_top(self):
        # API35 real Gboard shape: frame.top=63, contentInset.top=985.
        dump = '\n  Window #7 Window{8ed725b u0 InputMethod}:\n    mGivenContentInsets=[0,985][0,0] mGivenVisibleInsets=[0,985][0,0]\n    Frames: parent=[0,63][1080,1920] display=[0,63][1080,1920] frame=[0,63][1080,1920]\n    isOnScreen=true\n    isVisible=true\n'
        self.assertEqual(m.ime_top(dump), 1048)
        self.assertLess(809, m.ime_top(dump))  # Actual releasedraft OCR bottom.
        self.assertLess(962, m.ime_top(dump))  # Actual Send code OCR bottom.

    def test_impossible_content_inset_fails_closed(self):
        dump = '\n Window #1 Window{123 u0 InputMethod}:\n mGivenContentInsets=[0,2000][0,0]\n frame=[0,63][1080,1920]\n isVisible=true'
        with self.assertRaises(AssertionError):
            m.ime_top(dump)

    def test_hidden_or_other_window_not_ime(self):
        for dump in ('mFrame=[0,0][1080,2400]', '\n Window #1 Window{123 u0 Other}:\n frame=[0,1000][1080,2400]', '\n Window #1 Window{123 u0 InputMethod}:\n isOnScreen=false\n frame=[0,1200][1080,2400]'):
            with self.assertRaises(AssertionError):
                m.ime_top(dump)

    def test_onboarding_no_apple(self):
        self.assertTrue(m.onboarding('Jcode Continue with Google you@example.com'))
        self.assertFalse(m.onboarding('Jcode Google Apple'))
        self.assertFalse(m.onboarding('Core progress todos'))

    def test_draft_requires_actual_ocr_text(self):
        self.assertEqual(len(m.draft_rows([{"text": "releasedraft"}])), 1)
        self.assertEqual(m.draft_rows([{"text": "you@example.com"}]), [])


class Cleanup(unittest.TestCase):
    def make_runtime(self, directory):
        args = argparse.Namespace(serial="emulator-5554", apk=Path("not-read.apk"), out=Path(directory) / "evidence")
        return m.Runtime(args)

    def simulate(self, runtime, failure=None, cleanup_failure=False):
        calls = []
        settings = {"user_rotation": "3", "accelerometer_rotation": "null"}

        def exercise():
            runtime.validated = True
            runtime.installed = True
            runtime.settings = settings.copy()
            runtime.checks.append({"name": "synthetic stage", "passed": True})
            if failure:
                raise failure

        def adb(*args):
            calls.append(args)
            if cleanup_failure and args[:4] == ("shell", "settings", "put", "system"):
                raise OSError("synthetic settings failure")
            return b""

        def text(*args):
            calls.append(args)
            return settings[args[-1]]

        with patch.object(runtime, "exercise", exercise), patch.object(runtime, "adb", adb), patch.object(runtime, "text", text), patch.object(m.subprocess, "run", side_effect=AssertionError("No real subprocess allowed")):
            status = runtime.run()
        result = json.loads((runtime.out / "result.json").read_text())
        return status, result, calls

    def test_success_still_not_full_parity_and_settings_restore(self):
        with tempfile.TemporaryDirectory() as directory:
            status, result, calls = self.simulate(self.make_runtime(directory))
        self.assertEqual(status, 0)
        self.assertFalse(result["full_parity"])
        self.assertFalse(result["release_accepted"])
        self.assertIn(("shell", "settings", "put", "system", "user_rotation", "3"), calls)
        self.assertIn(("shell", "settings", "delete", "system", "accelerometer_rotation"), calls)
        self.assertIn(("shell", "am", "force-stop", m.PACKAGE), calls)

    def test_failure_restores_settings(self):
        with tempfile.TemporaryDirectory() as directory:
            status, result, calls = self.simulate(self.make_runtime(directory), AssertionError("OCR failed"))
        self.assertEqual(status, 1)
        self.assertFalse(result["passed"])
        self.assertTrue(any(c["name"] == "Restore user_rotation" and c["passed"] for c in result["checks"]))

    def test_interrupt_is_failure_and_restores(self):
        with tempfile.TemporaryDirectory() as directory:
            status, result, calls = self.simulate(self.make_runtime(directory), KeyboardInterrupt())
        self.assertEqual(status, 1)
        self.assertIn("KeyboardInterrupt", result["error"])
        self.assertTrue(any(c["name"] == "Restore user_rotation" for c in result["checks"]))

    def test_cleanup_failure_cannot_pass(self):
        with tempfile.TemporaryDirectory() as directory:
            status, result, calls = self.simulate(self.make_runtime(directory), cleanup_failure=True)
        self.assertEqual(status, 1)
        self.assertFalse(result["passed"])
        self.assertTrue(any(c["name"] == "Restore user_rotation" and not c["passed"] for c in result["checks"]))
        self.assertIn(("shell", "settings", "delete", "system", "accelerometer_rotation"), calls)

    def test_no_device_cleanup_on_preflight_failure(self):
        with tempfile.TemporaryDirectory() as directory:
            runtime = self.make_runtime(directory)
            with patch.object(runtime, "exercise", side_effect=AssertionError("preflight")), patch.object(runtime, "adb", side_effect=AssertionError("must not call adb")):
                self.assertEqual(runtime.run(), 1)
            result = json.loads((runtime.out / "result.json").read_text())
            self.assertFalse(result["side_effects"]["apk_install_completed"])

    def test_stale_evidence_directory_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            self.make_runtime(directory)
            with self.assertRaises(FileExistsError):
                self.make_runtime(directory)


if __name__ == "__main__":
    unittest.main()
