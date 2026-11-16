# App Store 1.0 resubmission, November 16, 2026

## Authorization and scope

User authorized replacing the November 5 submission, then explicitly authorized the fix-first production relay repair, exact-client validation, corrected IPA upload, listing refresh, withdrawal and resubmission. Apple approval and publication are not under our control.

## Initial read-only state

- App: `6830033634`, bundle `sh.jcode.mobile.rust`.
- Version: `3843f740-a8cf-4e97-841c-45f03de84763`, version 1.0, `WAITING_FOR_REVIEW`, release type `AFTER_APPROVAL`.
- Old build: `1793919915`, Apple build ID `7f86b2bf-296b-4db3-9b83-17f35db8445d`.
- Old review submission: `23e82137-5fc0-4634-a2a5-fc83bfeedc5a`, submitted `2026-11-05T23:10:04.378Z`.
- Latest uploaded build at preflight: `1794744728`, valid and App Store eligible. VM follow-up records identify a broken composer plus action and rejected voice configuration. It was not selected for this resubmission.

## Corrected candidate and provenance

Prepared candidate is build `1794749684`, mobile source `bdf1148d8ab23fa4958248ed1f48985e45171cfd`, shared voice source `2640998b88925ebb64b4e32c68cbd1fa243c23e3`, GPUI source `0d71abd893da4c4d63b9fed5c5a488f154903dba`.

Expected IPA SHA-256: `9e550033537ca5a8a12cf24ea91e5e458c787b23620688848d9929491237a421`, size 12,758,194 bytes. The VM's prior record reports 548 app tests and 165 core tests passed, plus iOS SDK and signing checks. Those are prior candidate receipts, not a new physical-device validation.

The VM mobile history diverges from local main. Preserve it and the source exports without resetting, merging wholesale, creating worktrees or overwriting unrelated local work. Local protected archive: `~/.appstoreconnect/builds/1794749684/`.

## Reviewer and listing preflight

- Production demo email-code sign-in succeeded. Demo account is active and has cloud compute access.
- Using the existing native core probe, the demo cloud machine woke, SSH connected, an agent received “Write a hello world in Rust”, executed tools and returned a completed answer. This exercised production transport and agent service, not physical iPhone UI. Probe scratch credentials were removed afterwards.
- The initial probe failures were caused by my scratch account using an API base without `/v1`, not an app defect. Correcting it produced the successful turn.
- The demo account currently has no voice access. Reviewer notes disclose this rather than claim all accounts can use voice.
- Corrected the old “Email me a code” instruction to the real “Send code” / “Verify code” labels. Settings is reached through the sessions drawer's account/gear row. Account deletion needs a second confirmation.
- Listing description conservatively includes Apple/Google/email sign-in, sessions, recent repositories, tool streaming and task progress. No new photo or voice marketing claim was added.
- Existing three 6.9-inch screenshots are `COMPLETE` in Apple and are preserved. No regenerated or fabricated screenshots.
- Support and privacy pages return HTTP 200 with a browser user agent. Python urllib's default user agent receives HTTP 403, which was not treated as a site outage.
- Preserve existing pricing, age rating, availability, credentials, app identity, screenshots and automatic release after approval. Do not run the legacy listing helper, which resets age-rating answers and selects the latest build implicitly.

## Production relay repair

The bounded VAD validator was recovered as a narrow three-file change, committed in the backend as `2c7c466756aa8a44ebb0b12978e6c0ae0e681816`. Deployment `9e11f331-ac90-4557-b670-ec3fc44f0631` activates version `3d299a85-ff17-4a79-a8ad-9e36434445f5` at 100%. Worker SHA-256: `c49d30ba94dac06c2f6e723d6468b9dccc899989f425e7f2660b6c54c34c1d7e`.

The existing live Worker was snapshotted and patched only in its voice validator, not replaced from the divergent source checkout. Reverse patch reproduced every original byte. Both live modules, all 47 bindings, settings, placement and exports were preserved under drift guards. No secret values or resources were rewritten. Original live bridge reproduced the bounded-VAD failure (22/23 passing), patched live bridge passed 23/23, full backend suite passed 684 tests with one skipped and zero failures. Post-deployment health was HTTP 200 and unauthenticated voice access remained HTTP 401.

Rollback version: `d88c8bb3-1999-46ff-a87a-dbb226e6c1b2`. Protected snapshot/report: `~/.jcode/scratch/voice-vad-production-20261116/REPAIR.md`. Guarded rollback script: `node ~/.jcode/scratch/voice-vad-deploy.mjs rollback`. These backend checks do not alone establish actual provider transcription acceptance.

## Release gates and outcome

Pending exact-client speech/silence acceptance and final artifact recovery. The old submission is preserved until these gates pass. This document must be updated with actual upload and final Apple review-state receipts before claiming resubmission or publication.
