# Shared GPUI Android validation, 2026-11-15

## Scope and provenance

Android runs the existing Rust GPUI MobileApp in a NativeActivity. Java adapters supply Android OS services, input and accessibility, not a replacement application UI. Earlier Java-widget acceptance is withdrawn and is not evidence for this implementation.

The GPUI fork preserves the previous iOS revision through an ordinary merge. Published baseline `5198b79914318170b5cae0ce785c0b1d61f901d5` includes the Android backend and iOS history. Published `2932937ef09f8b5f6fef4a8ad3c18751f3b3ebe2` additionally selects exactly one graphics backend before creating a native surface.

## Observed emulator results

- Android 15, API 35: all 13 shared-GPUI screenshot/OCR acceptance checks passed against the dual-ABI API36 debug APK with the Vulkan naming fix.
- Android 16, API 36: all 13 checks passed on the actual 16,384-byte-page emulator against `JcodeMobile-gpui-5198b79-api36-a11y-debug.apk`, SHA256 `1ac3fbedb05a16a95d11227ee2b20ca9df8f450fe49f6d4dd563ec0d6c4be450`.
- These checks exercise shared onboarding, absence of Apple-only login, real software keyboard, email-field text, keyboard avoidance, unsent draft preservation, existing shared Rust chat demo, composer input, rotation and surface/background recreation. They do not authenticate, send email or invoke a live model.
- Android 11, API 30: the initial combined Vulkan/GLES native-window ownership failure was fixed by `2932937`. Actual subsequent execution reaches shader pipeline creation, but fails GLES validation because a path-rasterization binding has size 104 instead of a multiple of 16. This remains a real compatibility failure until a rebuilt fix passes on the emulator.
- Actual shared attachment controls launch Android DocumentsUI and return to the same GPUI conversation on cancellation. Positive photo import, camera capture and microphone recording are not yet acceptance-tested.

The screenshot-based harness is `scripts/android-gpui-acceptance.py`. Evidence is under ignored `target/android/` acceptance directories. An observed checkout revision is not represented as an APK build revision. Debug demo content is explicitly offline and never represented as a live server/model response.

## Native services and accessibility

Actual API35 instrumentation repeatedly passed production Android Keystore AES-GCM operations inside an isolated test UID: non-exportable key, nonce uniqueness, ciphertext-only no-backup envelope, authenticated corruption rejection and clear. Actual NativeActivity clipboard round-trip and pause/resume callbacks also passed.

Full native input instrumentation has not passed. Testing identified and repaired discovery of NativeActivity's real NativeContentView, then enabled actual accessibility through UiAutomation. The remaining missing editable semantics in the shared custom TextInput are being repaired rather than accepting a Java-side mirror as evidence. Composing IME, semantic value updates and TalkBack remain acceptance requirements.

## Graphics bugs found by actual execution

The real 16KiB emulator exposed a Vulkan driver crash in debug-utils naming for wgpu's internal indirect-validation descriptor layout. The Android-only instance policy disables debug naming while retaining validation. The rebuilt app subsequently passed the real 16KiB shared-UI checks. Desktop flags are unchanged.

The API30 emulator exposed competing Vulkan/GLES ownership of the native window. Android now probes Vulkan without a native surface and chooses a single backend. Subsequent GLES shader validation exposed the separate alignment problem described above. No API26 compatibility claim is made.

## Build and source checks

The shared GPUI fork passed 470 library tests, 21 final backend tests and the actual Android NDK cross-check. The upstream preservation checker reports 441 files and zero violations. Apple host checks and active iOS-simulator font-kit cross-check passed. Active macOS cross-check remains blocked by existing Linux-host media bindings generation.

Dual-ABI locked Android builds, APK signing, ELF/APK 16KiB alignment and bundletool validation passed for debug artifacts. Matching AABs are unsigned test-only bundles, not production releases. An AAB-derived APK still requires runtime acceptance after the remaining fixes land.

## Publishing boundary

No Play upload, API edit or publishing commit has occurred. Production upload signing, authorized Play configuration and genuine complete parity evidence are required. Remote FCM push is not implemented and the backend remains APNs-only. Full authenticated cloud/network, TalkBack, composing IME, positive media workflows and release-bundle runtime acceptance are not claimed. Physical ARM64 runtime has not been tested because the user has no Android phone. Emulator checks will be exhausted without substituting unsupported hardware claims.
