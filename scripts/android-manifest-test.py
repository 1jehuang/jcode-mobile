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

    def test_font_scale_is_handled_without_dropping_existing_configuration_flags(self):
        source = Path(__file__).resolve().parents[1] / "android/app/src/main/AndroidManifest.xml"
        attr = "{" + manifest.ANDROID + "}"
        expected = {"keyboard", "keyboardHidden", "orientation", "screenSize",
                    "screenLayout", "smallestScreenSize", "uiMode", "density", "fontScale"}
        # Missing fontScale caused a real wm_relaunch_resume_activity at 1.5.
        # This contract must survive both API35 and API36 staging. Device-level
        # no-recreation and actual glyph growth remain separate acceptance gates.
        variants = [source.read_bytes()]
        for api in (35, 36):
            out = io.BytesIO()
            manifest.stage_manifest(source, out, api)
            variants.append(out.getvalue())
        for xml in variants:
            activity = ET.fromstring(xml).find("application/activity")
            self.assertEqual(activity.get(attr + "name"), ".JcodeActivity")
            flags = activity.get(attr + "configChanges").split("|")
            self.assertEqual(set(flags), expected)
            self.assertEqual(len(flags), len(expected))

    def test_old_target_rejected(self):
        with self.assertRaises(ValueError):
            manifest.stage_manifest(io.BytesIO(b"<manifest/>"), io.BytesIO(), 25)

    def test_missing_sdk_rejected(self):
        with self.assertRaisesRegex(ValueError, "uses-sdk"):
            manifest.stage_manifest(io.BytesIO(b"<manifest/>"), io.BytesIO(), 36)


if __name__ == "__main__":
    unittest.main()
