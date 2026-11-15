# Native GPUI Android accessibility adapter

This is an Android OS service bridge for the shared native GPUI UI, not a Java
widget UI. It does not establish TalkBack/device acceptance by itself.

## Components and ownership

- GPUI backend `crates/gpui_android/src/accessibility.rs` retains incremental
  AccessKit semantics and serializes immutable virtual-node snapshots. It maps
  roles, text, values, bounds/transforms, enabled/selected/checked state, headings,
  editability, click/focus/scroll/set-text actions, and redacts password values.
- Its Java JNI callbacks only enqueue bounded requests and wake the native
  event loop. `A11yBridge::process_events()` executes GPUI activation, actions and
  deactivation on the native GPUI thread. Removed/stale/unsupported/disabled
  action targets are rejected. Android virtual IDs are stable for live nodes,
  are not reused across root replacements, and never truncate AccessKit's u64
  node identifiers to Android's i32 virtual IDs.
- `android/app/src/main/java/org/jcode/mobile/GpuiAccessibility.java` installs an
  `AccessibilityNodeProvider` delegate on the existing native `SurfaceView`.
  It does not create or paint widgets. Java view operations are marshalled with
  `Activity.runOnUiThread`. It exposes virtual-node lookup/search, accessibility
  focus, input focus, hit testing and hover events, and queues supported actions
  back to Rust. Snapshot installation is atomic. Activity/bridge tokens reject
  late snapshots. Accessibility enable/disable changes activate/deactivate GPUI.

The snapshot is a flattened semantic traversal. Clickable controls merge their
text descendants to avoid duplicate spoken button labels. This initial adapter
is not a full AccessKit platform implementation: embedded subtrees, live-region
announcements, rich text cursor/selection granularity and every AccessKit role
or custom action still need implementation/acceptance as required by the app.
Do not describe this as complete Android accessibility parity.

## Backend hook contract

Backend owner must register the module and dependencies and forward window hooks:

```rust
// lib.rs, including test builds
#[cfg(any(target_os = "android", test))]
mod accessibility;

// Platform startup
crate::accessibility::initialize(app.create_waker());

// Per native window state
// accessibility: crate::accessibility::A11yBridge::new()
// PlatformWindow::a11y_init -> self.state.accessibility.init(callbacks)
// PlatformWindow::a11y_tree_update -> self.state.accessibility.apply(update)
// Every native event-loop iteration, on the GPUI thread:
// self.state.accessibility.process_events()
```

`accesskit.workspace = true` and `serde_json.workspace = true` are needed for
Android compilation and host unit tests. The owner of window/platform/Cargo/lib
files inserts those hooks. Java installation is initiated by Rust `init`, using
the Activity classloader because a NativeActivity thread otherwise has the boot
classloader. No Activity application-UI changes are necessary.

AccessKit coordinates are physical pixels relative to the native surface. The
Java provider adds the host's screen location for `boundsInScreen`. Do not apply
density scaling a second time. `set_scale` exists only for a caller whose tree
actually uses logical coordinates. Verify hit rectangles on the actual device.

## Validation commands and evidence boundary

```bash
JAVA_HOME="$HOME/Android/jdk-21" scripts/android-accessibility-test.sh
```

The script compiles the Java provider against the installed API35 JAR by default
(the adapter uses APIs available at minSdk26, with guarded heading APIs). Override
`ANDROID_A11Y_PLATFORM_JAR` for another installed platform. Compilation requires
the actual `android.jar`, not just a partially created SDK platform directory.
Then it runs backend native semantic unit tests using hosted `justrust`. It
fails if backend module registration is missing. It never touches an emulator,
changes accessibility settings, uploads a bundle or publishes anything.

While backend hooks were pending, the exact adapter/test sources were also run
in an isolated lightweight Cargo harness with real AccessKit/serde_json and the
same `A11yCallbacks` signature, not the entire GPUI backend. All 16 semantic tests
passed on hosted justrust. They cover incremental updates, repeated metadata,
merged labels, hidden/unhidden nodes, root replacements, transformed bounds,
stale/disabled actions, text updates/read-only state, passwords, cycles, and
native-loop-only callback dispatch. An isolated Android-target JNI `check`
passed with real android-activity/jni dependencies. JustRust selected a local
target-compatible compiler because the hosted sysroot lacked Android target
libraries. This is compile evidence only, not a native app/device test.

The coordinator owns emulator5554. Required acceptance still includes launching
the **real native GPUI app**, enabling an actual accessibility service/TalkBack,
inspecting virtual nodes and bounds, navigating labels/roles, exercising focus
and double-tap actions through JNI, editing text, checking scrolling and teardown,
and verifying spoken output without exposing passwords. Prior Java-shell results
are not acceptance evidence. Keep publishing blocked until root accepts actual
GPUI device results and the separate release gates pass.
