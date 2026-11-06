# Mobile performance pass, November 6, 2026

This records the initial pass. See [remaining performance priorities](PERFORMANCE_PRIORITIES.md)
for the subsequent measured scrolling, large-reply parsing, cache-admission and
history-transfer improvements. Cache cutoffs and churn results below describe
the initial implementation, not the later follow-up.

## Changes

- **Transcript windowing:** measure exact row heights on the first layout, then
  substitute equal-height spacers for unchanged offscreen message subtrees.
  Both active chat and parked swipe previews retain their existing scroll
  handles, padding, gaps, message IDs, and scroll geometry. Unknown geometry
  disables culling until measured. Preview culling conservatively covers
  keyboard/viewport-driven changes to its private scroll offset.
- **Correct invalidation:** row fingerprints include displayed content, role,
  prompt-number width, tool expansion/input/displayed output, content width, and
  Dynamic Type scale. Same-length edits invalidate measurements. Hidden tool
  output changes do not. Measurements retain at most 4,096 rows per app.
- **Markdown:** cache theme-independent parsed blocks, inline text and spans.
  Retention is limited to 128 entries and approximately 2 MiB of accounted
  allocations. Sources over 64 KiB and amplified parses over the budget bypass
  retention. Theme and text-size styling are still applied at render time.
- **Tools:** parse input JSON once rather than once per candidate headline key.
  Collapse/truncate headline text without allocating an input-sized temporary.
  Cache unchanged exact input keys, including negative results, with a limit of
  256 entries and 1 MiB of source bytes per UI thread. Individual inputs over
  128 KiB bypass retention. Streamed edits cannot reuse a different source.
- **Expanded results:** borrow short values and copy only the displayed UTF-8
  prefix of long output/failure values, rather than cloning the entire result
  before truncating it. The existing untruncated input fallback is preserved.
- **Idle work:** remove the permanent 125 ms root timer. Arm one-shot ticks only
  for active textual tool/browser spinners. Settings, idle/disconnected states,
  orb-only activity, and Reduce Motion do not keep that timer alive. Orbs retain
  their own animation lifecycle.
- **Onboarding:** reuse the 82,944-byte donut luminance buffer, cache exact
  width-dependent sample geometry, and reuse identical-orientation fields.
  Resolution, calibrated field math, animated orientation, and touch behavior
  are unchanged.
- **Platform metadata:** hash borrowed Spotlight source fields and allocate
  title/index records only when changed. Preserve account invalidation and
  empty-list replacement, and guard stale asynchronous index callbacks.
  Synchronize orb settings per entity rather than through a global synced flag.

## Measurements

These are **same-process comparative Linux test measurements**, not iPhone
frame-rate, energy, release-build, or production latency measurements. Caches
were warmed. The machine was shared with other builds, so absolute timings
and ratios can vary. No timing thresholds are asserted in tests.

| Workload | Baseline | Optimized | Interpretation |
| --- | ---: | ---: | --- |
| 250-message active chat, 20 real GPUI redraws, run 1 | 1,651 ms | 171 ms | 9.6x faster in this sample |
| Same chat, interleaved run 2 | 1,395 ms | 186 ms | 7.5x faster in this sample |
| Parked 250-message swipe preview, 20 real GPUI redraws | 1,623 ms | 254 ms | 6.4x faster in this sample |
| Message content subtrees built per active-chat frame | 250 | 5 | Exact wrapper geometry is retained |
| Unchanged 4,240-byte markdown, 2,000 parse lookups | 1,633 ms | 5.35 ms | Parsing/cache lookup only, not whole rendering |
| 80 unchanged short tool inputs, 1,000 frames | 370 ms | 12.4 ms | Headline computation only |
| 80 unchanged tool inputs with 2 KiB padding, 1,000 frames | 1,193 ms | 18.5 ms | Headline computation only |
| 6 MB tool result, 1,000 owned display prefixes | 577 ms | 0.266 ms | Full clone replaced by a roughly 4 KiB copy |
| Animated donut, 100 frames | 982 ms | 951 ms | Modest 1.03x CPU change, field allocations 100 to 0 after warmup |
| Identical reduced-motion donut, 100 frames | 769 ms | 4.52 ms | Avoids repeated unchanged field generation |
| 1,000 unchanged Spotlight sessions, 1,000 preparations | 770 ms | 628 ms | Borrowed hashing still scans source fields |

Receipts:

- Final active/preview draw comparison: `20261106-024824283-1174125`.
- Markdown, output, donut and Spotlight comparisons:
  `20261106-024310140-1120174`.
- Final bounded-headline comparison: `20261106-024948509-1188430`.
- Integrated check: `20261106-024703275-1159701`.
- App regression suite: `20261106-024707363-1160353`, 142 passed,
  zero failed, two ignored benchmarks. Benchmarks are not behavioral coverage.
- Final lint: `20261106-025444073-1226367`, explicit top-level
  `JUSTRUST_BUILDD_PREBUILD=0 justrust clippy --tests`, success with existing
  warnings. The earlier exact-source prebuild also passed.
- Final measured-height key regressions: `20261106-025059472-1197510`,
  four passed, zero failed.
- Full headless environment matrix: task `153022kk55`, exit zero, 317.69 s.
  Every dark/light, normal/reduced-motion, 1.0/1.6-scale cell passed all
  276 behavioral tests with zero failures and zero filtered tests, totaling
  2,208 passing executions. Eight ignored tests per cell are seven opt-in
  benchmarks and one inventory dump, not behavioral passes. First/final
  receipts: `20261106-024913084-1183937` and
  `20261106-025352472-1220505`.
- Shared core: `20261106-025152571-1205014`, 55 passed, zero failed.

## Reproduce

```bash
justrust check --tests
justrust test -- --test-threads=1
justrust clippy --tests

# Comparative draw benchmark, same binary with windowing disabled/enabled.
justrust test benchmark_long_transcript_frames -- --ignored --nocapture --test-threads=1

# All opt-in comparisons. --release is useful for a separate optimized sample.
justrust test benchmark_ -- --ignored --nocapture --test-threads=1

# Existing headless dark/light x normal/reduced motion x 1.0/1.6 scale matrix.
scripts/mobile-fuzz.sh --matrix
```

Use `justrust log <receipt>` for complete output. The matrix launcher defaults
to the local GPUI checkout. Set `GPUI_LOCAL=0` to exercise pinned GPUI instead.
Hosted compilation was attempted explicitly, but the service returned HTTP 404
for `POST /internal/builds/charge`. justrust's documented local fallback was
used, without bypassing billing or tenant isolation.

## Limits and stopping point

The first transcript layout and a complete width/scale invalidation still
measure every message. Very large individual messages still build their own
visible row subtree. This pass deliberately avoids changing scroll anchoring,
streaming event ordering, gesture ownership, or adopting estimated row heights.

Caches are bounded, not magic. A working set exceeding a cache's entry/byte
budget can churn. The headline benchmark deliberately includes 320 distinct
inputs: 1,359 ms uncached versus 2,955 ms cached across 1,000 sequential frames
in this debug sample. The cache covers the measured 32-, 80-, and 256-tool
working sets, but does not promise gains for every pathological cardinality.

Further changes should be driven by an on-device Instruments trace, especially
for cold attach/history layout, huge single-message output, GPU paint time,
and actual battery use. The current changes remove measured repeated work
without speculative transport rewrites, quality reductions, or broad project
restructuring.
