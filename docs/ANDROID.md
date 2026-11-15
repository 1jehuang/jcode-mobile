# Android shared GPUI build and acceptance

Android runs the same Rust app, demo seed data, and GPUI renderer as the other
platforms. `JcodeActivity` is NativeActivity lifecycle and OS integration glue,
not a second application UI.

## Build

Install Android SDK platform 36, build tools 35.0.0, NDK 27.2.12479018, and a JDK
(the scripts also find `~/Android/jdk-21`). Install the Rust targets
`aarch64-linux-android` and `x86_64-linux-android`.

```sh
./scripts/android-build.sh
```

The default output is `target/android/JcodeMobile.apk`, with both `arm64-v8a`
and `x86_64` libraries, compile/target SDK 36, and min SDK 26. Packaging requires
`--locked`, so a manifest/lock mismatch fails instead of silently changing
dependencies. Override the output
with `ANDROID_APK_OUTPUT`. `ANDROID_PROFILE=release` removes the debug demo
entry. Development APKs use a local debug signing key, not a publishing key.

Rust builds use `justrust build --local` with the documented exception that the
Android NDK is installed locally and is unavailable to hosted workers. Both ELF
LOAD segment alignment and uncompressed APK library alignment are checked for
16 KiB support. The script verifies the shared app's NativeActivity export.

## Debug-only shared demo

For an already installed debug APK, the acceptance owner can launch the existing
shared demo without account login or email sending:

```sh
adb -s "$ANDROID_SERIAL" shell am force-stop org.jcode.mobile
adb -s "$ANDROID_SERIAL" shell am start -W -n org.jcode.mobile/.JcodeActivity --es gpui_demo todos-expanded
```

Other allowed values are `todos`, `git`, `git-expanded`, `git-loading`,
`git-error`, and `git-clean`. The mode is configured before Rust startup,
uses the shared Rust seed methods, and disables account/diagnostic uploads.
Release APKs do not call or export this debug-only route. Use a fresh process
when changing modes, since the Rust startup selection is immutable per process.

## Device acceptance ownership

Do not run emulator helpers while another acceptance owner is using the device.
The helper creates/starts an emulator and installs the APK, so it is explicitly
not a read-only check. The assigned acceptance owner alone should run it.

`scripts/android-emulator.sh` defaults to API 36. Set
`ANDROID_EMULATOR_TAG=google_apis_ps16k` to select the 16 KiB image. The helper
checks the actual runtime API and, for ps16k, `getconf PAGE_SIZE` equals 16384.
`ANDROID_SERIAL` selects an existing emulator without starting a second one.

Static alignment checks do not establish runtime rendering, IME, insets,
TalkBack, or 16 KiB device acceptance. Those remain explicit device checks.
API 30+ system-bar/cutout and IME insets flow from Android WindowInsets through
JNI into the GPUI UI loop. Older Android versions use the backend content-rect
fallback. Verify keyboard show/hide, composing edits, rotation, and safe areas
on the installed shared app, not a Java mock.

See `ANDROID_ACCESSIBILITY.md` and `ANDROID_PUBLISHING.md` for the separate
accessibility and release evidence gates. Building an APK is not permission to
publish it.

## Immutable Android integration artifacts, November 15, 2026

All four GPUI manifest specifications and the lockfile pin the published revision
`c249c6cefd4f578867f6277c20fe037fb5523435`. It preserves the prior iOS revision
and includes the Android backend, composing input/insets interfaces, and the
Android-only Vulkan debug naming workaround. All existing registry versions and
checksums were retained, with nine additional Android dependency packages.

The local NDK locked build passed for both ABIs. The build verified the native
entry point, ELF LOAD segment alignment, 16 KiB APK ZIP alignment, and v2/v3
signatures. These artifacts are debug/test only and must not be published:

| Artifact under `target/android/` | SHA256 |
| --- | --- |
| `JcodeMobile-gpui-c249c6c-api36-a11y-debug.apk` | `51f45d38b9ebd18438c060c0e5f1699d5711dba3397bcadb713e941da7a95ff2` |
| `JcodeMobile-gpui-c249c6c-api36-a11y-debug-UNSIGNED-TEST-ONLY.aab` | `84247a41da5d8079d571b9287e151b4f82e04449e8346424abc1239c1928f827` |

These supersede the earlier `2932937` pairs. They retain the real NativeActivity
accessibility host fix and include app keyboard mapping `857110e` and shared
secure-aware text semantics `b95c662`. GPUI `c249c6ce` includes the surface-free
single-backend selection, 112-byte storage path stride (WebGL texture stride
remains 104), and shared keyboard configuration mapping. The saved camera
private-cache/URI-grant production patch (`5ccee62`) was also present before this
build started, not added after packaging.

The locked dual-ABI APK rebuild passed in 75.0 seconds, compiling the Android
backend/configuration bridge against its published immutable revision. All
registry versions/checksums were unchanged by this repin. Matching actual AAB
build/validation passed in 37.8 seconds and the AAB remains unsigned. Runtime
API30/API36 acceptance belongs to the coordinator, and API26 compatibility is
not claimed by this artifact record.

The actual AAB was built from that APK, not mocked: `aapt2 convert
--output-format proto`, `scripts/android-bundle.py`, then Google bundletool
1.18.2 `build-bundle` and `validate`. Validation passed with the base dex and both
native ABI libraries present. It is explicitly an **unsigned debug test bundle**,
not a release candidate. No production signing identity or publishing action was
used. `scripts/android-bundle.sh` remains the separate release-profile local
bundle workflow and still requires an explicit `BUNDLETOOL_JAR`.

This packaging lane performed no emulator start/stop/install operations. Runtime
acceptance of the final pinned artifact belongs to the root coordinator. Physical
ARM64 hardware, TalkBack traversal, full IME composition/selection, and release
signing/publishing remain separate evidence gates, not inferred from this build.
