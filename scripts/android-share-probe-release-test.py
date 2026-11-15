#!/usr/bin/env python3
"""Pure artifact fixtures and actual SDK Java debug/release compilation. No adb."""
import importlib.util
import os
from pathlib import Path
import re
import subprocess
import tempfile
import unittest
import zipfile

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("probe_release", Path(__file__).with_name("android-share-probe-release-check.py"))
gate = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gate)


class ArtifactGate(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="share-release-gate-", dir=os.environ.get("JCODE_SCRATCH_DIR"))
        self.addCleanup(self.tmp.cleanup)
        self.apk = Path(self.tmp.name) / "fixture.apk"

    def fixture(self, native=b"synthetic native fixture", dex=b"synthetic dex fixture", extra=None):
        with zipfile.ZipFile(self.apk, "w") as archive:
            archive.writestr("lib/arm64-v8a/libjcode_mobile.so", native)
            archive.writestr("classes.dex", dex)
            if extra:
                archive.writestr(*extra)

    def test_every_debug_marker_rejected_and_abis_all_checked(self):
        self.fixture()
        self.assertTrue(gate.inspect(self.apk)["passed"])
        self.assertFalse(gate.inspect(self.apk)["runtime_tested"])
        for marker in gate.NATIVE_FORBIDDEN:
            self.fixture(extra=("lib/x86_64/libjcode_mobile.so", b"prefix" + marker))
            with self.subTest(marker=marker), self.assertRaises(AssertionError):
                gate.inspect(self.apk)
        for marker in gate.DEX_FORBIDDEN:
            self.fixture(extra=("classes2.dex", b"prefix" + marker))
            with self.subTest(marker=marker), self.assertRaises(AssertionError):
                gate.inspect(self.apk)
        with zipfile.ZipFile(self.apk, "w") as archive:
            archive.writestr("assets/empty", "")
        with self.assertRaises(AssertionError):
            gate.inspect(self.apk)

    def test_actual_sdk_debug_branch_and_release_absence(self):
        sdk = Path(os.environ.get("ANDROID_HOME", os.environ.get("ANDROID_SDK_ROOT", Path.home() / "Android/Sdk")))
        java = Path(os.environ.get("JAVA_HOME", Path.home() / "Android/jdk-21")) / "bin"
        platform = sdk / "platforms/android-36/android.jar"
        sources = sorted((ROOT / "android/app/src/main/java").rglob("*.java"))
        for enabled in (False, True):
            out = Path(self.tmp.name) / str(enabled)
            out.mkdir()
            fixture = out / "BuildConfig.java"
            fixture.write_text("package org.jcode.mobile; public final class BuildConfig { public static final boolean DEBUG = " + str(enabled).lower() + "; }")
            subprocess.run([str(java / "javac"), "--release", "8", "-Xlint:-options", "-classpath", str(platform), "-d", str(out), str(fixture), *map(str, sources)], check=True)
            code = subprocess.check_output([str(java / "javap"), "-c", "-p", "-classpath", str(out), "org.jcode.mobile.JcodeActivity"], text=True)
            invoked = bool(re.search(r"invokestatic[^\n]*nativeConfigureDebugShareProbe", code))
            self.assertEqual(invoked, enabled)
            self.assertEqual("gpui_share_probe" in code, enabled)
            self.assertEqual(bool(re.search(r"invokestatic[^\n]*nativeConfigureDebugDemo", code)), enabled)
            # javac constant-folding must remove the debug Intent key from the
            # actual class constant pool, not merely from rendered source text.
            bytecode = (out / "org/jcode/mobile/JcodeActivity.class").read_bytes()
            self.assertEqual(b"gpui_share_probe" in bytecode, enabled)


if __name__ == "__main__":
    unittest.main()
