#!/usr/bin/env python3
"""Synthetic harness tests. No Android commands or acceptance claim."""
import importlib.util
import json
import os
from pathlib import Path
import shlex
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch
import zipfile

PATH = Path(__file__).with_name("android-incoming-share-acceptance.py")
spec = importlib.util.spec_from_file_location("share_acceptance", PATH)
harness = importlib.util.module_from_spec(spec)
spec.loader.exec_module(harness)


class HarnessTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="share-acceptance-test-", dir=os.environ.get("JCODE_SCRATCH_DIR"))
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.apk = self.root / "test.apk"
        self.zip_apk(b"unprobed production fixture")
        self.args = SimpleNamespace(run="emulator-synthetic", out=self.root / "evidence", apk=self.apk, observe_seconds=2)

    def zip_apk(self, content, name="lib/arm64-v8a/libjcode_mobile.so"):
        with zipfile.ZipFile(self.apk, "w") as archive:
            archive.writestr(name, content)

    def test_missing_probe_refuses_all_device_operations(self):
        runner = harness.Acceptance(self.args)
        with patch.object(runner, "command", side_effect=AssertionError("device operation forbidden")) as command:
            with self.assertRaises(SystemExit):
                runner.run()
            command.assert_not_called()
        result = json.loads((self.args.out / "result.json").read_text())
        self.assertFalse(result["passed"])
        self.assertFalse(result["release_accepted"])
        self.assertFalse(result["full_parity"])
        self.assertIn("lacks offline", result["error"])
        self.assertEqual(result["observations"], 0)

    def test_probe_requires_every_native_abi(self):
        with zipfile.ZipFile(self.apk, "w") as archive:
            archive.writestr("lib/arm64-v8a/libjcode_mobile.so", harness.PROBE_MARKER)
            archive.writestr("lib/x86_64/libjcode_mobile.so", b"wrong binary")
        with self.assertRaises(AssertionError):
            harness.require_probe(self.apk)
        self.zip_apk(harness.PROBE_MARKER)
        harness.require_probe(self.apk)
        self.zip_apk(harness.PROBE_MARKER, name="assets/irrelevant.txt")
        with self.assertRaises(AssertionError):
            harness.require_probe(self.apk)

    def test_shell_preserves_share_text_without_option_injection(self):
        runner = harness.Acceptance(self.args)
        payload = "spaces & --es evil yes # ' ✓\nsecond line"
        calls = []
        runner.command = lambda *args, **kwargs: calls.append(args)
        runner.shell("am", "start", "--es", "android.intent.extra.TEXT", payload)
        self.assertEqual(calls[0][0], "shell")
        self.assertEqual(shlex.split(calls[0][1]), ["am", "start", "--es", "android.intent.extra.TEXT", payload])

    def test_resume_requires_current_exact_activity(self):
        valid = " topResumedActivity=ActivityRecord{abc org.jcode.mobile/.JcodeActivity t2}"
        self.assertTrue(harness.shared_resumed(valid))
        for value in ("", valid.replace(".JcodeActivity", ".JcodeActivityOther"),
                      valid.replace("topResumedActivity", "historicalActivity"),
                      valid.replace("org.jcode.mobile/", "org.jcode.mobile.other/"),
                      valid + "\n topResumedActivity=null"):
            self.assertFalse(harness.shared_resumed(value))

    def test_wake_cleanup_runs_after_interrupted_sleep(self):
        runner = harness.Acceptance(self.args)
        calls = []
        runner.shell = lambda *args, **kwargs: calls.append(args)
        def interrupted():
            runner.sleep_requested = True
            raise KeyboardInterrupt()
        runner.exercise = interrupted
        with self.assertRaises(SystemExit):
            runner.run()
        self.assertEqual(calls, [("input", "keyevent", "KEYCODE_WAKEUP"), ("wm", "dismiss-keyguard")])
        self.assertFalse(json.loads((self.args.out / "result.json").read_text())["passed"])


if __name__ == "__main__":
    unittest.main()
