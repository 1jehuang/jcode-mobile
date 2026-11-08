# TestFlight release record

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
