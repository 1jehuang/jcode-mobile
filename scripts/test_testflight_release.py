"""Offline acceptance tests for TestFlight release eligibility and delivery receipts."""

import contextlib
import copy
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
from urllib.parse import unquote

import testflight_release as release


BUILD = '102'
BUILD_ID = 'build-102'
GROUP_ID = 'public-group'
PUBLIC_LINK = 'https://testflight.apple.com/join/AbCd1234'
BUILDS = '/v1/builds?filter[app]=6830033634&filter[version]=102&limit=2'
VERSION = f'/v1/builds/{BUILD_ID}/preReleaseVersion'
NOTES = f'/v1/builds/{BUILD_ID}/betaBuildLocalizations'
DETAIL = f'/v1/builds/{BUILD_ID}/buildBetaDetail'
GROUPS = '/v1/apps/6830033634/betaGroups'
MEMBERS = f'/v1/betaGroups/{GROUP_ID}/relationships/builds?limit=200'


def asc_fixture():
    build = {'type': 'builds', 'id': BUILD_ID, 'attributes': {
        'version': BUILD, 'processingState': 'VALID', 'expired': False,
        'expirationDate': '2099-01-01T00:00:00Z'}}
    return {
        BUILDS: (200, {'data': [build]}),
        VERSION: (200, {'data': {'id': 'version-1', 'attributes': {'version': '0.9.0'}}}),
        NOTES: (200, {'data': [
            {'id': 'fr', 'attributes': {'locale': 'fr-FR', 'whatsNew': 'French notes'}},
            {'id': 'en', 'attributes': {'locale': 'en-US', 'whatsNew': '- Faster chat\n- Voice fixes'}},
        ]}),
        DETAIL: (200, {'data': {'id': 'detail', 'attributes': {'externalBuildState': 'IN_BETA_TESTING'}}}),
        GROUPS: (200, {'data': [{'id': GROUP_ID, 'attributes': {
            'name': 'Public Beta', 'publicLinkEnabled': True, 'publicLink': PUBLIC_LINK}}]}),
        MEMBERS: (200, {'data': [copy.deepcopy(build)]}),
    }


class FakeASC:
    def __init__(self):
        self.responses = asc_fixture()
        self.calls = []

    def __call__(self, method, path, body=None):
        path = unquote(path)
        self.calls.append((method, path, body))
        if method != 'GET' or body is not None:
            raise AssertionError(f'Unexpected ASC mutation: {method} {path}')
        if path not in self.responses:
            raise AssertionError(f'Unexpected ASC path: {path}')
        return copy.deepcopy(self.responses[path])


class TestFlightEligibilityTests(unittest.TestCase):
    def test_loads_exact_public_external_build_and_english_notes(self):
        asc = FakeASC()
        item = release.load_release(BUILD, asc)
        self.assertEqual(item, {
            'build_id': BUILD_ID, 'build_number': BUILD, 'version': '0.9.0',
            'notes': '- Faster chat\n- Voice fixes', 'public_link': PUBLIC_LINK,
        })
        self.assertEqual([path for _, path, _ in asc.calls],
                         [BUILDS, VERSION, NOTES, DETAIL, GROUPS, MEMBERS])

    def assert_ineligible(self, change):
        asc = FakeASC()
        change(asc.responses)
        with self.assertRaises(ValueError):
            release.load_release(BUILD, asc)

    def test_missing_or_ambiguous_build_is_rejected(self):
        for count in (0, 2):
            with self.subTest(count=count):
                self.assert_ineligible(lambda responses: responses[BUILDS][1].update(
                    data=responses[BUILDS][1]['data'] * count))

    def test_processing_expired_or_wrong_build_is_rejected(self):
        for field, value in [('processingState', 'PROCESSING'), ('processingState', 'FAILED'),
                             ('expired', True), ('version', '101')]:
            with self.subTest(field=field, value=value):
                self.assert_ineligible(lambda responses: responses[BUILDS][1]['data'][0]['attributes'].update(
                    {field: value}))

    def test_external_beta_must_be_live(self):
        for state in ('WAITING_FOR_BETA_REVIEW', 'READY_FOR_BETA_TESTING', 'EXPIRED'):
            with self.subTest(state=state):
                self.assert_ineligible(lambda responses: responses[DETAIL][1]['data']['attributes'].update(
                    externalBuildState=state))

    def test_requires_nonempty_english_notes(self):
        for notes in ([], [{'attributes': {'locale': 'fr-FR', 'whatsNew': 'Bonjour'}}],
                      [{'attributes': {'locale': 'en-US', 'whatsNew': '  \n'}}]):
            with self.subTest(notes=notes):
                self.assert_ineligible(lambda responses: responses[NOTES][1].update(data=notes))

    def test_requires_enabled_public_beta_group_and_testflight_link(self):
        for field, value in [('name', 'Internal Beta'), ('publicLinkEnabled', False),
                             ('publicLink', ''), ('publicLink', 'https://example.com/join/abc')]:
            with self.subTest(field=field, value=value):
                self.assert_ineligible(lambda responses: responses[GROUPS][1]['data'][0]['attributes'].update(
                    {field: value}))

    def test_build_must_belong_to_public_group(self):
        for members in ([], [{'id': 'different-build'}]):
            with self.subTest(members=members):
                self.assert_ineligible(lambda responses: responses[MEMBERS][1].update(data=members))

    def test_membership_accepts_exact_id_among_other_group_builds(self):
        asc = FakeASC()
        asc.responses[MEMBERS][1]['data'] = [{'id': 'other'}, {'id': BUILD_ID}]
        self.assertEqual(release.load_release(BUILD, asc)['build_id'], BUILD_ID)

    def test_membership_follows_asc_pagination(self):
        asc = FakeASC()
        next_path = MEMBERS + '&cursor=next'
        asc.responses[MEMBERS] = (200, {'data': [{'id': 'other'}], 'links': {
            'next': 'https://api.appstoreconnect.apple.com' + next_path}})
        asc.responses[next_path] = (200, {'data': [{'id': BUILD_ID}]})
        self.assertEqual(release.load_release(BUILD, asc)['build_id'], BUILD_ID)
        self.assertIn(('GET', next_path, None), asc.calls)

    def test_membership_rejects_pagination_loop(self):
        self.assert_ineligible(lambda responses: responses[MEMBERS][1].update(
            data=[], links={'next': 'https://api.appstoreconnect.apple.com' + MEMBERS}))

    def test_membership_rejects_untrusted_pagination(self):
        self.assert_ineligible(lambda responses: responses[MEMBERS][1].update(
            data=[], links={'next': 'https://example.com/steal-credentials'}))

    def test_api_failure_is_not_eligible(self):
        self.assert_ineligible(lambda responses: responses.update({BUILDS: (403, {'errors': []})}))

    def test_message_contains_release_identity_notes_and_link(self):
        item = release.load_release(BUILD, FakeASC())
        message = release.format_message(item)
        for expected in ('0.9.0', BUILD, '- Faster chat', '- Voice fixes', PUBLIC_LINK):
            self.assertIn(expected, message)
        self.assertLessEqual(len(message), 2000)

    def test_long_notes_preserve_identity_and_public_link(self):
        item = release.load_release(BUILD, FakeASC())
        item['notes'] = 'A useful release note. ' * 1000
        message = release.format_message(item)
        self.assertLessEqual(len(message), 2000)
        for expected in ('0.9.0', BUILD, 'A useful release note.', PUBLIC_LINK):
            self.assertIn(expected, message)


class TestFlightAnnouncementTests(unittest.TestCase):
    def receipt(self, directory):
        files = list(Path(directory).rglob('*.json'))
        self.assertEqual(len(files), 1, files)
        return json.loads(files[0].read_text())

    def receipt_state(self, receipt):
        return receipt.get('status', receipt.get('state'))

    def retry_without_posting(self, directory, post):
        try:
            release.announce(BUILD, FakeASC(), webhook='webhook', execute=True, state_dir=directory)
        except (ValueError, RuntimeError):
            pass
        self.assertEqual(post.call_count, 1)

    @patch.object(release, 'post_to_discord')
    def test_default_preview_has_no_send_or_filesystem_writes(self, post):
        with tempfile.TemporaryDirectory() as root:
            state = Path(root) / 'not-created'
            with contextlib.redirect_stdout(io.StringIO()):
                release.announce(BUILD, FakeASC(), state_dir=state)
            self.assertFalse(state.exists())
            post.assert_not_called()

    @patch.object(release, 'post_to_discord')
    def test_execute_requires_webhook_before_writing_receipt(self, post):
        with tempfile.TemporaryDirectory() as root:
            with self.assertRaises(ValueError):
                release.announce(BUILD, FakeASC(), execute=True, state_dir=root)
            self.assertEqual(list(Path(root).iterdir()), [])
            post.assert_not_called()

    @patch.object(release, 'post_to_discord')
    def test_ineligible_build_never_posts_or_creates_receipt(self, post):
        asc = FakeASC()
        asc.responses[BUILDS][1]['data'][0]['attributes']['expired'] = True
        with tempfile.TemporaryDirectory() as root:
            with self.assertRaises(ValueError):
                release.announce(BUILD, asc, webhook='webhook', execute=True, state_dir=root)
            self.assertEqual(list(Path(root).iterdir()), [])
            post.assert_not_called()

    @patch.object(release, 'post_to_discord')
    def test_pending_is_durable_before_send_and_success_is_not_reposted(self, post):
        with tempfile.TemporaryDirectory() as root:
            def send(webhook, content):
                self.assertEqual(webhook, 'webhook')
                self.assertIn(PUBLIC_LINK, content)
                self.assertIn('- Faster chat', content)
                self.assertEqual(self.receipt_state(self.receipt(root)), 'posting_uncertain')
                return 'discord-message-123'
            post.side_effect = send
            release.announce(BUILD, FakeASC(), webhook='webhook', execute=True, state_dir=root)
            receipt = self.receipt(root)
            self.assertEqual(self.receipt_state(receipt), 'announced')
            self.assertEqual(receipt['message_id'], 'discord-message-123')
            self.assertEqual(receipt['build_id'], BUILD_ID)
            self.retry_without_posting(root, post)

    @patch.object(release, 'post_to_discord', side_effect=OSError('uncertain delivery'))
    def test_uncertain_delivery_keeps_pending_receipt_and_prevents_retry(self, post):
        with tempfile.TemporaryDirectory() as root:
            try:
                release.announce(BUILD, FakeASC(), webhook='webhook', execute=True, state_dir=root)
            except (OSError, ValueError, RuntimeError):
                pass
            self.assertEqual(self.receipt_state(self.receipt(root)), 'posting_uncertain')
            self.retry_without_posting(root, post)


if __name__ == '__main__':
    unittest.main()
