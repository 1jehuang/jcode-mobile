#!/usr/bin/env python3
"""Stage the native manifest with an explicit target SDK, without editing source."""
import argparse
from pathlib import Path
import xml.etree.ElementTree as ET

ANDROID = "http://schemas.android.com/apk/res/android"
ET.register_namespace("android", ANDROID)


def stage_manifest(source, destination, target_api):
    if target_api < 26:
        raise ValueError("Target SDK must be at least API 26")
    tree = ET.parse(source)
    sdk = tree.getroot().find("uses-sdk")
    if sdk is None:
        raise ValueError("Native manifest must declare uses-sdk")
    sdk.set(f"{{{ANDROID}}}targetSdkVersion", str(target_api))
    tree.write(destination, encoding="utf-8", xml_declaration=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    parser.add_argument("destination", type=Path)
    parser.add_argument("target_api", type=int)
    args = parser.parse_args()
    stage_manifest(args.source, args.destination, args.target_api)
