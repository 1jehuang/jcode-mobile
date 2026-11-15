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
import xml.etree.ElementTree as ET

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

    def test_os_harness_is_test_uid_only_and_without_network_permission(self):
        root = ET.parse(ROOT / 'android/agent-notification-tests/AndroidManifest.xml').getroot()
        android = '{http://schemas.android.com/apk/res/android}'
        package = 'org.jcode.mobile.agentnotificationtests'
        self.assertEqual(root.get('package'), package)
        self.assertEqual(root.find('instrumentation').get(android + 'targetPackage'), package)
        self.assertEqual({p.get(android + 'name') for p in root.findall('uses-permission')}, {'android.permission.POST_NOTIFICATIONS'})
        source = (ROOT / 'android/agent-notification-tests/src/org/jcode/mobile/agentnotificationtests/AgentNotificationsInstrumentation.java').read_text()
        self.assertIn('Process.myUid() != context.getPackageManager().getApplicationInfo(TARGET, 0).uid', source)
        self.assertIn('new DexClassLoader(', source)
        self.assertIn('allocateInstance', source)
        self.assertNotIn('org.jcode.mobile.JcodeActivity', source)
        self.assertNotIn('call("account', source)
        self.assertIn('failure.addSuppressed(cleanup)', source)
        self.assertIn('dispatchSession(firstAa, "Aa")', source)
        self.assertIn('dispatchSession(firstBb, "BB")', source)

    def test_actual_runner_cleanup_preserves_failures_and_restores_test_permission(self):
        runner = (ROOT / 'scripts/android-agent-notifications-test.sh').read_text()
        cleanup = 'cleanup() {' + runner.split('cleanup() {', 1)[1].split('\n}\ntrap cleanup EXIT', 1)[0] + '\n}'
        scratch = Path(os.environ.get('JCODE_SCRATCH_DIR', str(ROOT / 'target')))
        scratch.mkdir(parents=True, exist_ok=True)
        cases = [
            ('success-denied', 0, False, False, False, False, 'denied', 'false', 0),
            ('success-granted', 0, False, False, False, False, 'granted', 'true', 0),
            ('initial-failure', 7, False, False, False, False, 'denied', 'false', 7),
            ('cleanup-failure', 0, True, False, False, False, 'denied', 'false', 1),
            ('permission-failure', 0, False, True, False, False, 'denied', 'false', 1),
            ('probe-failure', 0, False, False, True, False, 'denied', 'false', 1),
            ('state-mismatch', 0, False, False, False, True, 'denied', 'false', 1),
        ]
        for name, initial, fail_cleanup, fail_permission, fail_probe, mismatch, original, enabled, expected in cases:
            with self.subTest(name=name), tempfile.TemporaryDirectory(prefix='agent-cleanup-', dir=scratch) as temp:
                path = Path(temp)
                stage = path / 'stage'
                stage.mkdir()
                mock = path / 'mock-cli'
                mock.write_text('#!/usr/bin/env bash\nprintf "%s\\n" "$*" > "$MOCK_CALL"\nexit "$MOCK_PERMISSION_EXIT"\n')
                mock.chmod(0o700)
                env = dict(os.environ, MOCK_CALL=str(path / 'permission-call'), MOCK_PERMISSION_EXIT='1' if fail_permission else '0')
                prelude = f'''set -uo pipefail
RUN=true
ORIGINAL={original}
ORIGINAL_ENABLED={enabled}
OWNS_NAMESPACE=true
ADB={mock}
SERIAL=emulator-mock
TEST=org.jcode.mobile.agentnotificationtests
OUT={path}
STAGE={stage}
run_phase() {{
  if [[ "$1" == cleanup ]]; then return {1 if fail_cleanup else 0}; fi
  if [[ "$1" == probe ]]; then
    printf 'INSTRUMENTATION_RESULT: notification_permission={original}\\nINSTRUMENTATION_RESULT: notifications_enabled={'wrong' if mismatch else enabled}\\nINSTRUMENTATION_RESULT: namespace_clean=true\\n' > "$2"
    return {1 if fail_probe else 0}
  fi
  return 99
}}
'''
                result = subprocess.run(['bash', '-c', prelude + cleanup + f'\n(exit {initial})\ncleanup\n'], env=env, capture_output=True, text=True)
                self.assertEqual(result.returncode, expected, result.stdout + result.stderr)
                self.assertEqual((path / 'permission-call').read_text().strip(), f'-s emulator-mock shell pm {"grant" if original == "granted" else "revoke"} org.jcode.mobile.agentnotificationtests android.permission.POST_NOTIFICATIONS')
                self.assertEqual('PASS:' in result.stdout, expected == 0)
                self.assertFalse(stage.exists())

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
