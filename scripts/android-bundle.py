#!/usr/bin/env python3
"""Rearrange aapt2's protobuf APK as a bundletool base module."""
from pathlib import PurePosixPath
import sys
import zipfile


def build_module(source, destination):
    with zipfile.ZipFile(source) as src, zipfile.ZipFile(destination, "w", compression=zipfile.ZIP_DEFLATED) as out:
        names = set(src.namelist())
        if "AndroidManifest.xml" not in names:
            raise ValueError("Missing protobuf Android manifest")
        if not any(name.startswith("lib/") and name.endswith("/libjcode_mobile.so") for name in names):
            raise ValueError("Missing shared GPUI app native library")
        for name in sorted(names):
            path = PurePosixPath(name)
            if path.is_absolute() or ".." in path.parts:
                raise ValueError(f"Unsafe archive path: {name}")
            if name.endswith("/") or name.startswith("META-INF/"):
                continue
            if name == "AndroidManifest.xml":
                target = "manifest/AndroidManifest.xml"
            elif name.endswith(".dex") and "/" not in name:
                target = "dex/" + name
            elif name == "resources.pb" or name.startswith(("res/", "lib/", "assets/")):
                target = name
            else:
                target = "root/" + name
            out.writestr(target, src.read(name))


if __name__ == "__main__":
    if len(sys.argv) != 3:
        raise SystemExit("usage: android-bundle.py PROTO_APK BASE_ZIP")
    build_module(sys.argv[1], sys.argv[2])
