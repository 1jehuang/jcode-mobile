# Composer action alignment, November 16, 2026

- Replaced text-font plus/up-arrow glyphs with bundled SVG icons using matching 20-point boxes, 2-unit rounded strokes and explicit native tint. Their alignment no longer depends on a text baseline or font scaling.
- Both controls keep 44-point touch targets, with smaller centered 36-point painted circles. Composer left/right padding is now symmetric, with equal 4-point gaps and a 52-point minimum pill height.
- Send, stop, voice-cancel, waiting and attachment actions preserve existing handlers and enabled-state gating. No message or photo was sent during native acceptance.
- New headless GPUI regression checks 320/390/430-point widths, empty/single/multiline drafts, equal button centerlines, exact nested bounds and no text-field overlap. Existing send/stop/attachment behavior tests remain green.
- Final hosted check, complete workspace library tests and Clippy passed. **687 passed, 0 failed, 16 ignored**. Test receipt `20261116-030759206-3501193`. Native release build receipt `20261116-030624380-3482326`.
- Actual Linux native preview inspected at 390 points/dark/1x and 320 points/light/1.6x, including typed draft/enabled send. The first vector preview exposed missing explicit SVG tint, which was corrected and visually rechecked before source freeze. This demonstrates why layout tests alone are insufficient.
- Private synthetic-fixture screenshots: `~/.jcode/scratch/render-deep-20261116/composer-refined-390-tinted.png`, `composer-refined-390-draft.png`, and `composer-refined-320-light-1.6.png`. These are not Apple Simulator or physical-iPhone screenshots.

User subsequently requested a new TestFlight build after this refinement. Exact-source signing, upload and Apple availability evidence are recorded separately in `docs/TESTFLIGHT.md` once complete.
