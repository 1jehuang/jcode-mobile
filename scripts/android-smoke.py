#!/usr/bin/env python3
"""Black-box emulator UI checks. No Appium, Android Studio or Python dependencies.

The test stops/relaunches Jcode and changes rotation/font scale temporarily.
It refuses physical devices. Artifacts go to target/android/smoke-<serial>/.
"""
import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import time
import xml.etree.ElementTree as ET

ROOT = Path(__file__).resolve().parents[1]
SDK = Path(os.environ.get("ANDROID_HOME", os.environ.get("ANDROID_SDK_ROOT", Path.home() / "Android/Sdk")))
ADB = str(SDK / "platform-tools/adb")
SERIAL = os.environ.get("ANDROID_SERIAL", "emulator-5554")
ARTIFACTS = ROOT / "target/android" / ("smoke-" + SERIAL)
CHECKS = []


def adb(*args, timeout=30, binary=False):
    result = subprocess.run([ADB, "-s", SERIAL, *args], check=True,
                            stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=timeout)
    return result.stdout if binary else result.stdout.decode("utf-8", errors="replace").strip()


def tree():
    raw = adb("exec-out", "uiautomator", "dump", "/dev/tty", timeout=20)
    start, end = raw.find("<?xml"), raw.rfind("</hierarchy>")
    if start < 0 or end < 0:
        raise AssertionError("No accessibility hierarchy: " + raw[:300])
    xml = raw[start:end + len("</hierarchy>")]
    (ARTIFACTS / "latest.xml").write_text(xml)
    return ET.fromstring(xml)


def matches(node, text=None, description=None, contains=None):
    return ((text is None or node.get("text") == text)
            and (description is None or node.get("content-desc") == description)
            and (contains is None or contains in node.get("text", "")))


def wait_node(timeout=30, **query):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        nodes = tree()
        for node in nodes.iter("node"):
            if matches(node, **query):
                return node
        time.sleep(.3)
    raise AssertionError("UI element not found: " + str(query))


def click(**query):
    node = wait_node(**query)
    bounds = list(map(int, re.findall(r"\d+", node.get("bounds", ""))))
    assert len(bounds) == 4, node.attrib
    adb("shell", "input", "tap", str((bounds[0] + bounds[2]) // 2), str((bounds[1] + bounds[3]) // 2))
    return node


def check(value, label):
    if not value:
        raise AssertionError(label)
    CHECKS.append(label)
    print("PASS", label, flush=True)


def screenshot(name):
    (ARTIFACTS / (name + ".png")).write_bytes(adb("exec-out", "screencap", "-p", binary=True))


def launch(demo=False):
    adb("shell", "am", "force-stop", "org.jcode.mobile")
    args = ["shell", "am", "start", "-W", "-n", "org.jcode.mobile/.MainActivity"]
    if demo:
        args += ["--es", "demo", "true"]
    adb(*args)
    wait_node(text="Jcode")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--endpoint", help="Optional local fixture endpoint, e.g. tcp://10.0.2.2:17643")
    args = parser.parse_args()
    assert adb("shell", "getprop", "ro.kernel.qemu") == "1", "Refusing to change settings on a physical device"
    ARTIFACTS.mkdir(parents=True, exist_ok=True)
    old_rotation = adb("shell", "settings", "get", "system", "user_rotation")
    old_auto = adb("shell", "settings", "get", "system", "accelerometer_rotation")
    old_font = adb("shell", "settings", "get", "system", "font_scale")
    log_start = adb("shell", "date", "+%m-%d\\ %H:%M:%S.000")
    try:
        adb("shell", "settings", "put", "system", "accelerometer_rotation", "0")
        adb("shell", "settings", "put", "system", "user_rotation", "0")
        launch()
        check(wait_node(description="Message to send").get("enabled") in ("true", "false"), "App launches with accessible native composer")
        check(wait_node(text="Send").get("enabled") == "false", "Disconnected send is disabled")
        screenshot("disconnected")
        launch(demo=True)
        wait_node(contains="demo")
        check(wait_node(text="Send").get("enabled") == "true", "Explicit offline demo enables native session")
        click(description="Message to send")
        adb("shell", "input", "text", "emulator%ssmoke")
        # Hide IME with Back, which must not exit the activity.
        adb("shell", "input", "keyevent", "KEYCODE_BACK")
        click(text="Send")
        wait_node(contains="no message was sent")
        check(True, "Native demo sends through JNI and renders assistant response")
        screenshot("demo-transcript")
        click(description="Message to send")
        adb("shell", "input", "text", "draft-kept")
        adb("shell", "input", "keyevent", "KEYCODE_BACK")
        adb("shell", "input", "keyevent", "KEYCODE_HOME")
        adb("shell", "am", "start", "-W", "-n", "org.jcode.mobile/.MainActivity")
        check(wait_node(description="Message to send").get("text") == "draft-kept", "Background/foreground preserves unsent draft")
        adb("shell", "settings", "put", "system", "user_rotation", "1")
        time.sleep(1)
        check(wait_node(description="Message to send").get("text") == "draft-kept", "Landscape rotation preserves draft")
        screenshot("landscape")
        adb("shell", "settings", "put", "system", "user_rotation", "0")
        adb("shell", "settings", "put", "system", "font_scale", "1.3")
        time.sleep(1)
        check(wait_node(text="Send").get("enabled") == "true", "Large font retains usable send control")
        screenshot("large-font")
        adb("shell", "settings", "put", "system", "font_scale", "1.0")
        if args.endpoint:
            launch()
            click(text="Settings")
            field = wait_node(description="Server endpoint")
            click(description="Server endpoint")
            # Move to end then clear a saved endpoint. Unlike keycombination,
            # repeated keyevents also work on older Android system images.
            adb("shell", "input", "keyevent", "KEYCODE_MOVE_END",
                *(["KEYCODE_DEL"] * max(1, len(field.get("text", "")))))
            adb("shell", "input", "text", args.endpoint)
            adb("shell", "input", "keyevent", "KEYCODE_BACK")
            click(text="Save & connect")
            wait_node(contains="connected")
            wait_node(contains="earlier")
            check(True, "UI connects through real Android TCP and loads server history")
            click(description="Message to send")
            adb("shell", "input", "text", "ui%semulator%sroundtrip")
            adb("shell", "input", "keyevent", "KEYCODE_BACK")
            click(text="Send")
            wait_node(contains="actual streamed reply")
            check(True, "Real send/stream renders in Android transcript")
            screenshot("network-transcript")
            click(text="Disconnect")
            check(wait_node(text="Send").get("enabled") == "false", "Disconnect disables send")
        logs = adb("logcat", "-d", "-t", log_start, timeout=30)
        (ARTIFACTS / "logcat.txt").write_text(logs)
        check("FATAL EXCEPTION" not in logs and "Fatal signal" not in logs, "No JVM/native fatal crash during smoke run")
        (ARTIFACTS / "result.json").write_text(json.dumps({"serial": SERIAL, "api": adb("shell", "getprop", "ro.build.version.sdk"), "checks": CHECKS}, indent=2) + "\n")
        print(f"{len(CHECKS)} UI checks passed. Artifacts: {ARTIFACTS}")
    except BaseException:
        screenshot("failure")
        (ARTIFACTS / "failure-logcat.txt").write_text(adb("logcat", "-d", "-t", "1000"))
        raise
    finally:
        for key, value in [("user_rotation", old_rotation), ("accelerometer_rotation", old_auto), ("font_scale", old_font)]:
            if value == "null":
                adb("shell", "settings", "delete", "system", key)
            else:
                adb("shell", "settings", "put", "system", key, value)


if __name__ == "__main__":
    main()
