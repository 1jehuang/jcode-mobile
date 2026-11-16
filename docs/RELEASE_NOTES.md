# Mobile release notes and Discord announcements

## Preferred iOS flow: notes belong to the TestFlight build

Write release-specific notes once, then attach them to the exact TestFlight
build's **What to Test** field when distributing it:

```sh
python3 scripts/asc_public_beta.py BUILD_NUMBER --notes-file release-notes.md
```

This uses the existing distribution process: sets en-US notes for that build,
adds it to Public Beta, and submits for beta review if needed. It does not build
or upload an IPA. Run it only for an approved, tested candidate whose upload has
already processed successfully. `BUILD_NUMBER` is the IPA's `CFBundleVersion`,
not the marketing version `1.0` or an Apple build UUID.

With a release webhook configured on the release machine, this command
also attempts a Discord announcement, using the notes **read back from Apple**.
It requires an explicit build and explicit `--notes` or `--notes-file` when the
webhook is configured, so it cannot accidentally announce a different newest
build or generic testing instructions. `--announce-discord` additionally requires
the webhook to exist before any distribution changes are made.

The Discord post identifies Jcode Mobile, marketing version, build number, iOS
TestFlight beta, the attached notes, and the Public Beta join link. It only posts
after Apple confirms all of the following:

- Processing is `VALID` and the build is not expired.
- External build state is `IN_BETA_TESTING`, not merely waiting for review.
- Public Beta has an enabled TestFlight public link.
- The exact Apple build ID belongs to that group, following pagination.

If beta review is still pending, notes remain saved on the build and no Discord
post is made. After approval, preview or announce **without re-uploading,
resubmitting, or changing the notes**:

```sh
python3 scripts/testflight_release.py --build BUILD_NUMBER
python3 scripts/testflight_release.py --build BUILD_NUMBER --execute
```

No separate GitHub release or GitHub token is needed for this flow. The standalone
command only reads Apple data. It does not publish or enable a TestFlight build.
The current integration makes one availability check after distribution. It does
not install a background poller for builds that remain in Apple review.

### Credentials and duplicate protection

The release machine needs the existing App Store Connect key used by `asc.py`
(Python `cryptography` dependency). Do not move or change the Apple key setup.
All three release scripts automatically load the announcements-channel webhook
from `~/.config/jcode-mobile/discord-release-webhook`, or
`$XDG_CONFIG_HOME/jcode-mobile/discord-release-webhook` when set. Provision this
UTF-8 file securely outside the repository on each release machine, including
the managed VM. It must be a nonempty regular file owned by the current user
with owner-only permissions (for example, `0600`), not a symlink. Surrounding
whitespace is stripped. Missing files disable optional announcements, while
invalid or insecure files fail before distribution changes.

`DISCORD_RELEASE_WEBHOOK`, when present, takes precedence over the file (an
empty override disables optional announcements). Keep the webhook outside Git
and never print it or put its value in command history. The GitHub Actions
secret is configured separately and is not automatically available to local
release scripts. This does not change the direct TestFlight release process.

Receipts are stored under
`~/.local/share/jcode-mobile/testflight-announcements/APPLE_BUILD_ID.json`.
Use one release machine, or pass `--state-dir` to the standalone command and
`--announcement-state-dir` to the distribution command pointing to the **same
persistent shared directory**. Receipts skip already announced builds. Exclusive
creation prevents concurrent commands from posting the same build twice.

Before sending, the receipt is durably marked `posting_uncertain`. If the network
fails or a process exits during posting, future attempts refuse to repost until
an operator checks Discord. If a message exists, set the receipt's state to
`announced` and save its `message_id`. Only remove an uncertain receipt after
confirming that no message was delivered. Do not delete receipts or use ephemeral
state directories to retry. There is no automatic cross-machine deduplication
unless those machines share the receipt directory.

For iOS, choose this build-linked flow **or** the GitHub-release flow below,
not both for the same update, since their deduplication keys are different.

## Optional GitHub-release flow

For Android or releases tracked on GitHub, use the body of a published release
in `1jehuang/jcode-mobile`. The `discord-release.yml` workflow posts it with a
**Jcode Mobile** heading. Prereleases are labeled **beta**.

Set the repository Actions secret `DISCORD_RELEASE_WEBHOOK` to the existing
announcements-channel webhook, using the same value configured for the CLI and
Desktop repositories. `gh secret set DISCORD_RELEASE_WEBHOOK --repo
1jehuang/jcode-mobile` reads the value from standard input. Secrets cannot be
read back from another repository.

Publish the GitHub release only after the build is actually distributed. Identify
the platform, version/build and availability in the notes. Do not claim Android
availability unless its build is actually distributed.

If a publisher uses `GITHUB_TOKEN` to create the release, it must explicitly queue
the announcement because that token suppresses follow-up release events:

```sh
gh workflow run discord-release.yml --repo 1jehuang/jcode-mobile --ref main \
  -f tag=RELEASE_TAG
```

That command also backfills existing published releases. Runs are serialized per
tag. A hidden marker and Discord message ID in the release body prevent repeats.
Do not remove those markers. If a post succeeds but recording the marker fails,
inspect the channel and manually record
`<!-- jcode-mobile-discord-announced:TAG -->` before retrying.

## Tests

```sh
python3 -m unittest discover -s scripts -p 'test_*release.py' -v
python3 -m unittest discover -s scripts -p 'test_asc_public_beta.py' -v
```

Both announcement flows disable mentions and preserve the destination link when
truncating long notes. Announcement failure never undoes a shipped build.
