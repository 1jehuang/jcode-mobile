#!/usr/bin/env python3
"""Offline safety checks, not simulated end-to-end acceptance."""
import importlib.util
import os
from pathlib import Path
import queue
import time
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("latency", Path(__file__).with_name("mobile_simulator_latency.py"))
latency = importlib.util.module_from_spec(spec)
spec.loader.exec_module(latency)


class Running:
    def poll(self):
        return None


class DriverTests(unittest.TestCase):
    def test_duration_units_are_converted_without_summing_overlapping_spans(self):
        for value, expected in [("20ns", .00002), ("300µs", .3), ("300us", .3), ("40ms", 40), ("2.5s", 2500)]:
            name, result = latency.parse_trace("startup_trace session ready +" + value)
            self.assertEqual(name, "session ready")
            self.assertAlmostEqual(result, expected)
        self.assertIsNone(latency.parse_trace("startup_trace session connect error auth"))

    def test_env_cannot_inherit_real_login_autosend_or_url(self):
        with patch.dict(os.environ, {"JCODE_MOBILE_USE_CLI_ACCOUNT": "1", "JCODE_MOBILE_SEND": "real work", "JCODE_MOBILE_OPEN_URL": "jcode://new?text=bad", "OPENAI_API_KEY": "secret", "WAYLAND_DISPLAY": "wayland-1", "ZED_HEADLESS": "1"}):
            env = latency.isolated_env(Path("/isolated"), ":123")
        for name in ("JCODE_MOBILE_USE_CLI_ACCOUNT", "JCODE_MOBILE_SEND", "JCODE_MOBILE_OPEN_URL", "OPENAI_API_KEY", "WAYLAND_DISPLAY", "ZED_HEADLESS"):
            self.assertNotIn(name, env)
        self.assertEqual(env["XDG_DATA_HOME"], "/isolated/data")
        self.assertEqual(env["XDG_CACHE_HOME"], "/isolated/cache")
        self.assertEqual(env["XDG_CONFIG_HOME"], "/isolated/config")
        self.assertEqual(env["XDG_STATE_HOME"], "/isolated/state")
        self.assertEqual(env["DISPLAY"], ":123")

    def test_cached_render_without_fresh_network_history_never_passes(self):
        events = queue.Queue()
        events.put((1, "startup_trace ui open to transcript render +1ms"))
        with self.assertRaises(TimeoutError):
            latency.wait_render(events, Running(), time.monotonic() + .02)

    def test_independent_thread_log_order_does_not_drop_completion(self):
        events = queue.Queue()
        for name in reversed(("session ready", "session history", "ui_open_to_history", "ui open to transcript render")):
            events.put((10, f"startup_trace {name} +2ms"))
        result = latency.wait_render(events, Running(), time.monotonic() + .5)
        self.assertEqual(len(result), 4)
        self.assertEqual(result["session ready"]["duration_ms"], 2)

    def test_cached_paint_before_fresh_history_is_not_a_fresh_render(self):
        events = queue.Queue()
        events.put((1, "startup_trace ui open to transcript render +1ms"))
        for name in ("session ready", "session history", "ui_open_to_history"):
            events.put((2, f"startup_trace {name} +2ms"))
        with self.assertRaises(TimeoutError):
            latency.wait_render(events, Running(), time.monotonic() + .02)
        events.put((3, "startup_trace ui open to transcript render +3ms"))
        for name in ("session ready", "session history", "ui_open_to_history"):
            events.put((2, f"startup_trace {name} +2ms"))
        self.assertEqual(latency.wait_render(events, Running(), time.monotonic() + .5)["ui open to transcript render"]["observed_ms"], 3)

    def test_visible_confirmation_rejects_blank_frame(self):
        with patch.object(latency, "capture"), patch.object(latency.subprocess, "run") as ocr:
            ocr.return_value.stdout = "Loading conversation"
            with self.assertRaises(TimeoutError):
                latency.visible_transcript({}, Path("/unused.png"), time.monotonic(), timeout=.001)

    def test_visible_confirmation_requires_fresh_fixture_text(self):
        with patch.object(latency, "capture"), patch.object(latency.subprocess, "run") as ocr:
            ocr.return_value.stdout = "SIMULATOR FRESH fixture-1"
            result = latency.visible_transcript({}, Path("/unused.png"), time.monotonic())
            self.assertGreaterEqual(result["frame_capture_upper_bound_ms"], 0)
            self.assertGreaterEqual(result["ocr_completed_ms"], result["frame_capture_upper_bound_ms"])

    def test_loopback_scenarios_never_include_send_or_production_endpoint(self):
        for options in latency.SCENARIOS.values():
            self.assertTrue(all(not value.startswith(("http", "--send")) for value in options))


if __name__ == "__main__":
    unittest.main()
