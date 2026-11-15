import json
import math
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

from mobile_readiness_budget import evaluate


def payload(events):
    return json.dumps({"batches": [{"run_id": "private-run", "events": [
        {"kind": "timing", "seq": index, "name": name, "duration_ms": value}
        for index, (name, value) in enumerate(events)
    ]}]})


class BudgetTests(unittest.TestCase):
    def test_cached_paint_cannot_pass_readiness(self):
        self.assertFalse(evaluate(payload([("ui_open_to_history", 14),
                                           ("ui_open_to_transcript_render", 20)]))["passed"])

    def test_both_real_milestones_required(self):
        self.assertFalse(evaluate(payload([("session_ready", 50)]))["passed"])
        self.assertTrue(evaluate(payload([("session_ready", 500), ("session_history", 900)]))["passed"])

    def test_budget_is_strict_and_durations_not_summed(self):
        self.assertTrue(evaluate(payload([("session_ready", 600), ("session_history", 900)]))["passed"])
        self.assertFalse(evaluate(payload([("session_ready", 999), ("session_history", 1000)]))["passed"])

    def test_slow_sample_cannot_hide_behind_fast_median(self):
        events = [("session_ready", 1), ("session_history", 2)] * 20
        events += [("session_history", 21628.5)]
        result = evaluate(payload(events))
        self.assertFalse(result["passed"])
        self.assertEqual(result["milestones"][1]["at_or_above_budget"], 1)

    def test_failed_attempts_cannot_hide_behind_fast_retries(self):
        result = evaluate(payload([("session_ready", 30), ("session_history", 40),
                                   ("session_connect_failed", 5000)]))
        self.assertFalse(result["passed"])
        self.assertEqual(result["failed_attempt_samples"], 1)

    def test_minimum_and_duplicate_exports(self):
        data = json.loads(payload([("session_ready", 30), ("session_history", 40)]))
        data["batches"] *= 2
        self.assertFalse(evaluate(json.dumps(data), minimum=2)["passed"])

    def test_invalid_duration_and_budget_fail(self):
        for value in [True, -1, math.inf, math.nan, "3"]:
            with self.assertRaises(ValueError):
                evaluate(payload([("session_ready", value)]))
        for budget in [0, -1, math.nan, math.inf]:
            with self.assertRaises(ValueError):
                evaluate(payload([]), budget_ms=budget)

    def test_log_input_and_output_privacy(self):
        result = evaluate("private-secret conn_timing session ready 100.0ms\n"
                          "private-secret conn_timing session history 200.0ms\n", log=True)
        self.assertTrue(result["passed"])
        self.assertNotIn("private", json.dumps(result))

    def test_cli_exit_codes(self):
        script = Path(__file__).with_name("mobile_readiness_budget.py")
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "input.json"
            for content, expected in [(payload([]), 1),
                                      (payload([("session_ready", 2), ("session_history", 3)]), 0),
                                      ("private-invalid-json", 2)]:
                source.write_text(content)
                run = subprocess.run([sys.executable, "-B", str(script), str(source)], capture_output=True, text=True)
                self.assertEqual(run.returncode, expected)
                self.assertNotIn("private", run.stdout + run.stderr)


if __name__ == "__main__":
    unittest.main()
