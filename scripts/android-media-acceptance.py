#!/usr/bin/env python3
"""Partial gallery/camera acceptance for a disposable Android emulator.

Requires adb, magick and tesseract. Installs the explicit debug APK without
clearing data, selects the shared offline todos demo, and never sends a message
or uploads media. Does not change orientation or settings. --camera must really
complete the external camera flow or the run fails. Evidence is not full parity.
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
import time
import xml.etree.ElementTree as ET

PACKAGE = "org.jcode.mobile"
FIXTURE = "jcode-emulator-photo.jpg"


def bounds_center(bounds):
    match = re.fullmatch(r"\[(\d+),(\d+)\]\[(\d+),(\d+)\]", bounds)
    if not match:
        raise AssertionError(f"Invalid UI bounds: {bounds!r}")
    left, top, right, bottom = map(int, match.groups())
    if right <= left or bottom <= top:
        raise AssertionError(f"Empty UI bounds: {bounds}")
    return (left + right) // 2, (top + bottom) // 2


def find_node(xml, predicate):
    nodes = [node for node in ET.fromstring(xml).iter("node") if predicate(node.attrib)]
    if len(nodes) != 1:
        raise AssertionError(f"Expected exactly one matching UI node, found {len(nodes)}")
    bounds_center(nodes[0].get("bounds", ""))
    return nodes[0].attrib


def fixture_node(xml):
    # DocumentsUI decorates the exact filename with type/date/size metadata.
    # Match the full filename token, never a generic photo or a coordinate guess.
    return find_node(
        xml, lambda a: a.get("package") in ("com.google.android.documentsui", "com.android.documentsui")
        and bool(re.search(r"(?<![\w.-])" + re.escape(FIXTURE) + r"(?![\w.-])",
                           a.get("content-desc", ""))))


def blue_pixels(rgb):
    return sum(abs(r - 99) <= 24 and abs(g - 149) <= 24 and abs(b - 236) <= 24
               for r, g, b in zip(rgb[0::3], rgb[1::3], rgb[2::3]))


def changed_pixels(before, after):
    if len(before) != len(after) or len(before) % 3:
        raise AssertionError("Incompatible RGB evidence")
    return sum(max(abs(a - b) for a, b in zip(before[i:i + 3], after[i:i + 3])) > 35
               for i in range(0, len(before), 3))


class Acceptance:
    def __init__(self, args):
        self.args = args
        self.out = args.out.resolve()
        self.out.mkdir(parents=True, exist_ok=True)
        sdk = Path(os.environ.get("ANDROID_HOME", Path.home() / "Android/Sdk"))
        self.adb_path = str(sdk / "platform-tools/adb")
        self.checks = []
        self.metadata = {"serial": args.serial, "camera_requested": args.camera}
        self.width = self.height = None
        self.started = time.strftime("%m-%d %H:%M:%S.000")
        self.validated_emulator = False

    def run_command(self, command, timeout=45):
        result = subprocess.run(command, capture_output=True, timeout=timeout)
        with (self.out / "commands.log").open("a") as log:
            log.write(json.dumps(command) + f" -> {result.returncode}\n")
            log.write(result.stderr.decode(errors="replace") + "\n")
        result.check_returncode()
        return result.stdout

    def adb(self, *command):
        return self.run_command([self.adb_path, "-s", self.args.serial, *command])

    def text(self, *command):
        return self.adb(*command).decode(errors="replace").strip()

    def check(self, name, condition):
        self.checks.append({"name": name, "passed": bool(condition)})
        print(("PASS " if condition else "FAIL ") + name, flush=True)
        if not condition:
            raise AssertionError(name)

    def tap(self, x, y):
        self.adb("shell", "input", "tap", str(x), str(y))

    def tap_node(self, node):
        self.tap(*bounds_center(node["bounds"]))

    def capture(self, name):
        path = self.out / f"{name}.png"
        path.write_bytes(self.adb("exec-out", "screencap", "-p"))
        data = path.read_bytes()
        self.width, self.height = int.from_bytes(data[16:20], "big"), int.from_bytes(data[20:24], "big")
        tsv = self.run_command(["tesseract", str(path), "stdout", "tsv"], 30).decode()
        (self.out / f"{name}.tsv").write_text(tsv)
        words = [r for r in csv.DictReader(io.StringIO(tsv), delimiter="\t") if r.get("text", "").strip()]
        text = " ".join(r["text"] for r in words)
        (self.out / f"{name}.txt").write_text(text + "\n")
        return path, text, words

    def xml(self, name):
        remote = "/sdcard/jcode-media-acceptance.xml"
        self.adb("shell", "uiautomator", "dump", remote)
        xml = self.text("shell", "cat", remote)
        (self.out / f"{name}.xml").write_text(xml)
        ET.fromstring(xml)
        return xml

    def tap_phrase(self, words, phrase):
        tokens = phrase.lower().split()
        matches = []
        for index in range(len(words) - len(tokens) + 1):
            rows = words[index:index + len(tokens)]
            if [re.sub(r"[^a-z]", "", r["text"].lower()) for r in rows] == tokens:
                matches.append(rows)
        if len(matches) != 1:
            raise AssertionError(f"Expected one OCR phrase {phrase!r}, found {len(matches)}")
        row = matches[0][0]
        self.tap(int(row["left"]) + int(row["width"]) // 2,
                 int(row["top"]) + int(row["height"]) // 2)

    def crop_rgb(self, path, box):
        x, y, width, height = box
        return self.run_command(["magick", str(path), "-crop", f"{width}x{height}+{x}+{y}",
                                 "+repage", "-depth", "8", "rgb:-"])

    def scaled(self, x, y, width, height):
        return (round(x * self.width / 1080), round(y * self.height / 1920),
                round(width * self.width / 1080), round(height * self.height / 1920))

    def shared_return(self, name):
        time.sleep(3)
        path, text, words = self.capture(name)
        self.xml(name)
        focus = self.text("shell", "dumpsys", "window", "windows")
        (self.out / f"{name}-window.txt").write_text(focus)
        focused = [line for line in focus.splitlines() if "mCurrentFocus=" in line]
        self.check(name + ": shared GPUI returned", any(PACKAGE in line for line in focused)
                   and "Core" in text and "progress" in text)
        self.check(name + ": app alive", bool(self.text("shell", "pidof", PACKAGE)))
        return path, text, words

    def open_plus(self, evidence_name):
        # Known shared Rust debug-todos composer anchor, scaled to the screenshot.
        self.tap(round(132 * self.width / 1080), round(1707 * self.height / 1920))
        time.sleep(1)
        return self.capture(evidence_name)

    def exercise(self):
        self.check("Explicit serial is an emulator", self.text("shell", "getprop", "ro.kernel.qemu") == "1")
        self.validated_emulator = True
        self.metadata["apk_sha256"] = hashlib.sha256(self.args.apk.read_bytes()).hexdigest()
        self.metadata["sdk"] = self.text("shell", "getprop", "ro.build.version.sdk")
        self.metadata["settings_before"] = {key: self.text("shell", "settings", "get", "system", key)
                                             for key in ("user_rotation", "accelerometer_rotation")}
        self.adb("install", "-r", str(self.args.apk.resolve()))
        self.adb("shell", "am", "force-stop", PACKAGE)
        self.adb("shell", "am", "start", "-W", "-n", PACKAGE + "/.JcodeActivity", "--es", "gpui_demo", "todos")
        path, _, _ = self.shared_return("shared-demo")
        self.check("Portrait fixture coordinates apply", self.width < self.height)
        fixture = self.out / FIXTURE
        self.run_command(["magick", "-size", "96x64", "xc:#6395ec", "-quality", "95", str(fixture)])
        remote = "/sdcard/Download/" + FIXTURE
        self.adb("push", str(fixture), remote)
        self.adb("shell", "am", "broadcast", "-a", "android.intent.action.MEDIA_SCANNER_SCAN_FILE",
                 "-d", "file://" + remote)
        time.sleep(2)
        _, _, words = self.open_plus("gallery-attachment-menu")
        self.tap_phrase(words, "Photo Library")
        time.sleep(2)
        self.capture("documents-picker")
        xml = self.xml("documents-picker")
        try:
            node = fixture_node(xml)
        except AssertionError:
            # Use real DocumentsUI navigation when Recent does not list the fixture.
            node = find_node(xml, lambda a: a.get("content-desc", "").lower() in ("show roots", "open navigation drawer"))
            self.tap_node(node)
            time.sleep(1)
            xml = self.xml("documents-roots")
            self.capture("documents-roots")
            self.tap_node(find_node(xml, lambda a: a.get("text") == "Downloads"))
            time.sleep(1)
            xml = self.xml("documents-downloads")
            self.capture("documents-downloads")
            node = fixture_node(xml)
        self.tap_node(node)
        gallery, _, _ = self.shared_return("gallery-return")
        crop = self.scaled(42, 1454, 145, 97)
        rgb = self.crop_rgb(gallery, crop)
        self.check("Blue fixture thumbnail exists in shared composer", blue_pixels(rgb) > len(rgb) // 3 * 0.45)
        before_second = self.crop_rgb(gallery, self.scaled(203, 1454, 145, 97))
        if self.args.camera:
            _, _, words = self.open_plus("camera-attachment-menu")
            self.tap_phrase(words, "Take Photo")
            time.sleep(3)
            self.capture("camera-open")
            xml = self.xml("camera-open")
            shutter = find_node(xml, lambda a: a.get("package") == "com.android.camera2"
                                and (a.get("resource-id", "").endswith(":id/shutter_button")
                                     or a.get("content-desc", "").lower() in ("shutter", "take photo")))
            self.check("Real external camera shutter available", bool(shutter))
            self.tap_node(shutter)
            time.sleep(2)
            self.capture("camera-review")
            xml = self.xml("camera-review")
            done = find_node(xml, lambda a: a.get("package") == "com.android.camera2"
                             and (a.get("resource-id", "").endswith(":id/done_button")
                                  or a.get("content-desc", "").lower() == "done" or a.get("text") == "Done"))
            self.tap_node(done)
            camera, _, _ = self.shared_return("camera-return")
            self.check("Gallery thumbnail retained after camera", blue_pixels(self.crop_rgb(camera, crop)) > len(rgb) // 3 * 0.45)
            after_second = self.crop_rgb(camera, self.scaled(203, 1454, 145, 97))
            self.check("Second camera thumbnail changes composer pixels",
                       changed_pixels(before_second, after_second) > len(after_second) // 3 * 0.35)
        settings = {key: self.text("shell", "settings", "get", "system", key)
                    for key in self.metadata["settings_before"]}
        self.check("Orientation settings preserved", settings == self.metadata["settings_before"])
        logs = self.text("logcat", "-d", "-v", "threadtime", "-T", self.started)
        (self.out / "logcat.txt").write_text(logs)
        self.check("App and external camera have no observed crashes", not any(
            marker in logs for marker in ("Fatal signal", "FATAL EXCEPTION", "SecurityException", "Permission Denial")))

    def run(self):
        error = None
        try:
            self.exercise()
        except (Exception, KeyboardInterrupt) as exc:
            error = f"{type(exc).__name__}: {exc}"
            self.checks.append({"name": "Run completed", "passed": False, "error": error})
            if self.validated_emulator:
                for name, action in (("failure", lambda: self.capture("failure")),
                                     ("failure-xml", lambda: self.xml("failure")),
                                     ("failure-logcat", lambda: (self.out / "logcat.txt").write_text(
                                         self.text("logcat", "-d", "-v", "threadtime", "-T", self.started)))):
                    try:
                        action()
                    except Exception as evidence_error:
                        self.metadata[name + "_error"] = str(evidence_error)
        finally:
            result = {**self.metadata, "checks": self.checks, "passed": error is None,
                      "error": error, "full_parity": False, "release_accepted": False,
                      "media_sent_or_uploaded": False,
                      "remaining": ["physical device media", "permission denial and cancellation", "release parity",
                                    *([] if self.args.camera else ["camera flow not requested"])]}
            (self.out / "result.json").write_text(json.dumps(result, indent=2) + "\n")
        if error:
            raise SystemExit(error)


def self_test():
    """Run synthetic selector/pixel/failure-report tests, without adb or devices."""
    import tempfile
    from types import SimpleNamespace

    assert bounds_center("[10,20][30,60]") == (20, 40)
    for invalid in ("", "[1,1][1,2]", "[-1,0][2,2]", "[3,4][2,5]"):
        try:
            bounds_center(invalid)
        except AssertionError:
            pass
        else:
            raise AssertionError("Invalid bounds accepted")
    node = ('<node package="com.google.android.documentsui" '
            'content-desc="Photo, jcode-emulator-photo.jpg, 96 x 64" bounds="[1,2][5,8]"/>')
    assert fixture_node("<hierarchy>" + node + "</hierarchy>")["bounds"] == "[1,2][5,8]"
    for xml in ("<hierarchy/>", "<hierarchy>" + node + node + "</hierarchy>",
                "<hierarchy>" + node.replace(".jpg,", ".jpg.bak,") + "</hierarchy>",
                "<hierarchy>" + node.replace("com.google.android.documentsui", "wrong.package") + "</hierarchy>"):
        try:
            fixture_node(xml)
        except AssertionError:
            pass
        else:
            raise AssertionError("Missing/ambiguous/nonfixture selection accepted")
    assert blue_pixels(bytes([99, 149, 236]) * 20) == 20
    assert blue_pixels(bytes([0, 0, 0]) * 20) == 0
    assert changed_pixels(bytes([0, 0, 0]) * 20, bytes([99, 149, 236]) * 20) == 20
    assert changed_pixels(bytes([99, 149, 236]) * 20, bytes([99, 149, 236]) * 20) == 0
    with tempfile.TemporaryDirectory(prefix="media-acceptance-test-") as temporary:
        runner = Acceptance(SimpleNamespace(out=Path(temporary), serial="synthetic", camera=True))
        runner.exercise = lambda: (_ for _ in ()).throw(AssertionError("camera unavailable"))
        try:
            runner.run()
        except SystemExit:
            pass
        else:
            raise AssertionError("Unavailable camera reported success")
        result = json.loads((Path(temporary) / "result.json").read_text())
        assert not result["passed"] and not result["full_parity"]
        assert not result["release_accepted"] and "camera unavailable" in result["error"]
    print("PASS synthetic bounds, exact selectors, ambiguity, pixels, and camera failure reporting")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--serial", required=True, help="Explicit disposable emulator serial")
    parser.add_argument("--apk", required=True, type=Path, help="Shared GPUI debug APK")
    parser.add_argument("--out", required=True, type=Path, help="Evidence output directory")
    parser.add_argument("--camera", action="store_true", help="Require real external-camera completion")
    args = parser.parse_args()
    if not args.apk.is_file():
        parser.error("--apk must be an existing file")
    Acceptance(args).run()


if __name__ == "__main__":
    main()
