#!/usr/bin/env python3
"""Dependency-free tests for the AAB module packager."""
import importlib.util
import io
from pathlib import Path
import unittest
import zipfile

spec = importlib.util.spec_from_file_location("android_bundle", Path(__file__).with_name("android-bundle.py"))
bundle = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bundle)


class BundleLayout(unittest.TestCase):
    def module(self, extra=()):
        source, output = io.BytesIO(), io.BytesIO()
        with zipfile.ZipFile(source, "w") as archive:
            for name in ("AndroidManifest.xml", "resources.pb", "classes.dex", "lib/arm64-v8a/libjcode_mobile.so", *extra):
                archive.writestr(name, b"fixture")
        source.seek(0)
        bundle.build_module(source, output)
        output.seek(0)
        return zipfile.ZipFile(output)

    def test_module_layout(self):
        with self.module(("META-INF/CERT.SF", "assets/font.ttf")) as archive:
            self.assertEqual(set(archive.namelist()), {
                "manifest/AndroidManifest.xml", "resources.pb", "dex/classes.dex",
                "lib/arm64-v8a/libjcode_mobile.so", "assets/font.ttf"})

    def test_unsafe_paths_rejected(self):
        for name in ("../escape", "/absolute"):
            with self.subTest(name=name), self.assertRaises(ValueError):
                self.module((name,))

    def test_native_app_required(self):
        source = io.BytesIO()
        with zipfile.ZipFile(source, "w") as archive:
            archive.writestr("AndroidManifest.xml", b"fixture")
        source.seek(0)
        with self.assertRaisesRegex(ValueError, "shared GPUI"):
            bundle.build_module(source, io.BytesIO())


if __name__ == "__main__":
    unittest.main()
