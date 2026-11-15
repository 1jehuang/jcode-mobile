# TestFlight release record

## Build 1794725000, recovered session fixes, November 15, 2026

- User requested a new release if the recovered sessions were all finished. Safe live cloud state checks at `2026-11-15T06:36:03Z` showed hedgehog, flamingo and scorpion idle with no pending interrupts; their final assistant messages explicitly confirmed task completion and passing focused tests. Unicorn had already completed and was not resumed. Raw read-only evidence: `~/.jcode/scratch/lifecycle/live-completion-check.jsonl` and `completion-summary.json`.
- Recovered commits: swipe snapping `39687486e0ccc0f2df58b84f6aa165928b4f633f`, progressive edge-overscroll plus/release-to-create `c13774ab0758a719f65ddc07abe93c07095be19c`, passage-menu coverage `c6e3b2db0c4dded0d617be56790b47f4b34355f1`. Missing behavior and tests were integrated on main as `22d2db1afd8cc26e93d71679c073a543370a7ad2`, preserving unrelated concurrent edits.
- Exact frozen mobile source `22d2db1afd8cc26e93d71679c073a543370a7ad2`; exact voice sibling `f63d8569a6c91c1684cd342a3bc90b6298050348`. Managed-VM signing and hosted acceptance both use immutable source archives and existing target caches. No worktrees, shared checkout resets or daemon restarts.
- Complete hosted exact-source acceptance passed: `justrust check --tests` record `20261114-224331739-202925`, core **139 passed / 3 ignored** record `20261114-224357696-204874`, app **473 passed / 11 ignored** record `20261114-224442934-210450`, all with zero failures. Independent integration-worker immutable app validation also passed all 473 tests. Both use the exact frozen production voice source, not a mutable sibling checkout.
- Managed-VM cross-build and distribution signing completed at `2026-11-15T06:45Z`. The helper initially looked for the IPA in the archive before copying it from the actual shared target; the existing signed IPA was recovered without rebuilding. Actual IPA CRC, four bundle build numbers, three profiles, camera permission, photo privacy and native bridge symbols passed. IPA SHA-256 `2286877cd077d492c35b4b224f830a4583f580e20936f248b1abd5e3460a79a4`; unstripped binary `11723c02c3cb81786939b48019e794c619c966b392d05f93f230418bc25e7d48`. Artifacts, source provenance, build and Apple receipts archived under `~/.appstoreconnect/builds/1794725000/` locally and on the managed VM.
- **Public Beta available.** Apple upload completed. Exact Apple build ID `301ba72d-a463-4f46-87c0-23104f9e6e1e`. Independent verification at `2026-11-15T06:49:35Z` confirmed `VALID`, unexpired, internal/external `IN_BETA_TESTING`, automatic notifications enabled, public link enabled and exact build membership in the paginated Public Beta relationship list. Release-specific notes are present. Physical iPhone gesture/VoiceOver acceptance is not claimed.

## Build 1794723251, image rendering and synced transcript parity, November 15, 2026

- **Public Beta available.** Exact Apple build ID `987d7664-4833-43cf-9426-018cfd1628dd`. Independent verification at `2026-11-15T06:30:07Z` confirmed `VALID`, unexpired, internal/external `IN_BETA_TESTING`, automatic notifications, public link enabled and exact membership in the paginated Public Beta build relationship. Image/photo-specific notes verified. Frozen production mobile source `a8234410f1e337c77eb0a243054ee8b95e124f17`, shared voice source `f63d8569a6c91c1684cd342a3bc90b6298050348`. Immutable managed-VM build, existing target cache, no worktrees.
- IPA SHA-256 `969107140619d59388a12437fa79cad064804688d8557dd69ac1832311802e49`, unstripped binary SHA-256 `e71b6ce9c778dd35eceb6f4d44645a1e103ce381612304ad4344f478b3276ab2`. Actual IPA CRC, four bundle build numbers, three profiles, camera permission and native media symbols verified. Receipts, provenance and logs: `~/.appstoreconnect/builds/1794723251/`.
- Assistant image-only responses, live/history image anchors and authenticated synced image previews now render inline. Synced structured tool calls/results use the cloud transcript cards, including errors, grouping and narrow layouts. Legacy prose fallback remains. Real PNG regression and four GPUI parity fixtures passed. No physical iPhone camera or pixel-identical cloud/synced acceptance is claimed.
- Hosted immutable acceptance: core 139 passed, 3 ignored. App 467 passed, 11 ignored, serial record `20261114-222108945-40062`. The app run used `ad35582a9c6f55057af0ace9f19530991c74d69f`, differing from the IPA source only by a test-only recorder-process race correction. Earlier app run had only that flaky test failure. Unique source archive paths avoided the independently verified shared hosted-mirror stale-source bug without local compilation.
- Backend additive image/tool columns and pinned combined worker deployed at `2026-11-15T06:19:32Z`: version `ba614d23-479a-4d43-9430-07ae4265d1cb`, deployment `1949aef9-e2b0-42c8-8215-66d246f3f6b0`. Candidate SHA-256 `977efa127e8e5b5ad50b03e2d78dd7c1ecaa62de5145827da7f4e9041bcf3b70`. Guarded rollout preserved 47 bindings, settings and unrelated modules. Backend suite 660 passed, 1 skipped; actual pinned-handler HTTP/SQLite tests passed.
- Desktop producer `f51a2aae9f5e9071467755ed46cec112dacf1491` activated by graceful session-preserving promotion/reload at `06:27:09Z`, daemon PID 5883 retained. Immutable runtime `mobile-parity-f51a2aae9-123b4c3b34bd`. Authenticated read-only production fetch of this existing session returned 892 messages, 2 images and 426 structured tool calls/results at `06:29:11Z`. No probe message was injected.
- Cloud disconnect incident: four sessions aborted together at `2026-11-14T22:58:36Z`, not normal task completion. Fix `7777c3e22` activated bridge-only at `2026-11-15T06:19:56Z`, without restarting the busy cloud daemon. Authorized original hedgehog/flamingo/scorpion sessions each received exactly one continuation. All three remained processing after the recovery clients disconnected. Completed unicorn was untouched. Existing long-lived old bridges pick up the fix on ordinary reconnect. Task completion is not claimed.
- Actual native GPUI phone-preview screenshot, not generated art and not Apple iOS Simulator: `~/.jcode/scratch/media-real-render/composer-before.png`. The media plus is inside the left edge of the bottom message composer. Screenshot SHA-256 `3a6b34ad6f1b0699649ce31b26797de303740b55d3121102c40141293780f048`.

## Build 1794697835, photo attachments, November 14, 2026

- **Public Beta available.** User explicitly requested TestFlight delivery and photo support in the current computer-synced session. Exact Apple build ID `4e05e71f-52e7-4e61-b47e-3e74bbb46abd`. Upload completed and processing is `VALID`. Independent verification at `2026-11-14T23:20:59Z` confirmed internal/external `IN_BETA_TESTING`, automatic notifications enabled, public link enabled, and exact build membership in the actual paginated Public Beta group relationship list. Photo-specific notes are present. Raw Apple receipts and upload/publication logs are archived with the build.
- Frozen mobile source: `bdef6cfd15250eab9f263234cde59771f3db2fb6`. Shared voice source: `1e62fbf0da21a018beab0d066e28b67499909360`. Built from an immutable `git archive` on the managed VM, mounting the existing target cache. No worktree, no duplicate target directory, and no changes to the VM checkout's unrelated live edits.
- Managed-VM build and distribution signing completed at `2026-11-14T23:12:54Z`. ZIP CRC clean, all four bundle IDs/build numbers correct, three embedded profiles present. Camera permission, photo privacy entry and both native media bridge symbols verified in the actual IPA.
- IPA SHA-256: `09336c11d5f93e6b4c8db3afc25023439e61207675a6849a3bb3ac901c1a55c3`. Unstripped binary SHA-256: `5cae4667bef26b03455af47014e8e09536dfc12fef5de862d8689d749f4ac4d3`. Artifacts, provenance, complete build log and verification receipt archived under `~/.appstoreconnect/builds/1794697835/` locally and the matching managed-VM build archive.
- Composer **+** offers camera and photo library, pending photo previews/removal and photo-only sends. Supports cloud and computer-synced chats. Camera JPEGs are bounded to 240 KiB each, with four attachments and 1 MiB decoded aggregate enforced before computer submission. Failed/uncertain computer delivery retains photos. Persistent synced transcript photo history and physical iPhone camera acceptance are not claimed.
- Backend deployed guarded image transport at `2026-11-14T23:02Z`, version `ea445aef-419a-433d-81e1-e0a97fbafb7f`, preserving all 47 bindings and settings. Old computers reject photo commands explicitly instead of silently dropping images. Backend acceptance: 650 tests, Rust mobile 98 tests, real HTTP/SQLite-to-Rust photo contract passed. Desktop runtime activation is tracked separately below.
- App hosted serial acceptance: 451 tests passed with 11 ignored; the inventory test read stale hosted checkout fixtures. Retrieved hosted-built test binary passed the exact inventory test against the frozen release archive using a read-only `bwrap` source mount. No local Rust compilation. Independent full native run likewise passed all 451 non-inventory tests, with only concurrent Android entrypoint ledger drift. Focused frozen-source photo tests passed 5/5 and attachment tests 7/7 after runtime reload. A fully read-only namespace trial blocked 12 temporary-file/microphone-fixture tests, not product failures. The corrected writable-runtime full frozen rerun was interrupted by the coordinated desktop reload, so no single combined frozen full-suite pass is claimed.

- Desktop safely activated at `2026-11-14T23:20Z` using snapshot-preserving promotion/reload, retaining all 45 preflight sessions including this one. Running shared-server/current immutable version `701dc57e5-dirty-7015563c3531`, `v0.99.1162-dev (701dc57e5)`, binary SHA-256 `b48cc1aa0570fadf99ce5d695872c2501cab2201200b4e7fd1b498a5c3fb4289`. Production D1 at `23:20:33Z` confirmed this session on `xps14` with fresh heartbeat and `live_images: 1`. No test message was injected into the user's session and no real photo/provider turn is claimed. Detailed protected receipts are referenced in backend `docs/MOBILE_PHOTOS_2026_11_14.md`.

## Build 1794694215, public November 14, 2026 (Pacific)

- **Public Beta available.** Source-freeze authorization received at `2026-11-14T22:06:58Z`. Separate explicit coordinator matrix-green authorization at `2026-11-14T22:26:45Z` approved upload and publication of this exact archived candidate and IPA hash.
- Exact frozen mobile source: `2142e7adc666cd31147132b7f5ca2e776e26bead` (`fix: preserve multiline editing in compact local chat layouts`), verified pushed before fast-forwarding the managed VM to that exact SHA. Shared voice source: `1e62fbf0da21a018beab0d066e28b67499909360` through `JCODE_SRC=/home/ec2-user/jcode`. Both tracked source trees were clean before and after building. Unrelated local source edits and VM Python bytecode were preserved.
- Managed-VM `scripts/cloud-ios-build.sh` completed at `2026-11-14T22:11:21Z`. Apple cross-build receipt `20261114-221028810-15` passed in 46.9 seconds. No workstation compilation or worktree. Coordinator acceptance tests remain separately owned and are not implied by this build.
- ZIP CRC clean. App, bridge, Live Activity and share extension have the four expected bundle IDs and build `1794694215`. Three embedded provisioning profiles are present. Distribution signing completed. `rcodesign verify` found no problems in all four executables, subject to its incomplete and potentially misleading verification warning, not proof of Apple acceptance.
- IPA SHA-256: `ea6ba66721908d4091e0d590ba8479d123faf2a0de2ab607db9e2b1bf90298d7`. Unstripped binary SHA-256: `ec673378baec8852a709d147f435d79a654165c248bb0ccde4423cebf1a73cd3`. Downloaded hashes match VM artifacts.
- Exact IPA, unstripped binary, complete build log and source provenance archived locally under `~/.appstoreconnect/builds/1794694215/` and on the VM under `~/.local/share/jcode-mobile-build/state/.appstoreconnect/builds/1794694215/`. Local archive also preserves compiler receipt, artifact checks and signature-verification log.
- Coordinator acceptance on immutable `2142e7a`: all eight full environment cells passed 442 tests each with 11 existing ignored; core passed 123 with three ignored. Extended seeds 50 through 57 passed 2,048 cases. Frozen hosted check, Clippy and build passed, together with native compact/landscape visual acceptance. Physical iPhone runtime acceptance and restart draft persistence are not claimed.
- Apple upload `COMPLETE` observed at `2026-11-14T22:30:19Z`. Exact Apple build/upload ID: `8eebf9a7-96ec-4bb7-bbb2-4a2df2e5a244`. Server reload interrupted client polling after all three upload parts were committed; the existing upload was recovered by ID and polled read-only, without duplicate upload. Exact build processing verified `VALID` before publication.
- Published exact build `1794694215` with en-US notes covering safer account switching and stale callbacks, pasted-code preservation, larger-text caret/layout, compact long-draft keyboard editing, and interrupted local delivery recovery when reopening tabs. No media feature or restart draft persistence claim. Submission initially returned `WAITING_FOR_REVIEW`.
- Independent Apple verification at `2026-11-14T22:30:41Z` (14:30 Pacific): processing `VALID`, not expired, external and internal `IN_BETA_TESTING`, `autoNotifyEnabled=true`, Public Beta group `publicLinkEnabled=true`, and exact build ID present in the group's actual paginated build relationship list. This records actual public-beta availability, not merely submission. Protected local archive preserves authorization, upload receipt/log, processing response, publication log, complete final Apple build/detail/group/membership/notes JSON, verification script and availability log.

## Build 1794631199, public November 13, 2026 (Pacific)

- **Public Beta available.** Coordinator explicitly authorized upload and public distribution at `2026-11-14T04:45:36Z` after the final matrix passed. Supersedes prepared build `1794629010`, which was not uploaded and must not be uploaded.
- Exact mobile source: `04058c1758d0207e8a6275a39c05ce5af609ad01`, pushed to `origin/main` and fast-forwarded on the managed VM. Shared voice source: `1e62fbf0da21a018beab0d066e28b67499909360` at `/home/ec2-user/jcode`, explicitly selected with `JCODE_SRC`. Both tracked source trees were clean before building. No app code was changed by release operations.
- `scripts/cloud-ios-build.sh` completed at `2026-11-14T04:40:53Z` (November 13, 20:40 Pacific), using the managed VM's Apple SDK/linker and existing signing setup. Cross-build receipt `20261114-044001660-16` passed in 46.3 seconds. No local workstation compilation. Test acceptance is separately owned by the coordinator.
- ZIP CRC clean. App, bridge, Live Activity and share extension all carry build `1794631199`. Distribution signing completed. `rcodesign verify` reported no executable signature problems across all four executables, subject to its incomplete-verification warning, not proof of Apple acceptance.
- IPA SHA-256: `3cbba297ca2d373437cc98d508f540732ab3801127ff2b288fe5f25cebc783d5`. Unstripped binary SHA-256: `563b54327d64d839b1bd16323a3069654776324b67e58b25db4d7ea8ca45bb48`. Downloaded artifact hashes match the VM artifacts.
- Exact IPA, unstripped binary, source provenance, build log, bundle/CRC/hash verification and executable signature verification are archived under `~/.appstoreconnect/builds/1794631199/` locally and `~/.local/share/jcode-mobile-build/state/.appstoreconnect/builds/1794631199/` on the VM. The local archive also preserves the compiler receipt.
- Final coordinator acceptance: four reduced-motion full matrix cells and four normal-motion full matrix cells each passed 426 tests with 11 ignored. Core passed 123 with three ignored. The 2,048-case stress run, nine focused interaction tests, 38 keyboard tests and four targeted motion modes were green. See the coordinator-owned simulator sweep record for scope and limitations. Physical iPhone runtime acceptance is not implied.
- Apple upload `COMPLETE` at `2026-11-14T04:47:42Z` (November 13, 20:47 Pacific). Apple build ID `a877dd9b-6ae2-4d03-a4bf-30dd74ab0e94`. Updated en-US test notes cover synced-chat rendering, local computer session settings, uncertain delivery status, landscape drawer spacing, and Reduce Motion keyboard/streaming/voice fixes. Added to Public Beta and submitted for beta review, initially `WAITING_FOR_REVIEW`.
- Final Apple verification at `2026-11-14T04:48:10Z` (November 13, 20:48 Pacific): processing `VALID`, external and internal `IN_BETA_TESTING`, automatic notifications enabled, and exact build present in the Public Beta group's build list. Authorization, upload/publication logs, processing response, final Apple build/group/notes receipt and availability log are preserved in the protected build archive. This records actual public-beta availability, not merely upload or submission.

## Build 1794629010, prepared November 13, 2026 (Pacific)

- **Prepared and signed only. Not uploaded or submitted.** Upload/public distribution remains gated on the coordinator's final acceptance authorization.
- Exact mobile source: `04f570517debba10a14af1b42949356d3445ecf3`, fetched from `origin/main` onto the managed VM with a fast-forward. Tracked source was clean. Unrelated VM Python bytecode was preserved.
- Shared voice source: `1e62fbf0da21a018beab0d066e28b67499909360`, clean tracked source at `/home/ec2-user/jcode`, explicitly selected with `JCODE_SRC` as in the prior verified release.
- `scripts/cloud-ios-build.sh` completed at `2026-11-14T04:04:57Z` (November 13, 20:04 Pacific). Apple cross-build receipt `20261114-040331960-16` passed in 80.2 seconds. This uses the managed VM's Apple SDK/linker, not local workstation compilation. Tests are tracked separately by the coordinator and are not implied by this build.
- ZIP CRC clean. App, bridge, Live Activity and share extension all carry build `1794629010`. Distribution signing completed, and `rcodesign verify` found no problems for all four Mach-O executables. The verifier warns that its checks are incomplete and potentially misleading, so this is not proof of Apple acceptance.
- IPA SHA-256: `08d8fe4df1bb4b315a0fd28985dad2980d06309dbe294710658f458a921e992a`. Unstripped binary SHA-256: `bd5468c6f17a4426282ccffc0d0c58ec6898e2c376e0f51189a77729d06c39b7`.
- Exact IPA, unstripped binary, source provenance, build log, bundle/CRC/hash verification and executable signature verification archived locally under `~/.appstoreconnect/builds/1794629010/` and on the VM under `~/.local/share/jcode-mobile-build/state/.appstoreconnect/builds/1794629010/`. Local downloaded IPA hash matches the VM artifact.

## Build 1794621085, submitted November 13, 2026 (Pacific)

- Version 1.0, source `6f91b68`, including live computer session chat, notification routing, richer Live Activity progress, and synced-session UI coverage. Source committed and pushed to `origin/main` before building.
- Built and distribution-signed on the managed VM with `scripts/cloud-ios-build.sh`, `JCODE_SRC=/home/ec2-user/jcode` (shared voice source `1e62fbf0d`). All four bundle build numbers verified as `1794621085`, ZIP CRC clean.
- Hosted compile check passed. Final hosted core library tests and all 19 filtered synced-session UI tests passed after fixing the expanded Live Activity capacity expectation and allowing the protocol error diagnostic. Hosted connectivity temporarily failed, then recovered. No local compilation was enabled on battery.
- IPA SHA-256: `8958f4bf501bdf719a6d1c0169677d73e0b7538f6e46c4b9a191e95af911c39c`. IPA and unstripped binary archived under `~/.appstoreconnect/builds/1794621085/`.
- Apple upload `COMPLETE`. Apple build ID `9d9b480b-0edb-4ae9-b226-a4df83988ca2`. Added to Public Beta with updated test notes, submission returned `WAITING_FOR_REVIEW` at approximately 18:03 Pacific (`2026-11-14T02:03:33Z`). This records submission, not approval.
- Follow-up at 18:04 Pacific confirmed external and internal `IN_BETA_TESTING`, with automatic notifications enabled. The build is available for external testing.
- Independent release verification at 18:06 Pacific (`2026-11-14T02:06Z`): Apple reported processing `VALID`, external and internal `IN_BETA_TESTING`, automatic notifications enabled, and build `1794621085` present in the Public Beta group's build list. The archived IPA SHA-256 matches the value above, its app and extension build numbers agree, and the archived Mach-O binary contains the live-local transport's `On your computer`, `Computer sync unavailable`, and `Message delivery uncertain` strings. Feature commits `b0781f6` and `d1d69ff` are ancestors of release source `6f91b68`; no duplicate upload was needed.
- Recovered hosted validation: workspace/test compilation passed (`20261113-180454274-1195229`), core mobile-sync tests passed 15 with one ignored benchmark (`20261113-180518339-1197228`), and all nine actual live-chat GPUI tests passed (`20261113-180545433-1200045`, filter `synced_sessions::chat::tests`). Validated HEAD `7efcaec` differs from release source `6f91b68` only in this release record, not application code. No local compilation was used.
- Compatible backend rollout completed at `2026-11-14T02:37Z`, version `77039949-c1ed-4d7e-bbcf-ad6b1f80200a`, preserving all 47 production bindings and non-annotation settings. Real production composer-API command `88598cde-2df1-4dba-a6e3-b9cde7595e8d` reached the same local desktop session, completed with `target=local`, and its exact reply appeared in both daemon history and synced transcript by `02:39:44Z`. No cloud transfer was used. Public Beta membership was reconfirmed at `02:28Z`; no additional app upload was needed for this server-only fix. Physical iPhone interaction is not claimed.

## Public beta: build 1794498559 submitted November 13, 2026 (Pacific)

- Public link: https://testflight.apple.com/join/ZHSYwcyf (group "Public Beta", no tester limit).
- Build 1794498559 added to the group with en-US "What to Test" notes and submitted for beta app review: `WAITING_FOR_REVIEW`. The previous public build was 1793952485 (Nov 6).
- Repeat for future builds with `python3 scripts/asc_public_beta.py [VERSION]`. Later builds of version 1.0 usually clear beta review quickly.
- App Store 1.0 (build 1793919915) has been `WAITING_FOR_REVIEW` since November 5.

## Build 1794616894, uploaded November 13, 2026 (Pacific)

- Source: `523382d` (clean tree). Changes since 1794600756:
  - Settings → "Sign cloud machine in to OpenAI" (`da6ffe1`). It runs OpenAI device-code login on the cloud machine, copies the code to the clipboard and opens the approval page.
  - A one-tap "Continue" action on finished-turn notifications (`db798e3`).
  - Live Activity rows with an origin chip ("Cloud", "XPS 14") and a live run timer (`523382d`).
- Shared `jcode-voice` from jcode `1e62fbf0d`, mounted via `JCODE_SRC=/home/ec2-user/jcode`. Built with `scripts/cloud-ios-build.sh` on the managed VM, reached over recovery SSH.
- The cloud runtime was updated to `v0.99.1097-dev (1e62fbf0d)`, which adds `jcode login --device-code`. A real start against OpenAI returned a code with the JSON keys the app parses.
- Tests: mobile workspace 503 passed and 1 failed. The failure was the coverage ledger, fixed in docs commit `0ed84b6` (only `docs/` changed). Worker 612 passed. jcode `agents_status` 6 passed.
- ZIP test clean. Signed IPA SHA-256: `b47e9cf48d642b4b91d36e4e23831313b21cbd3edff19f859a33378b51a39228`, archived under `~/.appstoreconnect/builds/1794616894/`. Upload state `COMPLETE`. Not submitted to the public group.
- Backend at upload time: production version `48106728-972a-455e-b8c0-cb95e88d6ea3`. The push payload carries `origin`, `repo` and `duration_secs`. Live items carry `origin` and `since`.

## Build 1794600756, uploaded November 13, 2026 (Pacific)

- Source: `3887354` (clean tree). App changes since 1794498559: Google sign-in sheet (`ab6cd58`), gestures (`8af0a54`), pull to reconnect (`bef3f24`).
- Shared `jcode-voice` from jcode `8ca50a9c9` (master), mounted via `JCODE_SRC=/home/ec2-user/jcode`. The default `/home/jeremy/jcode` checkout on the VM is stale (`c675d26f14`, no `connect_relay`) and fails the build.
- Built with `scripts/cloud-ios-build.sh` on the managed VM. The managed SSH proxy never reported the machine ready, so the VM was reached with the API's `recovery: true` connect and the pinned host keys.
- Tests: mobile workspace 484 passed (three of them only on a solo rerun). Backend 599 passed, 1 skipped.
- ZIP test clean, main app and both extension build numbers `1794600756`. Signed IPA SHA-256: `2c61b223c87cd06fec33bb107a3b8dd2355686102566cda47a41be0e1404a6e1`, archived under `~/.appstoreconnect/builds/1794600756/`. Upload state `COMPLETE`. Apple build ID `87f87cad-01aa-4f35-b29a-60eaf98b3ee4`, processing `VALID`, internal `READY_FOR_BETA_TESTING`, external `READY_FOR_BETA_SUBMISSION` (not submitted to the public group).
- Backend at upload time: production version `5332938c-9b04-4dcf-aaa9-c72a4a60db56`, source `c2a4217`, which shows the session name on the notification's second line (`docs/PUSH.md`). The notification layout is server-side and does not depend on this build.

## Build 1794498559, uploaded November 12, 2026 (Pacific)

- Source: `3030c2d`. Repeated pull-to-talk: one held finger can send several voice messages (`docs/VOICE_REPEAT.md`). Commit also carries another session's drawer-touch tests in `src/app/scroll_tests.rs`.
- Shared `jcode-voice` from release pin `3cccd6998228ce124c016fa671124e76e97b97ad` via `JCODE_SRC`.
- Tests: app suite 385 passed, 0 failed, 11 ignored. Core suite 89 passed. Backend 598 passed.
- All four bundle build numbers `1794498559`, ZIP test clean. Signed IPA SHA-256: `2d95ebd078d67916fe74211bf7862961258f3c2ae62fd43a933cd83fc1de533b`, archived under `~/.appstoreconnect/builds/1794498559/`. Upload state `COMPLETE`. Apple build ID `e2462b79-fd8f-460b-b9a3-4cb1f7b2bbfb`, processing `VALID`, internal `IN_BETA_TESTING`.
- Backend: raised voice start limits (12/min, 200/day), commit `5288d1c`, deployed 2026-11-13 as version `d843a2e0-42c9-42c3-9fa5-f4ff909cbc5d` using the local wrangler login. Verified live: 5 session starts inside one minute all returned 101 (the old limit rejected the 4th).

## Build 1794494539, uploaded November 12, 2026 (Pacific)

- Source: `809ac4a` (clean tree). Adds Settings → "New sessions start in", a configurable default directory for new sessions (`docs/DEFAULT_DIRECTORY.md`). Includes the outside-tap keyboard dismissal fix from 1794467643.
- Shared `jcode-voice` from release pin `3cccd6998228ce124c016fa671124e76e97b97ad` via `JCODE_SRC`, as for 1794467643.
- Tests: app suite 378 passed, 0 failed, 11 ignored (includes the coverage ledger). Core suite 89 passed.
- ZIP CRC passed, all four bundle build numbers `1794494539`. Signed IPA SHA-256: `3d2ae7ba8881535cff5d4b287fc71d98d5a74a9d5d9cc74551b3da044a2107ae`, archived under `~/.appstoreconnect/builds/1794494539/`. Upload state `COMPLETE`. Apple build ID `02ad8831-b40a-4a01-b462-ec904bfbe4e4`, processing `VALID`, internal `IN_BETA_TESTING`.
- Backend at upload time: production version `e22824e4` serves realtime voice and push. Voice verified end to end with a real account.

## Build 1794467643, uploaded November 11, 2026 (Pacific)

- Source: `13dbeeb` (clean tree). Pins GPUI `d2a9e0f8ae74a8b28899b0a2fc68ac4c761fd364`, which stops UIKit text-interaction recognizers from cancelling touches across the whole view. Tapping outside the composer now reaches the blur handler and dismisses the keyboard. See `docs/INPUT_FOCUS.md`.
- Shared `jcode-voice` built from release pin `3cccd6998228ce124c016fa671124e76e97b97ad`, mounted via `JCODE_SRC`, because the local `../jcode` checkout has diverged and lacks `connect_relay`.
- Built with `scripts/cloud-ios-build.sh` on the managed VM. ZIP CRC passed, all four bundle build numbers `1794467643`. Signed IPA SHA-256: `c2910ed5155f4b0b3342549825f9382bd9be0612274e45b7bd7d4f5398667a3b`, archived under `~/.appstoreconnect/builds/1794467643/`.
- Upload state `COMPLETE`. Apple build ID `6693c3ed-2128-437d-8fb9-afa30b3d5100`, processing `VALID`, internal `IN_BETA_TESTING`, auto-notify on.
- Known server issue, not fixed by this build: production `api.jcode.sh` returns 404 for `/v1/voice/realtime`, so the app shows "Cloud voice is not ready". The relay exists only on backend branch `feat/managed-cloud-hosts` and needs a Worker deploy.
- Headless tests do not exercise UIKit recognizers. Physical iPhone validation of outside-tap dismissal remains outstanding.

## Build 1794144761, uploaded November 8, 2026 (Pacific)

- Source: `f4706c316d0100806a3b371107668661b19ce903` (clean tree). Adds SSH zlib compression, always-on timing logs, floating git pull-tab card, connection/foreground revalidation fixes and RuntimeInfo open-latency reduction since 1794134170.
- Signed IPA SHA-256: `60ad44b677d64a8cf06f78a991dd041440f37c768be2aae6a0e0f6b100beeac1`. IPA and unstripped binary archived under `~/.appstoreconnect/builds/1794144761/`.
- Uploaded about 05:34 Pacific. Apple build ID `03bdf1b5-0d17-4ada-94ec-e8ff53f66684`, processing `VALID`, internal `IN_BETA_TESTING`. No new test run for this release.

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
