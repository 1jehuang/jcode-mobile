# Mobile vs desktop streaming transcript rendering

Compared November 7, 2026 (Pacific). Desktop checkout: `4262e5f`. Mobile comparison started after TestFlight source `30daa19` and its tail-anchor fix `daf60a5`; concurrent voice-only work subsequently committed as `59d4e95`. This investigation does not change application behavior or publish another build.

## Conclusion

Mobile does **not** currently use desktop's streaming presentation pipeline. The TestFlight fix removes the old delayed-scroll catch-up, but it does not port desktop's block reveal, fade, coordinated tail glide, or activity-row stabilization. Immediate line wrapping on mobile can therefore still look more abrupt. This is a code-backed explanation, not a measurement of physical iPhone frame pacing.

## Concrete differences

| Area | Current desktop | Current mobile | Consequence |
| --- | --- | --- | --- |
| When incoming text becomes visible | `StreamReveal` releases complete lines/paragraphs, otherwise waits for a sentence/word boundary or an idle stream. Releases are at least 140 ms apart, fallback hold is 260 ms, idle flush is 160 ms. | The reducer appends deltas directly to `TranscriptEntry.text`/`reasoning`; rendering uses the complete received strings. The receiver drains already queued events together, but has no timed reveal policy. | Desktop bounds visible text growth frequency. Mobile growth follows incoming update/batch cadence. |
| New text appearance | Newly released blocks fade for 220 ms. Reduced motion, large backlogs and nonanimated test paths snap. | No corresponding live-text block fade. | Presentation is not visually equivalent even if scroll position is correct. |
| Following the live tail | A finite 120 ms ease-out per growth step, bounded lag, post-layout growth wake-up and device-pixel snapping. Large restores and reduced motion snap. | Native `ScrollHandle::scroll_to_bottom` applies the new measured bottom before paint whenever follow is enabled. | Mobile favors immediate correctness, desktop coordinates a short glide with less-frequent text releases. Reintroducing the removed standalone mobile easing is not equivalent to desktop's full pipeline. |
| Thinking/activity row | `SteadyRow` keeps the activity row from bobbing below its resting place during tail glides. It stops overriding position while the user reads history. | A normal fixed-size orb row participates in transcript layout. No separate screen-position stabilization wrapper. | Desktop has an explicit anti-bob mechanism absent on mobile. The fixed mobile row height alone is not that mechanism. |
| History windowing | GPUI virtual list with logical row/within-row scroll position and targeted remeasurement. | Measured-height row windowing and coalesced spacers over a normal scroll handle. Following uses conservative suffix-height culling. | Different layout/anchoring machinery. The mobile same-frame tail visibility tests verify correctness, not motion parity. |
| Reasoning/answer/tool phases | Separate ordered items. Text arrival settles prior reasoning; renewed reasoning settles text; a tool start settles both before inserting the tool. | A streaming assistant entry holds separate reasoning, tool and text fields; the renderer always outputs reasoning, then all tools, then text. | Interleaved phases can lose their original chronological order on mobile. |
| Markdown | Streaming-tolerant renderer with nested/task lists, quotes, tables, math, links, Mermaid and other rich blocks. | Smaller paragraph/heading/list/code renderer. | Partial and completed rich Markdown are not guaranteed to look the same between apps. |

## Relevant source locations

Desktop, relative to `/home/jeremy/jcode-desktop/crates/jcode-desktop-ui/src/`:

- `panel_stream_reveal.rs`: block release timing, boundaries, fade and reduced-motion handling.
- `panel_tail_glide.rs`: short finite ease-outs, lag bounds, device-pixel snapping and post-layout growth observer.
- `panel_steady_activity.rs`: activity-row pinning/easing during follow.
- `panel.rs`, `Panel::apply_event`: ordered phase settlement.
- `panel.rs`, `Panel::render_item`: common live/settled renderer and live Markdown fade.
- `panel.rs`, `Panel::render`: virtual row insertion, targeted remeasurement and tail follow.

Mobile:

- `crates/jcode-mobile-core/src/session.rs`, `SessionState::apply` and `with_streaming_assistant`: immediate delta accumulation and combined assistant entries.
- `src/app.rs`, `MobileApp::pump`: queued-event coalescing, without a timed visible-prefix policy.
- `src/app.rs`, `render_chat_inner`: layout-time bottom anchoring and tail windowing.
- `src/app.rs`, `render_entry`: reasoning, tools, then answer field order.
- `src/markdown.rs`: simpler renderer with no streaming block-fade layer.

## Validation performed

All test commands used `justrust` and the listed focused suites passed:

| Suite | Passed | Receipt |
| --- | ---: | --- |
| Mobile `app::scroll_tests` | 7 | `20261107-213308768-189552` |
| Desktop `stream_reveal` | 7 | `20261107-213314296-189946` |
| Desktop `stream_scroll_tests` | 7 | `20261107-213352333-189945` |
| Desktop `tail_glide` | 6 | `20261107-213421400-201568` |
| Desktop `steady_activity` | 3 | `20261107-213448664-201567` |

The desktop chronology regression `reasoning_after_streamed_text_starts_a_new_block_below_it` also passed. It asserts `first thought -> partial answer -> second thought`, with the second thought below the answer, not merged into the first reasoning block.

An isolated scratch executable exercised the **actual mobile `SessionState` reducer**, then enumerated its fields in `render_entry` order. Receipt: `20261107-213648588-5950`. This confirmed:

- Events `reasoning:first thought -> text:partial answer -> reasoning:second thought` produce `reasoning:first thoughtsecond thought -> text:partial answer`. The second thought moves above the existing answer and is concatenated into the earlier reasoning field.
- Events `text:checking now -> ToolStart:read`, without an intervening `TextDone`, produce `tool:read -> text:checking now` in mobile render order. This is a specific event-sequence mismatch, not a claim that every provider always emits this sequence.

The probe lives under the session scratch directory and made no application-source changes.

Built and rendered the actual desktop streaming fixture with `scripts/screenshot.py` on a private Xvfb display, then inspected `target/streaming-comparison.png`. This is static styling evidence, not proof of timing or real-device motion parity. Dynamic evidence comes from the focused renderer tests above. No physical iPhone was connected.

## Recommended order for parity work

1. Preserve ordered reasoning/answer/tool phases in the mobile model, with regressions for interleaving and stream settlement.
2. Port or extract desktop's visible-prefix/block-reveal policy. Keep raw received content separate from presentation state and flush correctly on completion, replacement, cancellation and reconnect.
3. Coordinate tail motion and activity-row stabilization with that reveal policy, rather than easing toward stale scroll geometry independently.
4. Add deterministic token-by-token and burst fixtures at phone widths, with normal/reduced motion and larger text. Compare visible-prefix timing, screen-space activity position, first-frame tail visibility and reader-scroll preservation.
5. Validate the resulting motion on physical iOS before claiming the reported bouncing is resolved. Rich Markdown parity is a separate larger task.
