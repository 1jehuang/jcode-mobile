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

## Validation limits

The reported bouncing thinking/transcript rendering was observed by the user on physical iOS. No iPhone was connected to the release workstation during investigation. Linux GPUI layout tests can check transcript anchoring and reader-scroll behavior but cannot prove physical-device frame pacing or Metal presentation quality.
