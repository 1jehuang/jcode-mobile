import contextlib
import io
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import asc_public_beta as beta


class PublicBetaTests(unittest.TestCase):
    def setUp(self):
        self.requests = []
        self.localizations = []
        self.review_status = 200
        self.review = {'data': {'attributes': {'betaReviewState': 'APPROVED'}}}
        self.builds = [{'id': 'build-id', 'attributes': {'version': '123', 'expired': False}}]
        self.env = patch.dict(os.environ, {}, clear=True)
        self.env.start()
        self.addCleanup(self.env.stop)
        directory = self.enterContext(tempfile.TemporaryDirectory())
        os.environ['XDG_CONFIG_HOME'] = directory
        self.config = Path(directory) / 'jcode-mobile/discord-release-webhook'
        self.enterContext(patch.object(Path, 'home', return_value=Path(directory) / 'home'))

    def call(self, method, path, body=None):
        self.requests.append((method, path, body))
        if path.startswith('/v1/builds?'):
            return 200, {'data': self.builds}
        if path == '/v1/builds/build-id/betaBuildLocalizations':
            return 200, {'data': self.localizations}
        if path == '/v1/apps/6830033634/betaGroups':
            return 200, {'data': [{'id': 'group-id', 'attributes': {
                'name': 'Public Beta', 'publicLink': 'https://testflight.apple.com/join/abc'}}]}
        if path == '/v1/builds/build-id/betaAppReviewSubmission':
            return self.review_status, self.review
        if path == '/v1/betaAppReviewSubmissions':
            return 201, {'data': {'attributes': {'betaReviewState': 'WAITING_FOR_REVIEW'}}}
        return 200, {}

    def run_beta(self, args):
        with patch.object(beta, 'call', side_effect=self.call), contextlib.redirect_stdout(io.StringIO()):
            beta.main(args)

    def test_notes_file_attached_to_exact_build(self):
        with tempfile.TemporaryDirectory() as directory:
            notes = Path(directory) / 'notes.md'
            notes.write_text('- Voice improvements\n- Faster chat\n')
            self.run_beta(['123', '--notes-file', str(notes)])
        post = next(req for req in self.requests if req[:2] == ('POST', '/v1/betaBuildLocalizations'))
        self.assertEqual(post[2]['data']['attributes']['whatsNew'], '- Voice improvements\n- Faster chat')
        self.assertEqual(post[2]['data']['relationships']['build']['data']['id'], 'build-id')
        self.assertIn('filter%5Bversion%5D=123', self.requests[0][1])

    def test_updates_existing_build_notes(self):
        self.localizations = [{'id': 'loc-id', 'attributes': {'locale': 'en-US'}}]
        self.run_beta(['123', '--notes', 'Build-specific fixes'])
        request = next(req for req in self.requests if req[0] == 'PATCH')
        self.assertEqual(request[1], '/v1/betaBuildLocalizations/loc-id')
        self.assertEqual(request[2]['data']['attributes']['whatsNew'], 'Build-specific fixes')

    @patch('testflight_release.announce')
    def test_private_config_announces(self, announce):
        self.config.parent.mkdir()
        self.config.write_text(' private-test-webhook\n', encoding='utf-8')
        self.config.chmod(0o600)
        self.run_beta(['123', '--notes', 'Build fixes'])
        self.assertEqual(announce.call_args.args[2], 'private-test-webhook')

    def test_insecure_config_fails_before_apple_changes(self):
        self.config.parent.mkdir()
        self.config.write_text('private-test-webhook', encoding='utf-8')
        self.config.chmod(0o644)
        stderr = io.StringIO()
        with contextlib.redirect_stderr(stderr), self.assertRaises(SystemExit):
            self.run_beta(['123', '--notes', 'Build fixes'])
        self.assertFalse(self.requests)
        self.assertNotIn('private-test-webhook', stderr.getvalue())

    @patch('testflight_release.announce')
    def test_configured_webhook_announces_even_if_already_reviewed(self, announce):
        os.environ['DISCORD_RELEASE_WEBHOOK'] = 'private-webhook'
        self.run_beta(['123', '--notes', 'Build fixes', '--announcement-state-dir', '/state'])
        announce.assert_called_once()
        self.assertEqual(announce.call_args.args[0], '123')
        self.assertEqual(announce.call_args.kwargs, {'execute': True, 'state_dir': '/state'})
        self.assertFalse(any(req[1] == '/v1/betaAppReviewSubmissions' for req in self.requests))

    @patch('testflight_release.announce', side_effect=ValueError('Not available yet'))
    def test_pending_review_does_not_fail_distribution(self, announce):
        os.environ['DISCORD_RELEASE_WEBHOOK'] = 'private-webhook'
        self.review_status, self.review = 404, {}
        stderr = io.StringIO()
        with contextlib.redirect_stderr(stderr):
            self.run_beta(['123', '--notes', 'Build fixes'])
        self.assertTrue(any(req[1] == '/v1/betaAppReviewSubmissions' for req in self.requests))
        self.assertIn('--build 123 --execute', stderr.getvalue())
        self.assertNotIn('private-webhook', stderr.getvalue())

    def test_empty_notes_fail_before_network(self):
        with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
            self.run_beta(['123', '--notes', '   '])
        self.assertFalse(self.requests)

    def test_announcement_requires_exact_build_and_notes(self):
        os.environ['DISCORD_RELEASE_WEBHOOK'] = 'webhook'
        for args in [[], ['123'], ['--notes', 'Fixes']]:
            with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
                self.run_beta(args)
        self.assertFalse(self.requests)

    def test_explicit_announcement_requires_webhook(self):
        with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
            self.run_beta(['123', '--notes', 'Fixes', '--announce-discord'])
        self.assertFalse(self.requests)

    def test_review_read_error_never_submits(self):
        self.review_status, self.review = 503, {}
        with self.assertRaises(SystemExit):
            self.run_beta(['123', '--notes', 'Fixes'])
        self.assertFalse(any(req[1] == '/v1/betaAppReviewSubmissions' for req in self.requests))

    def test_wrong_or_missing_build_never_writes(self):
        for builds in [[], [{'id': 'wrong', 'attributes': {'version': '456'}}]]:
            self.requests.clear()
            self.builds = builds
            with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
                self.run_beta(['123', '--notes', 'Fixes'])
            self.assertTrue(all(req[0] == 'GET' for req in self.requests))


if __name__ == '__main__':
    unittest.main()
