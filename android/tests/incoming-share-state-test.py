#!/usr/bin/env python3
"""Synthetic failure-path tests only, never Android acceptance or device access."""
import importlib.util
import json
from pathlib import Path
import unittest

PATH = Path(__file__).with_name("incoming-share-state.py")
spec = importlib.util.spec_from_file_location("share_state", PATH)
state = importlib.util.module_from_spec(spec)
spec.loader.exec_module(state)


class StateTests(unittest.TestCase):
    def fixture(self, **updates):
        value = dict(probe="incoming-share-state-v1", offline=True, syntheticAccount=True, connectionPresent=False, autoSendPending=False,
                     activeTab=1, tabCount=1, composer="", submittedCount=0, frame=1)
        value.update(updates)
        return value

    def read(self, **updates):
        return state.read_state(json.dumps(self.fixture(**updates)))

    def test_exact_share_and_one_tab(self):
        before = self.read()
        after = self.read(activeTab=2, tabCount=2, composer="draft + & ✓", frame=2)
        state.require_draft(after, "draft + & ✓", before)
        state.require_unchanged(after, self.read(activeTab=2, tabCount=2, composer="draft + & ✓", frame=9))

    def test_invalid_or_unsafe_state_never_passes(self):
        for updates in ({"probe": "wrong"}, {"offline": False}, {"syntheticAccount": False}, {"connectionPresent": True}, {"autoSendPending": True},
                        {"frame": 0}, {"frame": True}, {"tabCount": 0}, {"submittedCount": -1},
                        {"composer": None}, {"offline": "true"}):
            with self.subTest(updates=updates), self.assertRaises(AssertionError):
                self.read(**updates)
        for key in self.fixture():
            missing = self.fixture()
            del missing[key]
            with self.subTest(missing=key), self.assertRaises(AssertionError):
                state.read_state(json.dumps(missing))
        for invalid in ("", "not JSON", "[]", "null"):
            with self.subTest(invalid=invalid), self.assertRaises((ValueError, TypeError, AssertionError)):
                state.read_state(invalid)

    def test_stale_evidence_and_any_replay_fail(self):
        before = self.read(composer="draft")
        with self.assertRaises(AssertionError):
            state.require_unchanged(before, before)
        for key, value in (("activeTab", 2), ("tabCount", 2), ("composer", ""), ("submittedCount", 1)):
            with self.subTest(key=key), self.assertRaises(AssertionError):
                state.require_unchanged(before, self.read(composer="draft", frame=2, **{key: value}) if key != "composer" else self.read(composer=value, frame=2))

    def test_no_marker_presence_heuristic(self):
        before = self.read()
        for updates in ({"composer": "draft extra"}, {"activeTab": 1}, {"tabCount": 3}, {"submittedCount": 1}):
            after = self.fixture(activeTab=2, tabCount=2, composer="draft", frame=2)
            after.update(updates)
            with self.subTest(updates=updates), self.assertRaises(AssertionError):
                state.require_draft(after, "draft", before)


if __name__ == "__main__":
    unittest.main()
