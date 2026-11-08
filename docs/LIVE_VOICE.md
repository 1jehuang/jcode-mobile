# Live voice in the composer

## Nari streaming, shared with desktop

Mobile and the desktop simulator use the transport-only `jcode-voice` crate, the same `NariSession` protocol implementation used by Jcode Desktop. Native microphone capture supplies bounded mono 16 kHz signed PCM16 frames through the signed-in account's authenticated Jcode cloud realtime relay. The cloud connects only to Nari. There are no batch previews, overlap merging, full-recording transcription uploads, or Groq fallback.

Nari sends full aggregate transcript revisions, including corrections and retractions. These replace the provisional speech in the actual input editor after the original typed draft. Stopping capture flushes the final PCM tail and finalizes the existing stream. Network setup and finalization run off the UI thread. Response latency still depends on the network and recognition service.

Mobile uses the account's Jcode bearer credential only against that account's HTTPS cloud API, upgraded to `wss` at `/voice/realtime`. The server owns `NARI_API_KEY`, authenticates and authorizes requests, and relays the bounded Nari protocol. No provider key is loaded or embedded on the phone, and there is no direct-provider fallback. Desktop's independent direct Nari configuration remains unchanged. Cloud rollout and a server-owned Nari credential are prerequisites, source changes alone do not activate the deployed endpoint.

The initial cloud pilot limits each session to 60 seconds of audio and 120 seconds of wall time, with account-wide start limits. These server limits are deliberately stricter than the native capture safety ceiling of five minutes. Exceeding a relay limit is an error, not an authoritative successful final result.

## Waveform and gestures

Measured microphone levels drive 24 compact bars beside the existing overscroll mic, without an extra card or status row. Linux meters real PCM in 50 ms windows. iOS uses AVAudioRecorder metering on its recording worker. Native objects remain off the UI thread.

Pull upward to begin recording, speak while holding, then move back down in the same gesture to finish. Passing the deep threshold does not lock while held. Only releasing while still deeply pulled locks recording. Cancel immediately discards capture and provisional speech.

## Draft and lifecycle safety

- Provisional text is separate from the saved original draft, selection and scroll state.
- Cancel or navigation restores the original only while the editor still contains our exact provisional text. Manual edits are never overwritten.
- Final recognition replaces provisional speech rather than appending it twice.
- Overscroll auto-send sends only the authoritative spoken text, never the typed draft. Disconnected results remain available for review.
- Failed or empty final recognition preserves available provisional speech as an unsent review draft.
- Generation, tab and capture-phase checks reject stale revisions. Cancellation also interrupts a pending network finalization.
- The PCM queue and latest-revision mailbox are bounded. Queue overflow fails closed rather than silently dropping speech.
- A retained sender gates stream EOF until capture stops successfully. Capture errors and cancellation raise the cancellation flag before releasing that guard, preventing an accidental successful commit.
- Audio and waveform history are ephemeral. This adds no audio/transcript logging or persistence.

## Native Wayland trackpad lifecycle

The desktop launcher's local GPUI backend must include commit `a535f3f` or later. Previously it emitted only scroll movement and ignored compositor axis-stop events, so a stationary speaking pause could be mistaken for release.

The backend now emits `Started` for the first finger delta and zero-delta `Ended` only after all active native axes stop. Pauses remain held. Mouse wheels retain legacy unphased behavior. Backend and application regressions cover pauses, diagonal stops, deep reversal and release-only locking.

## Validation boundaries

Offline tests cover draft ownership, aggregate corrections and retractions, cancellation, stale results, authoritative final replacement, waveform bounds, bounded PCM, final tail draining and commit ordering. Native simulator acceptance uses an isolated display and dedicated PipeWire monitor with synthetic speech, without touching the physical microphone or speakers. Current migration results are recorded after execution below.

No physical iOS gesture or device GPU frame-pacing claim is made. Source changes are not a TestFlight release.

## Migration acceptance, November 8, 2026

- The actual native simulator passed real Nari recognition, not a mocked transcription service. Synthetic speech reached `parec` through an isolated PipeWire monitor, appeared live beside the preserved typed draft, and finalized after continuous upward-then-downward movement. Deep release kept capture running, and Cancel stopped it without discarding the pre-gesture draft. The measured waveform was visible. Evidence is in the local scratch `voice-simulator/nari-*.png` files and acceptance task `387289567o`.
- All 90 mobile voice regressions passed against the launcher's local GPUI backend, hosted run `20261108-011514606-1192689`.
- The full iOS target check passed, run `20261108-011634186-1203765`, using the configured local Apple SDK fallback because the hosted sysroot does not support that target. This does not validate a physical iPhone microphone or provision its Nari credential.
- The unfiltered full workspace attempt reached 369 passing application tests, 11 ignored and one existing selection-overlay fixture failure: `selection_is_read_only_copyable_and_done_releases_focus`, asserting that its fixture exercises overlapping hit regions. Run `20261108-011714940-1223853`. No voice regression failed.
- Repeating the complete workspace with only that known fixture excluded passed 447 tests, with 14 ignored and one filtered out, hosted run `20261108-011940134-1242911`: `justrust test --workspace -- --test-threads=1 --skip selection_is_read_only_copyable_and_done_releases_focus`. This is not an unfiltered green suite.
- The old managed-cloud speech route was removed in cloud source commit `eec4834`, with 424 worker tests passing. Production was not redeployed and no secrets or billing settings were changed. The new simulator talks directly to Nari and does not depend on that route.

## Cloud relay follow-up, November 8, 2026

The preceding acceptance records describe the earlier direct-Nari implementation, not production acceptance of the cloud relay.

- Mobile cloud routing was committed as `92e2aa7`, using the account API origin and bearer. Shared transport support is `b0ae6557d`, with 15 focused hosted protocol tests passing, receipt `20261108-014208096-1406426`.
- All 93 mobile voice tests passed, hosted receipt `20261108-014252837-1410221`. The iOS target compiled using the configured Apple SDK fallback. No device microphone runtime claim is made.
- Full workspace validation passed **454 tests, 14 ignored, one known unrelated selection-overlay fixture excluded**, hosted receipt `20261108-014558842-1435211`. A duplicate composer/transcript connection-status selector exposed by this run was fixed in `8313cc1` before the successful run.
- Read-only production probe returned HTTP 404 for `/v1/voice/realtime`. Server secret-name inspection found no `NARI_API_KEY`. The Worker source deployment guard also reports missing existing production repairs and must not be bypassed. A dedicated server credential and safe production-source reconciliation are required before activation and live cloud acceptance.
- No personal desktop key was copied to the cloud, no provider credential was embedded on the phone, and no Worker deployment or replacement TestFlight is claimed for this follow-up. Build `1794130164` predates these cloud-client changes.
