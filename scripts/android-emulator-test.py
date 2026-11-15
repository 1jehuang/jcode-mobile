#!/usr/bin/env python3
"""Exercise emulator helper validation with mock tools, never a real device."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]


class EmulatorConfiguration(unittest.TestCase):
    def run_helper(self, api="36", pages="16384"):
        with tempfile.TemporaryDirectory(prefix="jcode-android-helper-") as temp:
            temp = Path(temp)
            adb = temp / "platform-tools/adb"
            emulator = temp / "emulator/emulator"
            adb.parent.mkdir()
            emulator.parent.mkdir()
            adb.write_text('''#!/usr/bin/env python3
import os, sys
from pathlib import Path
args = " ".join(sys.argv[1:])
with Path(os.environ["TEST_ADB_LOG"]).open("a") as log: log.write(args + "\\n")
if "sys.boot_completed" in args: print("1")
elif "ro.build.version.sdk" in args: print(os.environ["TEST_RUNTIME_API"])
elif "getconf PAGE_SIZE" in args: print(os.environ["TEST_RUNTIME_PAGES"])
''')
            emulator.write_text("#!/bin/sh\nexit 1\n")
            adb.chmod(0o755)
            emulator.chmod(0o755)
            apk = temp / "app.apk"
            apk.touch()
            log = temp / "adb.log"
            env = {**os.environ, "ANDROID_HOME": str(temp), "ANDROID_SERIAL": "mock-emulator",
                   "ANDROID_APK": str(apk), "ANDROID_EMULATOR_API": "36",
                   "ANDROID_EMULATOR_TAG": "google_apis_ps16k", "ANDROID_EMULATOR_PAGE_SIZE": "",
                   "TEST_ADB_LOG": str(log), "TEST_RUNTIME_API": api, "TEST_RUNTIME_PAGES": pages}
            result = subprocess.run(["bash", str(ROOT / "scripts/android-emulator.sh")],
                                    env=env, capture_output=True, text=True, timeout=10)
            return result, log.read_text()

    def test_api36_ps16k_proceeds_only_after_runtime_checks(self):
        result, log = self.run_helper()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertLess(log.index("getconf PAGE_SIZE"), log.index("install -r"))
        self.assertIn("org.jcode.mobile/.JcodeActivity", log)

    def test_wrong_page_size_does_not_install(self):
        result, log = self.run_helper(pages="4096")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Expected 16384-byte", result.stderr)
        self.assertNotIn("install -r", log)

    def test_wrong_api_does_not_install(self):
        result, log = self.run_helper(api="35")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Expected Android API 36", result.stderr)
        self.assertNotIn("install -r", log)


if __name__ == "__main__":
    unittest.main()
