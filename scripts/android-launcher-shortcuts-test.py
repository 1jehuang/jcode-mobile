#!/usr/bin/env python3
"""Pure launcher contracts and actual Java quota arithmetic, no device/network.

The source assertions guard privacy/acknowledgement ordering. They do not claim
ShortcutManager runtime acceptance, launcher visibility, or assistant support.
"""
from pathlib import Path
import os
import re
import subprocess
import tempfile
import unittest
import xml.etree.ElementTree as ET

ROOT = Path(__file__).resolve().parents[1]
MAIN = ROOT / "android/app/src/main"
A = "{http://schemas.android.com/apk/res/android}"


class LauncherContracts(unittest.TestCase):
    def test_only_main_launcher_activity_declares_shortcuts(self):
        root = ET.parse(MAIN / "AndroidManifest.xml").getroot()
        declarations = root.findall(f".//meta-data[@{A}name='android.app.shortcuts']")
        self.assertEqual(len(declarations), 1)
        self.assertEqual(declarations[0].get(A + "resource"), "@xml/shortcuts")
        activity = root.find("application/activity")
        self.assertIn(declarations[0], list(activity))
        self.assertEqual(activity.get(A + "name"), ".JcodeActivity")
        self.assertTrue(any(
            f.find(f"action[@{A}name='android.intent.action.MAIN']") is not None
            and f.find(f"category[@{A}name='android.intent.category.LAUNCHER']") is not None
            for f in activity.findall("intent-filter")))

    def test_static_entries_match_ios_donations_without_text_or_extras(self):
        shortcuts = ET.parse(MAIN / "res/xml/shortcuts.xml").getroot()
        labels = {s.get("name"): s.text for s in
                  ET.parse(MAIN / "res/values/shortcuts.xml").getroot()}
        swift = (ROOT / "ios-ext/Sources/JcodeBridge/Bridge.swift").read_text()
        donations = re.findall(r'\("sh\.jcode\.mobile\.([^" ]+)", "([^"]+)", "(jcode://[^"]+)"\)', swift)
        self.assertEqual(len(donations), 2)
        expected = {identifier: (label, url) for identifier, label, url in donations}
        self.assertEqual({s.get(A + "shortcutId") for s in shortcuts}, set(expected))
        for shortcut in shortcuts:
            label, url = expected[shortcut.get(A + "shortcutId")]
            self.assertEqual(shortcut.get(A + "enabled"), "true")
            self.assertEqual(labels[shortcut.get(A + "shortcutShortLabel").removeprefix("@string/")], label)
            self.assertEqual(len(shortcut), 1, "no extra text, categories, or assistant capabilities")
            intent = shortcut[0]
            self.assertEqual(intent.tag, "intent")
            self.assertEqual(len(intent), 0, "no supplied text or auto-submit extras")
            self.assertEqual(intent.attrib, {
                A + "action": "android.intent.action.VIEW", A + "data": url,
                A + "targetPackage": "org.jcode.mobile",
                A + "targetClass": "org.jcode.mobile.JcodeActivity"})

    def test_session_privacy_and_success_acknowledgement_are_preserved(self):
        source = (MAIN / "java/org/jcode/mobile/AndroidServices.java").read_text()
        method = source.split("public boolean indexSessions(String json) {", 1)[1].split("/** Only private cache", 1)[0]
        clear = "if (rows.length() == 0) { manager.removeAllDynamicShortcuts(); return true; }"
        throttle = "if (manager.isRateLimitingActive()) return false;"
        self.assertIn(clear, method)
        self.assertLess(method.index(clear), method.index(throttle))
        self.assertIn('"session-" + id', method)
        self.assertIn('Uri.encode(id)', method)
        self.assertIn('if (id.isEmpty() || title.trim().isEmpty()) continue;', method)
        self.assertIn("return manager.setDynamicShortcuts(shortcuts);", method)
        self.assertNotIn("removeLongLivedShortcuts", method)
        self.assertNotIn("addDynamicShortcuts", method)

    def test_actual_java_quota_block_reserves_static_entries_and_floors_at_zero(self):
        source = (MAIN / "java/org/jcode/mobile/AndroidServices.java").read_text()
        block = re.search(r"final int staticShortcutCount = \d+;\s*int capacity = [^;]+;\s*int count = [^;]+;", source)
        self.assertIsNotNone(block)
        static_count = len(ET.parse(MAIN / "res/xml/shortcuts.xml").getroot())
        self.assertIn(f"staticShortcutCount = {static_count};", block[0])
        java = '''public class QuotaContract {
          static class Manager { int max; int getMaxShortcutCountPerActivity() { return max; } }
          static class Rows { int size; int length() { return size; } }
          public static void main(String[] args) {
            Manager manager = new Manager(); Rows rows = new Rows();
            for (int max = 0; max <= 12; max++) {
              for (int size = 0; size <= 100; size++) {
                manager.max = max; rows.size = size;
                BLOCK
                if (count != Math.min(size, Math.max(0, max - STATIC)))
                  throw new AssertionError("quota " + max + "/" + size);
              }
            }
          }
        }'''.replace("BLOCK", block[0]).replace("STATIC", str(static_count))
        home = Path(os.environ.get("JAVA_HOME", str(Path.home() / "Android/jdk-21")))
        javac = str(home / "bin/javac") if (home / "bin/javac").exists() else "javac"
        java_bin = str(home / "bin/java") if (home / "bin/java").exists() else "java"
        scratch = Path(os.environ.get("JCODE_SCRATCH_DIR", str(ROOT / "target")))
        scratch.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(prefix="launcher-quota-", dir=scratch) as temp:
            path = Path(temp)
            (path / "QuotaContract.java").write_text(java)
            subprocess.run([javac, "--release", "8", str(path / "QuotaContract.java")], check=True)
            subprocess.run([java_bin, "-cp", temp, "QuotaContract"], check=True)


if __name__ == "__main__":
    unittest.main()
