#!/usr/bin/env python3
"""Offline synthetic regression tests for production Android agent notifications.

Executes the actual Java syncAgents/helper source against bounded fake OS APIs.
This is not evidence of Android runtime delivery or launcher/assistant parity.
The isolated instrumentation APK supplies the separate actual-OS acceptance path.
"""
from pathlib import Path
import os
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
JAVA = ROOT / "android/app/src/main/java/org/jcode/mobile/AndroidServices.java"


class AgentNotificationContracts(unittest.TestCase):
    def test_android_uses_shared_json_without_changing_ios_or_retry_semantics(self):
        source = (ROOT / "src/ios_ext.rs").read_text()
        android = source.split('#[cfg(target_os = "android")]\nmod imp {', 1)[1].split('#[cfg(not(any', 1)[0]
        self.assertIn('serde_json::to_string(state)', android)
        self.assertIn('call_bool("syncAgents", &json)', android)
        self.assertNotIn('call_bool("syncLive"', android)
        self.assertNotIn('emit_token(', android)
        self.assertIn('self.applied = None;', source)
        self.assertIn('if clearing { self.retry_at = None; }', source)

    def test_legacy_summary_method_still_exists_and_new_path_is_owned_only(self):
        source = JAVA.read_text()
        legacy = source.split('public boolean syncLive(String headline) {', 1)[1].split('public boolean postNotification', 1)[0]
        self.assertIn('manager.cancel("agents", 1)', legacy)
        self.assertIn('manager.notify("agents", 1, notification); return true;', legacy)
        block = source.split('// Account-wide local agent notifications.', 1)[1].split('\n', 1)[1].split('public boolean syncLive(', 1)[0]
        self.assertNotIn('cancelAll(', block)
        self.assertNotIn('hashCode(', block)
        self.assertNotIn('requestPermissions(', block)
        self.assertNotIn('deleteNotificationChannel(', block)
        self.assertIn('Uri.encode(id)', block)
        self.assertIn('PendingIntent.FLAG_IMMUTABLE', block)
        self.assertIn('Notification.VISIBILITY_PRIVATE', block)
        self.assertIn('setPublicVersion(', block)

    def test_actual_java_algorithm_against_synthetic_os(self):
        source = JAVA.read_text()
        block = source.split('// Account-wide local agent notifications.', 1)[1].split('\n', 1)[1].split('public boolean syncLive(', 1)[0]
        java = HARNESS.replace('PRODUCTION_METHODS', block)
        home = Path(os.environ.get('JAVA_HOME', str(Path.home() / 'Android/jdk-21')))
        javac = str(home / 'bin/javac') if (home / 'bin/javac').exists() else 'javac'
        runtime = str(home / 'bin/java') if (home / 'bin/java').exists() else 'java'
        scratch = Path(os.environ.get('JCODE_SCRATCH_DIR', str(ROOT / 'target')))
        scratch.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(prefix='agent-notification-contract-', dir=scratch) as temp:
            path = Path(temp)
            (path / 'AgentContract.java').write_text(java)
            subprocess.run([javac, '--release', '8', '-Xlint:-options', str(path / 'AgentContract.java')], check=True)
            subprocess.run([runtime, '-cp', temp, 'AgentContract'], check=True)


HARNESS = (ROOT / 'android/agent-notification-tests/AgentContract.java').read_text()

if __name__ == '__main__':
    unittest.main()
