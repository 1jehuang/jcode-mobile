# Archived synced transcript rendering, November 16, 2026

## Reproduction and fix

The native Linux GPUI phone preview reproduced an archived-session-specific mismatch. Live synced sessions already used the shared cloud transcript renderer, but `live: false` sessions opened an expanded sidebar row containing raw `role` / flattened `text`. Structured tools, Markdown and images bypassed their normal rendering there.

Archived local sessions now open a normal local-chat tab through the same transcript projection and rendering path as live synced chats. This applies to drawer clicks, known notification targets and freshly resolved deep links. The sidebar no longer builds the full raw transcript. Its explicit **Continue in cloud** controls remain available when reopened. No continuation or message is submitted just by opening a session.

The existing live-computer and connection guards still disable sending for archived sessions. Cloud-tab drafts are parked intact, notification replies become unsent drafts in the local tab, and subsequent notifications do not overwrite that draft.

Production fix: `30566f6`. Added regression coverage: `bf48c3d`.

## Validation

- Hosted `justrust check --workspace --tests` passed, receipt `20261116-014656224-3254874`.
- Final hosted `justrust test --lib synced -- --test-threads=1`: **43 passed**, receipt `20261116-014853076-3260094`.
- Final hosted `justrust test --workspace --lib -- --test-threads=1`: **664 passed, zero failures, 16 ignored**, receipt `20261116-014912758-3260780`.
- Drawn regressions include archived structured cards at 320/390/430 points, expansion/collapse, error-output geometry invalidation, grouped-result image retention, archived navigation, preserved drafts and non-sendable status.
- The rebuilt release binary was exercised locally on an isolated Xvfb display at 320/390/430 points. A fresh production GET-only snapshot contained 1,055 rows, 499 tool calls and 499 results. Inspection confirmed formatted assistant prose and shared tool controls instead of the original sidebar dump. Native scrolling and narrow-width tool expansion were exercised. Reopening the drawer showed the explicit continuation input/button without the full transcript.
- Preview binary SHA-256: `50effd1d297813a7447d546a77112cfa25a1cefdc69d6efa458502b4fd158d71`.

The replay used a fake account and isolated application storage. Its localhost server rejected non-GET requests. Real account access only fetched an existing transcript. No production messages or cloud transfers were submitted. Private snapshots/screenshots remain under `~/.jcode/scratch/synced-render-20261116/`, outside the repository.

## Scope

This is the Linux native mobile preview, not Apple Simulator or a physical iPhone. Shared renderer and state regressions establish parity of the covered paths, not pixel-identical output, recovery of metadata omitted by legacy sync, or physical-device acceptance. Rebuilding the local preview does not update an already running old process or publish a new iOS build.
