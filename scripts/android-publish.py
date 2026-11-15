#!/usr/bin/env python3
"""Prepare or explicitly stage a Google Play INTERNAL release. Offline by default."""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import sys

spec = importlib.util.spec_from_file_location('release_check', Path(__file__).with_name('android-release-check.py'))
check = importlib.util.module_from_spec(spec)
spec.loader.exec_module(check)
SCOPE = 'https://www.googleapis.com/auth/androidpublisher'
API = 'https://androidpublisher.googleapis.com/androidpublisher/v3/applications/'
REQUIRED = ('emulator_validated', 'gpui_parity_validated', 'release_artifact_tested',
            'native_16k_device_validated', 'metadata_reviewed', 'privacy_and_data_safety_reviewed',
            'play_console_app_ready', 'upload_authorized')


PARITY_CHECKS = ('shared_gpui_native_renderer', 'no_legacy_java_ui', 'emulator_launch',
                 'navigation_and_chat', 'keyboard_and_text_input', 'attachments_and_share',
                 'voice_and_media', 'haptics', 'notifications_and_device_push',
                 'authentication_and_deep_links', 'release_aab_install', 'native_16k_runtime',
                 'talkback_accessibility')


def parity_gate(report, approval):
    evidence = approval.get('android_parity_evidence')
    if not isinstance(evidence, dict):
        raise ValueError('Missing structured Android GPUI parity evidence')
    if evidence.get('aab_sha256') != report['aab_sha256'] or evidence.get('renderer') != 'shared_gpui_native':
        raise ValueError('Android parity evidence must identify the exact AAB and shared native GPUI renderer')
    if evidence.get('accepted_by_root') is not True or evidence.get('accepted_by') != approval.get('approved_by'):
        raise ValueError('Android parity evidence requires coordinating root acceptance')
    if not isinstance(evidence.get('source_revision'), str) or not evidence['source_revision'].strip():
        raise ValueError('Android parity evidence requires a source revision')
    checks = evidence.get('checks')
    if not isinstance(checks, dict):
        raise ValueError('Android parity evidence checks are missing')
    for name in PARITY_CHECKS:
        item = checks.get(name)
        if not isinstance(item, dict) or item.get('status') != 'passed':
            raise ValueError(f'Android GPUI parity check missing or incomplete: {name}')
        path = item.get('evidence_path')
        digest = item.get('evidence_sha256')
        if not isinstance(path, str) or not isinstance(digest, str):
            raise ValueError(f'Android GPUI parity evidence file/hash missing: {name}')
        try:
            payload = Path(path).read_bytes()
        except OSError:
            raise ValueError(f'Android GPUI parity evidence file unavailable: {name}') from None
        if not payload or hashlib.sha256(payload).hexdigest() != digest:
            raise ValueError(f'Android GPUI parity evidence empty or changed: {name}')


def gate(report, approval, commit=False):
    for field in REQUIRED + (('commit_authorized',) if commit else ()):
        if approval.get(field) is not True:
            raise ValueError(f'Root approval missing: {field}')
    for field in ('aab_sha256', 'package', 'version_code'):
        if approval.get(field) != report[field]:
            raise ValueError(f'Root approval {field} does not match validated artifact')
    if approval.get('track') != 'internal' or not approval.get('approved_by') or not approval.get('validation_evidence'):
        raise ValueError('Root must name approver, validation evidence, and internal track')
    parity_gate(report, approval)


def session():
    try:
        import google.auth
        from google.auth.transport.requests import AuthorizedSession
    except ImportError:
        raise ValueError('Install google-auth and requests in a dedicated virtual environment') from None
    try:
        credentials, _ = google.auth.default(scopes=[SCOPE])
    except Exception:
        raise ValueError('No usable Google ADC credentials. Configure an authorized Play service account outside the repository') from None
    return AuthorizedSession(credentials)


def request(client, method, url, **kwargs):
    response = client.request(method, url, timeout=900, **kwargs)
    if not response.ok:
        # Do not emit tokens, private keys, or potentially sensitive response bodies.
        raise ValueError(f'Play API {method} failed: HTTP {response.status_code}. Check app existence, API enablement and Console permissions')
    return response.json() if response.content else {}


def publish(client, aab, report, approval, commit, receipt):
    gate(report, approval, commit)
    # Re-read once and upload these exact validated bytes, preventing path replacement.
    payload = Path(aab).read_bytes()
    if hashlib.sha256(payload).hexdigest() != report['aab_sha256']:
        raise ValueError('Artifact changed after validation')
    base = API + report['package'] + '/edits'
    edit = request(client, 'POST', base, json={})
    edit_id = edit['id']
    receipt.update(edit_id=edit_id, expiry_time_seconds=edit.get('expiryTimeSeconds'), state='edit_created', committed=False)
    try:
        upload = 'https://androidpublisher.googleapis.com/upload/androidpublisher/v3/applications/' + report['package'] + '/edits/' + edit_id + '/bundles?uploadType=media'
        bundle = request(client, 'POST', upload, data=payload, headers={'Content-Type': 'application/octet-stream'})
        if int(bundle['versionCode']) != report['version_code']:
            raise ValueError('Play returned a different bundle versionCode')
        track_url = base + '/' + edit_id + '/tracks/internal'
        # Check actual track identity before writing. Production is never selectable.
        track = request(client, 'GET', track_url)
        if track.get('track') != 'internal':
            raise ValueError('Play did not confirm the internal track')
        body = {'track': 'internal', 'releases': [{'name': report['version_name'],
                'versionCodes': [str(report['version_code'])], 'status': 'completed'}]}
        request(client, 'PUT', track_url, json=body)
        request(client, 'POST', base + '/' + edit_id + ':validate', json={})
        receipt['state'] = 'validated_uncommitted_edit'
        if commit:
            result = request(client, 'POST', base + '/' + edit_id + ':commit', json={})
            if result.get('id') != edit_id:
                raise ValueError('Commit response did not confirm expected edit ID. Verify Console before retrying')
            receipt.update(state='internal_edit_committed', committed=True)
        return receipt
    except Exception:
        receipt['state'] = 'failed_or_commit_uncertain_inspect_console'
        # Never automatically retry commits or delete an edit after an uncertain commit.
        raise


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('aab')
    parser.add_argument('--policy', default='playstore/release-policy.json')
    parser.add_argument('--approval', help='Root-created approval JSON, bound to exact AAB hash')
    parser.add_argument('--execute', action='store_true', help='Create edit, upload, assign internal, validate. No commit by default')
    parser.add_argument('--commit', action='store_true', help='Also commit internal edit, requires separate root approval')
    parser.add_argument('--receipt', help='New receipt file, required with --execute, never overwrites')
    args = parser.parse_args()
    receipt = {'published': False, 'committed': False, 'track': 'internal'}
    handle = None
    try:
        if args.commit and not args.execute:
            raise ValueError('--commit requires --execute')
        report = check.inspect(args.aab, json.loads(Path(args.policy).read_text()))
        receipt.update(package=report['package'], version_code=report['version_code'], aab_sha256=report['aab_sha256'])
        if not args.execute:
            print(json.dumps({**receipt, 'state': 'offline_validated_no_network', 'next': 'Root must approve tested artifact and internal track before --execute'}, indent=2))
            return 0
        if not args.approval or not args.receipt:
            raise ValueError('--execute requires --approval and a new --receipt file')
        approval = json.loads(Path(args.approval).read_text())
        gate(report, approval, args.commit)
        # Reserve audit output before any network mutation.
        handle = open(args.receipt, 'x', encoding='utf-8')
        os.chmod(args.receipt, 0o600)
        client = session()
        publish(client, args.aab, report, approval, args.commit, receipt)
        print(json.dumps(receipt, indent=2))
        return 0
    except Exception as error:
        # Authentication libraries can include sensitive values in errors. Only expose our own ValueErrors.
        message = str(error) if isinstance(error, (ValueError, FileNotFoundError, FileExistsError)) else type(error).__name__
        print(f'NOT PUBLISHED / VERIFY RECEIPT: {message}', file=sys.stderr)
        return 1
    finally:
        if handle:
            json.dump(receipt, handle, indent=2)
            handle.write('\n')
            handle.close()


if __name__ == '__main__':
    sys.exit(main())
