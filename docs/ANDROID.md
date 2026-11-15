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
and `x86_64` libraries, compile/target SDK 36, and min SDK 26. Override the output
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
