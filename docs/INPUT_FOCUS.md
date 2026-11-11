# Input focus on mobile

The composer must be allowed to lose focus. Tapping outside a text field blurs
it and dismisses the software keyboard. Rendering, session updates, and saved
drafts must not refocus the field. Tapping the composer again restores editing
without losing its draft. Taps on software keyboard keys or its background must
not blur the field.

## iOS touch delivery

GPUI is pinned to `d2a9e0f8ae74a8b28899b0a2fc68ac4c761fd364`, which
includes the cloud-session fix for UIKit text interaction recognizers. These
recognizers are attached to the whole GPUI view, so their default cancellation
and delay settings previously prevented outside taps from reaching the app's
blur handler. The pinned fix keeps raw touches flowing to GPUI.

The dependency pin and application regression test were recovered from the
managed cloud checkout on November 10, 2026 (Pacific time). No TestFlight upload
is part of this synchronization.

## Regression checks

```sh
justrust check
GPUI_LOCAL=0 scripts/mobile-fuzz.sh raw_touch_chat_outside_tap_releases_composer_focus
GPUI_LOCAL=0 scripts/mobile-fuzz.sh keyboard_bridge
```

The raw-touch regression checks outside dismissal, continued blur after drawing,
draft preservation, and intentional tap-to-refocus. Keyboard bridge tests check
that software keyboard taps preserve focus and route to the selected field.
These headless Linux tests do not exercise UIKit gesture recognizers. Verify on
an iPhone that tapping the transcript dismisses the keyboard, that it remains
closed during incoming updates, and that tapping the composer restores the
draft. Also check native selection, loupe gestures, and software keyboard taps.
