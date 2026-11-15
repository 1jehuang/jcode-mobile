#!/usr/bin/env python3
"""Fail-closed checks for actual debug share-state evidence. Never invokes adb.

The root-run harness uses these checks on read-only state exported by the shared
GPUI app. A missing state bridge is an unavailable acceptance gate, not a pass.
"""
import json


def read_state(value):
    state = json.loads(value)
    required = {"probe": str, "offline": bool, "syntheticAccount": bool, "connectionPresent": bool, "autoSendPending": bool,
                "activeTab": int, "tabCount": int, "composer": str,
                "submittedCount": int, "frame": int}
    for key, kind in required.items():
        if key not in state or type(state[key]) is not kind:
            raise AssertionError(f"Missing/invalid observed shared state: {key}")
    if state["probe"] != "incoming-share-state-v1":
        raise AssertionError("Unsupported or unrelated shared-state probe")
    if not state["offline"] or not state["syntheticAccount"] or state["connectionPresent"]:
        raise AssertionError("Refusing non-offline or connected app state")
    if state["autoSendPending"]:
        raise AssertionError("Shared draft queued automatic submission")
    if state["tabCount"] < 1 or state["submittedCount"] < 0 or state["frame"] < 1:
        raise AssertionError("Invalid shared state counters")
    return state


def require_draft(state, text, previous=None):
    if state["composer"] != text:
        raise AssertionError("Observed composer does not equal delivered share")
    if previous is not None:
        if state["activeTab"] == previous["activeTab"] or state["tabCount"] != previous["tabCount"] + 1:
            raise AssertionError("Share did not create exactly one new tab")
        if state["submittedCount"] != previous["submittedCount"]:
            raise AssertionError("Share changed actual submitted-message count")


def require_unchanged(before, after):
    for key in ("activeTab", "tabCount", "composer", "submittedCount"):
        if before[key] != after[key]:
            raise AssertionError(f"Consumed share replay or unsolicited state change: {key}")
    if after["frame"] <= before["frame"]:
        raise AssertionError("Stale state cannot establish consume-once/no-submit acceptance")
