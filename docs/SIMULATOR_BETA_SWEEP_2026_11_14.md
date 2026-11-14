# Mobile simulator beta sweep, November 14, 2026

## Scope and fixes

This is a bounded workflow sweep, not a claim that every mobile defect has been eliminated. The visual environment was the Linux native GPUI phone preview, not Apple iOS Simulator or a physical iPhone. A read-only localhost replay used a real synced transcript with isolated fake account storage. Non-GET requests were rejected. No production messages were submitted by the visual sweep.

Production source `04058c1` incorporates:

- Synced transcript conversion, compact expandable tool output, stable sequence identities, and initial follow-state correction.
- Local-session Settings machine identity and protection against unsupported Clear conversation erasing visible history.
- Synced-only drawer layout using available height without clipping Open tabs.
- Terminal message acknowledgement clearing pending state and uncertainty acknowledgement preserving independent outage/offline warnings.
- Responsive simulated landscape keyboard and reachable directory Go controls.
- Preview hardware decorations constrained to actual safe-area space.
- Keyboard inset presentation and dismissal snapping immediately under Reduce Motion, preventing a moving composer target during re-focus.

The apparent Settings directory input overflow was an unsettled test animation, not a proven product defect. No speculative directory-input fix was retained.

## Validation and provenance

All Rust compilation used hosted justrust. Downloaded hosted-built executables ran locally without compilation to exercise actual `JCODE_MOBILE_*` environment settings, which the hosted environment forwarding allowlist does not include. Suites ran serially because these settings are global.

- Hosted workspace test check passed, receipt `20261113-195935604-2022437`.
- Hosted all-workspace test artifact compilation passed, receipt `20261113-200145490-2028110`.
- Hosted workspace/all-target clippy passed, receipt `20261113-200923597-2051682`, with existing warnings remaining.
- Core runtime: 123 passed, 3 ignored.
- App matrix completed: dark/light appearance, text scales 1.0/1.6, Reduce Motion off/on. All eight full cells passed, each reporting 426 passed and 11 ignored. The four normal cells preceded the final inset fix, whose production branch changes only reduced-motion behavior. All four reduced cells were rerun after the final fix, with 426 passed each (112.22, 107.24, 105.88, and 110.45 seconds).
- Extended raw-touch corpus seeds 42 through 49, 256 cases per seed: 2,048 cases, 3,056 contacts, 9,192 events, passed.
- Six initial Reduce Motion failures were normal-animation assumptions in tests. Test-only commit `69c4532` adds meaningful instant-presentation assertions without skipping tests or forcing normal motion. Both normal and reduced focused streaming/voice suites passed, 14 tests per mode.

- A subsequent light/reduced full-cell failure exposed a genuine product defect: keyboard inset easing ignored Reduce Motion. A deterministic slow-frame contact reproduced it. Commit `04058c1` fixes the inset behavior and asserts immediate dismiss/reopen geometry and retained draft/focus. Focused validation passed both motion modes at both text scales, 9 interaction tests, and 38 keyboard tests. All four full reduced cells passed after this production fix.

Initial app test executable SHA-256: `006aad0b9c51e845a6cbcbde05da043d8f1002a2a7a0ffea7f645c7099985e15`. Motion-test-corrected executable: `d705806a7ff8cd0a2a2e68c1d4468d577a60614a202bf442596573bb2620971f`, compiled hosted under receipt `20261113-201406336-2066501`. Normal full cells used the initial executable. The final four reduced full cells used executable `ed310ad82f12e6d320d6c8f416c08f1f6f858c5270a10431106555c15e5fd765`, retrieved under hosted receipt `20261113-203718879-2122606`, including actual product fix `04058c1`. Final workspace test compilation check passed under `20261113-203509387-2118062`. Package-only artifact transfers failed twice before workspace retrieval succeeded, without local compilation.

Private evidence is under `~/.jcode/scratch/mobile-sweep-20261114/`, including matrix logs, raw-touch log, and screenshots. Screenshots cover narrow light/large text, drawer, local Settings and disabled clear, normal portrait, landscape, and landscape keyboard with an unsent draft. Physical UIKit keyboard, microphone, VoiceOver, and real-device push delivery still require device acceptance.

## Release candidate

Prepared build `1794629010` from source `04f5705` is superseded and must not be uploaded because the later Reduce Motion product fix is not included. Final signed build `1794631199` was rebuilt from exact source `04058c1758d0207e8a6275a39c05ce5af609ad01` with shared voice source `1e62fbf0da21a018beab0d066e28b67499909360`. IPA SHA-256: `3cbba297ca2d373437cc98d508f540732ab3801127ff2b288fe5f25cebc783d5`. Root authorized upload/public distribution at `2026-11-14T04:45:36Z` after the final matrix passed. See `TESTFLIGHT.md` for Apple distribution evidence.
