# Reversible overscroll voice gestures

At the newest message, deliberately drag upward beyond the transcript's bottom:

- **START, 32 visible points:** begin microphone capture immediately while your finger stays down.
- **STOP, 16 visible points:** move back down to this point to finish and transcribe in the same continuous gesture. Even crossing the deep-lock threshold earlier does not prevent this reversal.
- **LOCK, 96 visible points, checked only on release:** lift your finger while still this deeply overscrolled to keep recording hands-free. Merely crossing this point while holding never locks. Moving back below it before releasing finishes normally.
- A shallow release after START also finishes. A pull that never reaches START records nothing.

After locking, tap the microphone to stop and transcribe, or use the cancel control in the input line to discard. Start and finish have one-shot light haptics. The lock confirmation happens on release, never during the upward pull. Reverse-to-finish consumes the rest of the drag, so moving up again before lifting cannot start a second recording.

The entire chat and composer translate together without adding cards or resizing from gesture status. A bare mic and 24 actual amplitude bars are revealed below the composer. Held displacement follows the finger, and released displacement settles with a critically damped spring. Reduce Motion snaps released travel without changing capture semantics.

Recognized previews appear in the input while recording. The current subscription speech API accepts incremental WAV uploads rather than a bidirectional stream. Stopping performs the authoritative full-recording transcription. Overscroll sends only the spoken result, never an unrelated typed draft. Disconnected results stay in the draft for review. Cancel and navigation restore owned provisional text without overwriting user edits. See `LIVE_VOICE.md` for details.

HTTP 401/403 on a speech preview stops capture and shows an actionable account error instead of silently retrying. Native audio failures, empty results and stale callbacks never send messages. A finish during permission/startup is remembered and applied when startup completes.

## Linux desktop simulator

The same recognizer runs in the native desktop phone preview with mouse-to-touch emulation. With Wayland GPUI backend support it can also receive hardware touch, which is a separate device acceptance requirement. About 40 raw logical pixels beyond the bottom reaches START. Roughly 240 raw pixels reaches the visible LOCK position, but hands-free mode begins only if the finger is released there.

Capture requires `parec` and a PulseAudio-compatible service such as PipeWire PulseAudio. Linux records mono PCM16 at 16 kHz and meters real 50 ms windows. Audio is bounded to five minutes and 10 MiB. `PULSE_SOURCE` can direct only a test simulator process to a virtual source without changing the system default microphone.

The account must have speech transcription access. A signed-in cloud account alone is not proof that voice access is enabled. Relaunch after rebuilding to load source changes.

## Visual-only preview

```sh
JCODE_VOICE_DEMO=1 justrust run -p jcode-mobile
```

This scripted preview uses the production recognizer and renderer but initializes no account, microphone or service. Its empty-history waveform is static silence, not measured audio. It demonstrates a short reverse-to-finish recording and a deep release-to-lock recording. It is not evidence of speech recognition or native device permission behavior.

## Regression tests

```sh
justrust check
justrust test --workspace voice -- --test-threads=1
```

Coverage includes exact release-position boundaries, deep up/down reversal, shallow release, consumed tails, cancellation, startup races, stale completions, typed-draft safety, disconnected/empty results, GPUI wheel routing, momentum exclusion, smooth settling, frame-rate consistency, finite bounds and Reduce Motion.
