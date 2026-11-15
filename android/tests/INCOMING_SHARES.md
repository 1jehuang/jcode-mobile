# Root-run incoming share acceptance

## Status

The probe wiring passed hosted full-library tests **489 passed, zero failed,
11 ignored**, run `20261115-003358763-779602`. Hosted ordinary check
`20261115-003351660-779009` and release check
`20261115-003425671-780736` passed. Four Rust probe tests cover real draft
routing, real submission mutation/count retention across tabs, parked automatic
send detection, atomic sampling/freshness and release cfg boundaries. Actual SDK
Java DEBUG false/true compilation and bytecode exclusion tests passed. These
checks do not substitute for Android target builds or emulator acceptance.

The harness is implemented and its synthetic safety tests pass. It has **not**
been run on any device. It intentionally refuses APKs without an explicit native
`incoming-share-state-v1` probe marker, before even invoking adb. The existing
`gpui_demo=todos` route alone is not proof of offline/account safety.

The explicitly opted-in debug probe implements the read-only contract below.
The Android share-probe demo is offline **before** any account load or connection. Only
the share probe creates a synthetic noncredential account and installs the real
shared deep-link receiver. Do not replace the observed state with fixtures.

## Commands

Synthetic tests, no adb:

```bash
PYTHONDONTWRITEBYTECODE=1 python3 android/tests/incoming-share-state-test.py
PYTHONDONTWRITEBYTECODE=1 python3 scripts/android-incoming-share-acceptance-test.py
```

Only the device coordinator runs acceptance, on an owned disposable emulator:

```bash
python3 scripts/android-incoming-share-acceptance.py \
  --run emulator-SERIAL --apk /path/to/debug-with-probe.apk \
  --out /path/to/evidence
```

The explicit APK is installed without clearing app data. The target app is
force-stopped for a cold process and only the previous test probe artifact is
removed. Both cold and warm ACTION_SEND deliveries use `text/plain`,
`gpui_demo=todos`, and `gpui_share_probe=true`. The test never presses Send,
authenticates, requests a code, or invokes a model. Shared text is generated
synthetic text, not clipboard/user content.

## Required shared-app probe contract

The marker must exist in every packaged `libjcode_mobile.so`. It is only a
preflight to reject unsafe/uninstrumented builds, never acceptance evidence.
The opt-in debug route writes actual state atomically via temporary-file rename
to app-private `files/incoming-share-state.json`:

```json
{
  "probe": "incoming-share-state-v1",
  "offline": true,
  "syntheticAccount": true,
  "connectionPresent": false,
  "autoSendPending": false,
  "activeTab": 2,
  "tabCount": 2,
  "composer": "exact current GPUI composer text",
  "submittedCount": 0,
  "frame": 17
}
```

- `offline`, `syntheticAccount`, `connectionPresent`, `autoSendPending` read actual shared
  application fields, including any parked tab's pending automatic sends.
- `submittedCount` is a monotonic app-lifetime count of actual outgoing submit
  mutations, across all tabs. Both the cloud prompt LocalIntent mutation and
  local-computer submission mutation increment it. It is not the active transcript length, which can
  decrease after changing tabs or include seeded user entries.
- `frame` is a monotonically increasing snapshot sequence emitted by actual
  GPUI rendering/pumping. It must continue updating during observation periods.
- `activeTab`, `tabCount` and `composer` must read actual shared tab/editor state.
- Production/release builds must not expose this probe, write user content to
  this file, or accept the debug opt-in. Probe output contains only synthetic
  fixture text in the opted-in offline mode.

## Release exclusion gates

```bash
PYTHONDONTWRITEBYTECODE=1 python3 scripts/android-share-probe-release-test.py
python3 scripts/android-share-probe-release-check.py --apk /path/to/release.apk \
  --report /path/to/probe-absence.json
```

The SDK test compiles the actual Java sources twice, with constant DEBUG false
and true, then inspects actual `javap` instructions and class bytes. The release
branch must contain neither the probe Intent key nor an invocation of either
debug JNI configurator. Rust syntax tests enforce Android+debug_assertions cfg
on the JNI export, private probe path, probe module and app state. Hosted release
compilation checks the excluded configuration. The artifact gate scans every
packaged native ABI and all DEX members for the probe export, version/path,
fixture credential marker and debug Intent key. It is a narrow absence gate,
not a substitute for the normal signing/manifest/runtime release checks.

The harness rejects any invalid/missing field, connected/nonoffline state or
pending automatic send. Cold delivery must yield exactly two tabs and zero
submits. Warm delivery must create exactly one additional tab and preserve the
submit count. Each draft must equal the complete payload, including Unicode and
query punctuation, not merely contain a marker.

Each observation period is five seconds by default, minimum two seconds. State
is sampled throughout, fields must remain unchanged and the final sequence must
be newer, so a frozen stale file cannot pass. Sleep/wake resumes the **existing
Intent** to exercise consume-once behavior instead of replacing it with MAIN.
The emulator must initially be awake, really enter sleep and return to the exact
shared Activity. Wake restoration runs after failures during the sleep phase.

## Evidence and boundaries

`states.jsonl` records every actual state read, `commands.jsonl` records command
status, and screenshots/current-activity dumps preserve cold, warm and resumed
checkpoints. `result.json` fails closed on missing state, timeouts or interrupted
runs. Passing evidence is limited to the explicit debug APK digest and bounded
offline observation intervals. It is **not** release/account sharing, process-
death restoration, physical-device acceptance or full platform parity.
