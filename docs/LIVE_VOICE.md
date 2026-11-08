# Live voice in the composer

## Behavior

While recording, measured microphone levels drive a compact 24-bar waveform beside the existing overscroll mic. The waveform stays inside the existing reveal and does not insert a card or status row. Linux meters actual PCM in 50 ms windows. iOS uses AVAudioRecorder metering on its recording worker. Native objects never move onto the UI thread.

Incremental transcription appears in the actual input editor after the original typed draft. Each preview consumes at least one second of new audio and sends at most eight seconds, including 400 ms of overlap. Word overlap is deduplicated. Only one preview is in flight, requests are spaced by at least 1.5 seconds, and previews time out after ten seconds. All-zero PCM does not trigger transcription uploads.

The current subscription endpoint accepts WAV uploads, not a bidirectional streaming session. These are incremental previews, not word-by-word websocket streaming. Response latency depends on the network and transcription service. A delayed request can omit intermediate words from the preview, but stopping always performs the existing authoritative full-recording transcription.

## Draft and lifecycle safety

- Provisional text is tracked separately from the saved original draft, selection and scroll state.
- Cancel or navigation restores the original draft only if the editor still contains our exact provisional text. User edits are never overwritten.
- A successful final result replaces the provisional transcript rather than appending it twice.
- Overscroll auto-send sends only the authoritative spoken text, never the typed draft.
- If the editor was changed manually, non-auto-send completion keeps that edited draft for review rather than overwriting it.
- Failed or empty final transcription keeps available provisional speech as an unsent review draft.
- Generation, tab, pending-request and capture-phase checks reject stale preview callbacks. Cancel drops capture immediately. An already-running synchronous HTTP preview may take up to its ten-second timeout to finish, but cannot update the canceled editor.
- Audio snapshots and waveform history are bounded and ephemeral. There are no additional audio/transcript logs or persistence paths.

## Validation

- Full hosted workspace: 422 passed, 0 failed, 13 ignored. Run `20261108-000827637-628168`, `justrust test --workspace -- --test-threads=1`.
- Native iOS target compilation: passed, run `20261108-000754580-621372`. The configured local Apple SDK fallback was required because the hosted Linux sysroot lacks iOS target libraries.
- Nine offline GPUI regressions cover real composer updates, overlap handling, stale callbacks, cancellation selection restoration, manual edits, authoritative replacement, voice-only sending, failure retention and rendered waveform bounds.
- Capture and pure rendering tests cover PCM amplitude, measured silence, fixed cadence, WAV repair, bounded histories and waveform geometry.

## Desktop simulator acceptance, November 8, 2026

The actual native Linux simulator was exercised in an isolated X11 display with mouse-driven emulated touch, actual `parec` PCM capture, and speech routed through a dedicated PipeWire monitor without using the physical microphone or speakers. Recorded WAV uploads contained nonzero measured audio. A localhost transcription fixture returned known text to validate the transport-to-editor path, not speech recognition accuracy.

Passed: live text in the real composer; measured waveform activity; a deep upward pull followed by downward reversal without lifting stops capture and finishes once; releasing while still deeply pulled keeps recording; Cancel stops capture and restores the pre-gesture draft; HTTP 403 stops capture automatically and displays an actionable error. Screenshots and fixture evidence are in the local scratch `voice-simulator` directory.

The real account's speech endpoint returned HTTP 403 and its account capability reported voice transcription disabled. Real subscription-service recognition remains unverified until voice access is enabled. No physical iOS gesture or GPU frame-pacing claim is made.

No physical microphone-to-service acceptance recording or device GPU frame-pacing measurement was performed. This source change alone is not a TestFlight release.
