# Sync reliability sweep, November 16, 2026

## Fixes

Following the archived rendering correction, the sweep reproduced and fixed:

- Local synced tabs not restarting transcript/receipt polling on reconnect or reopen.
- Late submission responses clearing a newly reopened tab's unsent draft.
- Continuation responses clearing edited drafts or navigating an unrelated tab.
- Late external lookups navigating after the active tab/session changed.
- Receipt identity/kind/source mismatches and unsafe transfer destinations.
- Uncertain continuation submissions becoming retryable without safe reconciliation.
- Native warning glyphs painting as a clipped single line despite reserving wrapped height. A separate flex slot improved normal text but still failed at 320 points and 1.6 text scale. Block layout with an absolute icon fixes the actual painted output.

Unknown-ID lost continuation submissions deliberately remain blocked. The current protocol cannot safely distinguish another client's same-source command from this client's lost response. Empty lists are not proof that retrying is safe. Known-ID malformed responses require exact valid command reconciliation.

## Evidence

- Final hosted workspace check and full library suite passed after the native banner refinement: **675 passed, zero failures, 16 ignored**, test receipt `20261116-021646650-3359760`. Final hosted clippy passed, receipt `20261116-021855847-3364126` (existing warnings remain). Whole-tree formatting checks encounter preexisting drift in unrelated files, which was not rewritten.
- Final native release binary SHA-256: `bf6d3e2ac5b23f7f22efd88ccbdb252589bb8cfd136771ef02dc3c4306162d67`.
- Eight actual environment configurations (dark/light, reduced motion on/off, text scale 1.0/1.6) each passed 51 sync and 9 composer tests: 480 passing executions. These are state/layout tests, not pixel acceptance.
- Extended raw-touch testing passed 2,048 cases across eight seeds and four viewport orientations/sizes. Stateful account/tab/draft/pending-prompt journeys also passed.
- A rebuilt native Linux GPUI preview used isolated storage and a localhost fake API. One synthetic message POST was queued/running/done, with no duplicate submission during repeated send attempts, a 503 transcript outage, recovery, and offline transition. Cached history and newer unsent drafts survived. Receipt polling retired after terminal completion.
- Native screenshots confirmed Markdown, tools, code blocks and transcript revision updates. Final 320-point light/1.6-scale screenshot visibly shows the complete four-line offline warning, unlike the pre-fix clipped screenshot. Private evidence: `~/.jcode/scratch/sync-sweep-20261116/block-light-320-scale1.6.png`.

No production messages or transfers were sent. The native preview is Linux GPUI, not Apple Simulator or a physical iPhone. Covered behavior is validated, not a universal proof of zero bugs or a published iOS release. Restart an old preview process to load the rebuilt binary.
