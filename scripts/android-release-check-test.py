#!/usr/bin/env python3
"""Network-free regression tests for release safety gates and API workflow."""
import importlib.util
import json
from pathlib import Path
import struct
import tempfile
import unittest
import zipfile
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('publish', Path(__file__).with_name('android-publish.py'))
publish = importlib.util.module_from_spec(spec)
spec.loader.exec_module(publish)
check = publish.check


class ReleaseTests(unittest.TestCase):
    def setUp(self):
        self.payload = b'test artifact'
        self.report = {'aab_sha256': publish.hashlib.sha256(self.payload).hexdigest(), 'package': 'org.jcode.mobile', 'version_code': 2, 'version_name': 'test'}
        self.approval = {key: True for key in publish.REQUIRED}
        self.approval.update(self.report, track='internal', approved_by='test-root', validation_evidence='synthetic-test')

    def test_template_is_not_approved(self):
        template = json.loads((Path(__file__).parent.parent / 'playstore/approval.template.json').read_text())
        with self.assertRaises(ValueError):
            publish.gate(self.report, template)

    def test_each_gate_is_required(self):
        for key in publish.REQUIRED:
            with self.subTest(key=key), self.assertRaises(ValueError):
                publish.gate(self.report, {**self.approval, key: False})

    def test_truthy_strings_are_not_approval(self):
        with self.assertRaises(ValueError):
            publish.gate(self.report, {**self.approval, 'upload_authorized': 'true'})

    def test_hash_package_version_and_track_bound(self):
        for key, value in [('aab_sha256', 'wrong'), ('package', 'other.app'), ('version_code', 9), ('track', 'production')]:
            with self.subTest(key=key), self.assertRaises(ValueError):
                publish.gate(self.report, {**self.approval, key: value})

    def test_commit_needs_separate_gate(self):
        publish.gate(self.report, self.approval)
        with self.assertRaises(ValueError):
            publish.gate(self.report, self.approval, True)

    def workflow(self, commit=False, mismatch=False, fail_commit=False):
        calls = []
        def request(client, method, url, **kwargs):
            calls.append((method, url, kwargs))
            if url.endswith('/edits'):
                return {'id': 'edit1'}
            if '/bundles?' in url:
                return {'versionCode': 999 if mismatch else 2}
            if method == 'GET':
                return {'track': 'internal'}
            if url.endswith(':commit'):
                if fail_commit:
                    raise ValueError('ambiguous connection failure')
                return {'id': 'edit1'}
            return {}
        receipt = {}
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'bundle.aab'
            path.write_bytes(self.payload)
            with patch.object(publish, 'request', request):
                if mismatch or fail_commit:
                    with self.assertRaises(ValueError):
                        publish.publish(None, path, self.report, {**self.approval, 'commit_authorized': True}, commit, receipt)
                else:
                    publish.publish(None, path, self.report, {**self.approval, 'commit_authorized': True}, commit, receipt)
        return calls, receipt

    def test_staging_does_not_commit(self):
        calls, receipt = self.workflow()
        self.assertEqual(receipt['state'], 'validated_uncommitted_edit')
        self.assertFalse(receipt['committed'])
        self.assertFalse(any(url.endswith(':commit') for _, url, _ in calls))
        self.assertTrue(any(url.endswith(':validate') for _, url, _ in calls))
        self.assertTrue(all('/production' not in url for _, url, _ in calls))

    def test_commit_confirmed(self):
        calls, receipt = self.workflow(commit=True)
        self.assertTrue(receipt['committed'])
        self.assertTrue(calls[-1][1].endswith(':commit'))

    def test_server_version_mismatch_stops_track_mutation(self):
        calls, receipt = self.workflow(mismatch=True)
        self.assertFalse(any(method == 'PUT' for method, _, _ in calls))
        self.assertFalse(receipt['committed'])

    def test_uncertain_commit_never_retries_or_deletes(self):
        calls, receipt = self.workflow(commit=True, fail_commit=True)
        self.assertEqual(sum(url.endswith(':commit') for _, url, _ in calls), 1)
        self.assertFalse(any(method == 'DELETE' for method, _, _ in calls))
        self.assertFalse(receipt['committed'])

    def test_artifact_change_stops_before_network(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'bundle.aab'
            path.write_bytes(b'changed')
            with patch.object(publish, 'request') as request, self.assertRaises(ValueError):
                publish.publish(None, path, self.report, self.approval, False, {})
            request.assert_not_called()

    def inspect_fixture(self, manifest_change=None, policy_change=None, unsigned=False, debug_cert=False):
        policy = json.loads((Path(__file__).parent.parent / 'playstore/release-policy.json').read_text())
        policy['upload_certificate_sha256'] = publish.hashlib.sha256(b'cert').hexdigest()
        if policy_change:
            policy.update(policy_change)
        manifest = '''<manifest xmlns:android="http://schemas.android.com/apk/res/android" package="org.jcode.mobile" android:versionCode="1" android:versionName="0.1.0"><uses-sdk android:minSdkVersion="26" android:targetSdkVersion="36"/><application/>''' + ''.join('<uses-permission android:name="' + p + '"/>' for p in policy['permissions']) + '</manifest>'
        if manifest_change:
            manifest = manifest.replace(*manifest_change)
        def tool(args):
            if '-verify' in args:
                return 'jar verified.' + (' unsigned entries' if unsigned else '')
            if '-rfc' in args:
                return '-----BEGIN CERTIFICATE-----\nY2VydA==\n-----END CERTIFICATE-----'
            return 'Owner: CN=Android Debug' if debug_cert else 'Owner: CN=Upload'
        def bundletool(*args):
            return manifest if args[0] == 'dump' else ''
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'bundle.aab'
            with zipfile.ZipFile(path, 'w') as archive:
                for name in ['BundleConfig.pb', 'base/manifest/AndroidManifest.xml', 'base/dex/classes.dex', 'base/lib/arm64-v8a/libjcode_mobile.so']:
                    archive.writestr(name, b'fixture')
            with patch.object(check, 'elf_alignments', return_value=[16384]), patch.object(check, 'bundletool', bundletool), patch.object(check, 'java_tool', side_effect=lambda name: name), patch.object(check, 'run', tool):
                return check.inspect(path, policy)

    def test_complete_validator_fixture(self):
        self.assertTrue(self.inspect_fixture()['offline_artifact_checks_passed'])

    def test_target_sdk_rejected(self):
        with self.assertRaisesRegex(ValueError, 'target SDK'):
            self.inspect_fixture(('targetSdkVersion="36"', 'targetSdkVersion="35"'))

    def test_debuggable_rejected(self):
        with self.assertRaisesRegex(ValueError, 'Debuggable'):
            self.inspect_fixture(('<application/>', '<application android:debuggable="true"/>'))

    def test_permission_drift_rejected(self):
        with self.assertRaisesRegex(ValueError, 'permissions'):
            self.inspect_fixture(('android.permission.RECORD_AUDIO', 'android.permission.CAMERA'))

    def test_signer_mismatch_rejected(self):
        with self.assertRaisesRegex(ValueError, 'signer SHA-256'):
            self.inspect_fixture(policy_change={'upload_certificate_sha256': ''})

    def test_unsigned_entry_rejected(self):
        with self.assertRaisesRegex(ValueError, 'signature'):
            self.inspect_fixture(unsigned=True)

    def test_debug_certificate_rejected(self):
        with self.assertRaisesRegex(ValueError, 'debug signing'):
            self.inspect_fixture(debug_cert=True)

    def test_reject_apk(self):
        with self.assertRaisesRegex(ValueError, 'real .aab'):
            check.inspect('fake.apk', {})

    def test_elf_alignment(self):
        data = bytearray(120)
        data[:6] = b'\x7fELF\x02\x01'
        struct.pack_into('<Q', data, 32, 64)
        struct.pack_into('<HH', data, 54, 56, 1)
        struct.pack_into('<I', data, 64, 1)
        struct.pack_into('<Q', data, 112, 16384)
        self.assertEqual(check.elf_alignments(data), [16384])
        struct.pack_into('<Q', data, 112, 4096)
        with self.assertRaises(ValueError):
            check.elf_alignments(data)

    def test_offline_default_never_authenticates(self):
        with patch.object(publish.sys, 'argv', ['android-publish.py', 'test.aab']), patch.object(check, 'inspect', return_value=self.report), patch.object(publish, 'session') as session:
            self.assertEqual(publish.main(), 0)
            session.assert_not_called()

    def test_execute_without_approval_never_authenticates(self):
        with patch.object(publish.sys, 'argv', ['android-publish.py', 'test.aab', '--execute']), patch.object(check, 'inspect', return_value=self.report), patch.object(publish, 'session') as session:
            self.assertEqual(publish.main(), 1)
            session.assert_not_called()


if __name__ == '__main__':
    unittest.main()
