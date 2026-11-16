#!/usr/bin/env python3
"""Ship a processed build to the public TestFlight group.

Usage: asc_public_beta.py [BUILD_VERSION] [--notes-file release-notes.md]
Defaults to the newest VALID build. Sets en-US "What to Test", adds the build
to the "Public Beta" group and submits it for beta app review (idempotent).
With DISCORD_RELEASE_WEBHOOK configured, announces the exact build's notes
only after external availability is confirmed. An exact build and explicit
notes are required when announcing.
"""
import argparse
import json, os, sys
from pathlib import Path
from urllib.parse import urlencode
sys.path.insert(0, os.path.dirname(__file__))
from asc import call

APP = "6830033634"
GROUP = "Public Beta"
DEFAULT_NOTES = (
    "Jcode for iPhone: chat with your Jcode Cloud coding agent from anywhere.\n\n"
    "Please try: signing in, starting a session, sending a task and following tool calls live, "
    "switching tabs, pull-to-talk voice dictation (pull the transcript up past the bottom), "
    "and the default directory setting for new sessions.\n\n"
    "Send feedback with a screenshot from TestFlight, or email jeremyhuang55555@gmail.com."
)


def must(res, what):
    status, data = res
    if status >= 300:
        sys.exit(f"{what} failed ({status}): {json.dumps(data)[:600]}")
    return data


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('build', nargs='?', help='Exact CFBundleVersion, defaults to newest VALID build')
    source = parser.add_mutually_exclusive_group()
    source.add_argument('--notes', help='Release-specific What to Test text')
    source.add_argument('--notes-file', type=Path, help='UTF-8 release notes attached to this build')
    parser.add_argument('--announce-discord', action='store_true', help='Announce when available, requires webhook')
    parser.add_argument('--announcement-state-dir', help='Persistent shared Discord receipt directory')
    args = parser.parse_args(argv)
    notes = args.notes_file.read_text(encoding='utf-8') if args.notes_file else args.notes
    if notes is not None and (not notes.strip() or len(notes) > 4000):
        parser.error('Release notes must contain 1 to 4000 characters')
    webhook = os.environ.get('DISCORD_RELEASE_WEBHOOK')
    if args.announce_discord and not webhook:
        parser.error('--announce-discord requires DISCORD_RELEASE_WEBHOOK')
    if (args.announce_discord or webhook) and (not args.build or notes is None):
        parser.error('Discord announcements require an exact build and --notes or --notes-file')
    notes = notes.strip() if notes is not None else DEFAULT_NOTES
    filters = {'filter[app]': APP, 'filter[processingState]': 'VALID', 'sort': '-uploadedDate', 'limit': 2 if args.build else 1}
    if args.build:
        filters['filter[version]'] = args.build
    q = '/v1/builds?' + urlencode(filters)
    builds = must(call('GET', q), 'find build')['data']
    if len(builds) != 1:
        parser.error('Expected exactly one matching processed build')
    build = builds[0]
    bid, ver = build["id"], build["attributes"]["version"]
    if args.build and ver != args.build:
        parser.error('Apple returned a different build number')
    if build['attributes'].get('expired') is True:
        parser.error('Cannot distribute an expired build')
    print("build", ver, bid)

    locs = must(call("GET", f"/v1/builds/{bid}/betaBuildLocalizations"), "localizations")["data"]
    loc = next((l for l in locs if l["attributes"]["locale"] == "en-US"), None)
    if loc:
        must(call("PATCH", f"/v1/betaBuildLocalizations/{loc['id']}", {"data": {
            "type": "betaBuildLocalizations", "id": loc["id"], "attributes": {"whatsNew": notes}}}), "notes")
    else:
        must(call("POST", "/v1/betaBuildLocalizations", {"data": {"type": "betaBuildLocalizations",
             "attributes": {"locale": "en-US", "whatsNew": notes},
             "relationships": {"build": {"data": {"type": "builds", "id": bid}}}}}), "notes")
    print("notes set")

    groups = must(call("GET", f"/v1/apps/{APP}/betaGroups"), "groups")["data"]
    group = next(g for g in groups if g["attributes"]["name"] == GROUP)
    must(call("POST", f"/v1/betaGroups/{group['id']}/relationships/builds",
              {"data": [{"type": "builds", "id": bid}]}), "add to group")
    print("added to", GROUP, group["attributes"].get("publicLink"))

    status, sub = call("GET", f"/v1/builds/{bid}/betaAppReviewSubmission")
    if status == 200 and sub.get("data"):
        print("beta review already", sub["data"]["attributes"]["betaReviewState"])
    elif status in (200, 404):
        data = must(call("POST", "/v1/betaAppReviewSubmissions", {"data": {"type": "betaAppReviewSubmissions",
                    "relationships": {"build": {"data": {"type": "builds", "id": bid}}}}}), "submit beta review")
        print("beta review", data["data"]["attributes"]["betaReviewState"])
    else:
        must((status, sub), 'read beta review before submitting')

    if webhook:
        from testflight_release import announce
        try:
            announce(ver, call, webhook, execute=True, state_dir=args.announcement_state_dir)
        except Exception as error:
            # Apple publication succeeded. Do not suggest rerunning distribution or
            # expose network exceptions containing the webhook's private token.
            print(f'Discord not confirmed ({type(error).__name__}). Build notes are saved in TestFlight. '
                  f'When available, run: python3 scripts/testflight_release.py --build {ver} --execute',
                  file=sys.stderr)


if __name__ == "__main__":
    main()
