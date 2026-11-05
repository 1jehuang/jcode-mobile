# iOS system integrations

jcode-mobile is cross-built from Linux (`scripts/ios-bundle.sh`). This document
covers the parts that go beyond the Rust/GPUI binary.

## Layout

```
jcode.app/
  jcode                         Rust/GPUI binary
  Frameworks/JcodeBridge.framework   Swift dylib, dlopen'd by src/ios_ext.rs
  PlugIns/JcodeLive.appex       WidgetKit extension (Live Activity + Dynamic Island)
  PlugIns/JcodeShare.appex      Share extension
```

The Swift sources are a SwiftPM package in `ios-ext/`, built by
`scripts/ios-ext.sh` with the Darwin Swift SDK
(`swift build --build-system native --swift-sdk arm64-apple-ios`).
`NO_IOS_EXT=1` skips it. Bundle IDs and provisioning profiles for the
extensions come from `scripts/asc_extensions.py`.

## Deep links: `jcode://`

Everything enters through one URL scheme, parsed in `src/ios_ext.rs`
(`DeepLink`) and applied in `src/app/deep_link_glue.rs`:

| URL | Effect |
|-----|--------|
| `jcode://new` | new session tab |
| `jcode://new?text=...` | new tab, prompt prefilled and sent |
| `jcode://ask?text=...` | same as above |
| `jcode://session/<id>` | open/focus that session |

GPUI (`gpui_ios/src/platform.rs`) forwards `application:openURL:`,
home-screen quick actions, and `continueUserActivity` (Siri/Shortcuts) as URLs
to `App::on_open_urls`. For dev: `JCODE_MOBILE_OPEN_URL=jcode://... ` on launch.

## Live Activity / Dynamic Island

`sync_live_activity()` derives a `LiveState` (status, session title, tool) on
state changes and calls `jcode_live_start/update/end` in JcodeBridge (ActivityKit
has no C API, so Swift owns the `Activity`). The widget renders Lock Screen and
Dynamic Island compact/expanded/minimal views. Tapping opens
`jcode://session/<id>`. Requires `NSSupportsLiveActivities` (set in Info.plist).

## Share extension

Collects shared text and URLs, then opens `jcode://new?text=<shared>` via the
responder chain, so the main app does all the work.

**Why no App Group:** a shared container would need an App Group capability
on both bundle IDs, plus provisioning changes, and the app would still have to
poll the container. Handing the payload over in the URL is stateless and needs
no entitlements. The limit is URL length, which is fine for text and links.
Files and images are not supported.

## Shortcuts / Siri (App Intents limitation)

**App Intents cannot be built from Linux today.** An `AppIntent` only shows up
in Shortcuts if the bundle contains `Metadata.appintents`. Xcode generates this
with `appintentsmetadataprocessor` by reading the Swift compiler's const-value
output. That tool ships only as a macOS binary inside Xcode (the Darwin Swift
SDK bundle contains the macOS build too), and the format is undocumented. Without
the metadata, compiled intents are invisible.

What we ship instead, all buildable on Linux:

- **NSUserActivity shortcuts:** JcodeBridge's `jcode_donate_shortcuts` donates
  "New jcode session" and "Ask jcode" (declared in `NSUserActivityTypes`).
  They appear in Shortcuts, Siri Suggestions and Spotlight, and can be given a
  Siri phrase. They arrive through `continueUserActivity` with a `url` in
  `userInfo`.
- **Home-screen quick action:** "New session" (`UIApplicationShortcutItems`).
- **Shortcuts "Open URL"** with any `jcode://` link above, including
  `jcode://ask?text=` combined with "Ask for Input" for a parameterized prompt.

To add real App Intents later, build `ios-ext` once on a Mac with Xcode and
copy the resulting `Metadata.appintents` into the bundle (it only changes when
the intent definitions change).

## VoiceOver

GPUI builds an AccessKit tree. `gpui_ios/src/accessibility.rs` flattens it into
`UIAccessibilityElement`s on the root view. It is active when VoiceOver or
Switch Control is running (or `GPUI_IOS_A11Y=1`). Buttons merge their child text
into one label, and activation sends AccessKit `Click`.

GPUI only exposes elements with an explicit role, so app code tags controls
with `src/app/a11y.rs`: `.a11y_button("label")` for icon-only controls (≡, +,
↓, ×, ↻) and `.a11y_text_button()` for text buttons and composer pills.
