#!/usr/bin/env python3
"""Native GPUI launch evidence, not full shared-UI acceptance.

Captures screenshots and logs from an emulator. No Java widget hierarchy or
legacy JNI fixture can establish that the shared GPUI renderer works.
"""
import json
import os
from pathlib import Path
import subprocess
import time

ROOT = Path(__file__).resolve().parents[1]
SDK = Path(os.environ.get("ANDROID_HOME", os.environ.get("ANDROID_SDK_ROOT", Path.home() / "Android/Sdk")))
SERIAL = os.environ.get("ANDROID_SERIAL", "emulator-5554")
ADB = str(SDK / "platform-tools/adb")
OUT = ROOT / "target/android" / ("gpui-smoke-" + SERIAL)


def adb(*args, binary=False):
    result = subprocess.run([ADB, "-s", SERIAL, *args], check=True,
                            capture_output=True, timeout=40)
    return result.stdout if binary else result.stdout.decode(errors="replace").strip()


def main():
    assert adb("shell", "getprop", "ro.kernel.qemu") == "1", "Emulator required"
    OUT.mkdir(parents=True, exist_ok=True)
    adb("shell", "am", "force-stop", "org.jcode.mobile")
    start = adb("shell", "date", "+%m-%d %H:%M:%S.000")
    launch = adb("shell", "am", "start", "-W", "-n", "org.jcode.mobile/.JcodeActivity")
    (OUT / "launch.txt").write_text(launch)
    time.sleep(5)
    pid = adb("shell", "pidof", "org.jcode.mobile").split()[0]
    logs = adb("logcat", "--pid", pid, "-d", "-t", start)
    (OUT / "logcat.txt").write_text(logs)
    (OUT / "launch.png").write_bytes(adb("exec-out", "screencap", "-p", binary=True))
    assert "FATAL EXCEPTION" not in logs and "Fatal signal" not in logs, "Native launch crashed"
    assert "shared GPUI NativeActivity bootstrap" in logs, "Shared app native bootstrap was not reached"
    assert "jcode-mobile: window opened" in logs, "Shared MobileApp window was not opened"
    adb("shell", "input", "keyevent", "KEYCODE_HOME")
    adb("shell", "am", "start", "-W", "-n", "org.jcode.mobile/.JcodeActivity")
    time.sleep(2)
    assert adb("shell", "pidof", "org.jcode.mobile"), "Process died on resume"
    (OUT / "resume.png").write_bytes(adb("exec-out", "screencap", "-p", binary=True))
    (OUT / "result.json").write_text(json.dumps({
        "serial": SERIAL, "pid": pid, "native_launch": True,
        "shared_gpui_acceptance": False,
        "remaining": "Inspect GPUI screenshots and exercise shared composer, input, scrolling and services on device."
    }, indent=2) + "\n")
    print(f"Native launch/resume evidence: {OUT}. Shared GPUI UI acceptance is NOT asserted.")


if __name__ == "__main__":
    main()
