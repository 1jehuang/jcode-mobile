#!/usr/bin/env python3
"""Offline artifact gate: debug share probe/export/Intent must be absent in release.

This is a narrow static gate, not signing, release runtime or platform parity.
Never invokes adb or launches application code.
"""
import argparse
import hashlib
import json
from pathlib import Path
import zipfile

NATIVE_FORBIDDEN = (b"nativeConfigureDebugShareProbe", b"incoming-share-state-v1",
                    b"incoming-share-state.json", b"android-offline-share-fixture-not-a-key")
DEX_FORBIDDEN = (b"gpui_share_probe",)


def inspect(apk):
    inspected = []
    with zipfile.ZipFile(apk) as archive:
        native = [n for n in archive.namelist() if n.startswith("lib/") and n.endswith("/libjcode_mobile.so")]
        dex = [n for n in archive.namelist() if n.startswith("classes") and n.endswith(".dex") and "/" not in n]
        if not native or not dex:
            raise AssertionError("APK lacks real native/DEX members; no absence proof possible")
        for member in native + dex:
            data = archive.read(member)
            forbidden = NATIVE_FORBIDDEN if member in native else DEX_FORBIDDEN
            for marker in forbidden:
                if marker in data:
                    raise AssertionError("Debug share probe leaked into release member " + member + ": " + marker.decode())
            inspected.append({"member": member, "sha256": hashlib.sha256(data).hexdigest()})
    return {"passed": True, "apk_sha256": hashlib.sha256(Path(apk).read_bytes()).hexdigest(),
            "members": inspected, "scope": "Static debug share probe absence only",
            "release_accepted": False, "runtime_tested": False}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--apk", required=True, type=Path)
    parser.add_argument("--report", type=Path)
    args = parser.parse_args()
    try:
        result = inspect(args.apk)
    except Exception as error:
        result = {"passed": False, "error": str(error), "release_accepted": False, "runtime_tested": False}
    if args.report:
        args.report.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result, indent=2))
    if not result["passed"]:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
