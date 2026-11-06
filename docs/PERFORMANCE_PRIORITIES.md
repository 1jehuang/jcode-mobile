# Remaining performance priorities, November 6, 2026

## Priority order

1. **Long-conversation scrolling and CPU paint latency.** This is the largest measured repeated interactive cost. The 500-message moving-scroll benchmark averaged 59.369 ms with per-message hidden spacers versus 35.358 ms after coalescing hidden runs, about 40% less elapsed draw time. Individual optimized passes still had p95 values of 47.747 and 66.087 ms. Investigate the full `transcript.scroll_to_paint` interval before adding more small caches. See [instrumentation and detailed measurements](performance.md).
2. **Long and streaming replies.** Large inline parses can consume several milliseconds, while a reply outside the old 64 KiB cache cutoff was reparsed on every redraw. The new parser batches plain UTF-8 text, indexes long unmatched backtick searches, and retains eligible large replies within the existing 2 MiB cache budget. Preserve formatting, streaming invalidation, and exact delimiter semantics rather than truncating content.
3. **Cold history and width/scale invalidation.** These still measure every row to preserve exact scroll geometry. A fresh 500-message first draw in the new benchmark took 19.178 ms and constructed 500 message subtrees, versus a warm mean of 15.501 ms constructing six. A 90-message first draw took 5.555 ms, with warm mean 15.121 ms. These are different workloads from the moving-scroll benchmark, not a before/after comparison or evidence that cold layout always dominates. Measure larger real histories and device attach before changing anchoring or introducing estimated heights.
4. **Over-capacity tool working sets.** The old bounded LRU could become slower than uncached parsing above 256 distinct inputs. Address admission churn without increasing memory indefinitely or returning stale partial JSON summaries. This is smaller than the measured whole-frame scrolling cost and should not distract from it.
5. **History transfer allocations.** Incoming text is now moved into transcript entries instead of copied, including an extra discarded assistant copy. Reasoning prefixes are decoded in place, with oversized mostly-consumed buffers shrunk. This reduces one-time attach work, not per-frame cost.

## New measurements and reproducibility

These are Linux headless draw measurements and separately identified optimized standalone parser/cache measurements on a shared, heavily contended machine. They are elapsed CPU-side draw or parser times, not release iPhone presentation time, GPU completion, FPS, or energy measurements. No timing assertions are used.

### Long-reply parser

An optimized standalone exact-module harness compared the retained scalar parser
with the new parser, including output allocation and exact formatting equality:

| Inline workload | Scalar baseline | Chunked/indexed parser |
| --- | ---: | ---: |
| 336 KB plain UTF-8 prose | 6.497 ms/op | 0.610 ms/op |
| 225 KB mixed inline formatting | 4.492 ms/op | 1.188 ms/op |
| 31.9 KB unmatched backtick runs | 7.651 ms/op | 0.151 ms/op |
| Mixed formatting, successive 1 KiB streaming prefixes | 3.362 ms/prefix | 1.412 ms/prefix |

These isolated optimized parser numbers are **not debug whole-frame timings**.
Warm cached parses of unchanged large replies took 0.0513 ms/op for 336 KB plain
and 0.00953 ms/op for 250 KB mixed input in the standalone harness. Retained
sources can now be up to 512 KiB, but parsed entries plus source keys must still
fit the existing approximately accounted 2 MiB budget and 128-entry limit.
Amplified parses and larger inputs still bypass retention. Differential tests
cover exhaustive short combinations, randomized UTF-8, nested/escaped markers,
long unmatched runs, recursive slices, and partial streaming prefixes.

```sh
justrust test markdown::tests -- --test-threads=1
justrust test benchmark_inline_workloads -- --ignored --nocapture --test-threads=1
```

Use a separate `--release` run for optimized app-binary timings. The committed
ignored benchmark retains both parser paths so future comparisons need no
historical checkout or timing thresholds.

The integrated debug app binary also passed all 11 Markdown regressions
(`20261106-032412738-1582649`) and its same-binary ignored comparison
(`20261106-032432502-1586749`): plain 82.442 to 2.410 ms/op, mixed 74.869 to
12.758 ms/op, unmatched backticks 103.331 to 2.303 ms/op, and streaming prefixes
18.438 to 5.238 ms/prefix. These larger debug times reinforce the need to avoid
reparsing unchanged text, but should not be extrapolated to device latency.

### Cold-layout benchmark

```sh
justrust test benchmark_cold_transcript_layout -- --ignored --nocapture --test-threads=1
```

Receipt `20261106-031942593-1517653`. Each workload uses fresh row IDs, measures the first draw, then 20 notified warm draws. Source generation is outside the timer. The first draw builds every unknown-height row; later draws build six content subtrees in this sample. The test checks these counts without imposing machine-dependent timing thresholds.

### History buffer transfer

```sh
justrust test -p jcode-mobile-core benchmark_history_buffer_transfer -- --ignored --nocapture --test-threads=1
```

Receipt `20261106-031844861-1501719`. Twenty loads of 250 assistant messages:

| Message size | Old double-copy work only | New complete history reducer |
| --- | ---: | ---: |
| 4 KiB | 0.634 ms/load | 0.273 ms/load |
| 64 KiB | 8.400 ms/load | 2.589 ms/load |

This is deliberately a **copy-only lower-bound baseline**, not an exact old/new whole-reducer comparison. It does not measure network, JSON deserialization, first layout, or service latency. Message creation is excluded from both timers. Pointer-identity regressions verify direct transfer for user, assistant, and system entries, with tagged reasoning, Unicode, literal Markdown, and capacity-retention coverage. Existing optimistic-message reconciliation tests remain enabled.

## Deliberate limits

### Tool-cache overload

The cache now admits only every sixteenth miss when at entry/byte pressure,
instead of evicting a hot entry on every miss. Exact-source comparisons and
the 256-entry, 1 MiB source-byte and 128 KiB per-source limits are unchanged.
This is deterministic admission, not a larger unbounded cache.

A scratch-only optimized harness compared old/new/new/old/uncached in one
process, five repeats, 500 frames per leg. It recorded both elapsed wall time
and Linux process CPU time to distinguish scheduling noise. Receipt
`20261106-032614479-1612545`:

| Distinct short inputs per frame | Old cache median wall / CPU ms | New cache median wall / CPU ms | Uncached median wall / CPU ms |
| --- | ---: | ---: | ---: |
| 320 | 0.382 / 0.366 | 0.095 / 0.085 | 0.131 / 0.122 |
| 1,000 | 1.415 / 1.336 | 1.013 / 1.004 | 0.385 / 0.373 |

At 1,000 inputs, old/new CPU ranges were nonoverlapping (1.179–1.490 versus
0.937–1.127 ms/frame). This controlled run did not reproduce an earlier noisy
debug regression. **The new cache still loses to uncached parsing at 1,000
inputs.** It improves the previous cache, not every possible working set.
The committed ignored benchmark preserves both implementations and supports
`HEADLINE_BENCH_FRAMES` for shorter runs:

```sh
HEADLINE_BENCH_FRAMES=100 justrust test benchmark_cached_headline_frames -- --ignored --nocapture --test-threads=1
```

Regression tests enforce exact streamed-input invalidation, negative results,
entry/byte bounds, oversized bypass, and at least 150 cache hits per sequential
frame after warmup for both 320- and 1,000-input workloads. They assert behavior,
not machine-dependent performance thresholds.

### Rendering and device acceptance

The first layout still measures all rows. Very large individual messages still build their visible subtree. Streaming prefixes are not assumed immutable, and caches retain exact-source identity with explicit memory bounds. No event ordering, gesture ownership, scroll anchoring, or rendered content quality is relaxed for speed.

Before a larger rendering redesign, capture release-device CPU/GPU and presentation traces while scrolling, attaching a large history, streaming a long response, resizing/Dynamic Type changes, and swiping between conversations. The opt-in local spans help locate CPU work but cannot establish display smoothness or battery savings.
