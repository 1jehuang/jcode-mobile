# Hold-to-send voice gestures

At the newest message, deliberately scroll beyond the end of the transcript:

- **START, 32 visible points:** begin microphone capture while the finger is held.
- **STOP, 16 visible points:** reverse to this point to finish a short recording. Releasing before lock also finishes it.
- **LOCK, 96 visible points:** snap into hands-free recording. Releasing no longer stops capture. Use the composer's microphone stop button to transcribe and send, or Cancel dictation to discard.

Feedback stays in a compact 44-point status row: three gently animated text-like strokes and a short Listening or Transcribing label. There is no gesture ruler, moving microphone puck, or pull-driven transcript displacement. Start, lock, and finish retain their one-shot haptic confirmations. Reduce Motion keeps the strokes static.

Quick and locked gesture recordings send only their own transcribed text. They never send or clear an unrelated typed draft. Tap-to-dictate retains its existing draft-only behavior. A disconnected result is preserved in the draft instead of sent. Empty results, cancellations, stale completions, and navigation changes do not send a message. Stopping during microphone startup is remembered and applied when startup completes.

Touch Ended, rather than wheel inactivity, finishes a held gesture. Post-release momentum cannot activate the mic again. Failed capture and reverse-to-stop consume the remainder of the original drag.

## Safe visual preview

```sh
JCODE_VOICE_DEMO=1 justrust run -p jcode-mobile
```

The native phone-sized preview reuses the production `voice_gesture` physics and `voice_visual` renderer. It alternates a short reverse-to-stop recording and a longer locked recording, including transcribing feedback. It initializes neither the account/network services nor the microphone. Audio and transcription states are explicitly simulated. The text-like strokes are a symbolic activity indicator, not a live transcript or measured audio waveform.

The preview is for visual review, not proof of native permission handling or transcription-service delivery. Those require an iPhone.

## Regression tests

```sh
justrust check
justrust test -p jcode-mobile
```

Coverage includes exact thresholds, reverse-to-stop hysteresis, sticky release, cancellation, startup-stop races, stale completions, unrelated drafts, disconnected/empty results, GPUI wheel routing, momentum exclusion, spring rebound, frame-rate consistency, finite input bounds, and Reduce Motion.
