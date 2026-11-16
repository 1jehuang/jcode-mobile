#!/usr/bin/env python3
"""Announce a published Jcode Mobile GitHub release. Dry-run unless --execute."""
import argparse
import json
import os
import re
import sys
import urllib.parse
import urllib.request

MARKER = 'jcode-mobile-discord-announced'
LIMIT = 2000


def marker(tag):
    return f'<!-- {MARKER}:{tag} -->'


def format_message(release):
    tag = release['tag_name']
    notes = re.sub(r'<!--.*?-->', '', release.get('body') or '', flags=re.DOTALL).strip()
    if not notes:
        raise ValueError('Release must contain release notes before announcing')
    title = f"**Jcode Mobile {tag[:120]}**" + (' (beta)' if release.get('prerelease') else '')
    # Always retain the full notes link, even when the body must be shortened.
    link = release['html_url']
    if len(link) > 500:
        raise ValueError('Release URL is too long')
    suffix = f'\n\nFull release notes: <{link}>'
    budget = LIMIT - len(title) - 1 - len(suffix)
    if len(notes) > budget:
        notes = notes[:budget - 1] + '…'
    return title + '\n' + notes + suffix


def github_request(repository, path, token, payload=None):
    request = urllib.request.Request(
        f'https://api.github.com/repos/{repository}/releases/{path}',
        data=None if payload is None else json.dumps(payload).encode(),
        method='GET' if payload is None else 'PATCH',
        headers={'Authorization': f'Bearer {token}', 'Accept': 'application/vnd.github+json',
                 'Content-Type': 'application/json', 'User-Agent': 'jcode-mobile-release-bot'})
    with urllib.request.urlopen(request, timeout=30) as response:
        return json.load(response)


def post_to_discord(webhook, content):
    parts = urllib.parse.urlsplit(webhook)
    if (parts.scheme != 'https' or parts.hostname not in ('discord.com', 'discordapp.com')
            or not parts.path.startswith('/api/webhooks/')):
        raise ValueError('Expected a Discord HTTPS webhook URL')
    query = [(key, value) for key, value in urllib.parse.parse_qsl(parts.query) if key != 'wait']
    url = urllib.parse.urlunsplit((parts.scheme, parts.netloc, parts.path,
                                  urllib.parse.urlencode(query + [('wait', 'true')]), ''))
    request = urllib.request.Request(url, data=json.dumps({
        'content': content, 'allowed_mentions': {'parse': []},
    }).encode(), headers={'Content-Type': 'application/json', 'User-Agent': 'jcode-mobile-release-bot'})
    with urllib.request.urlopen(request, timeout=30) as response:
        message = json.load(response)
    if not message.get('id'):
        raise ValueError('Discord did not acknowledge the announcement. Inspect channel before retrying')
    return message['id']


def announce(repository, tag, token, webhook=None, execute=False):
    path = 'tags/' + urllib.parse.quote(tag, safe='')
    release = github_request(repository, path, token)
    if release.get('draft') or not release.get('published_at'):
        raise ValueError('Only published releases can be announced')
    if marker(tag) in (release.get('body') or ''):
        print(f'{tag} already announced, skipping')
        return
    content = format_message(release)
    if not execute:
        print(content)
        return
    if not webhook:
        raise ValueError('Missing DISCORD_RELEASE_WEBHOOK')
    message_id = post_to_discord(webhook, content)
    print(f'Posted Jcode Mobile {tag} as Discord message {message_id}')
    try:
        # Preserve edits made while posting. Include message ID for manual recovery.
        latest = github_request(repository, path, token)
        body = latest.get('body') or ''
        if marker(tag) not in body:
            github_request(repository, str(release['id']), token,
                           {'body': body.rstrip() + f'\n\n{marker(tag)}\n<!-- Discord message: {message_id} -->\n'})
    except Exception:
        # Do not invite an automatic retry after the public side effect succeeded.
        print(f'::warning::Discord message {message_id} posted, but recording the release marker failed. '
              'Record the marker manually before rerunning.', file=sys.stderr)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--tag', required=True)
    parser.add_argument('--repository', default=os.environ.get('GITHUB_REPOSITORY', '1jehuang/jcode-mobile'))
    parser.add_argument('--execute', action='store_true', help='Actually post, default only previews')
    args = parser.parse_args()
    token = os.environ.get('GH_TOKEN') or os.environ.get('GITHUB_TOKEN')
    if not token:
        parser.error('GH_TOKEN or GITHUB_TOKEN required')
    try:
        announce(args.repository, args.tag, token, os.environ.get('DISCORD_RELEASE_WEBHOOK'), args.execute)
        return 0
    except Exception as error:
        # Network exceptions can contain the webhook token. Never print their URLs or bodies.
        print(f'Announcement failed ({type(error).__name__}). Inspect release and channel before retrying.', file=sys.stderr)
        return 1


if __name__ == '__main__':
    sys.exit(main())
