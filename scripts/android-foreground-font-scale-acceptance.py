#!/usr/bin/env python3
"""Root-run foreground font-scale OCR acceptance. Never launches or taps the app.

Run only on a reserved, already-awake signed-out emulator. This measures visual
control reachability, not authentication or touch dispatch. No reduced-motion claim.
"""
import argparse
import csv
import hashlib
import importlib.util
import io
import json
from pathlib import Path
import re
import signal
import subprocess
import time

spec = importlib.util.spec_from_file_location("appearance", Path(__file__).with_name("android-foreground-appearance-acceptance.py"))
appearance = importlib.util.module_from_spec(spec)
spec.loader.exec_module(appearance)
LABELS = {"heading": "Sign in to Jcode Cloud", "google": "Continue with Google",
          "email": "you@example.com", "submit": "Send code"}


def font_row(text):
    rows = [line for line in text.splitlines() if line.partition("=")[0] == "font_scale"]
    if not rows:
        return None
    if len(rows) != 1 or not re.fullmatch(r"font_scale=(?:\d+(?:\.\d*)?|\.\d+)", rows[0]):
        raise AssertionError("Ambiguous or unsupported font_scale row")
    value = rows[0].split("=", 1)[1]
    if not 0.5 <= float(value) <= 3:
        raise AssertionError("Original font scale outside supported range")
    return value


def configuration_scale(text):
    markers = re.findall(r"^\s*topResumedActivity\s*[:=]\s*ActivityRecord\{([a-fA-F0-9]+) ", text, re.MULTILINE)
    markers = markers or re.findall(r"^\s*mResumedActivity\s*[:=]\s*ActivityRecord\{([a-fA-F0-9]+) ", text, re.MULTILINE)
    if len(markers) != 1:
        raise AssertionError("Require unique top-resumed Activity configuration")
    blocks = re.split(r"(?=^\s*\* Hist\s+#\d+:)", text, flags=re.MULTILINE)
    blocks = [block for block in blocks if re.match(r"\s*\* Hist\s+#\d+:\s*ActivityRecord\{" + markers[0] + r"\s", block)]
    if len(blocks) != 1:
        raise AssertionError("Cannot isolate current Activity configuration")
    values = re.findall(r"^\s*CurrentConfiguration=\{(\d+(?:\.\d+)?)\s", blocks[0], re.MULTILINE)
    if len(values) != 1:
        raise AssertionError("Cannot establish unique actual Activity font scale")
    return float(values[0])


def normalize(text):
    return re.sub(r"[^a-z0-9]", "", text.lower())


def measure(tsv, width, height):
    words = []
    for row in csv.DictReader(io.StringIO(tsv), delimiter="\t"):
        if row["level"] != "5" or not row["text"].strip():
            continue
        word = {key: int(row[key]) for key in ("left", "top", "width", "height")}
        word.update(text=normalize(row["text"]), confidence=float(row["conf"]))
        if word["width"] <= 0 or word["height"] <= 0:
            raise AssertionError("Invalid actual OCR word bounds")
        if not word["text"]:
            continue
        words.append(word)
    result = {}
    for name, label in LABELS.items():
        target = normalize(label)
        candidates = []
        for start in range(len(words)):
            for end in range(start + 1, min(start + 9, len(words)) + 1):
                group = words[start:end]
                if "".join(word["text"] for word in group) != target:
                    continue
                if any(not 45 <= word["confidence"] <= 100 for word in group):
                    continue
                left = min(w["left"] for w in group)
                top = min(w["top"] for w in group)
                right = max(w["left"] + w["width"] for w in group)
                bottom = max(w["top"] + w["height"] for w in group)
                # Leave physical margins for status/navigation chrome and control padding.
                if left < 8 or right > width - 8 or top < height * .05 or bottom > height * .94:
                    raise AssertionError("Clipped/offscreen onboarding label: " + name)
                candidates.append({"bounds": [left, top, right, bottom],
                                   "word_heights": [w["height"] for w in group],
                                   "word_widths": [w["width"] for w in group]})
        if len(candidates) != 1:
            raise AssertionError("Missing/ambiguous actual onboarding OCR label: " + name)
        result[name] = candidates[0]
    boxes = [v["bounds"] for v in result.values()]
    for index, a in enumerate(boxes):
        for b in boxes[index + 1:]:
            if max(a[0], b[0]) < min(a[2], b[2]) and max(a[1], b[1]) < min(a[3], b[3]):
                raise AssertionError("Overlapping onboarding control labels")
    return result


def require_growth(before, after):
    ratios = {}
    for name in LABELS:
        old, new = before[name]["word_heights"], after[name]["word_heights"]
        if len(old) != len(new) or not old or min(old) <= 0:
            raise AssertionError("OCR token alignment changed: " + name)
        values = [b / a for a, b in zip(old, new)]
        # Every corresponding actual glyph box must grow, not just the layout.
        if not all(1.20 <= ratio <= 1.90 for ratio in values):
            raise AssertionError("Actual OCR glyph bounds did not grow at 1.5: " + name)
        ratios[name] = values
    return ratios


class Acceptance(appearance.Acceptance):
    def row(self):
        return font_row(self.shell("settings", "--user", "0", "list", "system"))

    def continuity(self, name, expected=None):
        state = self.current(name)
        raw = (self.out / (name + "-activities.txt")).read_text()
        scale = configuration_scale(raw)
        if expected is not None and abs(scale - expected) > .001:
            raise AssertionError("Actual Activity configuration font scale mismatch")
        stat = self.shell("cat", "/proc/" + state["pid"] + "/stat")
        fields = stat.rsplit(") ", 1)[1].split()
        start = fields[19]
        if hasattr(self, "process_start") and start != self.process_start:
            raise AssertionError("Process start time changed")
        self.process_start = start
        if self.events is not None:
            appearance.event_delta(self.events, self.events_snapshot(name))
        return scale

    def capture_font(self, name, expected):
        self.continuity(name + "-before", expected)
        png = self.command("exec-out", "screencap", "-p")
        width, height = appearance.png_dimensions(png)
        path = self.out / (name + ".png")
        path.write_bytes(png)
        tsv = subprocess.run(["tesseract", str(path), "stdout", "tsv"], check=True,
                             capture_output=True, timeout=20).stdout.decode("utf-8")
        (self.out / (name + ".tsv")).write_text(tsv)
        appearance.require_onboarding(" ".join(row["text"] for row in csv.DictReader(io.StringIO(tsv), delimiter="\t")))
        labels = measure(tsv, width, height)
        observation = {"name": name, "scale": expected, "dimensions": [width, height],
                       "labels": labels, "png_sha256": hashlib.sha256(png).hexdigest()}
        self.observations.append(observation)
        (self.out / "observations.json").write_text(json.dumps(self.observations, indent=2) + "\n")
        self.continuity(name + "-after", expected)
        return observation

    def exercise(self):
        if not re.fullmatch(r"emulator-\d+", self.args.run):
            raise AssertionError("Explicit emulator serial required")
        if self.shell("getprop", "ro.kernel.qemu") != "1" or self.shell("am", "get-current-user") != "0":
            raise AssertionError("Require primary user disposable emulator")
        if self.shell("id", "-u") != "2000":
            raise AssertionError("Require actual adb shell UID 2000")
        if "mWakefulness=Awake" not in self.shell("dumpsys", "power"):
            raise AssertionError("Require already awake device")
        ime = self.shell("dumpsys", "input_method")
        if "mInputShown=false" not in ime or "mInputShown=true" in ime:
            raise AssertionError("Require keyboard hidden")
        self.metadata["sdk"] = self.shell("getprop", "ro.build.version.sdk")
        apk = self.shell("pm", "path", appearance.PACKAGE)
        if not re.fullmatch(r"package:/[^\n]+\.apk", apk):
            raise AssertionError("Require single installed APK")
        self.metadata["installed_apk_sha256"] = self.shell("sha256sum", apk[8:])
        self.identity = self.current("preflight")
        self.events = self.events_snapshot("baseline")
        appearance.require_resume_evidence(self.events, self.identity["pid"])
        original = {"row": self.row(), "effective": self.continuity("original")}
        self.capture_font("original", original["effective"])
        self.original = original
        (self.out / "original-settings.json").write_text(json.dumps(original, indent=2) + "\n")
        samples = []
        for scale in ("1.0", "1.5"):
            self.changed = True  # command failure may still have mutated the provider
            self.shell("settings", "--user", "0", "put", "system", "font_scale", scale)
            time.sleep(self.args.settle_seconds)
            if self.row() != scale:
                raise AssertionError("Requested font_scale row not applied exactly")
            samples.append(self.capture_font("scale-" + scale, float(scale)))
        if samples[0]["dimensions"] != samples[1]["dimensions"]:
            raise AssertionError("Screenshot geometry changed")
        self.metadata["glyph_height_ratios"] = require_growth(samples[0]["labels"], samples[1]["labels"])

    def restore(self):
        if not self.changed or self.original is None:
            return
        value = self.original["row"]
        if value is None:
            self.shell("settings", "--user", "0", "delete", "system", "font_scale")
        else:
            self.shell("settings", "--user", "0", "put", "system", "font_scale", value)
        time.sleep(self.args.settle_seconds)
        restored = {"row": self.row()}
        (self.out / "restored-settings.json").write_text(json.dumps(restored, indent=2) + "\n")
        if restored["row"] != value:
            raise AssertionError("Exact original font_scale row restoration failed")
        restored["effective"] = self.continuity("restored", self.original["effective"])
        self.capture_font("restored-screen", self.original["effective"])
        (self.out / "restored-settings.json").write_text(json.dumps(restored, indent=2) + "\n")

    def run(self):
        error = None
        try:
            self.exercise()
        except (Exception, KeyboardInterrupt) as exc:
            error = type(exc).__name__ + ": " + str(exc)
        finally:
            try:
                self.restore()
            except (Exception, KeyboardInterrupt) as exc:
                error = (error + "; " if error else "") + "RESTORATION FAILED: " + str(exc)
            result = {"passed": error is None, "error": error, "serial": self.args.run,
                      "metadata": self.metadata, "observations": self.observations,
                      "font_scale_accepted": error is None, "reduce_motion_accepted": False,
                      "settings_changed": self.changed, "release_accepted": False,
                      "limits": ["OCR label visibility and nonoverlap only, no touch/authentication dispatch tested",
                                 "Sampled Activity plus retained lifecycle events and process start identity, no native probe",
                                 "No reduced-motion, network, account or physical-device acceptance"]}
            (self.out / "result.json").write_text(json.dumps(result, indent=2) + "\n")
        if error:
            raise SystemExit(error)
        print("PASS: actual onboarding glyph growth 1.0 to 1.5, visible control labels, continuous Activity/process, exact font_scale restoration. No touch/auth or reduced-motion claim.")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--run", required=True)
    parser.add_argument("--out", required=True, type=Path)
    parser.add_argument("--settle-seconds", type=float, default=1.5)
    args = parser.parse_args()
    if not re.fullmatch(r"emulator-\d+", args.run) or not .5 <= args.settle_seconds <= 3:
        parser.error("Require emulator serial and settle interval .5..3 seconds")
    def interrupted(signum, frame):
        raise KeyboardInterrupt("Signal " + str(signum))
    signal.signal(signal.SIGTERM, interrupted)
    Acceptance(args).run()


if __name__ == "__main__":
    main()
