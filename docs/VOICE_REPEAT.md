# Repeated pull-to-talk

One held finger can send several voice messages in a row:

1. Overscroll past the bottom of the chat (32 px). Recording starts.
2. Speak, then move back down (to 16 px or less) without lifting. That segment
   finishes and is transcribed and sent in the background.
3. Pull out again in the same motion. A new segment records immediately.
4. Repeat as often as you like. Lifting mid-recording still sends, and lifting
   while deeply pulled (96 px or more) still locks the mic on.

## How it works

- `voice_gesture.rs`: reverse-to-stop emits `Finish` and returns to `Pulling`
  instead of the terminal `Finished` state. START/STOP hysteresis (32/16 px)
  keeps finger jitter from re-triggering. `Finished` is now reserved for a
  capture startup failure, which still ignores the rest of that drag.
- `app/voice.rs`: an auto-send recording that finishes is moved into a
  `segments` queue (`queue_voice_segment`). The gesture and mic are free at
  once. Each segment finalizes on a background task and results are delivered
  strictly in spoken order (`voice_segment_done`), even if a later one finishes
  first. Failed segments keep their live preview in the draft for review.
- `voice_stream.rs`: `Recording::release_microphone` stops capture without
  closing the network session. A `MicGate` makes the next segment's startup wait
  (off the UI thread, at most 3 s) until the previous capture released the
  microphone, because iOS capture is exclusive.
- Pulling out again while the microphone is still starting simply keeps that
  startup capture rather than opening a second one.
- The cancel (×) button and navigation discard every queued segment too.

## Backend

Each segment is a separate realtime voice session. The relay limits were
raised from 3/min, 30/day to 12/min, 200/day (`workers/api/src/voice_realtime.js`).
Past that limit a segment fails with "Cloud voice is not ready" style errors.

## Tests

- `voice_gesture::tests::reverse_stop_rearms_so_one_held_finger_records_repeatedly`
- `voice_gesture::tests::rearm_needs_start_again_and_hysteresis_ignores_jitter`
- `voice_controls_tests::one_held_finger_sends_each_pulled_message_in_order`
- `voice_controls_tests::cancel_discards_every_queued_voice_message`
- `voice_controls_tests::failed_queued_segment_keeps_its_live_preview_for_review`
- `voice_controls_tests::reverse_during_startup_then_pull_again_keeps_the_same_capture`
- `voice_stream::tests::mic_gate_waits_for_release_and_opens_on_every_exit`
