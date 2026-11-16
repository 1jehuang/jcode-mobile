# Unified synced tool rendering, November 16, 2026

## Behavior

Synced transcripts no longer have a separate `Synced tool output` renderer, disclosure identity, output bounding policy, or layout-cache branch. Structured calls/results project into the same `ToolCall` model and `MobileApp::tool_line` used by cloud/live sessions. Call names, inputs, success/failure, output expansion and image grouping follow the existing shared path. Structured metadata supersedes flattened compatibility text, so it is not displayed twice.

Older output-only snapshots also project into the shared tool model. Their generic `tool` label and neutral `Unknown` status deliberately do not invent a call name, input, correlation or successful result. Stable synthetic identities avoid collision with actual tool IDs. Flattened assistant prose is not parsed heuristically into fabricated calls.

The desktop producer now persists an explicit wire version. A schema upgrade replays existing snapshots even when file revisions and visible fingerprints are unchanged. Upload progress is only advanced after the existing successful upload path. Cloud's existing same-sequence upsert preserves the structured fields and retries idempotently. No backend schema or production deployment was needed for this change.

## Verification

- Hosted `justrust check --tests`, `justrust test --workspace --lib`, release build and Clippy passed. Full workspace: **686 passed, 0 failed, 16 ignored**. Test receipt `20261116-025756237-3454681`, release receipt `20261116-025820001-3455270`.
- Desktop producer: hosted check and focused mobile-sync tests passed, **31 passed, 4 ignored**. Commit `2e74a62f4`.
- Cloud contract: **23 scoped Node tests passed**, including same-sequence legacy-to-structured upgrade, nested inputs, errors, empty display text, stable counts and idempotent retries. Commit `6caca31`.
- Core regressions cover ordinary cloud-history phase equivalence, no duplicate flattened output, legacy identity collisions and truthful unknown execution status.
- Headless GPUI taps expand/collapse legacy output using the normal card header and validate unchanged raw Unicode content, reader mode and expansion-sensitive geometry. The reviewed interaction ledger no longer lists the deleted renderer.
- Native Linux GPUI preview uses a synthetic account and GET-only loopback fixture. At 390 points/dark/1x and 320 points/light/1.6x, successful Read, failed bash and neutral legacy cards render through the normal UI, expand through actual mouse-to-touch input, and show no special synced-output heading or duplicate compatibility text. The expanded 320-point Read output wraps within the viewport. Private screenshots are under `/home/jeremy/.jcode/scratch/render-deep-20261116/shared-tools-*`.

These are source, local protocol and Linux native acceptance results, not a claim of physical iPhone or Apple Simulator validation. No production messages, transfers, deployments or account changes were performed. Existing desktop unrelated edits and Python cache directories were preserved.

## Repository publication

Publish mobile `main`, desktop `master`, and the cloud checkout's existing `feat/browser-code-subscribe` branch to their respective upstreams. The cloud repository is `/home/jeremy/solosystems-backend`, not a separate `jcode-cloud` directory. Do not merge its unrelated feature branch into `master` or claim a production deployment. Old snapshots acquire real metadata once the updated desktop producer runs and uploads them.
