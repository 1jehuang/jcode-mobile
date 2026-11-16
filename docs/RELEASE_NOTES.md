# Mobile release notes and Discord announcements

Mobile release notes are the body of a published GitHub release in
`1jehuang/jcode-mobile`. The `discord-release.yml` workflow posts them to the
same announcements channel used by the CLI and Desktop, with a **Jcode Mobile**
heading. Prereleases are labeled **beta**.

## One-time setup

Set the repository Actions secret `DISCORD_RELEASE_WEBHOOK` to the existing
announcements-channel webhook, using the same value configured for the CLI and
Desktop repositories. Never commit or print the webhook URL. For example,
`gh secret set DISCORD_RELEASE_WEBHOOK --repo 1jehuang/jcode-mobile` reads it
from standard input. Secrets cannot be read back from another repository.

## Shipping an update

1. Build, test and distribute the mobile build using the existing iOS or Android
   process. An upload or pending TestFlight review is not a public release.
2. Write concise user-facing notes explaining what changed. Identify the
   platform, version/build, availability, and relevant fixes or features. For
   iOS betas include the public TestFlight link from the README. Do not claim
   Android availability unless the build is actually distributed.
3. Publish a GitHub release pointing at the shipped source commit, with those
   notes. Mark TestFlight releases as prereleases. Publishing triggers Discord.
   Example, after the build is available:

   ```sh
   gh release create ios-v0.1.0-build.123 --repo 1jehuang/jcode-mobile \
     --target SHIPPED_COMMIT --prerelease --title 'Jcode Mobile 0.1.0 (123)' \
     --notes-file release-notes.md
   ```

If a future publisher uses `GITHUB_TOKEN` to create the release, it must explicitly
queue the announcement because that token suppresses follow-up release events:

```sh
gh workflow run discord-release.yml --repo 1jehuang/jcode-mobile --ref main \
  -f tag=ios-v0.1.0-build.123
```

That command also backfills existing published releases. The workflow serializes
runs for each tag and records a hidden marker and Discord message ID in the
release body to skip repeat announcements. Do not remove those markers.

## Preview and tests

```sh
GH_TOKEN="$(gh auth token)" python3 scripts/post_discord_release.py --tag TAG
python3 -m unittest discover -s scripts -p 'test_post_discord_release.py' -v
```

The script only previews unless `--execute` is supplied. It rejects draft and
empty-note releases, disables mentions, and truncates long notes while preserving
the full release link. Discord failure does not undo an already shipped build.
There is no atomic transaction between Discord and GitHub: if a post succeeds
but recording its marker fails, the script warns with the message ID. Inspect the
channel and manually record `<!-- jcode-mobile-discord-announced:TAG -->` in the
release body before retrying. Do not blindly rerun after a network timeout.
