# Shared GPUI Android validation, 2026-11-15

## Scope and provenance

Android runs the existing Rust GPUI MobileApp in a NativeActivity. Java adapters supply Android OS services, input and accessibility, not a replacement application UI. Earlier Java-widget acceptance is withdrawn and is not evidence for this implementation.

The GPUI fork preserves the previous iOS revision through an ordinary merge. Published baseline `5198b79914318170b5cae0ce785c0b1d61f901d5` includes the Android backend and iOS history. Published `2932937ef09f8b5f6fef4a8ad3c18751f3b3ebe2` additionally selects exactly one graphics backend before creating a native surface.

## Observed emulator results

### Latest integrated artifact

`JcodeMobile-gpui-c249c6c-api36-a11y-debug.apk`, SHA256 `51f45d38b9ebd18438c060c0e5f1699d5711dba3397bcadb713e941da7a95ff2`, includes the published keyboard mapping, GLES storage padding, shared text-input semantics and private camera provider.

- **API35 Android15: all14 shared-GPUI checks passed in38.5s against the latest combined APK.** Evidence: `target/android/acceptance-c249-api35-final/`.
- **API30 Android11: all 14 shared-GPUI checks passed in49.0s.** Actual GLES shader creation and rendering now work. Evidence: `target/android/acceptance-c249-api30/`.
- **API36 Android16, actual16384-byte pages: all14 checks passed in46.8s.** Evidence: `target/android/acceptance-c249-api36-16k/`.
- **API35 native instrumentation passed in14.4s** against this installed artifact: actual GPUI field values after composing/replacing/committing/deleting through the production InputConnection, production email/OTP/composer EditorInfo mappings, unchanged-hint suppression, composing-shadow preservation, real temporary accessibility activation/restoration, real clipboard round-trip, actual content-host discovery, pause/resume callbacks, and production Keystore AES-GCM checks. This is not TalkBack acceptance or a claim about every keyboard's restart timing.
- The matching test AAB is `JcodeMobile-gpui-c249c6c-api36-a11y-debug-UNSIGNED-TEST-ONLY.aab`, SHA256 `84247a41da5d8079d571b9287e151b4f82e04449e8346424abc1239c1928f827`. It remains unsigned test-only, not a production release.

- **API36 native instrumentation also passed**, including real composing InputConnection and production Keystore operations. Saved native log: `/home/jeremy/.jcode/scratch/native-c249-api36/NativeInstrumentation.txt`.
- **API30 native checks and post-finish cleanup passed.** On Android11, the main instrumentation intentionally returns `NEEDS_CLEANUP_PROBE`, never PASS. A separate fresh instrumentation then verifies the real accessibility enabled state and unchanged original user-preference fingerprint after framework disconnect. Both phases and storage checks passed. Evidence: `/home/jeremy/.jcode/scratch/native-c249-api30-final/`.
- **Latest AAB-derived universal APK passed all14 checks on actual16KiB API36 in40.2s.** APK SHA256 `b58208e29f06b19b5993cc1110c2a23320b3342c3a09cee5de1d278f0798726f`. Evidence: `target/android/acceptance-c249-aab-api36-16k-ime-ready/`. The harness now waits for stable real keyboard readiness before injecting adb text, saves command failures, and preserves cleanup errors rather than hiding them.
- **Actual gallery plus virtual-camera capture passed14 checks on API35 in88.7s.** Real DocumentsUI import, external camera shutter/review acceptance, both thumbnails retained in the shared GPUI composer, and return without observed crashes. Evidence: `target/android/media-c249-api35-recovered/`.
- **Distinct-UID camera grant security checks passed** with a unique terminal PASS/-1: external recipient reads/writes actual JPEG through the exact production URI, sibling/traversal access is denied, production decoding and cleanup execute, and the same still-live external recipient loses read/write access after explicit revocation. Evidence: `/home/jeremy/.jcode/scratch/android-native-services-tests/camera-grant-roundtrip.txt`. Earlier malformed Parcelable/false shell-success output is not counted.

The chronological observations below retain the earlier failures and artifact-specific results. Those older failures are not attributed to this latest artifact unless explicitly retested.

- Android 15, API 35: all 13 shared-GPUI screenshot/OCR acceptance checks passed against the dual-ABI API36 debug APK with the Vulkan naming fix.
- Android 16, API 36: all 13 checks passed on the actual 16,384-byte-page emulator against `JcodeMobile-gpui-5198b79-api36-a11y-debug.apk`, SHA256 `1ac3fbedb05a16a95d11227ee2b20ca9df8f450fe49f6d4dd563ec0d6c4be450`.
- These checks exercise shared onboarding, absence of Apple-only login, real software keyboard, email-field text, keyboard avoidance, unsent draft preservation, existing shared Rust chat demo, composer input, rotation and surface/background recreation. They do not authenticate, send email or invoke a live model.
- Android 11, API 30: the initial combined Vulkan/GLES native-window ownership failure was fixed by `2932937`. Actual subsequent execution reaches shader pipeline creation, but fails GLES validation because a path-rasterization binding has size 104 instead of a multiple of 16. This remains a real compatibility failure until a rebuilt fix passes on the emulator.
- Actual shared attachment controls launch Android DocumentsUI and return to the same GPUI conversation on cancellation. Positive API35 import also passed: a locally generated 96x64 blue JPEG was selected through the actual OS picker and returned to the live shared GPUI conversation, rendering a 145x97 thumbnail at screen offset42,1454. Screenshot `target/android/acceptance-photo-import.png` contains12,671 matching normalized blue pixels. No photo or model request was sent. Camera capture is separately tracked below; microphone recording remains untested.

The screenshot-based harness is `scripts/android-gpui-acceptance.py`. Evidence is under ignored `target/android/` acceptance directories. An observed checkout revision is not represented as an APK build revision. Debug demo content is explicitly offline and never represented as a live server/model response.

## Native services and accessibility

Actual API35 camera execution reached the external Android camera, shutter and review screen. Accepting the picture exposed a real external-camera crash: the pending MediaStore image was writable only by its owner despite an explicit URI grant. The GPUI app survived and cleaned up the cancelled capture, but no new attachment arrived. A private-cache, exact-URI-granted provider replacement is being validated. Camera capture is therefore tested but not yet passing. Microphone capture remains untested.

Actual API35 instrumentation repeatedly passed production Android Keystore AES-GCM operations inside an isolated test UID: non-exportable key, nonce uniqueness, ciphertext-only no-backup envelope, authenticated corruption rejection and clear. Actual NativeActivity clipboard round-trip and pause/resume callbacks also passed.

Full native input instrumentation has not passed. Testing identified and repaired discovery of NativeActivity's real NativeContentView, then enabled actual accessibility through UiAutomation. The remaining missing editable semantics in the shared custom TextInput are being repaired rather than accepting a Java-side mirror as evidence. Composing IME, semantic value updates and TalkBack remain acceptance requirements.

## Graphics bugs found by actual execution

The real 16KiB emulator exposed a Vulkan driver crash in debug-utils naming for wgpu's internal indirect-validation descriptor layout. The Android-only instance policy disables debug naming while retaining validation. The rebuilt app subsequently passed the real 16KiB shared-UI checks. Desktop flags are unchanged.

The API30 emulator exposed competing Vulkan/GLES ownership of the native window. Android now probes Vulkan without a native surface and chooses a single backend. Subsequent GLES shader validation exposed the separate alignment problem described above. No API26 compatibility claim is made.

## Build and source checks

The shared GPUI fork passed 470 library tests, 21 final backend tests and the actual Android NDK cross-check. The upstream preservation checker reports 441 files and zero violations. Apple host checks and active iOS-simulator font-kit cross-check passed. Active macOS cross-check remains blocked by existing Linux-host media bindings generation.

Dual-ABI locked Android builds, APK signing, ELF/APK 16KiB alignment and bundletool validation passed for debug artifacts. Matching AABs are unsigned test-only bundles, not production releases. The actual `2932937` test AAB was converted with bundletool to a universal APK using only the debug signer. That AAB-derived APK passed all 13 real API36/16KiB UI checks in 49.3 seconds. APK SHA256: `135171bf2e71796f7cb1e4ffac26096daf538d9601afe6dbe397c6b2482330b3`. Evidence: `target/android/acceptance-2932937-aab-api36-16k/`. This validates the test bundle path, not production signing or release readiness.

## Publishing boundary

Actual offline release inspection of the test AAB with official bundletool configured rejects it with `Debuggable/test-only bundle is not releasable`. No network or publication operation is needed for that gate.

No Play upload, API edit or publishing commit has occurred. Production upload signing, authorized Play configuration and genuine complete parity evidence are required. Remote FCM push is not implemented and the backend remains APNs-only. Full authenticated cloud/network, TalkBack, microphone/voice workflows and production release-bundle acceptance remain unverified. Composing InputConnection, positive gallery/virtual-camera workflows and the debug AAB-derived runtime now pass as recorded in the latest integrated-artifact section. Physical ARM64 runtime has not been tested because the user has no Android phone. Emulator checks will be exhausted without substituting unsupported hardware claims.
