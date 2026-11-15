#!/usr/bin/env python3
"""Partial TEST-ONLY release-profile onboarding acceptance, run by device owner only.

Installs the explicit APK on an explicit disposable emulator, without clearing
app data. Never taps authentication actions or sends email/model requests.
Requires SDK build-tools, NDK llvm-readelf, tesseract and ImageMagick. Screenshots
and OCR stay in the local output directory. This is NOT full parity acceptance.
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
import zipfile

PACKAGE = "org.jcode.mobile"
COMPONENT = PACKAGE + "/.JcodeActivity"
DRAFT = "releasedraft"


def manifest_is_non_debug(xmltree):
    # aapt2 typed booleans: false, 0, or 0x00000000. Missing defaults false.
    for name in ("debuggable", "testOnly"):
        values = re.findall(r"android:" + name + r"\([^)]*\)\s*=\s*([^\s]+)", xmltree)
        if any(v.lower() not in ("false", "0", "0x00000000", "0x0") for v in values):
            return False
        if name in xmltree and not values:
            return False
    return 'package="org.jcode.mobile"' in xmltree


def package_is_non_debug(dump):
    # Require the real package flags field, not merely absence of a substring.
    flags = re.findall(r"^\s*(?:pkgFlags|flags)=\[([^\]]*)\]", dump, re.MULTILINE)
    return bool(flags) and all(not {"DEBUGGABLE", "TEST_ONLY"}.intersection(value.split()) for value in flags)


def ime_top(window_dump):
    """Use actual IME content geometry, not its potentially full-screen window."""
    blocks = re.split(r"(?=\n\s*Window #\d+ Window\{)", window_dump)
    candidates = []
    for block in blocks:
        header = block.splitlines()[:3]
        if not any(re.search(r"\bInputMethod\}", line) for line in header):
            continue
        if "isOnScreen=false" in block or "isVisible=false" in block:
            continue
        insets = re.search(r"mGivenContentInsets=\[(\d+),(\d+)\]\[(\d+),(\d+)\]", block)
        inset_top = int(insets.group(2)) if insets else 0
        for _, top, right, bottom in re.findall(
                r"(?:mFrame|\bframe)=\[(\d+),(\d+)\]\[(\d+),(\d+)\]", block):
            # IME windows may cover nearly the whole display with a transparent
            # upper area. Given content insets are window-relative, so the
            # occluding keyboard begins at frame.top + contentInset.top.
            content_top = int(top) + inset_top
            if int(bottom) > content_top > 0 and int(right) > 0:
                candidates.append(content_top)
    if not candidates:
        raise AssertionError("No visible InputMethod frame found in window dump")
    return min(candidates)


def kernel_page_bytes(observation):
    """Parse one actual /proc/self/smaps observation, never infer from SDK."""
    match = re.fullmatch(r"KernelPageSize:[ \t]+([0-9]+)[ \t]+kB", observation.strip())
    if not match or int(match.group(1)) <= 0:
        raise AssertionError("Cannot establish actual kernel page size")
    return str(int(match.group(1)) * 1024)


def onboarding(text):
    return "jcode" in text.lower() and "google" in text.lower() and "apple" not in text.lower()


def draft_rows(words):
    return [w for w in words if DRAFT in re.sub(r"[^a-z]", "", w["text"].lower())]


class Runtime:
    def __init__(self, args):
        self.args = args
        self.out = args.out.resolve()
        self.out.mkdir(parents=True, exist_ok=False)  # Never mix stale evidence.
        sdk = Path(os.environ.get("ANDROID_HOME", os.environ.get("ANDROID_SDK_ROOT", Path.home() / "Android/Sdk")))
        self.adb_path = sdk / "platform-tools/adb"
        self.tools = sdk / "build-tools" / os.environ.get("ANDROID_BUILD_TOOLS", "35.0.0")
        self.ndk = Path(os.environ.get("ANDROID_NDK_HOME", sdk / "ndk/27.2.12479018")) / "toolchains/llvm/prebuilt/linux-x86_64/bin"
        self.checks = []
        self.settings = {}
        self.validated = False
        self.installed = False
        self.metadata = {"serial": args.serial, "apk": str(args.apk.resolve()),
                         "scope": "TEST-ONLY release onboarding, no authentication actions"}

    def command(self, argv, timeout=45):
        argv = [str(a) for a in argv]
        entry = {"argv": argv, "time_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())}
        try:
            p = subprocess.run(argv, capture_output=True, timeout=timeout)
            entry.update(returncode=p.returncode, stderr=p.stderr.decode(errors="replace"))
            if "screencap" not in argv:
                entry["stdout"] = p.stdout.decode(errors="replace")
            p.check_returncode()
            return p.stdout
        except BaseException as error:
            entry["error"] = str(error)
            raise
        finally:
            with (self.out / "commands.jsonl").open("a") as f:
                f.write(json.dumps(entry) + "\n")

    def adb(self, *args):
        return self.command([self.adb_path, "-s", self.args.serial, *args])

    def text(self, *args):
        return self.adb(*args).decode(errors="replace").strip()

    def check(self, name, condition):
        self.checks.append({"name": name, "passed": bool(condition)})
        print(("PASS " if condition else "FAIL ") + name, flush=True)
        if not condition:
            raise AssertionError(name)

    def capture(self, name):
        path = self.out / (name + ".png")
        data = self.adb("exec-out", "screencap", "-p")
        self.check("Valid PNG " + name, data.startswith(b"\x89PNG\r\n\x1a\n"))
        path.write_bytes(data)
        tsv = self.command(["tesseract", path, "stdout", "tsv"], 30).decode()
        (self.out / (name + ".tsv")).write_text(tsv)
        words = [w for w in csv.DictReader(io.StringIO(tsv), delimiter="\t") if w.get("text", "").strip()]
        text = " ".join(w["text"] for w in words)
        (self.out / (name + ".txt")).write_text(text + "\n")
        return text, words

    def launch(self, demo=False):
        args = ["shell", "am", "start", "-W", "-n", COMPONENT]
        if demo:
            args += ["--es", "gpui_demo", "todos"]
        response = self.text(*args)
        self.check("Activity launch succeeded", "Error:" not in response and "Status: ok" in response)
        time.sleep(3)

    def fresh(self, demo=False):
        self.adb("shell", "am", "force-stop", PACKAGE)
        self.launch(demo)

    def keyboard(self, shown):
        deadline = time.monotonic() + 15
        stable = 0
        while time.monotonic() < deadline:
            state = self.text("shell", "dumpsys", "input_method")
            (self.out / ("ime-shown.txt" if shown else "ime-hidden.txt")).write_text(state)
            stable = stable + 1 if ("mInputShown=" + str(shown).lower()) in state else 0
            if stable >= 3:
                return
            time.sleep(0.3)
        raise AssertionError("IME did not reach stable " + ("shown" if shown else "hidden") + " state")

    def dismiss_observed_keyboard(self):
        # NativeActivity can restore email focus independently on each fresh
        # launch. Hide only an observed IME, then wait for its exit animation
        # before exact onboarding OCR. Never relax the required Jcode proof.
        if "mInputShown=true" in self.text("shell", "dumpsys", "input_method"):
            self.adb("shell", "input", "keyevent", "KEYCODE_BACK")
            self.keyboard(False)
            time.sleep(1)

    def preflight(self):
        self.check("Explicit emulator serial", bool(re.fullmatch(r"emulator-\d+", self.args.serial)))
        self.check("Explicit APK exists", self.args.apk.is_file())
        self.metadata["apk_sha256"] = hashlib.sha256(self.args.apk.read_bytes()).hexdigest()
        manifest = self.command([self.tools / "aapt2", "dump", "xmltree", self.args.apk, "--file", "AndroidManifest.xml"]).decode()
        (self.out / "apk-manifest.txt").write_text(manifest)
        self.check("APK effective debuggable and testOnly are false", manifest_is_non_debug(manifest))
        signature = self.command([self.tools / "apksigner", "verify", "--verbose", "--print-certs", self.args.apk]).decode()
        (self.out / "apk-signature.txt").write_text(signature)
        with zipfile.ZipFile(self.args.apk) as archive:
            for abi in ("arm64-v8a", "x86_64"):
                lib = self.out / (abi + ".so")
                lib.write_bytes(archive.read("lib/" + abi + "/libjcode_mobile.so"))
                symbols = self.command([self.ndk / "llvm-readelf", "--dyn-syms", "--wide", lib]).decode()
                (self.out / (abi + "-symbols.txt")).write_text(symbols)
                self.check("Native bootstrap and no debug JNI export " + abi,
                           "ANativeActivity_onCreate" in symbols and "nativeConfigureDebugDemo" not in symbols)
            dex = self.out / "classes.dex"
            dex.write_bytes(archive.read("classes.dex"))
        dis = self.command([self.tools / "dexdump", "-d", dex]).decode()
        (self.out / "dexdump.txt").write_text(dis)
        self.check("Debug Intent branch absent from DEX", "gpui_demo" not in dis and not re.search(r"invoke-.*nativeConfigureDebugDemo", dis))

    def observe_page_size(self, sdk):
        if sdk == "26":
            observation = self.text("shell", "grep", "-m", "1", "KernelPageSize", "/proc/self/smaps")
            (self.out / "kernel-page-size.txt").write_text(observation + "\n")
            self.metadata["page_size_source"] = "/proc/self/smaps first KernelPageSize"
            return kernel_page_bytes(observation)
        observation = self.text("shell", "getconf", "PAGE_SIZE")
        if not re.fullmatch(r"[0-9]+", observation) or int(observation) <= 0:
            raise AssertionError("Invalid getconf PAGE_SIZE observation")
        self.metadata["page_size_source"] = "getconf PAGE_SIZE"
        return observation

    def exercise(self):
        self.preflight()
        self.check("Device is emulator", self.text("shell", "getprop", "ro.kernel.qemu") == "1")
        self.validated = True
        sdk = self.text("shell", "getprop", "ro.build.version.sdk")
        self.check("Valid observed Android SDK", bool(re.fullmatch(r"[0-9]+", sdk)) and int(sdk) >= 26)
        self.metadata.update(sdk=sdk, page_size=self.observe_page_size(sdk))
        for key in ("user_rotation", "accelerometer_rotation"):
            value = self.text("shell", "settings", "get", "system", key)
            self.check("Valid original setting " + key, value == "null" or bool(re.fullmatch(r"[0-3]", value)))
            self.settings[key] = value
        self.adb("shell", "settings", "put", "system", "accelerometer_rotation", "0")
        self.adb("shell", "settings", "put", "system", "user_rotation", "0")
        self.adb("install", "-r", str(self.args.apk.resolve()))
        self.installed = True
        package = self.text("shell", "dumpsys", "package", PACKAGE)
        (self.out / "installed-package.txt").write_text(package)
        self.check("Installed package is non-debug", package_is_non_debug(package))
        self.adb("shell", "wm", "dismiss-keyguard")
        self.fresh()
        self.dismiss_observed_keyboard()
        text, words = self.capture("onboarding")
        # Do not guess permissions or account state. Unexpected dialogs/auth state fail closed.
        self.check("Shared onboarding with no Apple sign-in", onboarding(text))
        self.fresh(demo=True)
        self.dismiss_observed_keyboard()
        text, words = self.capture("release-demo-ignored")
        self.check("Fresh release process ignores gpui_demo=todos", onboarding(text) and "progress" not in text.lower())
        pid = self.text("shell", "pidof", PACKAGE).split()[0]
        self.metadata["native_pid"] = pid
        logs = self.text("logcat", "--pid", pid, "-d")
        (self.out / "bootstrap-logcat.txt").write_text(logs)
        self.check("Actual shared GPUI native bootstrap", "shared GPUI NativeActivity bootstrap" in logs)
        self.check("No native bootstrap crash", "Fatal signal" not in logs and "FATAL EXCEPTION" not in logs)
        fields = [w for w in words if "example.com" in w["text"].lower()]
        self.check("Exactly one visible email placeholder", len(fields) == 1)
        field = fields[0]
        self.adb("shell", "input", "tap", str(int(field["left"]) + int(field["width"]) // 2), str(int(field["top"]) + int(field["height"]) // 2))
        self.keyboard(True)
        self.adb("shell", "input", "text", DRAFT)  # Invalid email, never tap Send/code/Google.
        time.sleep(2)
        text, words = self.capture("email-keyboard")
        windows = self.text("shell", "dumpsys", "window", "windows")
        (self.out / "keyboard-windows.txt").write_text(windows)
        top = ime_top(windows)
        self.check("Actual email field above keyboard contains draft", any(int(w["top"]) + int(w["height"]) < top for w in draft_rows(words)))
        actions = [w for w in words if w["text"].lower() in ("send", "code") and int(w["top"]) + int(w["height"]) < top]
        if not {"send", "code"}.issubset({w["text"].lower() for w in actions}):
            # OCR real low-contrast pixels above IME, never fabricate an expected label.
            png = (self.out / "email-keyboard.png").read_bytes()
            width = int.from_bytes(png[16:20], "big")
            draft = next(w for w in draft_rows(words) if int(w["top"]) + int(w["height"]) < top)
            y = int(draft["top"]) + int(draft["height"]) + 10
            self.check("Space for actual email action above IME", top > y + 10)
            crop = self.out / "email-action.png"
            self.command(["magick", self.out / "email-keyboard.png", "-crop", f"{int(width*.65)}x{top-y}+{int(width*.175)}+{y}", "+repage", "-resize", "200%", crop])
            action = self.command(["tesseract", crop, "stdout", "--psm", "6"]).decode()
            (self.out / "email-action.txt").write_text(action)
            self.check("Email action visibly avoids keyboard", bool(re.search(r"send\s+code", action, re.I)))
        else:
            self.check("Email action visibly avoids keyboard", True)
        self.adb("shell", "input", "keyevent", "KEYCODE_BACK")
        self.keyboard(False)
        text, words = self.capture("email-back")
        self.check("Back preserves email draft and activity", bool(draft_rows(words)) and self.text("shell", "pidof", PACKAGE).split() == [pid])
        self.adb("shell", "input", "keyevent", "KEYCODE_HOME")
        time.sleep(1)
        self.launch()
        text, words = self.capture("email-resume")
        self.check("Email draft survives home and resume", bool(draft_rows(words)))
        self.adb("shell", "settings", "put", "system", "accelerometer_rotation", "0")
        self.adb("shell", "settings", "put", "system", "user_rotation", "1")
        time.sleep(3)
        text, words = self.capture("email-landscape")
        data = (self.out / "email-landscape.png").read_bytes()
        self.check("Landscape actually applied", int.from_bytes(data[16:20], "big") > int.from_bytes(data[20:24], "big"))
        self.check("Email draft survives rotation", bool(draft_rows(words)))
        self.adb("shell", "settings", "put", "system", "user_rotation", "0")
        time.sleep(3)
        text, words = self.capture("email-portrait")
        data = (self.out / "email-portrait.png").read_bytes()
        self.check("Portrait actually applied", int.from_bytes(data[20:24], "big") > int.from_bytes(data[16:20], "big"))
        self.check("Email draft survives return to portrait", bool(draft_rows(words)))
        logs = self.text("logcat", "--pid", pid, "-d")
        (self.out / "final-logcat.txt").write_text(logs)
        self.check("Native activity remains alive without crash", self.text("shell", "pidof", PACKAGE).split() == [pid] and "Fatal signal" not in logs and "FATAL EXCEPTION" not in logs)

    def run(self):
        error = None
        try:
            self.exercise()
        except (Exception, KeyboardInterrupt) as caught:
            error = repr(caught)
            print(error, file=sys.stderr)
        finally:
            if self.validated:
                # Stop only the explicitly installed target, never other apps/emulators.
                if self.installed:
                    try:
                        self.adb("shell", "am", "force-stop", PACKAGE)
                    except Exception as caught:
                        error = error or repr(caught)
                        self.checks.append({"name": "Cleanup target process", "passed": False, "error": repr(caught)})
                for key, value in self.settings.items():
                    try:
                        if value == "null":
                            self.adb("shell", "settings", "delete", "system", key)
                        else:
                            self.adb("shell", "settings", "put", "system", key, value)
                        restored = self.text("shell", "settings", "get", "system", key)
                        if restored != value:
                            raise AssertionError("Setting restore mismatch: " + key)
                        self.checks.append({"name": "Restore " + key, "passed": True})
                    except Exception as caught:
                        error = error or repr(caught)
                        self.checks.append({"name": "Restore " + key, "passed": False, "error": repr(caught)})
            result = {**self.metadata, "checks": self.checks, "error": error,
                      "passed": error is None and bool(self.checks) and all(c["passed"] for c in self.checks),
                      "full_parity": False, "release_accepted": False,
                      "remaining": ["authenticated chat and cloud workflows", "composing IME/selection", "TalkBack", "physical ARM64", "real microphone/speaker round-trip", "production signing and Play publication"],
                      "side_effects": {"apk_install_completed": self.installed,
                                       "data_cleared": False, "auth_actions_clicked": False,
                                       "rotation_restore_attempted": bool(self.settings),
                                       "target_stop_attempted": self.installed}}
            (self.out / "result.json").write_text(json.dumps(result, indent=2) + "\n")
        return 0 if result["passed"] else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--serial", required=True, help="Explicit disposable emulator-NNNN serial")
    parser.add_argument("--apk", required=True, type=Path, help="Explicit non-debug release-profile TEST-ONLY APK")
    parser.add_argument("--out", required=True, type=Path, help="New local evidence directory, must not exist")
    args = parser.parse_args()
    return Runtime(args).run()


if __name__ == "__main__":
    sys.exit(main())
