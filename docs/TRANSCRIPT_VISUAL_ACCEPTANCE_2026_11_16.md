# Rich transcript visual acceptance, November 16, 2026

## Why another pass was needed

The earlier sync reliability sweep exercised formatted paragraphs and simple bullets but did not establish native pixel correctness for long ordered/bulleted lists. Correct measured bounds were not sufficient: native GPUI reserved multiple lines of height while painting one clipped flex-text line. This pass used richer synthetic transcripts and inspected actual native screenshots rather than treating the passing geometry suite as proof.

## Reproduced and corrected

- **Ordered and unordered lists:** replaced flex text sizing with block-flow bodies and absolute markers. Full continuation lines now paint at 320 points and 1.6 text scale instead of leaving a blank region. Bounded nested indentation is retained.
- **Quotes and inline links:** quote markers become a quote gutter, and inline links display their labels rather than raw Markdown syntax. Link labels are presentation-only, not a new clickable URL control.
- **Long code and indentation:** literal code lines remain intact inside a bounded horizontal code pane. Deep indentation no longer produces hundreds of repeated-indentation soft wraps. Horizontal native touch-style dragging reached the end-of-line suffix.
- **Code/tab gesture ownership:** horizontal code reading is consumed by its pane rather than also starting session paging. A dispatched integration regression exercises the real chat tree, not just an isolated Markdown widget.
- **Cross-tab code state:** identical sequence IDs in different sessions originally reused horizontal code position. The new regression failed with content at -220.5 points in the second tab instead of its own -9-point initial position. Scoping the transcript element to its tab fixes this; the dispatched regression passes after the fix.
- **Structured tool expansion:** expanding a long result previously kept tail-follow enabled and jumped past the header to the end of the output. It now enters reader mode just like legacy synced output. The new regression failed before the fix; native screenshots confirm the header and `START OF RESULT` remain visible afterward.
- **Tool summaries:** whitespace-only fields no longer hide useful headlines, nested JSON fields no longer masquerade as a top-level summary, and whitespace-prefixed JSON does not leak as a raw headline.
- **Prompt number badges:** badge dimensions scale with their Dynamic Type glyphs.

Production commits include `0970745`, `cb24926`, `c5c336f`, `964712b` and code identity/gesture correction `748b9b9`. The reviewed interaction ledger maps new code panes, scope IDs and list renderer to tests without claiming exhaustive device acceptance.

## Native evidence

The GET-only localhost fixture uses a fake account, isolated storage and synthetic PNGs. No production messages or continuations are sent.

Private screenshots in `~/.jcode/scratch/render-deep-20261116/`:

- `markdown-top-before.png`, `markdown-tail-before.png`: clipped list lines, blank reserved height, raw quote/link syntax.
- `markdown-top-after.png`, `markdown-tail-after.png`: multiline list glyphs, nested indentation and quote gutter at 320/dark/1.6 text scale.
- `tool-tail-jump-before.png`, `tool-reader-after.png`: tool expansion jumping to its end versus keeping its beginning visible.
- `code-touch-after.png`: native horizontal drag reaches a hidden code suffix without navigating to another tab.
- `rich-light-430.png`: whole rich fixture at 430/light/normal text size.
- `images-before.png`, `user-system-before.png`: image-only prompts, assistant images, long user text, explicit newlines and wrapped system notices were inspected as well.

Final validation on the integrated code:

- Hosted full library suite: **684 passed, zero failures, 16 ignored**, receipt `20261116-024456755-3426914`.
- Twelve actual environment configurations (dark/light, reduced motion on/off, text scales 1.0/1.6/3.1), three targeted suites each: **444 passing executions**, zero failures. Each configuration runs 19 Markdown, 8 synced-render and 10 tool-summary tests.
- Final hosted check passed before the full run. Clippy passed with existing warnings, receipt `20261116-024732661-3432559`. Release build passed, receipt `20261116-024735600-3432740`.
- Final release SHA-256: `e5db51f45ed6f31992c6c1a1564a2097b128746adaad60a96ca8dc470384ac71`.
- That exact artifact was launched at 390/dark/1.6 after the code-tab integration fix. `final-rich-390.png`, `final-code-drag-390.png` and `final-code-return-390.png` show wrapped list/quote glyphs, native horizontal code dragging to the suffix, and reversing to its beginning without changing tabs.

Geometry/dispatch checks and Linux native pixels are complementary evidence, not a universal proof of zero bugs. This does not update TestFlight or establish physical-iPhone acceptance.
