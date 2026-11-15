# Native transcript layout follow-up, November 15, 2026

## Attribution

The opt-in `transcript.layout_ready` interval starts in the transcript content wrapper's layout request and ends when that wrapper enters prepaint. It includes native layout, text measurement, sibling work and scheduling. It is not pure CPU execution time or a display fence. Its nested request/prepaint/paint intervals must not be added together.

Actual native release preview, 250 messages of 4096 bytes each, isolated loopback HTTP and pinned SSH fixture:

| Phase maximum, ms | Cold | Warm tab | Process relaunch |
| --- | ---: | ---: | ---: |
| Layout ready | 2165.842 | 2710.209 | 1368.655 |
| Request layout tree | 4.196 | 4.020 | 3.760 |
| Prepaint | 1.312 | 3.880 | 1.604 |
| CPU paint | 11.994 | 15.814 | 11.677 |

Evidence: `~/.jcode/scratch/mobile-sim-latency-20261115/profile-layout/large-history/`. The same 250 rows shortened to 128 bytes produced layout-ready maxima 48.157 / 227.115 / 51.241 ms (`profile-short250/`). Paragraph length dominates this native workload. Headless GPUI tests use different text measurement and cannot establish native shaping performance.

## Rejected or unproven candidates

Replacing the measurement-only absolute canvas with a transparent height observer removes a layout leaf without changing constraints or measured heights. Direct observer-versus-legacy-canvas regression tests cover row/child bounds, scroll range and heights. Native timing did not establish a speedup:

| Layout ready, ms | Cold | Warm tab | Relaunch |
| --- | ---: | ---: | ---: |
| Observer | 1144.084 | 1327.096 | 1069.166 |
| Legacy canvas | 1233.618 | 1101.353 | 1064.759 |

Evidence: `observer-candidate1/` and `observer-baseline1/`. This is a structural simplification, not a claimed latency improvement.

An explicit user-bubble maximum-width experiment also failed to improve the native workload and was removed rather than shipped:

| Layout ready, ms | Cold | Warm tab | Relaunch |
| --- | ---: | ---: | ---: |
| Width experiment | 1180.339 | 1230.717 | 1009.032 |
| Observer baseline | 899.010 | 763.784 | 606.812 |

Evidence: `width-candidate1/` and `width-baseline1/`. These sequential runs on a contended host are not rigorous speedup estimates. Neither comparison supports retaining the width constraint.

## Exact measured-geometry reuse candidate

A bounded secondary cache reuses native-measured row heights across transcript identities only for settled user/assistant prose. Keys include rendered-content fingerprints, role, prompt badge digit width, available column width, text scale and actual window pixel density. Images, tools, reasoning, streaming and synced tool output are excluded. The cache stores hashes and heights, not raw text or account identifiers, is capped at 4096 entries, and lives only within the app process. A miss always invokes normal native layout. Full scroll geometry and earlier history remain available.

This avoids reshaping unchanged offscreen prose on a warm reopen. It does not eliminate cold-process measurement or server-imposed waits.

### Native acceptance

Three alternating candidate/baseline/candidate runs used frozen release binaries, the identical 250 × 4096-byte fixture, and continuous frame capture before OCR. All nine cold/warm/relaunch opens passed fresh network history, ordered UI application/render and session-specific pixel verification.

| Native run | Cold layout (ms) | Warm layout (ms) | Relaunch layout (ms) | Warm fresh pixels upper bound (ms) |
| --- | ---: | ---: | ---: | ---: |
| Shared cache, first | 724.990 | 4.527 | 841.388 | 151.755 |
| Observer baseline | 1041.660 | 1390.960 | 1685.916 | 1620.509 |
| Shared cache, repeat | 1001.654 | 16.409 | 1084.393 | 250.364 |

Evidence: `shared-candidate1/`, `shared-baseline1/`, `shared-candidate2/` beside the earlier campaigns. The large reduction repeats for the targeted warm path. Host contention makes these small samples unsuitable for phone/p95 or exact percentage guarantees. Cold and process-relaunch layout remain substantial, and their differences are not claimed improvements. Warm screenshot verification checks a newly created session suffix, not merely cached text. No earlier history is discarded.

Causal geometry tests additionally compare all row/child bounds, measured heights, full scroll range and offset against a fresh render. They establish cross-tab/id reuse with the primary identity absent and invalidation for same-byte-length edits, role, badge digits, width, text scale and pixel density. Exclusion tests cover reasoning, streaming, tools, system output and images. Focused hosted acceptance: **3 passed, 1 ignored**, receipt `20261115-061935560-2664859`.

Integrated hosted app acceptance: **495 passed, 0 failed, 12 ignored**, receipt `20261115-062128862-2700221`. Final cache/geometry suite including bounded eviction and invalid measurement rejection: **10 passed, 0 failed, 2 ignored**, receipt `20261115-062555876-2786573`. All 10 Python driver safety tests passed. A final nine-scenario campaign against the cache binary passed all 27 cold/warm/relaunch opens (`shared-final-nine/`), with ready/history/render ordering and fresh-session pixels checked in every case.

Frozen native candidate SHA-256: `faa6f2dc14287f92ac825f26f05447ff9be9cdeb16b57da9e08a7969cdefcc20`. Observer baseline: `ec5ad54bcc8ee535646707f98f5474e3ef67616bb736570778136bed889ec555`. No production backend deployment, timeout reduction, authentication/pinning change or new phone latency claim accompanies this optimization.

## Remaining cold-path investigation

User-space `samply` recording was attempted but the installed profiler requires `perf_event_paranoid <= 1`; the host is configured at 2. No system security setting was changed. A bounded alternative launched only the isolated fake-account native app under its own parent GDB and took 24 main-thread snapshots per cold/relaunch process, with no attachment to unrelated processes. The first diagnostic attempt produced empty sample files and was discarded. The corrected samples are `cold-stack-2854509.txt` and `cold-stack-2854911.txt`, alongside the successful `cold-stack-campaign2/` freshness checks.

The corrected stacks show substantial native text work in `harfrust` GSUB/GPOS shaping, `cosmic_text::shape::ShapeWord`, `ShapeLine::layout_to_buffer`, Unicode/script/bidi handling and font matching. Initial startup also samples font database parsing and Linux graphics initialization. These interrupted snapshots are qualitative attribution, not unbiased CPU percentages or performance measurements. Debugger-run latency is deliberately excluded from the comparison tables.

This remaining cost is specifically in the Linux simulator's Cosmic/Harfrust font backend. It is not evidence that iOS CoreText has the same bottleneck. Disabling shaping features, guessing row heights or temporarily dropping older history would change correctness to improve a simulator number, so none was done. The next device-targeted optimization requires a corresponding iOS cold-layout capture. App-level warm geometry reuse is independently validated and does not depend on that backend diagnosis.

The installed Cosmic 0.19.0 source also exposes an optional `shape-run-cache` feature. It was inspected rather than blindly enabled: its helper retains raw run text/glyphs in a map, provides age trimming rather than a hard byte cap, and the pinned GPUI backend does not currently integrate those trimming calls. A safe adoption needs bounded retention and Unicode/direction correctness validation in the toolkit, not merely a new dependency feature. No toolkit or dependency changes were made in this round.

### Device follow-through

The final app now records `ui slow transcript layout` only for content-layout request-to-prepaint intervals strictly over 50 ms, even without opt-in local profiling. It uses a fixed privacy-allowlisted label and duration only, with no row text, identifiers or dynamic error details. Threshold boundaries are tested without sleeps. This provides a corresponding iOS signal before changing the wrong platform's text renderer. It is measurement, not another claimed speedup.

Hosted final diagnostic checks passed: app check `20261115-063126231-2873947`, 10 privacy/diagnostic tests `20261115-063138002-2876696`, six motion/threshold tests `20261115-063156572-2879986`, and native release build `20261115-063208375-2881738`. With opt-in profiling absent, native large-history runs emitted the diagnostic for 1479 ms cold and 602 ms relaunch layout, and no warm slow-layout stage. Final awake fresh-pixel upper bounds were 346 / 123 / 225 ms. The large-history smoke still had a 3364 ms cold pixel upper bound and 1287 ms relaunch bound, while warm was 156 ms. These negative cold results are retained, not represented as universally subsecond. Evidence: `diagnosed-final/` and `diagnosed-awake/`. Final-source full app acceptance after adding the device diagnostic: **497 passed, 0 failed, 12 ignored**, hosted receipt `20261115-063334444-2913905`. The final diagnosed native artifact SHA-256 is `90db0203dc93fccba5c5c72735fe19e6c66bd5465d72ca9691f78ee90b5ec400`.
