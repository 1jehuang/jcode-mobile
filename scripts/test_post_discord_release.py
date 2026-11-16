import contextlib
import io
import json
import unittest
from unittest.mock import patch, MagicMock

import post_discord_release as release


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
