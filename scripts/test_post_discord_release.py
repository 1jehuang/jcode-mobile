import contextlib
import io
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch, MagicMock

import post_discord_release as release


class WebhookConfigTests(unittest.TestCase):
    def setUp(self):
        self.root = Path(self.enterContext(tempfile.TemporaryDirectory()))
        self.enterContext(patch.dict(os.environ, {'XDG_CONFIG_HOME': str(self.root)}, clear=True))
        self.enterContext(patch.object(Path, 'home', return_value=self.root / 'home'))
        self.enterContext(patch.object(release.urllib.request, 'urlopen', side_effect=AssertionError('No network')))
        self.path = self.root / 'jcode-mobile' / 'discord-release-webhook'
        self.path.parent.mkdir()

    def write_config(self, content='  private-test-webhook\n', mode=0o600):
        self.path.write_text(content, encoding='utf-8')
        self.path.chmod(mode)

    def test_missing_is_optional(self):
        self.assertIsNone(release.load_webhook())

    def test_private_utf8_file_is_stripped(self):
        self.write_config('  private-test-webhook-é\n')
        self.assertEqual(release.load_webhook(), 'private-test-webhook-é')

    def test_env_wins_without_opening_config_even_if_empty(self):
        for value in ('environment-webhook', ''):
            os.environ['DISCORD_RELEASE_WEBHOOK'] = value
            with patch.object(release.os, 'open', side_effect=AssertionError('No file access')):
                self.assertEqual(release.load_webhook(), value)

    def test_default_home_config(self):
        del os.environ['XDG_CONFIG_HOME']
        self.path = self.root / 'home/.config/jcode-mobile/discord-release-webhook'
        self.path.parent.mkdir(parents=True)
        self.write_config()
        self.assertEqual(release.load_webhook(), 'private-test-webhook')

    def test_empty_or_non_utf8_rejected_without_disclosure(self):
        for content in (b'  \n', b'private-test-webhook\xff'):
            self.path.write_bytes(content)
            self.path.chmod(0o600)
            with self.assertRaises(ValueError) as caught:
                release.load_webhook()
            self.assertNotIn('private-test-webhook', str(caught.exception))

    def test_all_group_and_other_permissions_rejected(self):
        for bit in (0o040, 0o020, 0o010, 0o004, 0o002, 0o001):
            self.write_config(mode=0o600 | bit)
            with self.assertRaises(ValueError):
                release.load_webhook()

    def test_symlink_directory_and_fifo_rejected(self):
        target = self.root / 'target'
        target.write_text('private-test-webhook')
        target.chmod(0o600)
        self.path.symlink_to(target)
        with self.assertRaises(ValueError):
            release.load_webhook()
        self.path.unlink()
        self.path.mkdir(mode=0o700)
        with self.assertRaises(ValueError):
            release.load_webhook()
        self.path.rmdir()
        os.mkfifo(self.path, 0o600)
        with self.assertRaises(ValueError):
            release.load_webhook()

    def test_wrong_owner_rejected(self):
        self.write_config()
        with patch.object(release.os, 'getuid', return_value=os.getuid() + 1):
            with self.assertRaises(ValueError):
                release.load_webhook()

    def test_github_main_uses_loader(self):
        self.write_config()
        os.environ['GH_TOKEN'] = 'test-token'
        with patch('sys.argv', ['post_discord_release.py', '--tag', 'v1']), patch.object(release, 'announce') as announce:
            self.assertEqual(release.main(), 0)
        self.assertEqual(announce.call_args.args[3], 'private-test-webhook')

    def test_testflight_main_uses_loader(self):
        import testflight_release
        self.write_config()
        with patch('sys.argv', ['testflight_release.py', '--build', '123']), patch.object(testflight_release, 'announce') as announce:
            self.assertEqual(testflight_release.main(), 0)
        self.assertEqual(announce.call_args.args[2], 'private-test-webhook')


def fixture(**kwargs):
    return dict(id=12, tag_name='v0.1.0', body='- Faster chat\n- Voice fixes',
                html_url='https://github.com/1jehuang/jcode-mobile/releases/tag/v0.1.0',
                published_at='2026-11-16T00:00:00Z', draft=False, **kwargs)


class ReleaseTests(unittest.TestCase):
    def test_mobile_title_beta_and_notes(self):
        text = release.format_message(fixture(prerelease=True))
        self.assertIn('**Jcode Mobile v0.1.0** (beta)', text)
        self.assertIn('- Faster chat', text)

    def test_truncation_keeps_link_and_hides_markers(self):
        item = fixture()
        item['body'] = '<!-- private marker -->' + 'x' * 10000
        text = release.format_message(item)
        self.assertLessEqual(len(text), 2000)
        self.assertNotIn('private marker', text)
        self.assertTrue(text.endswith('<' + item['html_url'] + '>'))

    def test_empty_notes_rejected(self):
        item = fixture()
        item['body'] = '<!-- hidden -->'
        with self.assertRaises(ValueError):
            release.format_message(item)

    @patch.object(release, 'post_to_discord')
    @patch.object(release, 'github_request')
    def test_dry_run_never_posts(self, github, post):
        github.return_value = fixture()
        with contextlib.redirect_stdout(io.StringIO()):
            release.announce('owner/repo', 'v0.1.0', 'token')
        post.assert_not_called()
        self.assertEqual(github.call_count, 1)

    @patch.object(release, 'post_to_discord')
    @patch.object(release, 'github_request')
    def test_draft_and_unpublished_rejected(self, github, post):
        for field, value in [('draft', True), ('published_at', None)]:
            item = fixture()
            item[field] = value
            github.return_value = item
            with self.assertRaises(ValueError):
                release.announce('owner/repo', 'v0.1.0', 'token', 'webhook', True)
        post.assert_not_called()

    @patch.object(release, 'post_to_discord')
    @patch.object(release, 'github_request')
    def test_duplicate_skipped(self, github, post):
        item = fixture()
        item['body'] += release.marker('v0.1.0')
        github.return_value = item
        release.announce('owner/repo', 'v0.1.0', 'token', 'webhook', True)
        post.assert_not_called()

    @patch.object(release, 'post_to_discord', return_value='123')
    @patch.object(release, 'github_request')
    def test_marks_success_preserving_new_edits(self, github, post):
        latest = fixture()
        latest['body'] += '\nNew edit'
        github.side_effect = [fixture(), latest, {}]
        release.announce('owner/repo', 'v0.1.0', 'token', 'webhook', True)
        post.assert_called_once()
        body = github.call_args.args[3]['body']
        self.assertIn('New edit', body)
        self.assertIn(release.marker('v0.1.0'), body)
        self.assertIn('Discord message: 123', body)

    @patch.object(release, 'post_to_discord', return_value='123')
    @patch.object(release, 'github_request')
    def test_marker_failure_warns_without_retry(self, github, post):
        github.side_effect = [fixture(), OSError('private webhook token')]
        stderr = io.StringIO()
        with contextlib.redirect_stderr(stderr):
            release.announce('owner/repo', 'v0.1.0', 'token', 'webhook', True)
        self.assertIn('123 posted', stderr.getvalue())
        self.assertNotIn('private webhook token', stderr.getvalue())
        post.assert_called_once()

    @patch.object(release.urllib.request, 'urlopen')
    def test_no_mentions_and_wait_for_acknowledgement(self, urlopen):
        response = MagicMock()
        response.read.return_value = b'{"id": "123"}'
        urlopen.return_value.__enter__.return_value = response
        release.post_to_discord('https://discord.com/api/webhooks/1/token?wait=false', '@everyone')
        request = urlopen.call_args.args[0]
        self.assertIn('wait=true', request.full_url)
        self.assertNotIn('wait=false', request.full_url)
        self.assertEqual(json.loads(request.data)['allowed_mentions'], {'parse': []})

    @patch.object(release.urllib.request, 'urlopen')
    def test_invalid_webhook_rejected_before_network(self, urlopen):
        with self.assertRaises(ValueError):
            release.post_to_discord('https://example.com/api/webhooks/1/token', 'notes')
        urlopen.assert_not_called()


if __name__ == '__main__':
    unittest.main()
