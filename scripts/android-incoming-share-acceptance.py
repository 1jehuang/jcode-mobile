#!/usr/bin/env python3
"""Root-run Android draft-share acceptance, debug offline probe only.

No adb command runs without --run emulator-SERIAL. Requires an explicit debug
APK containing the incoming-share-state-v1 read-only shared-state probe. A missing
probe fails BEFORE launching the app. Never clears app data, authenticates, taps
Send, or uses text-presence/OCR as evidence of submission state. Source bridge
contract is documented in android/tests/incoming-share-state.py.
"""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import shlex
import subprocess
import time
import zipfile

ROOT = Path(__file__).resolve().parents[1]
PACKAGE = "org.jcode.mobile"
COMPONENT = PACKAGE + "/.JcodeActivity"
STATE_FILE = "files/incoming-share-state.json"
PROBE_MARKER = b"incoming-share-state-v1"
spec = importlib.util.spec_from_file_location("share_state", ROOT / "android/tests/incoming-share-state.py")
state_checks = importlib.util.module_from_spec(spec)
spec.loader.exec_module(state_checks)


def require_probe(apk):
    with zipfile.ZipFile(apk) as archive:
        libraries = [n for n in archive.namelist() if n.startswith("lib/") and n.endswith("/libjcode_mobile.so")]
        if not libraries or any(PROBE_MARKER not in archive.read(name) for name in libraries):
            raise AssertionError("APK lacks offline shared-state probe. Refusing launch, not an acceptance pass.")


def shared_resumed(activities):
    markers = re.findall(r"^\s*topResumedActivity\s*[:=]\s*(.*)$", activities, re.MULTILINE)
    markers = markers or re.findall(r"^\s*mResumedActivity\s*[:=]\s*(.*)$", activities, re.MULTILINE)
    component = r"(?<![\w.])org\.jcode\.mobile/(?:\.JcodeActivity|org\.jcode\.mobile\.JcodeActivity)(?![\w.$])"
    return bool(markers) and all(re.search(component, marker) for marker in markers)


class Acceptance:
    def __init__(self, args):
        self.args = args
        self.out = args.out.resolve()
        self.out.mkdir(parents=True, exist_ok=True)
        sdk = Path(os.environ.get("ANDROID_HOME", os.environ.get("ANDROID_SDK_ROOT", Path.home() / "Android/Sdk")))
        self.adb_path = str(sdk / "platform-tools/adb")
        self.checks = []
        self.observations = []
        self.can_capture = False
        self.sleep_requested = False

    def command(self, *args, allow_failure=False):
        result = subprocess.run([self.adb_path, "-s", self.args.run, *args], capture_output=True, timeout=45)
        with (self.out / "commands.jsonl").open("a") as log:
            log.write(json.dumps({"args": args, "code": result.returncode,
                                  "stderr": result.stderr.decode(errors="replace")}) + "\n")
        if not allow_failure:
            result.check_returncode()
        return result

    def shell(self, *args, allow_failure=False):
        # adb shell joins arguments. Quote the complete remote command so shared
        # spaces, ampersands, newlines and Unicode cannot become shell syntax.
        return self.command("shell", shlex.join(args), allow_failure=allow_failure)

    def text(self, *args):
        return self.shell(*args).stdout.decode(errors="strict").strip()

    def check(self, name, condition):
        self.checks.append({"name": name, "passed": bool(condition)})
        if not condition:
            raise AssertionError(name)
        print("PASS " + name, flush=True)

    def capture(self, name):
        (self.out / (name + ".png")).write_bytes(self.command("exec-out", "screencap", "-p").stdout)
        (self.out / (name + "-activities.txt")).write_text(self.text("dumpsys", "activity", "activities"))

    def state(self):
        result = self.shell("run-as", PACKAGE, "cat", STATE_FILE, allow_failure=True)
        if result.returncode:
            return None
        # Atomic app-side rename is required. Invalid JSON/state fails closed,
        # never silently treated as an expected startup delay.
        current = state_checks.read_state(result.stdout.decode())
        self.observations.append({"at_monotonic": time.monotonic(), "state": current})
        with (self.out / "states.jsonl").open("a") as evidence:
            evidence.write(json.dumps(self.observations[-1], ensure_ascii=False) + "\n")
        return current

    def await_draft(self, payload, previous=None):
        deadline = time.monotonic() + 15
        while time.monotonic() < deadline:
            current = self.state()
            if current is not None and current["composer"] == payload:
                state_checks.require_draft(current, payload, previous)
                self.check("Actual draft equals exact delivered text, with no auto-send", True)
                return current
            time.sleep(0.1)
        raise AssertionError("Actual GPUI draft state unavailable or did not equal delivered share")

    def observe_unchanged(self, before, seconds):
        deadline = time.monotonic() + seconds
        newest = before
        while time.monotonic() < deadline:
            current = self.state()
            if current is None:
                raise AssertionError("Shared state disappeared during no-submit observation")
            # Require fields unchanged on every observation. Freshness is checked
            # separately at the end, because reads can straddle a single frame.
            for key in ("activeTab", "tabCount", "composer", "submittedCount"):
                if current[key] != before[key]:
                    raise AssertionError("Consumed share replay/submission changed " + key)
            newest = current
            time.sleep(0.1)
        state_checks.require_unchanged(before, newest)
        return newest

    def send(self, payload):
        result = self.text("am", "start", "-W", "-n", COMPONENT,
                           "-a", "android.intent.action.SEND", "-t", "text/plain",
                           "--es", "android.intent.extra.TEXT", payload,
                           "--es", "gpui_demo", "todos", "--ez", "gpui_share_probe", "true")
        if "Error:" in result or "Exception" in result:
            raise AssertionError("Android did not accept ACTION_SEND: " + result)

    def exercise(self):
        # Static marker is a safety preflight only, not evidence that the app
        # actually remained offline. Every observed state must also prove that.
        require_probe(self.args.apk)
        self.check("Explicit serial names an emulator", self.args.run.startswith("emulator-")
                   and self.text("getprop", "ro.kernel.qemu") == "1")
        self.can_capture = True
        self.command("install", "-r", str(self.args.apk.resolve()))
        package = self.text("dumpsys", "package", PACKAGE)
        flags = [line for line in package.splitlines() if "pkgFlags=[" in line or "flags=[" in line]
        self.check("Installed target is explicitly debuggable", any("DEBUGGABLE" in line for line in flags))
        self.check("Emulator starts awake", "mWakefulness=Awake" in self.text("dumpsys", "power"))
        self.shell("am", "force-stop", PACKAGE)
        # Delete only this harness's previous probe artifact, not application data.
        self.shell("run-as", PACKAGE, "rm", "-f", STATE_FILE)
        cold_text = "sharecold + &draft=0 # % ✓"
        self.send(cold_text)
        cold = self.await_draft(cold_text)
        self.check("Cold share creates exactly one extra tab and submits nothing",
                   cold["tabCount"] == 2 and cold["submittedCount"] == 0)
        cold = self.observe_unchanged(cold, self.args.observe_seconds)
        self.capture("cold-draft-no-submit")
        warm_text = "sharewarm https://example.invalid/?a=1&b=2 ✓"
        self.send(warm_text)
        warm = self.await_draft(warm_text, cold)
        warm = self.observe_unchanged(warm, self.args.observe_seconds)
        self.capture("warm-draft-no-submit")
        # Sleep/wake resumes the existing Intent, unlike starting MAIN, which
        # could replace it and conceal an ACTION_SEND replay defect.
        self.sleep_requested = True
        self.shell("input", "keyevent", "KEYCODE_SLEEP")
        deadline = time.monotonic() + 8
        while time.monotonic() < deadline:
            if "mWakefulness=Asleep" in self.text("dumpsys", "power"):
                break
            time.sleep(0.1)
        else:
            raise AssertionError("Emulator never slept; consume-once lifecycle not established")
        self.shell("input", "keyevent", "KEYCODE_WAKEUP")
        self.shell("wm", "dismiss-keyguard")
        self.check("Emulator awake after existing-intent resume", "mWakefulness=Awake" in self.text("dumpsys", "power"))
        self.sleep_requested = False
        deadline = time.monotonic() + 8
        while time.monotonic() < deadline:
            if shared_resumed(self.text("dumpsys", "activity", "activities")):
                break
            time.sleep(0.1)
        else:
            raise AssertionError("Existing shared Activity did not resume after wake")
        resumed = self.observe_unchanged(warm, self.args.observe_seconds)
        state_checks.require_unchanged(warm, resumed)
        self.check("Existing-intent resume neither duplicates tabs nor submits", True)
        self.capture("resumed-consume-once")

    def run(self):
        error = None
        try:
            self.exercise()
        except (Exception, KeyboardInterrupt) as exc:
            error = f"{type(exc).__name__}: {exc}"
            if self.can_capture:
                try:
                    self.capture("failure")
                except Exception as capture_error:
                    error += "; evidence capture: " + str(capture_error)
        finally:
            if self.sleep_requested:
                try:
                    self.shell("input", "keyevent", "KEYCODE_WAKEUP")
                    self.shell("wm", "dismiss-keyguard")
                except Exception as cleanup_error:
                    error = (error or "Wake restoration failed") + "; cleanup: " + str(cleanup_error)
            result = {"passed": error is None, "error": error, "checks": self.checks,
                      "apk_sha256": hashlib.sha256(self.args.apk.read_bytes()).hexdigest(),
                      "serial": self.args.run, "observations": len(self.observations),
                      "observation_seconds_each": self.args.observe_seconds,
                      "full_parity": False, "release_accepted": False,
                      "scope": "Debug offline shared draft state only, no auth or model sends",
                      "remaining": ["release real-account sharing", "process-death restoration", "physical device"]}
            (self.out / "result.json").write_text(json.dumps(result, indent=2) + "\n")
        if error:
            raise SystemExit(error)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--run", required=True, metavar="emulator-SERIAL", help="Explicit root-owned disposable emulator")
    parser.add_argument("--apk", required=True, type=Path)
    parser.add_argument("--out", required=True, type=Path)
    parser.add_argument("--observe-seconds", type=float, default=5)
    args = parser.parse_args()
    if not args.run.startswith("emulator-") or not args.apk.is_file() or args.observe_seconds < 2:
        parser.error("Require emulator serial, existing debug APK and observation >=2 seconds")
    Acceptance(args).run()


if __name__ == "__main__":
    main()
