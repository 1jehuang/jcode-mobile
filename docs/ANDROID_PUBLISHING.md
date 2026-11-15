# Android publishing preparation

Status on **November 14, 2026: NOT READY / NOT PUBLISHED**. This tooling does not
establish GPUI UI parity, emulator success, store eligibility, or permission to
publish by itself. No upload or edit creation is allowed until the coordinating
root confirms the tested release and internal track. Production is intentionally
not implemented.

## Current Google requirements and primary references

Checked against Google documentation on November 14, 2026:

- New mobile apps and updates must target Android 16 / API 36 starting August
  31, 2026. The November 1, 2026 extension date has passed.
  <https://support.google.com/googleplay/android-developer/answer/11926878>
- Native Android 15+ submissions must support 16 KiB page sizes. Check both ELF
  LOAD alignment and installation/runtime behavior of APKs generated from the
  release AAB on a 16 KiB device or emulator. ELF checks alone are insufficient.
  <https://developer.android.com/guide/practices/page-sizes>
- AABs are signed with `jarsigner`, not `apksigner`.
  <https://developer.android.com/build/building-cmdline>
- Enable the Android Publisher API and grant the service account appropriate
  Play Console app permissions. Cloud IAM access alone is not Play access.
  <https://developers.google.com/android-publisher/getting_started>
- Publishing edits apply to an existing Console app. Bootstrap the app and first
  artifact through Console as required. Creating an edit can invalidate another
  edit, and Console changes can invalidate an open API edit. Coordinate first.
  Changes are not live until commit.
  <https://developers.google.com/android-publisher/edits>
- API bundle upload and track schema:
  <https://developers.google.com/android-publisher/api-ref/rest/v3/edits.bundles/upload>
  <https://developers.google.com/android-publisher/api-ref/rest/v3/edits.tracks>

Recheck policy at release time. The validator enforces at least target 36, not an
indefinite promise of compliance with future policies.

## Safe local commands

From the repository root, with Python 3 and a JDK:

```bash
export JAVA_HOME="$HOME/Android/jdk-21"
export BUNDLETOOL_JAR=/absolute/path/to/bundletool-all.jar
python3 scripts/android-release-check-test.py
python3 scripts/android-release-check.py target/android/JcodeMobile.aab
python3 scripts/android-publish.py target/android/JcodeMobile.aab
```

The last two commands are offline and do not load Google credentials. Missing
AAB/tooling/signature pin or a policy mismatch exits nonzero. The publisher's
default mode is an artifact dry run, **not full publishing readiness**.

`playstore/release-policy.json` pins the manifest package, version, minimum SDK,
exact permissions, target SDK floor, and upload certificate SHA-256. Its signer
is deliberately blank. Review the actual release manifest and independently
verify the real upload certificate in Play Console before filling the pin. Never
pin the debug key to make a check green. Version codes must also be unused and
monotonically increasing in Console, which offline checks cannot determine.

The validator requires bundletool validation, genuine AAB layout, arm64 native
code, 16 KiB ELF LOAD alignment, manifest policy match, non-debug application,
JAR signature verification without unsigned entries, one pinned signer, and no
standard Android Debug certificate identity. Certificate chain trust is not a
public CA requirement for Android upload keys. A successful artifact check is
not a successful Play validation or device test.

## Credentials and blockers discovered safely

Bounded discovery on this machine found:

- No signing or Google/Play credential environment variable names were present.
- `gcloud` was not on PATH. `~/.config/gcloud` contained `gce` and `logs`, but no
  application-default credentials file was observed.
- `google.auth` was not installed in the default Python environment.
- The only keystore found in the bounded home-directory scan was
  `~/.android/jcode-debug.keystore`, which is forbidden for publication.
- A JDK exists at `~/Android/jdk-21`. This is not a release credential.

No secret contents, tokens, private keys, or account identities were printed.
This is not proof that the user has no Google account, only that usable Play
API authorization and a production upload signer have not been established.
No authenticated API probe or browser login was performed. Play app existence,
API enablement, grants, developer verification, and account eligibility remain
unverified. Do not create accounts, make payments, or grant permissions silently.

When root is ready, configure an existing authorized service account via ADC
outside the checkout, for example an externally stored credentials file named by
`GOOGLE_APPLICATION_CREDENTIALS`, or a properly configured workload identity.
Use a dedicated environment with `google-auth` and `requests`. Do not commit key
files or paste tokens into commands, logs, or approvals. Grant only necessary
app-scoped testing release permissions, not financial/production access.

## Human review and tested-artifact root gate

`playstore/listing.template.json` is intentionally blank and never uploaded by
these scripts. Owners must supply truthful listing copy, support contacts,
privacy policy, actual screenshots, content/age/ads declarations, review access,
data-safety answers, permission purposes, and account-deletion review as
applicable. Do not infer privacy claims from permission names. Review audio,
notifications, backend connections, chat content, authentication and any SDK data
flows against actual code and deployment. Complete Console declarations and
Play App Signing setup manually with the account owner.

After GPUI parity and emulator validation, test installable APKs generated from
the exact release AAB, including 16 KiB operation. Root copies
`playstore/approval.template.json` to a local file, records evidence, approver,
package/version and exact validated AAB SHA-256, and explicitly sets only facts
that have been verified. Every approval defaults false. General user intent to
publish is not a substitute for this tested-artifact gate.

Booleans and a free-form evidence string alone are not accepted. The approval
must also contain `android_parity_evidence` bound to the same AAB SHA-256,
`renderer=shared_gpui_native`, a source revision, and coordinating root acceptance
with the same approver identity. All 13 named checks in the template must have
`status=passed`, a nonempty real evidence file path, and its exact SHA-256. Paths
are resolved from the invocation directory, so absolute local paths are safest.
Reports may cover multiple checks, but must actually document each covered
check. Keep screenshots, emulator logs, interaction reports, native build/source
trace and device-service results as reviewable evidence. Missing, blocked,
failed, skipped, changed, empty or absent evidence rejects the remote operation.
This intentionally includes notification **device push**, not merely local
notifications. FCM being unconfigured is a blocker, not a passing test. The
`talkback_accessibility` check requires actual native GPUI device/TalkBack
acceptance, not adapter compilation, synthetic trees, or prior Java-shell tests.
Hash verification establishes evidence integrity, not its truth. Root must read
and accept genuine results rather than generating synthetic passing reports.

Only after root confirms the release and internal track:

```bash
# Mutates remote state: creates an edit, uploads AAB, assigns internal, validates.
# DOES NOT COMMIT or publish. Approval must have upload_authorized=true.
python3 scripts/android-publish.py target/android/JcodeMobile.aab \
  --execute --approval /secure/path/root-approval.json \
  --receipt /secure/path/new-staged-receipt.json
```

The receipt records the edit ID and expiry if returned. A staged edit is not a
release. To inspect/commit an already staged edit, use the Console/API explicitly
under root's separate approval before expiry. This CLI does not resume edits.
Do not rerun upload expecting it to resume, since a new edit can invalidate the
old one and the version code might already be consumed.

If root approves upload **and commit together** for a fresh attempt, use the same
command with `--commit` and `commit_authorized=true`. This is internal-only and
sets release status `completed`, so committing may deliver to configured internal
testers. There is no staged percentage rollout on this path. The script checks
the API's internal track identity and returned version code, validates, and
requires the commit response to confirm the edit ID. Receipts never claim public
publication. A successful internal commit is not proof of store availability,
review approval, tester installation, or a public launch.

Receipts are exclusive-created mode 0600 and never overwritten. On an ambiguous
failure, inspect the receipt and Console before retrying. The script deliberately
does not automatically retry or delete an edit after a possibly successful
commit. Do not run concurrently with another publisher or Console editor.

## Tests and limitations

`android-release-check-test.py` runs entirely without network: false approvals,
truthy-string rejection, artifact/package/version/track binding, separate commit
gate, offline defaults, modified artifact rejection, API sequencing, version
mismatch, ambiguous commit handling, and structured Android parity evidence
missing/incomplete/unaccepted/stale-file rejection before network. API behavior
is mocked. These tests do
not prove real service-account permission, successful AAB build, UI/device
validation, or actual publication. Run the real offline validator against the
final signed AAB, then the explicitly gated workflow only when authorized.
