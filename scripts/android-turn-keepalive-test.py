#!/usr/bin/env python3
"""Offline production lease policy execution and packaging/lifecycle contracts."""
from pathlib import Path
import os
import subprocess
import tempfile
import unittest
import xml.etree.ElementTree as ET

ROOT = Path(__file__).resolve().parents[1]
JAVA = ROOT / 'android/app/src/main/java/org/jcode/mobile'


class TurnKeepaliveContracts(unittest.TestCase):
    def test_production_policy(self):
        home = Path(os.environ.get('JAVA_HOME', str(Path.home() / 'Android/jdk-21')))
        with tempfile.TemporaryDirectory(prefix='turn-lease-', dir=os.environ.get('JCODE_SCRATCH_DIR')) as out:
            subprocess.run([str(home / 'bin/javac'), '--release', '8', '-d', out,
                            str(JAVA / 'TurnLease.java'),
                            str(ROOT / 'android/turn-keepalive-tests/TurnLeaseContract.java')], check=True)
            subprocess.run([str(home / 'bin/java'), '-cp', out, 'org.jcode.mobile.TurnLeaseContract'], check=True)

    def test_production_manifest(self):
        root = ET.parse(ROOT / 'android/app/src/main/AndroidManifest.xml').getroot()
        ns = '{http://schemas.android.com/apk/res/android}'
        service = root.find("application/service[@%sname='.TurnKeepAliveService']" % ns)
        self.assertIsNotNone(service)
        self.assertEqual(service.get(ns + 'foregroundServiceType'), 'shortService')
        self.assertEqual(service.get(ns + 'exported'), 'false')
        self.assertEqual(service.get(ns + 'stopWithTask'), 'true')
        permissions = {p.get(ns + 'name') for p in root.findall('uses-permission')}
        self.assertIn('android.permission.FOREGROUND_SERVICE', permissions)
        self.assertNotIn('android.permission.WAKE_LOCK', permissions)
        self.assertFalse(any('DATA_SYNC' in p or 'REMOTE_MESSAGING' in p or 'SPECIAL_USE' in p for p in permissions))

    def test_narrow_jni_and_teardown(self):
        source = (JAVA / 'AndroidServices.java').read_text()
        self.assertIn('TurnKeepAliveService.update(activity, "true".equals(busy), foreground)', source)
        self.assertIn('public void onDestroy() { foreground = false; TurnKeepAliveService.cancel(activity);', source)
        self.assertIn('public void accountClear() throws Exception {\n        new Handler(Looper.getMainLooper()).post(() -> TurnKeepAliveService.cancel(activity));', source)
        notify = (ROOT / 'src/notify.rs').read_text().split('#[cfg(target_os = "android")]\nmod imp {')[1].split('#[cfg(not(any')[0]
        self.assertIn('call_bool("keepAlive", if busy { "true" } else { "false" })', notify)
        source = (JAVA / 'TurnKeepAliveService.java').read_text()
        for needle in ['START_NOT_STICKY', 'onTimeout(int startId)', 'onTimeout(int startId, int fgsType)',
                       'handler.removeCallbacks(expire)', 'stopForeground(STOP_FOREGROUND_REMOVE)',
                       'Notification.VISIBILITY_PRIVATE', 'setPublicVersion(publicVersion)',
                       'setLocalOnly(true)', 'setOngoing(true)', 'Build.VERSION.SDK_INT >= 31',
                       'Build.VERSION.SDK_INT >= 34']:
            self.assertIn(needle, source)
        for forbidden in ['cancelAll(', 'notify("agents"', 'WAKE_LOCK', 'AlarmManager', 'START_STICKY', 'session_id', 'getMessage()']:
            self.assertNotIn(forbidden, source)

    def test_isolated_manifest(self):
        root = ET.parse(ROOT / 'android/turn-keepalive-tests/AndroidManifest.xml').getroot()
        ns = '{http://schemas.android.com/apk/res/android}'
        package = 'org.jcode.mobile.turnkeepalivetests'
        self.assertEqual(root.get('package'), package)
        self.assertEqual(root.find('instrumentation').get(ns + 'targetPackage'), package)
        self.assertNotIn('android.permission.INTERNET', {p.get(ns + 'name') for p in root.findall('uses-permission')})


if __name__ == '__main__':
    unittest.main()
