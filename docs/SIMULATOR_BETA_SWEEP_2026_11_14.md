# Mobile simulator beta sweep, November 14, 2026

## Scope and fixes

This is a bounded workflow sweep, not a claim that every mobile defect has been eliminated. The visual environment was the Linux native GPUI phone preview, not Apple iOS Simulator or a physical iPhone. A read-only localhost replay used a real synced transcript with isolated fake account storage. Non-GET requests were rejected. No production messages were submitted by the visual sweep.

Production source `04f5705` incorporates:

- Synced transcript conversion, compact expandable tool output, stable sequence identities, and initial follow-state correction.
- Local-session Settings machine identity and protection against unsupported Clear conversation erasing visible history.
- Synced-only drawer layout using available height without clipping Open tabs.
- Terminal message acknowledgement clearing pending state and uncertainty acknowledgement preserving independent outage/offline warnings.
- Responsive simulated landscape keyboard and reachable directory Go controls.
- Preview hardware decorations constrained to actual safe-area space.

The apparent Settings directory input overflow was an unsettled test animation, not a proven product defect. No speculative directory-input fix was retained.

## Validation and provenance

All Rust compilation used hosted justrust. Downloaded hosted-built executables ran locally without compilation to exercise actual `JCODE_MOBILE_*` environment settings, which the hosted environment forwarding allowlist does not include. Suites ran serially because these settings are global.

- Hosted workspace test check passed, receipt `20261113-195935604-2022437`.
- Hosted all-workspace test artifact compilation passed, receipt `20261113-200145490-2028110`.
- Hosted workspace/all-target clippy passed, receipt `20261113-200923597-2051682`, with existing warnings remaining.
- Core runtime: 123 passed, 3 ignored.
- App matrix: dark/light appearance, text scales 1.0/1.6, Reduce Motion off/on. Each green full cell reports 426 passed, 11 ignored. Final matrix completion is recorded below when the remaining gate passes.
- Extended raw-touch corpus seeds 42 through 49, 256 cases per seed: 2,048 cases, 3,056 contacts, 9,192 events, passed.
- Six initial Reduce Motion failures were normal-animation assumptions in tests. Test-only commit `69c4532` adds meaningful instant-presentation assertions without skipping tests or forcing normal motion. Both normal and reduced focused streaming/voice suites passed, 14 tests per mode.

Initial app test executable SHA-256: `006aad0b9c51e845a6cbcbde05da043d8f1002a2a7a0ffea7f645c7099985e15`. Motion-corrected executable: `d705806a7ff8cd0a2a2e68c1d4468d577a60614a202bf442596573bb2620971f`, compiled hosted under receipt `20261113-201406336-2066501`. Normal full cells used the initial executable. Reduced full cells use the corrected executable. These differ only in test assertions, not production source.

Private evidence is under `~/.jcode/scratch/mobile-sweep-20261114/`, including matrix logs, raw-touch log, and screenshots. Screenshots cover narrow light/large text, drawer, local Settings and disabled clear, normal portrait, landscape, and landscape keyboard with an unsent draft. Physical UIKit keyboard, microphone, VoiceOver, and real-device push delivery still require device acceptance.

## Release candidate

Build `1794629010` was built and signed from exact production source `04f570517debba10a14af1b42949356d3445ecf3`, shared voice source `1e62fbf0da21a018beab0d066e28b67499909360`. Signed IPA SHA-256: `08d8fe4df1bb4b315a0fd28985dad2980d06309dbe294710658f458a921e992a`. Four bundle build numbers and archive CRC were checked. Upload is held pending the final focus matrix gate. See `TESTFLIGHT.md` for Apple distribution evidence after authorization.
