#!/usr/bin/env python3
"""Exercise the real shared GPUI app on a disposable Android emulator.

Requires adb, tesseract and ImageMagick. Uses screenshots/OCR, not the deleted Java app's
widget IDs. Never sends authentication emails or model requests. The debug
Intent selects the existing shared Rust demo and is absent from release builds.
This is partial acceptance, not a substitute for service/TalkBack/live-network
or release-bundle validation.
"""
import argparse
import csv
import hashlib
import io
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--serial", required=True)
    parser.add_argument("--apk", type=Path, required=True)
    parser.add_argument("--out", type=Path)
    args = parser.parse_args()
    sdk = Path(os.environ.get("ANDROID_HOME", Path.home() / "Android/Sdk"))
    adb_path = sdk / "platform-tools/adb"
    out = args.out or ROOT / "target/android" / ("shared-acceptance-" + args.serial)
    out.mkdir(parents=True, exist_ok=True)
    checks = []

    def adb(*command, binary=False):
        result = subprocess.run([str(adb_path), "-s", args.serial, *command],
                                capture_output=True, timeout=45)
        if result.returncode:
            with (out / "adb-errors.txt").open("a") as log:
                log.write(json.dumps(command) + f" -> {result.returncode}\n")
                log.write(result.stdout.decode(errors="replace") + "\n")
                log.write(result.stderr.decode(errors="replace") + "\n")
            result.check_returncode()
        return result.stdout if binary else result.stdout.decode(errors="replace").strip()

    def check(name, condition):
        checks.append({"name": name, "passed": bool(condition)})
        print(("PASS " if condition else "FAIL ") + name, flush=True)
        if not condition:
            raise AssertionError(name)

    def capture(name):
        path = out / (name + ".png")
        path.write_bytes(adb("exec-out", "screencap", "-p", binary=True))
        result = subprocess.run(["tesseract", str(path), "stdout", "tsv"],
                                check=True, capture_output=True, text=True, timeout=30)
        (out / (name + ".tsv")).write_text(result.stdout)
        words = [row for row in csv.DictReader(io.StringIO(result.stdout), delimiter="\t")
                 if row.get("text", "").strip()]
        text = " ".join(row["text"] for row in words)
        (out / (name + ".txt")).write_text(text + "\n")
        return text, words

    def tap_word(words, needle):
        word = next(row for row in words if needle.lower() in row["text"].lower())
        adb("shell", "input", "tap", str(int(word["left"]) + int(word["width"]) // 2),
            str(int(word["top"]) + int(word["height"]) // 2))
        return word

    def draft_present(text):
        return "paritydraft" in re.sub(r"[^a-z]", "", text.lower())

    def await_keyboard():
        # adb input bypasses the IME's own event buffering. Cold-boot keyboard
        # startup can exceed one second, so wait for observed shown state to
        # remain stable before injecting keys, not merely for a focus tap.
        deadline = time.monotonic() + 15
        stable = 0
        while time.monotonic() < deadline:
            state = adb("shell", "dumpsys", "input_method")
            stable = stable + 1 if "mInputShown=true" in state else 0
            if stable >= 3:
                return
            time.sleep(0.3)
        raise AssertionError("Native keyboard did not reach stable shown state")

    rotation = None
    accelerometer = None
    metadata = {}
    try:
        check("Device is an emulator", adb("shell", "getprop", "ro.kernel.qemu") == "1")
        metadata = {"serial": args.serial,
                    "sdk": adb("shell", "getprop", "ro.build.version.sdk"),
                    "page_size": adb("shell", "getconf", "PAGE_SIZE"),
                    "apk_sha256": hashlib.sha256(args.apk.read_bytes()).hexdigest(),
                    "observed_workspace_revision": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()}
        rotation = adb("shell", "settings", "get", "system", "user_rotation")
        accelerometer = adb("shell", "settings", "get", "system", "accelerometer_rotation")
        adb("install", "-r", str(args.apk.resolve()))
        adb("shell", "wm", "dismiss-keyguard")
        adb("shell", "am", "force-stop", "org.jcode.mobile")
        adb("shell", "am", "start", "-W", "-n", "org.jcode.mobile/.JcodeActivity")
        time.sleep(4)
        if "mInputShown=true" in adb("shell", "dumpsys", "input_method"):
            adb("shell", "input", "keyevent", "KEYCODE_BACK")
            time.sleep(1)
        text, words = capture("onboarding")
        for _ in range(2):
            if "notifications" not in text or "allow" not in text.lower():
                break
            # Decline the emulator's first-run prompt. Reinstall can leave an
            # orphan permission-controller task after the old app process dies.
            tap_word(words, "Don't")
            time.sleep(1)
            adb("shell", "am", "start", "-W", "-n", "org.jcode.mobile/.JcodeActivity")
            time.sleep(2)
            text, words = capture("onboarding")
        check("Shared GPUI onboarding renders", "Jcode" in text and "Google" in text)
        check("Apple-only sign-in is absent on Android", "Apple" not in text)
        tap_word(words, "example.com")
        await_keyboard()
        adb("shell", "input", "text", "paritydraft%ssample")
        time.sleep(2)
        text, words = capture("onboarding-keyboard")
        check("Native keyboard is actually visible", "mInputShown=true" in adb("shell", "dumpsys", "input_method"))
        check("Native text reaches GPUI email field", draft_present(text))
        action_text = text
        if "Send" not in action_text or "code" not in action_text:
            # Whole-screen OCR can omit the low-contrast disabled button. Read
            # its real pixels below the observed email field, never synthesize
            # expected labels or accept keyboard suggestion text as evidence.
            field = next(row for row in words if "paritydraft" in row["text"].lower())
            top = int(field["top"]) + int(field["height"]) + 30
            crop = out / "email-action.png"
            screenshot = out / "onboarding-keyboard.png"
            width = int.from_bytes(screenshot.read_bytes()[16:20], "big")
            crop_width, left = int(width * 0.65), int(width * 0.175)
            subprocess.run(["magick", str(screenshot),
                            "-crop", f"{crop_width}x180+{left}+{top}", "+repage", "-resize", "200%", str(crop)],
                           check=True, capture_output=True, timeout=20)
            action_text = subprocess.check_output(["tesseract", str(crop), "stdout", "--psm", "6"],
                                                 text=True, stderr=subprocess.DEVNULL, timeout=20)
        check("Email action stays visible above keyboard", "Send" in action_text and "code" in action_text)
        adb("shell", "input", "keyevent", "KEYCODE_BACK")
        adb("shell", "input", "keyevent", "KEYCODE_HOME")
        adb("shell", "am", "start", "-W", "-n", "org.jcode.mobile/.JcodeActivity")
        time.sleep(2)
        text, _ = capture("onboarding-resume")
        check("Unsent email draft survives background and resume", draft_present(text))
        adb("shell", "am", "force-stop", "org.jcode.mobile")
        adb("shell", "am", "start", "-W", "-n", "org.jcode.mobile/.JcodeActivity", "--es", "gpui_demo", "todos")
        time.sleep(3)
        text, words = capture("shared-demo")
        check("Existing shared Rust chat demo renders", "Core" in text and "progress" in text)
        tap_word(words, "Message")
        await_keyboard()
        adb("shell", "input", "text", "paritydraft%ssample")
        time.sleep(2)
        text, _ = capture("composer-keyboard")
        check("Shared chat composer receives native input", draft_present(text))
        adb("shell", "input", "keyevent", "KEYCODE_BACK")
        time.sleep(1)
        text, _ = capture("composer-back")
        check("Android Back hides keyboard and preserves composer draft",
              "mInputShown=false" in adb("shell", "dumpsys", "input_method")
              and draft_present(text) and bool(adb("shell", "pidof", "org.jcode.mobile").strip()))
        adb("shell", "settings", "put", "system", "accelerometer_rotation", "0")
        adb("shell", "settings", "put", "system", "user_rotation", "1")
        time.sleep(3)
        text, _ = capture("landscape")
        check("Composer draft survives landscape rotation", draft_present(text))
        adb("shell", "settings", "put", "system", "user_rotation", "0")
        time.sleep(3)
        adb("shell", "input", "keyevent", "KEYCODE_HOME")
        time.sleep(1)
        adb("shell", "am", "start", "-W", "-n", "org.jcode.mobile/.JcodeActivity")
        time.sleep(2)
        text, _ = capture("chat-resume")
        check("Composer draft survives background and surface recreation", draft_present(text))
        pid = adb("shell", "pidof", "org.jcode.mobile").split()[0]
        logs = adb("logcat", "--pid", pid, "-d")
        (out / "logcat.txt").write_text(logs)
        check("Native bootstrap is the shared GPUI app", "shared GPUI NativeActivity bootstrap" in logs)
        check("Native app remains alive without crash", "Fatal signal" not in logs and "FATAL EXCEPTION" not in logs)
    finally:
        cleanup_failed = False
        for key, value in (("user_rotation", rotation), ("accelerometer_rotation", accelerometer)):
            if value is not None:
                try:
                    if value == "null":
                        adb("shell", "settings", "delete", "system", key)
                    else:
                        adb("shell", "settings", "put", "system", key, value)
                except (subprocess.SubprocessError, OSError) as error:
                    cleanup_failed = True
                    checks.append({"name": f"Restore {key}", "passed": False,
                                   "error": str(error)})
        (out / "result.json").write_text(json.dumps({**metadata, "checks": checks,
            "full_parity": False, "release_accepted": False,
            "remaining": ["real composing IME and TalkBack", "native platform services", "authenticated network workflows", "signed release AAB generated APK validation", "FCM backend delivery"]}, indent=2) + "\n")
        if cleanup_failed and sys.exc_info()[0] is None:
            raise AssertionError("Emulator settings cleanup failed, inspect result.json and adb-errors.txt")


if __name__ == "__main__":
    main()
