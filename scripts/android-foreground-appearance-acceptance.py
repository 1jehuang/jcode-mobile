#!/usr/bin/env python3
"""Root-run same-Activity GPUI theme acceptance, without production probes.

Requires an already-resumed, signed-out onboarding screen on an explicitly
reserved disposable emulator. Never installs/launches/stops the app, taps UI,
clears data/logs, changes accounts, or requests auth/model/network operations.
Changes only night mode and restores exact captured night-related secure values.
Font scale/reduced motion are explicitly NOT accepted by this harness.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shlex
import signal
import subprocess
import time

PACKAGE = "org.jcode.mobile"
COMPONENT = r"org\.jcode\.mobile/(?:\.JcodeActivity|org\.jcode\.mobile\.JcodeActivity)"
SETTING = re.compile(r"(?:ui_night_mode(?:_[a-z_]+)?|dark_theme_custom_(?:start|end)_time)\Z")
LIFECYCLE = re.compile(r"(?:am|wm)_(?:(?:on_)?(?:create|start|restart|resume|resumed|pause|paused|stop|destroy|relaunch|finish|top_resumed)(?:_|$)|set_resumed_activity|proc_(?:start|died|bound))")
APP = re.compile(r"(?<![\w.])org\.jcode\.mobile(?=(?:/|\.JcodeActivity(?![\w.$])|[ ,\]}]|$))")
SERVICE_FIELDS = ("mNightMode", "mNightModeLocked", "mUiModeLocked", "mComputedNightMode",
                  "mOverrideNightModeOn", "mOverrideNightModeOff", "mCurUiMode")
SHELL_REQUIRED_PERMISSIONS = ("android.permission.MODIFY_DAY_NIGHT_MODE", "android.permission.CHANGE_CONFIGURATION")


def shell_capability(uid, dump):
    """Actual shell UID and concordant package/shared-UID install grants, never requests."""
    if uid.strip() != "2000":
        raise AssertionError("Require actual adb shell UID 2000, not root or an inferred package UID")
    def section(name):
        matches = re.findall(r"^" + re.escape(name) + r":[ \t]*\n(.*?)(?=^\S|\Z)", dump, re.MULTILINE | re.DOTALL)
        if len(matches) != 1:
            raise AssertionError("Missing/ambiguous shell package section " + name)
        return matches[0]
    packages = section("Packages")
    headers = re.findall(r"^  Package \[([^\]]+)\] \(([a-f0-9]+)\):[ \t]*$", packages, re.MULTILINE)
    if len(headers) != 1 or headers[0][0] != "com.android.shell":
        raise AssertionError("Require uniquely scoped com.android.shell package record")
    package_token = headers[0][1]
    if re.findall(r"^    appId=(\d+)[ \t]*$", packages, re.MULTILINE) != ["2000"]:
        raise AssertionError("Actual shell package appId does not match caller UID")
    shared = re.findall(r"^    sharedUser=SharedUserSetting\{([a-f0-9]+) android\.uid\.shell/2000\}[ \t]*$", packages, re.MULTILINE)
    if len(shared) != 1:
        raise AssertionError("Cannot establish shell shared UID ownership")
    users = re.findall(r"^    User 0:([^\n]*)$", packages, re.MULTILINE)
    if len(users) != 1 or not all(re.search(r"\b" + value + r"\b", users[0]) for value in ("installed=true", "hidden=false", "suspended=false")):
        raise AssertionError("Shell package is not explicitly installed/available for primary user 0")
    shared_users = section("Shared users")
    shared_headers = re.findall(r"^  SharedUser \[([^\]]+)\] \(([a-f0-9]+)\):[ \t]*$", shared_users, re.MULTILINE)
    if shared_headers != [("android.uid.shell", shared[0])]:
        raise AssertionError("Ambiguous/mismatched shell shared UID permission record")
    if re.findall(r"^    appId=(\d+)[ \t]*$", shared_users, re.MULTILINE) != ["2000"]:
        raise AssertionError("Shared UID appId does not match actual shell caller")
    member = r"^      PackageSetting\{" + package_token + r" com\.android\.shell/2000\}[ \t]*$"
    if len(re.findall(member, shared_users, re.MULTILINE)) != 1:
        raise AssertionError("Shell package is not linked to this shared UID grant record")
    def install_grants(block):
        headers = list(re.finditer(r"^    install permissions:[ \t]*$", block, re.MULTILINE))
        if len(headers) != 1:
            raise AssertionError("Cannot isolate shell install-permission grants")
        lines = []
        for line in block[headers[0].end():].splitlines():
            if not line.strip():
                continue
            if not line.startswith("      "):
                break
            lines.append(line)
        body = "\n".join(lines)
        for permission in SHELL_REQUIRED_PERMISSIONS:
            values = re.findall(r"^      " + re.escape(permission) + r": ([^\n]*)$", body, re.MULTILINE)
            if len(values) != 1 or not re.fullmatch(r"granted=true(?:, flags=\[[^\]\n]*\])?", values[0]):
                raise AssertionError("Missing/denied/ambiguous actual shell grant: " + permission)
    install_grants(packages)
    install_grants(shared_users)
    for permission in SHELL_REQUIRED_PERMISSIONS:
        records = re.findall(r"^\s*" + re.escape(permission) + r": ([^\n]*)$", dump, re.MULTILINE)
        if len(records) != 2 or not all(re.fullmatch(r"granted=true(?:, flags=\[[^\]\n]*\])?", value) for value in records):
            raise AssertionError("Conflicting or unexplained shell permission records: " + permission)
    return {"uid": "2000", "app_id": "2000", "package": "com.android.shell",
            "permissions": {permission: True for permission in SHELL_REQUIRED_PERMISSIONS}}



def night_mode(text):
    match = re.fullmatch(r"Night mode:\s*(yes|no)", text.strip())
    if not match:
        raise AssertionError("Only explicit yes/no night modes are safely restorable here, not auto/custom/unknown: " + text)
    return match.group(1)


def night_settings(text):
    values = {}
    for line in text.splitlines():
        key, separator, value = line.partition("=")
        if key.startswith("ui_night_mode") and not SETTING.fullmatch(key):
            raise AssertionError("Unknown night setting key: " + key)
        if SETTING.fullmatch(key):
            if not separator or key in values or not re.fullmatch(r"-?\d{1,20}", value):
                raise AssertionError("Unknown/duplicate night setting encoding: " + line)
            values[key] = value
    if len(values) > 16:
        raise AssertionError("Unexpectedly broad night settings scope")
    return values


def service_state(text, capability=None):
    result = {}
    override_fields = ("mOverrideNightModeOn", "mOverrideNightModeOff")
    for field in SERVICE_FIELDS:
        if field in override_fields:
            continue
        matches = re.findall(r"(?<![\w])" + field + r"=([^\s,]+)", text)
        if len(matches) != 1:
            raise AssertionError("Cannot establish unique uimode service field " + field)
        result[field] = matches[0]
    paired = re.findall(r"(?<![\w])mOverrideOn/Off=([^\s,]*)", text)
    separate = [re.findall(r"(?<![\w])" + field + r"=([^\s,]*)", text) for field in override_fields]
    keys = ("mOverrideOn/Off",) + override_fields
    parsed_counts = (len(paired),) + tuple(len(values) for values in separate)
    for key, parsed_count in zip(keys, parsed_counts):
        mentions = re.findall(r"(?<![\w])" + re.escape(key) + r"(?![\w])", text)
        if len(mentions) != parsed_count:
            raise AssertionError("Malformed uimode override field " + key)
    if paired:
        if len(paired) != 1 or any(separate):
            raise AssertionError("Duplicate/ambiguous uimode override representations")
        pair = re.fullmatch(r"(true|false)/(true|false)", paired[0])
        if not pair:
            raise AssertionError("Malformed paired uimode override flags")
        flags = pair.groups()
    else:
        if any(len(values) != 1 or values[0] not in ("true", "false") for values in separate):
            raise AssertionError("Cannot establish both explicit uimode override flags")
        flags = tuple(values[0] for values in separate)
    result.update(zip(override_fields, flags))
    for key in ("mNightModeLocked", "mUiModeLocked", "mComputedNightMode"):
        if result[key] not in ("true", "false"):
            raise AssertionError("Unknown uimode boolean encoding: " + key)
    if any(result[key] != "false" for key in ("mOverrideNightModeOn", "mOverrideNightModeOff")):
        raise AssertionError("Active night override is outside safe restoration scope")
    if result["mUiModeLocked"] != "false":
        raise AssertionError("UI mode locked remains outside supported appearance acceptance")
    if result["mNightModeLocked"] != "false":
        authorized = (isinstance(capability, dict) and capability.get("uid") == "2000"
                      and capability.get("app_id") == "2000" and capability.get("package") == "com.android.shell"
                      and all(capability.get("permissions", {}).get(permission) is True for permission in SHELL_REQUIRED_PERMISSIONS))
        if not authorized:
            raise AssertionError("Night mode locked: actual shell caller capability must be verified before mutation")
    return result


def activity_state(text, pid):
    markers = re.findall(r"^\s*topResumedActivity\s*[:=]\s*(.*)$", text, re.MULTILINE)
    markers = markers or re.findall(r"^\s*mResumedActivity\s*[:=]\s*(.*)$", text, re.MULTILINE)
    if len(markers) != 1:
        raise AssertionError("Require one unambiguous top-resumed Activity")
    match = re.fullmatch(r"ActivityRecord\{([a-fA-F0-9]+) u\d+ " + COMPONENT + r" t(\d+)\}", markers[0].strip())
    if not match:
        raise AssertionError("Shared GPUI Activity is not exactly top-resumed")
    token, task = match.groups()
    blocks = re.split(r"(?=^\s*\* Hist\s+#\d+:)", text, flags=re.MULTILINE)
    candidates = [block for block in blocks if re.match(r"\s*\* Hist\s+#\d+:\s*ActivityRecord\{" + token + r"\s", block)]
    if len(candidates) != 1:
        raise AssertionError("Cannot isolate current Activity record")
    block = candidates[0].split("\n  * Task", 1)[0]
    process = re.findall(r"app=ProcessRecord\{([a-fA-F0-9]+) (\d+):org\.jcode\.mobile/[^}]+\}", block)
    launches = re.findall(r"\blaunchCount=(\d+)\b", block)
    config = re.findall(r"^\s*CurrentConfiguration=(.*)$", block, re.MULTILINE)
    if len(process) != 1 or process[0][1] != str(pid) or len(launches) != 1 or len(config) != 1:
        raise AssertionError("Cannot establish Activity process/launch/config identity")
    if not re.search(r"\bstate=RESUMED\b", block) or "finishing=false" not in block or "?night" in config[0]:
        raise AssertionError("Activity is not an observable live resumed configuration")
    return {"token": token, "task": task, "process_token": process[0][0], "pid": str(pid),
            "launch_count": launches[0], "night": bool(re.search(r"(?<![?\w])night(?!\w)", config[0]))}


def require_same_activity(before, after):
    for key in ("token", "task", "process_token", "pid", "launch_count"):
        if before[key] != after[key]:
            raise AssertionError("Activity/process restart or ownership changed: " + key)


def event_lines(text):
    return [line for line in text.splitlines() if line.strip() and not line.startswith("---------")]


def event_fields(line):
    match = re.fullmatch(r"\s*\d+\.\d+\s+(\d+)\s+\d+\s+[A-Z]\s+([\w]+)\s*:\s*(.*)", line)
    if not match:
        raise AssertionError("Unsupported events log encoding: " + line[:180])
    return match.groups()


def require_resume_evidence(text, pid):
    lines = event_lines(text)
    if len(lines) < 8:
        raise AssertionError("Insufficient retained events for continuity anchor")
    parsed = [event_fields(line) for line in lines]
    if not any(p == str(pid) and tag in ("wm_on_resume_called", "am_on_resume_called") and APP.search(data)
               for p, tag, data in parsed):
        raise AssertionError("No real target onResume event retained. Cannot prove lifecycle logging is observable")


def event_delta(before, after):
    old, new = event_lines(before), event_lines(after)
    if len(old) < 8:
        raise AssertionError("Missing lifecycle continuity anchor")
    anchor = old[-8:]
    offsets = [i for i in range(len(new) - 7) if new[i:i + 8] == anchor]
    if len(offsets) != 1:
        raise AssertionError("Events buffer cleared/rolled over/ambiguous: no unique retained continuity anchor")
    delta = new[offsets[0] + 8:]
    for line in delta:
        pid, tag, data = event_fields(line)
        if tag == "chatty" or (APP.search(data) and LIFECYCLE.match(tag)):
            raise AssertionError("Lifecycle transition or suppressed event data during foreground-only window: " + line)
    return delta


def png_dimensions(data):
    if len(data) < 24 or data[:8] != b"\x89PNG\r\n\x1a\n" or data[12:16] != b"IHDR":
        raise AssertionError("Not an actual PNG screenshot")
    width, height = int.from_bytes(data[16:20], "big"), int.from_bytes(data[20:24], "big")
    if not (320 <= width <= 8192 and 320 <= height <= 8192):
        raise AssertionError("Unsupported screenshot dimensions")
    return width, height


def background_fractions(rgb):
    if not rgb or len(rgb) % 3:
        raise AssertionError("Invalid actual screenshot RGB crop")
    count = len(rgb) // 3
    dark = light = 0
    for offset in range(0, len(rgb), 3):
        pixel = rgb[offset:offset + 3]
        dark += all(channel <= 8 for channel in pixel)
        light += all(abs(channel - 246) <= 8 for channel in pixel)
    return {"dark": dark / count, "light": light / count, "pixels": count}


def require_background(fractions, mode):
    target, other = ("dark", "light") if mode == "yes" else ("light", "dark")
    if fractions[target] < 0.70 or fractions[other] > 0.05:
        raise AssertionError("Actual central GPUI pixels do not show expected " + target + " palette: " + str(fractions))


def require_onboarding(text):
    normalized = re.sub(r"[^a-z]", "", text.lower())
    if "signintojcodecloud" not in normalized or "continuewithgoogle" not in normalized:
        raise AssertionError("Actual OCR does not establish signed-out GPUI onboarding; no UI interaction attempted")
    if "waitingforgoogle" in normalized or "sendingacode" in normalized or "signingin" in normalized:
        raise AssertionError("Authentication already in progress; refusing settings campaign")


class Acceptance:
    def __init__(self, args):
        self.args = args
        self.out = args.out.resolve()
        self.out.mkdir(parents=True, exist_ok=False, mode=0o700)
        sdk = Path(os.environ.get("ANDROID_HOME", os.environ.get("ANDROID_SDK_ROOT", Path.home() / "Android/Sdk")))
        self.adb = str(sdk / "platform-tools/adb")
        self.original = None
        self.changed = False
        self.identity = None
        self.events = None
        self.observations = []
        self.metadata = {}
        self.capability = None

    def command(self, *args):
        result = subprocess.run([self.adb, "-s", self.args.run, *args], capture_output=True, timeout=20)
        with (self.out / "commands.jsonl").open("a") as log:
            log.write(json.dumps({"args": args, "code": result.returncode, "stderr": result.stderr.decode(errors="replace")}) + "\n")
        result.check_returncode()
        if len(result.stdout) > 32 * 1024 * 1024:
            raise AssertionError("Unexpectedly large bounded observation")
        return result.stdout

    def shell(self, *args):
        return self.command("shell", shlex.join(args)).decode(errors="strict").strip()

    def events_snapshot(self, name):
        text = self.command("logcat", "-b", "events", "-d", "-v", "epoch").decode(errors="strict")
        (self.out / (name + "-events.txt")).write_text(text)
        return text

    def current(self, name):
        pid = self.shell("pidof", PACKAGE)
        if not re.fullmatch(r"\d+", pid):
            raise AssertionError("Require one existing shared app process, never start it implicitly")
        activities = self.shell("dumpsys", "activity", "activities")
        (self.out / (name + "-activities.txt")).write_text(activities + "\n")
        state = activity_state(activities, pid)
        if self.identity:
            require_same_activity(self.identity, state)
        return state

    def capture(self, name, mode):
        state = self.current(name + "-before")
        if state["night"] != (mode == "yes"):
            raise AssertionError("Actual Activity Configuration has not reached requested night mode")
        png = self.command("exec-out", "screencap", "-p")
        width, height = png_dimensions(png)
        path = self.out / (name + ".png")
        path.write_bytes(png)
        # Crop only central app content, excluding status/navigation bars. No resize,
        # synthesized pixels, expected image, or OEM chrome color is accepted.
        crop_width, crop_height = int(width * .6), int(height * .6)
        crop = f"{crop_width}x{crop_height}+{int(width * .2)}+{int(height * .2)}"
        pixels = subprocess.run(["magick", str(path), "-crop", crop, "+repage", "-alpha", "off", "-depth", "8", "RGB:-"],
                                check=True, capture_output=True, timeout=15).stdout
        if len(pixels) != crop_width * crop_height * 3:
            raise AssertionError("ImageMagick did not return the complete actual RGB crop")
        ocr = subprocess.run(["tesseract", str(path), "stdout"], check=True, capture_output=True, timeout=15).stdout.decode(errors="strict")
        (self.out / (name + ".txt")).write_text(ocr)
        fractions = background_fractions(pixels)
        observation = {"name": name, "mode": mode, "state": state, "crop": crop,
                       "png_sha256": hashlib.sha256(png).hexdigest(), "background": fractions}
        self.observations.append(observation)
        (self.out / "observations.json").write_text(json.dumps(self.observations, indent=2) + "\n")
        self.current(name + "-after")
        if self.events is not None:
            event_delta(self.events, self.events_snapshot(name))
        require_onboarding(ocr)
        require_background(fractions, mode)
        return observation

    def verify_caller(self):
        uid = self.shell("id", "-u")
        (self.out / "shell-caller-uid.txt").write_text(uid + "\n")
        if uid != "2000":
            raise AssertionError("Require actual adb shell UID 2000, not root or another UID")
        raw = self.shell("dumpsys", "package", "com.android.shell")
        (self.out / "shell-package-permissions.txt").write_text(raw + "\n")
        self.capability = shell_capability(uid, raw)
        self.metadata["verified_caller"] = self.capability

    def snapshot(self, name="original"):
        mode = night_mode(self.shell("cmd", "uimode", "night"))
        raw_service = self.shell("dumpsys", "uimode")
        # Keep actual unsupported layouts for diagnosis without any mutation.
        (self.out / (name + "-uimode.txt")).write_text(raw_service + "\n")
        state = service_state(raw_service, self.capability)
        expected_mode, expected_bits = ("2", 0x20) if mode == "yes" else ("1", 0x10)
        try:
            current_bits = int(state["mCurUiMode"], 0) & 0x30
        except ValueError as exc:
            raise AssertionError("Unknown effective uimode encoding") from exc
        if state["mNightMode"] != expected_mode or current_bits != expected_bits:
            raise AssertionError("Night command/service/effective state is inconsistent")
        settings = night_settings(self.shell("settings", "list", "secure"))
        return {"mode": mode, "service": state, "settings": settings}

    def exercise(self):
        if not re.fullmatch(r"emulator-\d+", self.args.run):
            raise AssertionError("Explicit emulator serial required before any device operation")
        if self.shell("getprop", "ro.kernel.qemu") != "1":
            raise AssertionError("Refusing non-emulator")
        if self.shell("am", "get-current-user") != "0":
            raise AssertionError("Only explicit primary user 0 is supported")
        if "mWakefulness=Awake" not in self.shell("dumpsys", "power"):
            raise AssertionError("Require already-awake emulator, no wake/resume workaround")
        ime = self.shell("dumpsys", "input_method")
        if "mInputShown=false" not in ime or "mInputShown=true" in ime:
            raise AssertionError("Require keyboard already hidden, never inject Back to conceal lifecycle")
        self.verify_caller()
        self.metadata["sdk"] = self.shell("getprop", "ro.build.version.sdk")
        apk = self.shell("pm", "path", PACKAGE)
        if not re.fullmatch(r"package:/[^\n]+\.apk", apk):
            raise AssertionError("Require one explicit installed production base APK")
        path = apk.removeprefix("package:")
        digest = self.shell("sha256sum", path)
        if not re.fullmatch(r"[a-f0-9]{64}  " + re.escape(path), digest):
            raise AssertionError("Cannot establish installed production APK digest")
        self.metadata.update({"installed_apk": path, "apk_sha256": digest[:64]})
        self.identity = self.current("preflight")
        self.events = self.events_snapshot("baseline")
        require_resume_evidence(self.events, self.identity["pid"])
        original = self.snapshot()
        self.capture("original", original["mode"])
        # Save a complete durable recovery record BEFORE the first mutation.
        self.original = original
        (self.out / "original-settings.json").write_text(json.dumps(original, indent=2) + "\n")
        opposite = "no" if original["mode"] == "yes" else "yes"
        for index, mode in enumerate((opposite, original["mode"]), 1):
            self.changed = True
            self.shell("cmd", "uimode", "night", mode)
            if night_mode(self.shell("cmd", "uimode", "night")) != mode:
                raise AssertionError("Unsupported/ignored night-mode setting")
            time.sleep(self.args.settle_seconds)
            self.capture(f"transition-{index}-{mode}", mode)
        self.current("completed")
        delta = event_delta(self.events, self.events_snapshot("completed"))
        self.metadata["observed_event_delta_lines"] = len(delta)

    def restore(self):
        if not self.changed or self.original is None:
            return
        errors = []
        try:
            self.shell("cmd", "uimode", "night", self.original["mode"])
        except Exception as exc:
            errors.append("restore service command: " + str(exc))
        try:
            current = night_settings(self.shell("settings", "list", "secure"))
            for key in sorted(set(current) | set(self.original["settings"])):
                value = self.original["settings"].get(key)
                if current.get(key) == value:
                    continue
                try:
                    if value is None:
                        self.shell("settings", "delete", "secure", key)
                    else:
                        self.shell("settings", "put", "secure", key, value)
                except Exception as exc:
                    errors.append("restore raw " + key + ": " + str(exc))
        except Exception as exc:
            errors.append("read restoration settings: " + str(exc))
        try:
            restored = self.snapshot("restored")
            (self.out / "restored-settings.json").write_text(json.dumps(restored, indent=2) + "\n")
            if restored != self.original:
                raise AssertionError("Exact night-mode service/raw settings restoration mismatch")
            self.current("restored")
            event_delta(self.events, self.events_snapshot("restored"))
        except Exception as exc:
            errors.append("verify restoration: " + str(exc))
        if errors:
            raise AssertionError("; ".join(errors))

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
                      "settings_changed": self.changed, "theme_only": True,
                      "font_scale_accepted": False, "reduce_motion_accepted": False,
                      "full_parity": False, "release_accepted": False,
                      "limits": ["Sampled dumpsys plus continuous retained OS event-buffer evidence, not a native callback probe",
                                 "Explicit yes/no modes only; auto/custom/active overrides/UI lock/unverified night lock fail before mutation",
                                 "No font-scale or reduced-motion mutation/acceptance", "No authentication, account, model, network or physical-device acceptance"]}
            (self.out / "result.json").write_text(json.dumps(result, indent=2) + "\n")
        if error:
            raise SystemExit(error)
        print("PASS: actual foreground GPUI light/dark pixels and onboarding OCR, same Activity/process, no observed lifecycle transition, exact night-state restoration. Theme only.")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--run", required=True, metavar="emulator-SERIAL")
    parser.add_argument("--out", required=True, type=Path, help="New private local evidence directory")
    parser.add_argument("--settle-seconds", type=float, default=1.5)
    args = parser.parse_args()
    if not re.fullmatch(r"emulator-\d+", args.run) or not 0.5 <= args.settle_seconds <= 3:
        parser.error("Require explicit emulator serial and settle interval 0.5..3 seconds")
    def interrupted(signum, frame):
        raise KeyboardInterrupt("Signal " + str(signum))
    signal.signal(signal.SIGTERM, interrupted)
    Acceptance(args).run()


if __name__ == "__main__":
    main()
