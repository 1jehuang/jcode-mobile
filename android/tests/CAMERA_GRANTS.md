# Camera grant security instrumentation

Run `scripts/android-camera-grant-test.sh` to build and verify the signed test APK only. No device command runs by default. The coordinator exclusively owns `--run emulator-SERIAL` device execution.

Device-free checks:

```sh
scripts/android-camera-grant-test.sh --self-test
scripts/android-camera-grant-test.sh --check-log /path/to/instrumentation.txt
```

The parser requires exactly one `INSTRUMENTATION_RESULT: result=PASS` and a final `INSTRUMENTATION_CODE: -1`. It rejects failure markers anywhere, a FAIL result, canceled code 0, missing records, duplicate sessions, or trailing output. Earlier partial PASS fields are not success. The parser is a standalone command under `set -e`, not an `&&` list whose failure can be hidden by a later success message.

All ResultReceiver instances sent across processes are reparceled through the framework `ResultReceiver.CREATOR`. Anonymous receivers belong to the instrumentation APK classloader and cannot safely appear as the parcelable class in a Bundle read by the production target classloader. The plain framework parcelable retains the callback Binder endpoint without requiring the recipient to load the anonymous test class. This applies to both owner replies and the external Activity control receiver.

The test keeps a distinct-UID Activity alive while production cleanup revokes URI access. It checks exact URI JPEG read/write, sibling/traversal denial, production decoding and cleanup, and revoked read/write denial. It does not alter the production provider's non-exported, grant-only configuration. Build-only success is not a live grant security acceptance result or a camera UI acceptance claim.
