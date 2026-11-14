# Synced transcript rendering follow-up, November 14, 2026

## Reproduction

The current native Linux mobile preview reproduced the synced transcript issue
using an actual 792-row synced snapshot, replayed through a private GET-only
localhost API. The snapshot contained 369 tool-result rows, with approximately
793,000 text characters in those results alone. Tool results were projected as plain system
text, so one diff or command response could fill many phone screens and obscure
the useful conversation. `before.png` and `before-reader.png` record this.

This was a native GPUI/X11 preview on an isolated Xvfb display, not Apple's iOS
Simulator or a physical iPhone. A fake account key and isolated app-data directory
prevented production sends, cloud transfers, and changes to the real mobile login.
The localhost fixture rejects every non-GET request. Real production access was
limited to fetching the existing account's transcript before replay.

## Changes

- Synced tool results now use a neutral **Synced tool output** disclosure rather
  than a permanently expanded plain-text block. Sync does not carry tool call
  identities or success metadata, so the UI does not fabricate either.
- The disclosure has a dedicated 44-point accessible header. Expanded output uses
  the existing UTF-8-safe tool display bound without modifying the original text.
  Opening output releases live tail-follow so reading starts at the opening
  position rather than jumping to the end of a long result.
- Measured transcript row keys include disclosure expansion state, preventing a
  collapsed spacer height from being reused for an expanded result, and vice versa.
- Opening a new local-chat tab resets tail-follow and stream motion, just as a new
  cloud connection does. The previously parked tab retains its own reader mode.
  The new regression failed on the old code before this reset was added.
- Synced snapshot projection shares the existing history decoder for explicitly
  tagged assistant reasoning, preserves sequence gaps and flattened tool text,
  and ignores empty/unknown-role rows. Untagged emphasis is not interpreted as
  reasoning. Desktop sync still omits actual reasoning blocks and structured
  tool metadata, so this is compatibility decoding, not recovery of omitted data.

## Regression coverage

`src/app/synced_render_tests.rs` exercises actual drawn phone layouts:

1. Open a synced tab from a previously scrolled tab, check immediate tail
   anchoring, and restore the earlier tab without changing its reader mode.
2. Draw 90 mixed-role entries at 320, 390 and 430 points, check visible tail,
   ordered/non-overlapping rows, viewport bounds and stable reader position across
   unchanged snapshot polls.
3. Use mouse-to-touch events to expand/collapse the actual disclosure header,
   check compact/expanded geometry, original Unicode content preservation,
   expansion-sensitive cache keys and explicit reader-mode handoff.

The mandatory interaction inventory remains enabled. Its new disclosure row
references these exact tests. Pre-existing live-local composer/drawer inventory
drift was reconciled against the already implemented controls and their named
regressions, retaining an explicit gap for the acknowledgement button's rendered
activation and actual iPhone VoiceOver behavior.

## Observed validation

- Hosted `justrust check --workspace --tests` passed, receipt
  `20261113-193102453-1865268`.
- Core converter worker ran 123 core tests successfully, with three ignored.
- Final complete mobile UI suite passed: 414 passed, zero failed, 11 ignored,
  receipt `20261113-193842648-1906947`. This includes all three new drawn
  regressions and the mandatory unchanged interaction inventory guard.
- Final hosted release preview build passed, receipt
  `20261113-194109398-1926024`. Downloaded binary SHA-256:
  `feca155e95a7e49a1d09e4193dea38769eb66cc6dac6eac4e50ae31273fc4ee1`.
- Native final screenshots at 320 and 430 points show wrapped summaries and
  compact tool results. Real pointer activation at 320 points expanded the
  result from its beginning with the header still visible, then collapsed it
  without losing the surrounding messages. The earlier 390-point compact
  screenshot and automated three-width layout checks also passed.

A hosted admission interruption and two agent-server reloads were recovered;
no local compilation or battery restriction override was used. Final test and
build success above postdates those interruptions.

## Release boundary

These rendering changes postdate public TestFlight build `1794621085`. Building
and visually checking the native preview does not update an installed TestFlight
app. A new signed iOS build/upload is required before claiming these fixes are
available to public testers. No physical-iPhone acceptance is claimed here.

Private screenshots, fixed transcript replay and operational logs remain under
`~/.jcode/scratch/synced-render-20261114/`. They are not committed because the
snapshot contains real conversation content.
