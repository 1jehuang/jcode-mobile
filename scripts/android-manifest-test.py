#!/usr/bin/env python3
"""Manifest staging regression tests, with no emulator or SDK mutation."""
import importlib.util
import io
from pathlib import Path
import unittest
import xml.etree.ElementTree as ET

spec = importlib.util.spec_from_file_location("android_manifest", Path(__file__).with_name("android-manifest.py"))
manifest = importlib.util.module_from_spec(spec)
spec.loader.exec_module(manifest)


class ManifestStaging(unittest.TestCase):
    def test_target_override_preserves_native_activity_and_permissions(self):
        source = Path(__file__).resolve().parents[1] / "android/app/src/main/AndroidManifest.xml"
        original = source.read_bytes()
        for api in (35, 36):
            with self.subTest(api=api):
                out = io.BytesIO()
                manifest.stage_manifest(source, out, api)
                root = ET.fromstring(out.getvalue())
                attr = "{" + manifest.ANDROID + "}"
                self.assertEqual(root.find("uses-sdk").get(attr + "targetSdkVersion"), str(api))
                self.assertEqual(root.find("application/activity").get(attr + "name"), ".JcodeActivity")
                metadata = root.find("application/activity/meta-data")
                self.assertEqual(metadata.get(attr + "value"), "jcode_mobile")
                self.assertTrue(root.findall("uses-permission"))
        self.assertEqual(source.read_bytes(), original)

    def test_old_target_rejected(self):
        with self.assertRaises(ValueError):
            manifest.stage_manifest(io.BytesIO(b"<manifest/>"), io.BytesIO(), 25)

    def test_missing_sdk_rejected(self):
        with self.assertRaisesRegex(ValueError, "uses-sdk"):
            manifest.stage_manifest(io.BytesIO(b"<manifest/>"), io.BytesIO(), 36)


if __name__ == "__main__":
    unittest.main()
