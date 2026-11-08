# TestFlight release record

## Existing release verified November 7, 2026 (Pacific)

- App Store Connect app: `6830033634` (`sh.jcode.mobile.rust`).
- Version: `1.0`, build: `1793974515`.
- Uploaded: November 6, 2026 at 06:16:05 Pacific (`2026-11-06T14:16:05Z`).
- Apple processing: `VALID`. Internal testing: `IN_BETA_TESTING`.
- Archived unstripped binary SHA-256: `42232275a8db75b51c828ced89a71153429a5ee7d055ddc1795bcdda9e06a023`.
- This matches the local iOS release binary built at 04:39 Pacific, after the last application-source change `f9388ec` at 04:18. Subsequent commits through `4fa442e` changed cloud build tooling and documentation, not application behavior.
- The Internal group has access to all builds. Public Beta distribution is separate and is not implied by internal availability.

## Validation limits

The reported bouncing thinking/transcript rendering was observed by the user on physical iOS. No iPhone was connected to the release workstation during investigation. Linux GPUI layout tests can check transcript anchoring and reader-scroll behavior but cannot prove physical-device frame pacing or Metal presentation quality.
