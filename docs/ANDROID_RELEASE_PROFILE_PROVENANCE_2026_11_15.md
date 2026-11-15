# Android release-profile artifact provenance, November 15, 2026

## Safety and scope

These are **TEST-ONLY**, non-debug Rust release-profile GPUI artifacts. The APK
uses the already-existing Android debug signer solely for coordinator-owned
emulator acceptance. The AAB is unsigned. Neither is a production release or
approved for publishing. No production signing identity was available, created,
or used. No Play API, upload, publication, device, or emulator operation was
performed by this packaging lane.

Before building, `/home` had 1.9 TiB free (51% used), AC was online, and the
battery was charging at 32%. At completion it was charging at 37%, with the same
reported disk headroom. `justrust local status` reported ON before building.
This lane did not change that policy or run `justrust local on`. The existing
`scripts/android-build.sh` lines 51-55 and `docs/ANDROID.md` explicitly document
the local Android NDK exception because hosted workers lack this installed NDK.
Every Rust invocation used justrust, with that reason, the NDK clang/lld linker,
and `JUSTRUST_LINKER=off`. No profile, dependency, or source change was made by
this lane, and no branch/worktree was created.

## Final artifacts

Paths are relative to the existing `/home/jeremy/jcode-mobile` checkout.

| Artifact | Bytes | SHA256 |
| --- | ---: | --- |
| `target/android/JcodeMobile-gpui-c249c6c-api36-release-TEST-ONLY.apk` | 67613300 | `1c758a637153d2ddcd137cadc3bce2457a8bce1fe032b6c2bb93e79f56b0ee24` |
| `target/android/JcodeMobile-gpui-c249c6c-api36-release-UNSIGNED-TEST-ONLY.aab` | 27552963 | `49cd9821e2d9fc862ec9c8a26ccdd1da6f6df1dfb44922fd127b667b320788e5` |

The APK's matching `.idsig` sidecar was copied from the same final bundle run.
The final APK is an exact copy of `target/android/JcodeMobile-bundle-release.apk`.
The AAB is an exact copy of `target/android/JcodeMobile-unsigned.aab`.

Both contain arm64-v8a and x86_64, compile/target SDK 36 and min SDK 26,
package `org.jcode.mobile`, version name `0.1.0`, version code `1`. These metadata
values do not establish compatibility acceptance or production readiness.

## Source identity and concurrency

GPUI remains pinned in all four manifest specifications and the lockfile to
`c249c6cefd4f578867f6277c20fe037fb5523435`. The path dependency checkout
`../jcode` was at `5d6154f2527ed65acadc90c9c0b67622a0521b9d`, with no working
diff in `crates/jcode-voice`, `crates/jcode-core`, or `crates/jcode-sdk`.

The initial release build started from `5cb83ab93715da21bfb7e85a59da23889b9e03fb`.
Concurrent coordinator work changed app layout/test sources while that build
ran. The initial APK and first bundle were therefore superseded, not delivered
as the final matched pair. The final bundle run took a full tracked-input
snapshot at 07:44:58 UTC and verified it was unchanged after packaging at
07:45:23 UTC. Its base commit was `981093d673c34e72d4374df53eadd167854c728f`
plus the captured four-file source patch. That exact source content was then
committed by its owner as `dfd27eec58d5511baaa5aff0b1817fe0fa90c353`.
A fresh tracked-input hash after that commit matched the final-build snapshot.

Local evidence under `target/android/release-provenance/`:

- `final-source-before.sha256` and `final-source-after.sha256` each have SHA256
  `c408e367727b500e60e704a0c64ac08b3dba317b3dcf3c9f9abde9719ee70652`.
  They list `git ls-files src android/app scripts Cargo.toml Cargo.lock` contents.
- `final-source-working.diff` SHA256:
  `1d91c52f24736322bfd8fbc6f6251fe5f1757c2dfeff4074873034ff049f44ed`.
- `Cargo.toml` SHA256:
  `95b2c76a7b268fab8f55f9aa23e0dc797967a1c2ac6a10481774862ac089798f`.
- `Cargo.lock` SHA256:
  `c4e9a54150c8ed7c48d9a5ec206f7c02529bca5bc36d94aca1471ad78763845d`.
- `scripts/android-build.sh` SHA256:
  `286355a7dfb638a61c22e5d4a08b1f8385f911a14692dcf39d093c65eb79e3e6`.
- `scripts/android-bundle.sh` SHA256:
  `9008ab0a4506a962f4b24d9c726ab59e0130d5516230359456138fb9552f2f28`.

## Exact build commands and tools

The existing debug keystore `~/.android/jcode-debug.keystore` was confirmed to
exist before scripts ran, so no key was generated. Commands were run from the
repository root, preserving the existing profiles and dependency configuration.

```sh
ANDROID_PROFILE=release \
ANDROID_APK_OUTPUT="$PWD/target/android/JcodeMobile-gpui-c249c6c-api36-release-TEST-ONLY.apk" \
JUSTRUST_LINKER=off ./scripts/android-build.sh

BUNDLETOOL_JAR=/home/jeremy/.jcode/scratch/android-bundletool/bundletool-all-1.18.2.jar \
JUSTRUST_LINKER=off ./scripts/android-bundle.sh

# Final consistent bundle run, after recording source hashes, repeats:
BUNDLETOOL_JAR=/home/jeremy/.jcode/scratch/android-bundletool/bundletool-all-1.18.2.jar \
JUSTRUST_LINKER=off ./scripts/android-bundle.sh

cmp target/android/release-provenance/final-source-before.sha256 \
    target/android/release-provenance/final-source-after.sha256
cp target/android/JcodeMobile-bundle-release.apk \
   target/android/JcodeMobile-gpui-c249c6c-api36-release-TEST-ONLY.apk
cp target/android/JcodeMobile-bundle-release.apk.idsig \
   target/android/JcodeMobile-gpui-c249c6c-api36-release-TEST-ONLY.apk.idsig
cp target/android/JcodeMobile-unsigned.aab \
   target/android/JcodeMobile-gpui-c249c6c-api36-release-UNSIGNED-TEST-ONLY.aab
python3 target/android/release-provenance/verify-final.py
```

The build script expands each ABI build to:

```sh
JUSTRUST_LINKER=off CARGO_TARGET_DIR="$PWD/target" \
  justrust build --locked \
  --local="Android cross-compilation requires the locally installed NDK" \
  -p jcode-mobile --lib --target aarch64-linux-android --release
# Same invocation with --target x86_64-linux-android for the second ABI.
```

It sets each target's linker/CC to NDK 27.2.12479018 Android API26 clang and
AR to its llvm-ar. Both target RUSTFLAGS are exactly
`-C link-arg=-fuse-ld=lld -C link-arg=-Wl,-z,max-page-size=16384`.
The bundle script invokes release packaging, `aapt2 convert --output-format
proto`, `scripts/android-bundle.py`, and official bundletool `build-bundle`
and `validate`, not a synthetic AAB.

Tools: justrust `0.1.0-alpha.27`, system rustc
`1.99.1 (58cf8f9c3 2026-10-13)`, SDK build tools `35.0.0`, SDK platform 36,
NDK `27.2.12479018`, SDK-local JDK21, bundletool `1.18.2`. The existing JAR SHA256
is `378b5434cd1378bef6b2bc527b8c7f0ff2584b273830335bce54d6d0813c8584`.
justrust explicitly recorded the target-compatible system toolchain because
its default jr toolchain lacks Android target libraries.

The initial clean release-profile build completed in 452.84 seconds, with
justrust runs `20261114-233609243-502360` (arm64, 231.9 seconds) and
`20261114-234001373-541387` (x86_64, 216.3 seconds). Its waste report identified
cold flag-specific dependency cache misses and dependency rebuilds. No attempt
was made to alter flags/profiles/dependencies to accelerate it. The first bundle
run took 44.03 seconds. The final consistent bundle run took 24.68 seconds.
Final arm64 run `20261114-234458673-586954` passed in 12.3 seconds.
Final x86_64 used justrust's exact-source local prebuild
`20261114-234417908-2690726-pre`, passed in 13.3 seconds, finished 40 seconds
before requested. Both `justrust show` records report local NDK exception,
`--release`, exit 0, and the four-file source snapshot. No tests ran in these
build invocations. Existing unused-code/deprecation warnings remain.

## Offline artifact verification

`verify-final.py` is an artifact-only local verification harness, not a source
change. SHA256: `1402d888d7aec8e64849461dd3e65bad9ca7c81a0b70a1feba592828da1db436`.
Its assertions passed and output is in `final-verification.json`. Tool output
is retained alongside it as `final-*` evidence. Checks performed:

- `aapt2 dump xmltree APK --file AndroidManifest.xml` and bundletool `dump
  manifest --module=base` show no `android:debuggable` or `android:testOnly`
  attribute. Effective debuggable is **false**, the platform default. This is
  not a claim that an explicit `debuggable="false"` attribute was inserted.
- `dexdump -d` shows no `gpui_demo` literal and no invoke of
  `nativeConfigureDebugDemo`. `JcodeActivity.onCreate` starts at
  `nativeInitInput`, with the debug Intent branch constant-folded away.
  The unused Java native method declaration can remain, but the callable route
  and both native implementations are absent.
- `llvm-readelf --dyn-syms --wide` confirms `ANativeActivity_onCreate` for both
  ABIs and no `Java_org_jcode_mobile_JcodeActivity_nativeConfigureDebugDemo`.
- Every ELF LOAD segment has alignment 16384, and
  `zipalign -c -P 16 4 APK` passed.
- `apksigner verify --verbose --print-certs` passed v2/v3 verification. Signer
  DN is `CN=Android Debug, O=Android, C=US`, certificate SHA256
  `27254903adb5b35c24575528b3e63289e7c9565fac6d23e6b3dc980608db668c`.
  This is a debug identity, never production signing.
- bundletool `validate` passed the final actual AAB. No JAR signing entries are
  present. Its DEX and both native libraries are byte-identical to the APK:

| Payload | SHA256 |
| --- | --- |
| `classes.dex` | `e9d3159aeae1594a25be1b7ad774790dd13828b9125b65ecae90ef91985ab103` |
| arm64 `libjcode_mobile.so` | `08285f183a83d75080ce2449c36959c4ab815585f7461cf1a67c9bd262a07968` |
| x86_64 `libjcode_mobile.so` | `0c9599c3f50325d72635cf3577dae063ce40a6bd47e53e54a75e8871a6f9e436` |

## Limits and handoff

Root/coordinator alone owns device acceptance. This record makes no release
runtime, physical ARM64, API26/API30/API36, TalkBack, real audio round-trip,
IME/composition, safe-area, or 16 KiB runtime claim. Static alignment is not
runtime acceptance. Removing the debug Intent route also means prior debug-demo
acceptance cannot simply be attributed to this release-profile APK. Production
signer, upload key, Play credentials/account access, production metadata and
publication approval remain missing or separately gated. Nothing was uploaded.
Only this provenance document is committed by the packaging lane. Binary
artifacts and local verification evidence remain under ignored `target/`.
