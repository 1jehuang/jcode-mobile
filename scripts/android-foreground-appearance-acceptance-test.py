#!/usr/bin/env python3
"""Pure synthetic tests for foreground evidence and cleanup. Never invoke adb."""
import argparse
import copy
import importlib.util
import json
import os
from pathlib import Path
import shlex
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("foreground_appearance", Path(__file__).with_name("android-foreground-appearance-acceptance.py"))
m = importlib.util.module_from_spec(spec)
spec.loader.exec_module(m)


def activities(token="abc", pid="1234", night=False, launches="1"):
    return f"""ACTIVITY MANAGER ACTIVITIES
Display #0:
  * Task{{aaa #42 type=standard}}
    topResumedActivity=ActivityRecord{{{token} u0 org.jcode.mobile/.JcodeActivity t42}}
    * Hist #0: ActivityRecord{{{token} u0 org.jcode.mobile/.JcodeActivity t42}}
      app=ProcessRecord{{def {pid}:org.jcode.mobile/u0a123}}
      CurrentConfiguration={{1.0 port {'night' if night else ''} finger}}
      launchFailed=false launchCount={launches} lastLaunchTime=-1s
      state=RESUMED delayedResume=false finishing=false
  * Task{{aaa #1 type=home}}
    * Hist #0: ActivityRecord{{ffff u0 other.app/.Home t1}}
      app=ProcessRecord{{aaa 900:other.app/u0a1}}
      CurrentConfiguration={{1.0}}
      launchCount=9
      state=STOPPED finishing=false
"""


def service():
    return "mNightMode=1 (no)\nmNightModeLocked=false mUiModeLocked=false\nmComputedNightMode=false\nmOverrideNightModeOn=false mOverrideNightModeOff=false\nmCurUiMode=0x11\n"


def event(index, tag="notification_enqueue", data="[other.app,1]", pid=900):
    return f"1794730000.{index:03d} {pid:5d} {pid:5d} I {tag}: {data}"


def baseline():
    lines = [event(1, "wm_on_resume_called", "[0,123,org.jcode.mobile.JcodeActivity,RESUME_ACTIVITY]", 1234)]
    lines.extend(event(i) for i in range(2, 11))
    return "--------- beginning of events\n" + "\n".join(lines) + "\n"


class Parsing(unittest.TestCase):
    def test_explicit_modes_only(self):
        for value in ("yes", "no"):
            self.assertEqual(m.night_mode("Night mode: " + value + "\n"), value)
        for value in ("", "yes", "Night mode: auto", "Night mode: custom", "Night mode: no\nNight mode: yes", "Error: no permission"):
            with self.subTest(value=value), self.assertRaises(AssertionError):
                m.night_mode(value)

    def test_snapshot_only_bounded_night_secure_keys(self):
        self.assertEqual(m.night_settings("unrelated_secret=do-not-save\nui_night_mode=1\nui_night_mode_override_on=0\ndark_theme_custom_start_time=72000000\n"),
                         {"ui_night_mode": "1", "ui_night_mode_override_on": "0", "dark_theme_custom_start_time": "72000000"})
        self.assertEqual(m.night_settings("unrelated=anything"), {})
        for value in ("ui_night_mode=unknown", "ui_night_mode=1\nui_night_mode=2", "ui_night_mode", "ui_night_mode_X=1", "ui_night_mode=1; reboot"):
            with self.subTest(value=value), self.assertRaises(AssertionError):
                m.night_settings(value)

    def test_service_snapshot_requires_complete_unlocked_unoverridden(self):
        self.assertEqual(m.service_state(service())["mCurUiMode"], "0x11")
        for value in ("", service() + "mNightMode=2", service().replace("mNightModeLocked=false", "mNightModeLocked=true"),
                      service().replace("mOverrideNightModeOn=false", "mOverrideNightModeOn=true"), service().replace("mCurUiMode=0x11", "")):
            with self.subTest(value=value), self.assertRaises(AssertionError):
                m.service_state(value)

    def test_actual_paired_override_alias_has_same_canonical_state(self):
        paired = service().replace("mOverrideNightModeOn=false mOverrideNightModeOff=false", "mOverrideOn/Off=false/false")
        self.assertEqual(m.service_state(paired), m.service_state(service()))
        self.assertEqual(m.service_state(paired)["mOverrideNightModeOn"], "false")
        self.assertEqual(m.service_state(paired)["mOverrideNightModeOff"], "false")

    def test_override_alias_never_defaults_missing_malformed_or_duplicate_flags(self):
        base = service().replace("mOverrideNightModeOn=false mOverrideNightModeOff=false", "")
        for flags in ("", "mOverrideOn/Off=false", "mOverrideOn/Off=0/0", "mOverrideOn/Off=FALSE/FALSE",
                      "mOverrideOn/Off=false/false/false", "mOverrideOn/Off=true/false", "mOverrideOn/Off=false/true",
                      "mOverrideOn/Off=false/false mOverrideOn/Off=false/false",
                      "mOverrideNightModeOn=false", "mOverrideNightModeOff=false",
                      "mOverrideNightModeOn=false mOverrideNightModeOff=unknown",
                      "mOverrideOn/Off= mOverrideNightModeOn=false mOverrideNightModeOff=false",
                      "mOverrideOn/Off mOverrideNightModeOn=false mOverrideNightModeOff=false",
                      "mOverrideOn/Off=false/false mOverrideNightModeOn",
                      "mOverrideOn/Off=false/false mOverrideNightModeOn=false mOverrideNightModeOff=false"):
            with self.subTest(flags=flags), self.assertRaises(AssertionError):
                m.service_state(base + flags)

    def test_actual_api35_layout_retains_locked_state_refusal(self):
        # Exact root-saved read-only service text, no screenshots or device access.
        actual = """Current UI Mode Service state:
  mDockState=0 mLastBroadcastState=0
 mStartDreamImmediatelyOnDock=true  mNightMode=1 (no)  mOverrideOn/Off=false/false  mAttentionModeThemeOverlay=1000 mNightModeLocked=true
  mCarModeEnabled=false (carModeApps=
 mWaitForDeviceInactive=false mComputedNightMode=false customStart=22:00 customEnd06:00 mCarModeEnableFlags=0 mEnableCarDockLaunch=false
  mCurUiMode=0x11 mUiModeLocked=false mSetUiMode=0x11
  mHoldingConfiguration=false mSystemReady=true
  mTwilightService.getLastTwilightState()=null
"""
        with self.assertRaisesRegex(AssertionError, "mode locked"):
            m.service_state(actual)
        # A synthetic unlocked variant tests the parser, not observed permission.
        parsed = m.service_state(actual.replace("mNightModeLocked=true", "mNightModeLocked=false"))
        self.assertEqual(parsed["mNightMode"], "1")
        self.assertEqual(parsed["mCurUiMode"], "0x11")

    def test_unknown_boolean_service_values_refused(self):
        for key in ("mNightModeLocked", "mUiModeLocked", "mComputedNightMode"):
            with self.subTest(key=key), self.assertRaises(AssertionError):
                m.service_state(service().replace(key + "=false", key + "=unknown"))

    def test_activity_exact_top_identity_and_config(self):
        before = m.activity_state(activities(), "1234")
        after = m.activity_state(activities(night=True), "1234")
        self.assertFalse(before["night"])
        self.assertTrue(after["night"])
        m.require_same_activity(before, after)
        for key in ("token", "task", "process_token", "pid", "launch_count"):
            changed = dict(after, **{key: "different"})
            with self.subTest(key=key), self.assertRaises(AssertionError):
                m.require_same_activity(before, changed)

    def test_historical_or_other_app_records_never_accepted(self):
        for value in (activities().replace("topResumedActivity", "historicalActivity"),
                      activities().replace("topResumedActivity=ActivityRecord{abc u0 org.jcode.mobile/.JcodeActivity", "topResumedActivity=ActivityRecord{abc u0 other.app/.Home"),
                      activities().replace(".JcodeActivity", ".JcodeActivityOther"),
                      activities().replace("state=RESUMED", "state=PAUSED"), activities().replace("finishing=false", "finishing=true"),
                      activities().replace("port  finger", "port ?night finger"), activities() + " topResumedActivity=null\n"):
            with self.subTest(value=value), self.assertRaises(AssertionError):
                m.activity_state(value, "1234")
        with self.assertRaises(AssertionError):
            m.activity_state(activities(pid="5000"), "1234")

    def test_events_require_real_current_pid_resume_log(self):
        m.require_resume_evidence(baseline(), "1234")
        for text, pid in (("", "1234"), (baseline(), "4321"), (baseline().replace("wm_on_resume_called", "wm_resume_activity"), "1234"),
                          (baseline().replace("org.jcode.mobile.JcodeActivity", "org.jcode.mobile.platformworkflowtests.WorkflowActivity"), "1234"),
                          (baseline() + "malformed output", "1234")):
            with self.subTest(text=text, pid=pid), self.assertRaises(AssertionError):
                m.require_resume_evidence(text, pid)

    def test_delta_requires_retained_unique_anchor(self):
        before = baseline()
        safe = event(11, "configuration_changed", "[0,32]")
        self.assertEqual(m.event_delta(before, before + safe + "\n"), [safe])
        self.assertEqual(m.event_delta(before, before), [])
        for after in ("", "\n".join(m.event_lines(before)[-7:]), before + before, before + "malformed line\n"):
            with self.subTest(after=after), self.assertRaises(AssertionError):
                m.event_delta(before, after)

    def test_every_target_lifecycle_transition_fails_even_same_end_state(self):
        tags = ("wm_on_resume_called", "wm_on_paused_called", "wm_on_destroy_called", "wm_on_create_called", "wm_on_start_called", "wm_on_stop_called",
                "wm_on_restart_called", "wm_on_top_resumed_lost_called", "wm_on_top_resumed_gained_called", "wm_relaunch_activity", "wm_pause_activity",
                "wm_set_resumed_activity", "am_proc_start", "am_proc_died")
        for tag in tags:
            with self.subTest(tag=tag), self.assertRaises(AssertionError):
                m.event_delta(baseline(), baseline() + event(11, tag, "[0,org.jcode.mobile/.JcodeActivity]", 1234) + "\n")
        with self.assertRaises(AssertionError):
            m.event_delta(baseline(), baseline() + event(11, "chatty", "identical events suppressed") + "\n")
        other = event(11, "wm_on_resume_called", "[org.jcode.mobile.platformworkflowtests/.WorkflowActivity]")
        self.assertEqual(m.event_delta(baseline(), baseline() + other + "\n"), [other])

    def test_real_rgb_thresholds_not_mean_or_system_bar_change(self):
        light = bytes([246, 246, 246]) * 100
        dark = b"\0\0\0" * 100
        m.require_background(m.background_fractions(light), "no")
        m.require_background(m.background_fractions(dark), "yes")
        for pixels, mode in ((light, "yes"), (dark, "no"), (bytes([123, 123, 123]) * 100, "yes"), (light[:210] + dark[:90], "no")):
            with self.subTest(mode=mode), self.assertRaises(AssertionError):
                m.require_background(m.background_fractions(pixels), mode)
        for pixels in (b"", b"\0", b"\0\0"):
            with self.assertRaises(AssertionError):
                m.background_fractions(pixels)

    def test_onboarding_requires_real_signin_words_not_generic_or_auth_in_progress(self):
        m.require_onboarding("Sign in to Jcode Cloud\nContinue with Google")
        for text in ("", "Jcode Google", "Core progress todos", "Sign in to Jcode Cloud Continue with Google Waiting for Google", "Sign in to Jcode Cloud Continue with Google Sending a code"):
            with self.subTest(text=text), self.assertRaises(AssertionError):
                m.require_onboarding(text)

    def test_png_header_dimensions_and_bounds(self):
        valid = b"\x89PNG\r\n\x1a\n" + (13).to_bytes(4, "big") + b"IHDR" + (1080).to_bytes(4, "big") + (1920).to_bytes(4, "big")
        self.assertEqual(m.png_dimensions(valid), (1080, 1920))
        for data in (b"", b"not a screenshot", valid[:20], valid[:16] + (0).to_bytes(4, "big") + valid[20:]):
            with self.assertRaises(AssertionError):
                m.png_dimensions(data)


class Campaign(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="foreground-pure-", dir=os.environ.get("JCODE_SCRATCH_DIR"))
        self.addCleanup(self.temp.cleanup)
        self.args = argparse.Namespace(run="emulator-5554", out=Path(self.temp.name) / "evidence", settle_seconds=.5)
        self.runner = m.Acceptance(self.args)
        # Any accidentally unmocked subprocess is a test failure, never adb execution.
        guard = patch.object(m.subprocess, "run", side_effect=AssertionError("Unmocked subprocess forbidden in pure test"))
        guard.start()
        self.addCleanup(guard.stop)

    def test_non_emulator_serial_fails_before_commands(self):
        self.args.run = "physical-device"
        with patch.object(self.runner, "command", side_effect=AssertionError("No device operation")) as command:
            with self.assertRaises(SystemExit):
                self.runner.run()
            command.assert_not_called()
        result = json.loads((self.args.out / "result.json").read_text())
        self.assertFalse(result["passed"])
        self.assertFalse(result["settings_changed"])
        self.assertFalse(result["font_scale_accepted"])
        self.assertFalse(result["reduce_motion_accepted"])

    def test_shell_uses_one_quoted_remote_command(self):
        with patch.object(self.runner, "command", return_value=b"value") as command:
            self.runner.shell("settings", "put", "secure", "ui_night_mode", "1; reboot")
            self.assertEqual(shlex.split(command.call_args.args[1]), ["settings", "put", "secure", "ui_night_mode", "1; reboot"])

    def test_restore_preserves_absent_keys_and_continues_after_individual_failure(self):
        self.runner.original = {"mode": "no", "service": {}, "settings": {"ui_night_mode": "1", "ui_night_mode_override_on": "0"}}
        self.runner.changed = True
        calls = []
        def shell(*args):
            calls.append(args)
            if args == ("settings", "list", "secure"):
                return "ui_night_mode=2\nui_night_mode_last_computed=1"
            if args == ("settings", "put", "secure", "ui_night_mode", "1"):
                raise OSError("injected individual failure")
            return ""
        self.runner.shell = shell
        with patch.object(self.runner, "snapshot", return_value=copy.deepcopy(self.runner.original)), patch.object(self.runner, "current"), patch.object(self.runner, "events_snapshot", return_value=baseline()), patch.object(m, "event_delta"):
            with self.assertRaisesRegex(AssertionError, "injected individual failure"):
                self.runner.restore()
        self.assertIn(("settings", "delete", "secure", "ui_night_mode_last_computed"), calls)
        self.assertIn(("settings", "put", "secure", "ui_night_mode_override_on", "0"), calls)
        self.assertNotIn("font_scale", str(calls))
        self.assertNotIn("animator_duration_scale", str(calls))

    def test_exact_restore_mismatch_never_passes(self):
        self.runner.original = {"mode": "no", "service": {}, "settings": {}}
        self.runner.changed = True
        self.runner.shell = lambda *args: ""
        with patch.object(self.runner, "snapshot", return_value={"mode": "yes", "service": {}, "settings": {}}):
            with self.assertRaisesRegex(AssertionError, "restoration mismatch"):
                self.runner.restore()

    def test_interruption_always_restores_and_failure_persists(self):
        for failure in (AssertionError("injected workflow failure"), KeyboardInterrupt("interrupted")):
            with self.subTest(failure=type(failure).__name__):
                with patch.object(self.runner, "exercise", side_effect=failure), patch.object(self.runner, "restore") as restore:
                    with self.assertRaises(SystemExit):
                        self.runner.run()
                    restore.assert_called_once()
                self.assertFalse(json.loads((self.args.out / "result.json").read_text())["passed"])

    def test_cleanup_failure_prevents_pass_and_is_durable(self):
        with patch.object(self.runner, "exercise"), patch.object(self.runner, "restore", side_effect=AssertionError("exact state mismatch")):
            with self.assertRaises(SystemExit):
                self.runner.run()
        result = json.loads((self.args.out / "result.json").read_text())
        self.assertFalse(result["passed"])
        self.assertIn("RESTORATION FAILED", result["error"])

    def test_stale_evidence_directory_refused(self):
        with self.assertRaises(FileExistsError):
            m.Acceptance(self.args)

    def test_capture_refuses_truncated_rgb_before_any_ocr_acceptance(self):
        self.runner.current = lambda name: {"night": False}
        header = b"\x89PNG\r\n\x1a\n" + (13).to_bytes(4, "big") + b"IHDR" + (320).to_bytes(4, "big") + (640).to_bytes(4, "big")
        self.runner.command = lambda *args: header
        with patch.object(m.subprocess, "run", return_value=SimpleNamespace(stdout=b"\xf6\xf6\xf6")) as run:
            with self.assertRaisesRegex(AssertionError, "complete actual RGB"):
                self.runner.capture("test", "no")
            self.assertEqual(run.call_count, 1)

    def test_complete_synthetic_campaign_orders_mutations_after_durable_preflight(self):
        mode = ["no"]
        calls = []
        captures = []
        def shell(*args):
            calls.append(args)
            if args == ("getprop", "ro.kernel.qemu"):
                return "1"
            if args == ("am", "get-current-user"):
                return "0"
            if args == ("dumpsys", "power"):
                return "mWakefulness=Awake"
            if args == ("dumpsys", "input_method"):
                return "mInputShown=false"
            if args == ("getprop", "ro.build.version.sdk"):
                return "35"
            if args == ("pm", "path", m.PACKAGE):
                return "package:/data/app/base.apk"
            if args == ("sha256sum", "/data/app/base.apk"):
                return "a" * 64 + "  /data/app/base.apk"
            if args == ("cmd", "uimode", "night"):
                return "Night mode: " + mode[0]
            if args[:3] == ("cmd", "uimode", "night") and len(args) == 4:
                self.assertTrue((self.args.out / "original-settings.json").is_file())
                mode[0] = args[3]
                return ""
            if args == ("dumpsys", "uimode"):
                return service()
            if args == ("settings", "list", "secure"):
                return "ui_night_mode=1"
            raise AssertionError("Unexpected synthetic command: " + str(args))
        self.runner.shell = shell
        def capture(name, expected):
            self.assertEqual(mode[0], expected)
            captures.append((name, expected))
        self.runner.capture = capture
        self.runner.current = lambda name: m.activity_state(activities(night=mode[0] == "yes"), "1234")
        with patch.object(self.runner, "events_snapshot", return_value=baseline()), patch.object(m.time, "sleep"):
            self.runner.exercise()
        mutations = [args for args in calls if args[:3] == ("cmd", "uimode", "night") and len(args) == 4]
        self.assertEqual(mutations, [("cmd", "uimode", "night", "yes"), ("cmd", "uimode", "night", "no")])
        self.assertEqual(captures, [("original", "no"), ("transition-1-yes", "yes"), ("transition-2-no", "no")])
        self.assertTrue(self.runner.changed)
        self.assertEqual(self.runner.metadata["apk_sha256"], "a" * 64)

    def test_no_mutation_or_restore_if_preflight_rejects(self):
        self.runner.original = None
        self.runner.changed = False
        with patch.object(self.runner, "shell", side_effect=AssertionError("Preflight unavailable")) as shell:
            with self.assertRaises(SystemExit):
                self.runner.run()
            shell.assert_called_once_with("getprop", "ro.kernel.qemu")
        self.assertFalse(json.loads((self.args.out / "result.json").read_text())["settings_changed"])

    def test_snapshot_refuses_inconsistent_effective_mode_before_mutation(self):
        for dump in (service().replace("mCurUiMode=0x11", "mCurUiMode=0x21"),
                     service().replace("mCurUiMode=0x11", "mCurUiMode=unknown"),
                     service().replace("mNightMode=1", "mNightMode=2")):
            def shell(*args):
                if args == ("cmd", "uimode", "night"):
                    return "Night mode: no"
                if args == ("dumpsys", "uimode"):
                    return dump
                raise AssertionError("No further observations/mutations after inconsistent service state")
            self.runner.shell = shell
            with self.subTest(dump=dump), self.assertRaises(AssertionError):
                self.runner.snapshot()
            self.assertFalse(self.runner.changed)
            self.assertEqual((self.args.out / "original-uimode.txt").read_text(), dump + "\n")

    def test_snapshot_never_overwrites_original_dump_during_restore(self):
        def shell(*args):
            if args == ("cmd", "uimode", "night"):
                return "Night mode: no"
            if args == ("dumpsys", "uimode"):
                return service()
            return "ui_night_mode=1"
        self.runner.shell = shell
        self.runner.snapshot()
        original = (self.args.out / "original-uimode.txt").read_text()
        self.runner.snapshot("restored")
        self.assertEqual((self.args.out / "original-uimode.txt").read_text(), original)
        self.assertTrue((self.args.out / "restored-uimode.txt").is_file())


if __name__ == "__main__":
    unittest.main()
