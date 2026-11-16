#!/usr/bin/env python3
"""Preview or announce the actual release notes attached to a TestFlight build."""
import argparse
import json
import os
from pathlib import Path
import sys
from urllib.parse import urlencode, urlsplit

from post_discord_release import post_to_discord

APP = '6830033634'


def load_release(build_number, call):
    def get_page(path):
        status, response = call('GET', path)
        if status != 200:
            raise ValueError(f'App Store Connect read failed (HTTP {status})')
        return response

    def get(path):
        return get_page(path)['data']

    def all_items(path):
        seen = set()
        while path:
            if path in seen:
                raise ValueError('App Store Connect pagination loop')
            seen.add(path)
            page = get_page(path)
            yield from page['data']
            next_url = page.get('links', {}).get('next')
            if next_url:
                parts = urlsplit(next_url)
                if parts.scheme != 'https' or parts.netloc != 'api.appstoreconnect.apple.com':
                    raise ValueError('Unexpected App Store Connect pagination URL')
                path = parts.path + ('?' + parts.query if parts.query else '')
            else:
                path = None

    builds = get('/v1/builds?' + urlencode({'filter[app]': APP,
                 'filter[version]': build_number, 'limit': 2}))
    if len(builds) != 1:
        raise ValueError('Expected exactly one matching TestFlight build')
    build = builds[0]
    attrs = build['attributes']
    if str(attrs.get('version')) != str(build_number):
        raise ValueError('Apple returned a different build number')
    if attrs.get('processingState') != 'VALID' or attrs.get('expired') is not False:
        raise ValueError('TestFlight build is not valid or has expired')
    bid = build['id']
    version = get(f'/v1/builds/{bid}/preReleaseVersion')['attributes']['version']
    localizations = all_items(f'/v1/builds/{bid}/betaBuildLocalizations')
    notes = next((loc['attributes'].get('whatsNew', '').strip() for loc in localizations
                  if loc['attributes'].get('locale') == 'en-US'), '')
    if not notes:
        raise ValueError('Build has no en-US What to Test release notes')
    detail = get(f'/v1/builds/{bid}/buildBetaDetail')
    if detail['attributes'].get('externalBuildState') != 'IN_BETA_TESTING':
        raise ValueError('Build is not yet available to external TestFlight testers')
    groups = all_items(f'/v1/apps/{APP}/betaGroups')
    group = next((g for g in groups if g['attributes'].get('name') == 'Public Beta'), None)
    if not group or group['attributes'].get('publicLinkEnabled') is not True:
        raise ValueError('Public Beta group has no enabled public link')
    link = group['attributes'].get('publicLink', '')
    parts = urlsplit(link)
    if parts.scheme != 'https' or parts.hostname != 'testflight.apple.com' or not parts.path.startswith('/join/'):
        raise ValueError('Invalid TestFlight public link')
    members = all_items(f"/v1/betaGroups/{group['id']}/relationships/builds?limit=200")
    if not any(member['id'] == bid for member in members):
        raise ValueError('Build has not been assigned to the Public Beta group')
    return {'build_id': bid, 'build_number': attrs['version'], 'version': version,
            'notes': notes, 'public_link': link}


def format_message(release):
    title = f"**Jcode Mobile {release['version']} (build {release['build_number']})**\niOS TestFlight beta\n"
    suffix = f"\n\nTestFlight: <{release['public_link']}>"
    budget = 2000 - len(title) - len(suffix)
    if budget < 1:
        raise ValueError('Build metadata is too long for Discord')
    notes = release['notes']
    if len(notes) > budget:
        notes = notes[:budget - 1] + '…'
    return title + notes + suffix


def announce(build_number, call, webhook=None, execute=False, state_dir=None):
    release = load_release(build_number, call)
    content = format_message(release)
    if not execute:
        print(content)
        return
    if not webhook:
        raise ValueError('Missing DISCORD_RELEASE_WEBHOOK')
    root = Path(state_dir or Path.home() / '.local/share/jcode-mobile/testflight-announcements')
    root.mkdir(parents=True, exist_ok=True, mode=0o700)
    # ASC IDs are not paths. Reject unexpected data before constructing a filename.
    bid = release['build_id']
    if not bid or any(c not in 'abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-' for c in bid):
        raise ValueError('Invalid App Store Connect build ID')
    receipt = root / f'{bid}.json'
    record = {**release, 'state': 'posting_uncertain'}
    try:
        # Exclusive creation is a cross-process lock and a durable retry barrier.
        fd = os.open(receipt, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    except FileExistsError:
        saved = json.loads(receipt.read_text())
        if saved.get('state') == 'announced':
            print(f"TestFlight build {build_number} already announced, skipping")
            return
        raise ValueError('Previous post may have succeeded. Inspect Discord and receipt before retrying') from None
    with os.fdopen(fd, 'w') as handle:
        json.dump(record, handle, indent=2)
        handle.flush()
        os.fsync(handle.fileno())
    # Do not automatically retry an ambiguous timeout, even across invocations.
    message_id = post_to_discord(webhook, content)
    record.update(state='announced', message_id=str(message_id))
    with receipt.open('w') as handle:
        json.dump(record, handle, indent=2)
        handle.flush()
        os.fsync(handle.fileno())
    print(f'Posted TestFlight build {build_number} as Discord message {message_id}')
    return message_id


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--build', required=True, help='Exact CFBundleVersion, never defaults to newest')
    parser.add_argument('--execute', action='store_true', help='Post to Discord, default previews only')
    parser.add_argument('--state-dir', help='Persistent shared receipt directory on the release machine')
    args = parser.parse_args()
    from asc import call
    try:
        announce(args.build, call, os.environ.get('DISCORD_RELEASE_WEBHOOK'), args.execute, args.state_dir)
        return 0
    except ValueError as error:
        print(str(error), file=sys.stderr)
        return 1
    except Exception as error:
        print(f'TestFlight announcement failed ({type(error).__name__}). Inspect receipt and channel before retrying.', file=sys.stderr)
        return 1


if __name__ == '__main__':
    sys.exit(main())
