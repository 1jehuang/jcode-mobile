# TestFlight release record

## Build 1794134170, uploaded November 8, 2026 (Pacific)

- Fixes voice TLS trust on iOS. The previous native-root loader searched for Unix/OpenSSL certificate files absent on iPhone. Shared transport now explicitly uses bundled trusted roots, preserving certificate, hostname and expiry verification.
- Mobile source: `c3fb286ecfcec955071534659a37b6eb750fff29`. Shared source: `3cccd6998228ce124c016fa671124e76e97b97ad`, including implementation `a74b45569`. Both source fixes are committed. Account-only cloud relay remains unchanged, with no device provider key or direct-provider fallback.
- Built from a committed-source archive, sharing the existing Rust target directory. No branch or worktree was created. Unrelated unfinished Git UI edits and demo hooks were excluded and preserved untouched in the original checkout.
- Real production rootless probe reproduced the original `IoNotFound` and proved the fixed transport completes verified TLS and reaches expected HTTP 401 using a deliberately invalid bearer. No microphone audio, valid credential or billable transcription was used.
- Hosted voice integration: **95 passed**, receipt `20261108-023920669-1861761`. Hosted workspace: **453 passed, 14 ignored, 3 filtered out**, receipt `20261108-023835310-1856843`.
- Exclusions: existing `selection_is_read_only_copyable_and_done_releases_focus`, Git inventory `production_event_inventory_matches_reviewed_coverage_ledger`, and Git geometry `git_overlay_owns_metadata_header_and_controls`. The unfiltered workspace is not claimed green. A separate asynchronous snapshot fixture race was corrected without changing production capture behavior.
- Signed iOS build passed using the local Apple SDK/linker, exact-source prebuild receipt `20261108-023828886-1840477-pre`. ZIP CRC, microphone permission description and all four bundle build numbers passed.
- Signed IPA SHA-256: `18d9b3beeecf60437e65d1f9235fcc2d0779811d8ca4e5f4d8cae80b3116d765`. Unstripped binary SHA-256: `e9d7ab1452e93a2e2d34ff51f947f2f6e72113245a9043c83b677c07639faba4`. Exact artifacts, probe source, logs and provenance archived under `~/.appstoreconnect/builds/1794134170/`.
- Apple upload date: **02:41:15 Pacific** (`2026-11-08T10:41:15Z`). Upload `COMPLETE`, processing `VALID`, internal `IN_BETA_TESTING`, automatic notifications enabled, verified at **02:42 Pacific**. Apple build ID: `665cf853-1183-415d-8e8e-3e4fc0d0bf14`. External `READY_FOR_BETA_SUBMISSION` is not public-beta approval.
- Published en-US notes explain the secure-connection fix and request iPhone microphone/live-text/finish/cancel validation. Physical iPhone runtime acceptance remains outstanding.

## Existing release verified November 7, 2026 (Pacific)

- App Store Connect app: `6830033634` (`sh.jcode.mobile.rust`).
- Version: `1.0`, build: `1793974515`.
- Uploaded: November 6, 2026 at 06:16:05 Pacific (`2026-11-06T14:16:05Z`).
- Apple processing: `VALID`. Internal testing: `IN_BETA_TESTING`.
- Archived unstripped binary SHA-256: `42232275a8db75b51c828ced89a71153429a5ee7d055ddc1795bcdda9e06a023`.
- This matches the local iOS release binary built at 04:39 Pacific, after the last application-source change `f9388ec` at 04:18. Subsequent commits through `4fa442e` changed cloud build tooling and documentation, not application behavior.
- The Internal group has access to all builds. Public Beta distribution is separate and is not implied by internal availability.

## Build 1794115398, prepared November 7, 2026 (Pacific)

- Source: `30daa1972f17`, including thinking-scroll fix `daf60a5` and voice-gesture UI `30daa19`.
- Thinking/transcript following now anchors against freshly measured layout before paint instead of easing toward the previous frame's scroll limit. Conservative tail windowing keeps cached tail rows visible on the first jump/append frame.
- Voice overscroll now reveals a pull-sized bottom semicircle with a microphone, without reflowing the transcript.
- Signed IPA SHA-256: `b60cba35544b35abd00befd16d87f6fb87a422ecbc2b769a97888df9bd3ae472`.
- ZIP CRC passed. App, bridge, Live Activity and share extension all have build `1794115398`.
- Release iOS cross-build used `justrust` locally because the Apple SDK and linker are installed on this machine.
- Hosted check passed: `20261107-211731128-74143`.
- Final serialized app suite: **293 passed, 0 failed, 11 ignored**, receipt `20261107-212308726-116301`, source `30daa1972f17`, clean tree.
- Core suite: **60 passed, 0 failed, 2 ignored**, receipt `20261107-211746485-75184`.
- Larger-text (1.3x) scroll suite: **7 passed**, receipt `20261107-211936167-86164`.
- Test runs used automatic local fallback following a verified hosted DNS timeout. The hosted check receipt explicitly reports `remote on hosted`.
- Uploaded November 7, 2026 at **21:25:15 Pacific** (`2026-11-08T05:25:15Z`). Upload state `COMPLETE`.
- App Store Connect build ID: `d5508cb5-3d2d-433a-9283-9ecad730501b`. Verified `VALID`, internal `IN_BETA_TESTING`, automatic notifications enabled. External state remains `READY_FOR_BETA_SUBMISSION`, not public-beta approval.
- Added TestFlight notes describing both changes and requesting real-iPhone checks for thinking/tool/response transitions, scrolling, keyboard resize and voice overscroll.

## Build 1794117691, prepared November 7, 2026 (Pacific)

- Source: `28c1738ba8b480ec968f9200c875338c360e6f8c`. Includes chronological streaming phases, desktop-timed block reveal/fades, finite paint-only tail glide, steady thinking/activity placement, and manual-scroll handoff.
- Also includes the latest stable bare-mic voice smoothing (`2736c97`) and pinch session map (`d678ece`). No extra voice status cards or composer reflow.
- Full hosted workspace suite: **398 passed, 0 failed, 13 ignored**, receipt `20261107-220010809-163927`. Hosted check: `20261107-220009514-163849`.
- Genuine 1.3x text runs: streaming **7 passed**, scroll **7 passed**, local receipts `20261107-220125183-169276` and `20261107-220136750-170358`. Reduce Motion scroll **7 passed** and chronology **1 passed**, receipts `20261107-220233519-176391` and `20261107-220234478-176669`. Local execution is required for these environment overrides because the hosted allowlist does not forward them.
- Signed iOS build passed using the local Apple SDK/linker, receipt `20261107-220138724-170580`. ZIP CRC passed. App, bridge and both extensions have build `1794117691`.
- Signed IPA SHA-256: `22d1c2059474691f80fcdd6e986dc0b2fc802787da709b48fe4d443b085fe928`. Unstripped binary SHA-256: `85dcad269320c4845a2199a6c97f128f5b03c5fa05ec629fa67faa7072ec3e88`.
- Exact IPA, unstripped binary and provenance manifest archived under `~/.appstoreconnect/builds/1794117691/`.
- Uploaded November 7, 2026 at **22:03:28 Pacific** (`2026-11-08T06:03:28Z`), upload state `COMPLETE`. Apple build ID: `2a51d4fe-e6d6-4eb8-87eb-79620acf6ba9`.
- Verified Apple processing `VALID`, internal state `IN_BETA_TESTING`, automatic notifications enabled. External state `READY_FOR_BETA_SUBMISSION` is not public-beta approval.
- Updated existing en-US TestFlight notes with the fixes and explicit request for real-iPhone streaming/scroll/keyboard/large-text validation.

## Validation limits

The reported bouncing thinking/transcript rendering was observed by the user on physical iOS. No iPhone was connected to the release workstation during investigation. Linux GPUI layout tests can check transcript anchoring and reader-scroll behavior but cannot prove physical-device frame pacing or Metal presentation quality.

## Build 1794124180, uploaded November 7, 2026 (Pacific)

- Source: `5fa86ca` (same application code as 1794117691, docs-only delta).
- Signed IPA SHA-256: `e77b020c772756ea369cb9bceb1906095f403770a6857a9c77e95d6cc0702c6e`. IPA and unstripped binary archived under `~/.appstoreconnect/builds/1794124180/`.
- Uploaded at about 23:52 Pacific, upload state `COMPLETE`.

## Build 1794132459, uploaded November 8, 2026 (Pacific)

- Source: `19e636742c41a49fae6a82d14c93dd2ce713f52b`. Mobile dictation uses only the signed-in Jcode account and authenticated cloud relay. No Nari provider-key setting, local key loader or direct-provider fallback is used.
- Signed iOS build, distribution signing, ZIP CRC and all four bundle build numbers passed. Unrelated edits in `src/app/git_info.rs` and `src/app/modal_ownership_coverage.rs` were preserved, so this was not a clean-tree build. Archive-time working patch and provenance are saved with the release.
- IPA SHA-256: `ded061bf1f397cca08093da7acd4344e3f26e802d70d4696ace214804f538fc1`. Unstripped binary SHA-256: `8b2fb2e60a495989fbdd1c883a11d80dccad46100a0a17b7712cd6507b62213e`. Exact artifacts archived under `~/.appstoreconnect/builds/1794132459/`.
- Latest hosted workspace validation: **455 passed**, receipt `20261108-020908223-1625292`, excluding the known unrelated `selection_is_read_only_copyable_and_done_releases_focus` fixture. The unfiltered suite is not claimed green.
- Production cloud version `e4aee21f-997a-4de1-af1e-593d4baffecf`, source `beaf1a5`, is active. Canonical existing services and bindings were preserved through the unchanged deployment guard. All **483 backend tests passed**, including the upgrade-timeout lifetime regressions.
- Real production acceptance verified invalid bearer HTTP 401, paid-account WebSocket configuration, live partials, final transcript and final commit acknowledgment beyond ten seconds. Native desktop simulator without `NARI_API_KEY` passed live composer text, continuous reversal finish, deep-release locking and cancellation preserving the draft. The updated normal desktop simulator was reopened.
- Dedicated provider credential is stored only as a cloud secret. Existing desktop key was untouched, and temporary provisioning files were removed. No payment or balance top-up was performed. Pilot limits are 60 seconds audio, 120 seconds connection lifetime, three starts per minute and thirty per day per paid account.
- Apple build ID: `df914296-0490-45c2-94a3-818210a94984`. Upload `COMPLETE`, processing `VALID`, internal `IN_BETA_TESTING`, automatic notifications enabled, verified at about **02:22 Pacific**. External `READY_FOR_BETA_SUBMISSION` is not public-beta approval.
- Published en-US notes explain account-only cloud dictation and gestures. Physical iPhone microphone/runtime/frame-pacing validation remains outstanding.

## Build 1794130164, uploaded November 8, 2026 (Pacific)

- Includes shared Nari realtime dictation, actual microphone waveform, continuous pull/reverse completion, release-only deep-pull locking, and microphone/connection status above the composer input.
- Build-start source: `9567d38`. Concurrent documentation-only commit `bb1d93e` followed compilation. Existing unrelated tracked edits in `src/app/git_info.rs` and `src/app/modal_ownership_coverage.rs` were preserved. This was not a clean-tree build. Later concurrent agent changes are not claimed included.
- Signed iOS compilation passed, receipt `20261108-012924162-1301464`. Distribution signing and packaging were repeated successfully after a server reload interrupted packaging. ZIP CRC passed and app, bridge and both extensions carry build `1794130164`.
- Signed IPA SHA-256: `1c0409086923d9a7bd0907435c3b75fac10186c7fb8f2909126fdabac06f5f90`. Unstripped binary SHA-256: `db64d5e762ce6882d9b09028d217e3937772d9604ba8acc531f72dc114503c77`.
- IPA, unstripped binary, archive-time working patch and provenance manifest archived under `~/.appstoreconnect/builds/1794130164/`. The patch is an archive-time snapshot, not an atomic build-start snapshot. Voice source fingerprints remained unchanged through packaging.
- Latest focused validation: **91 voice tests passed**, receipt `20261108-012624004-1276822`; interaction coverage ledger passed, receipt `20261108-012629246-1277624`. Prior workspace validation: **447 passed, 14 ignored, one known unrelated overlay fixture excluded**, receipt `20261108-011940134-1242911`. The unfiltered workspace was not green.
- Uploaded at **01:34:36 Pacific** (`2026-11-08T09:34:36Z`), upload state `COMPLETE`. Apple build ID: `82d19f0d-99ec-4317-9bd0-0ce74140b5f7`.
- Verified Apple processing `VALID`, internal `IN_BETA_TESTING`, automatic notifications enabled. External state remains `READY_FOR_BETA_SUBMISSION`, not external approval.
- Published en-US TestFlight notes explaining gestures and status placement, requesting physical-iPhone validation, and explicitly disclosing that **iPhone Nari credentials still require device provisioning**. No personal desktop key is embedded and automatic phone credential provisioning is not implemented. Physical iOS voice runtime remains untested.
