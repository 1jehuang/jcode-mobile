# Android incoming plain-text share contract

`IncomingShare.java` is a platform adapter, not a second UI or a new network
workflow. It accepts an explicit `android.intent.action.SEND` Intent with exact
MIME `text/plain` and a plain Java String `Intent.EXTRA_TEXT`. It returns
`jcode://new?text=<UTF-8 form encoded text>&draft=1` for the existing shared URL
router. The shared router must implement **draft-only** `draft=1` semantics
before wiring this helper. The older `jcode://new?text=...` route auto-sends
when connected, so silently dropping `draft=1` is unsafe and is not supported.

## Accepted bounded payload

- Nonempty text containing something other than whitespace, at most 4096 UTF-8
  bytes and at most 4096 UTF-16 code units before encoding.
- Valid UTF-16, including surrogate pairs for ordinary emoji.
- LF and tab are allowed. Other C0 controls, C1/DEL, carriage returns, bidi
  control/override/isolate characters, U+FFFE and U+FFFF are rejected.
- A single redundant plain-text ClipData item is accepted only if its String
  exactly equals EXTRA_TEXT and it contains no URI, embedded Intent, or HTML.
- Input query delimiters, percent signs, literal plus signs, and hashes are
  encoded as data. They cannot introduce a second `draft` or other URL option.

Wrong/missing MIME, missing/wrong-type/styled text, URI data, EXTRA_STREAM,
URI grant flags, multiple/HTML/URI/embedded-Intent clips, malformed parcel data,
unsafe controls, and oversize input are rejected. No content resolver, file
access, attachment access, permission request, auth action, or model send is
performed. Android still applies its own Binder payload limits before delivery.

## Exactly-once consumption and lifecycle integration

Call `IncomingShare.consume(intent)` on the UI thread in the existing native
resume adapter. It returns null for non-SEND Intents without modifying them.
For every SEND, accepted or rejected, it changes the action to an app-local
consumed marker **before accessing extras**, then clears all extras, ClipData,
data/MIME, and URI-grant flags in a finally block. Reusing that Intent after
resume or Activity recreation therefore cannot replay the share. A distinct
new delivery with the same text remains a separate intentional share.

The integration owner should use the returned URL with the same `nativeResume`
path as existing deep links, with no fallback to a rejected SEND's incoming URL.
The ordinary native resume lifecycle still runs for rejected or absent shares.
Do not create another Activity, UI widget, global text queue, or auth bypass.
Add only a `SEND` + `DEFAULT` + `text/plain` manifest filter when the shared
router's draft-only behavior and signed-out common handling are ready. There
is no SEND_MULTIPLE filter, attachment support, or new Android permission.

Signed-out handling must stay with the existing shared deep-link account gate.
This adapter does not retain text for later login or create its own account/UI
state. Signed-in handling must prefill a draft, clear any inherited auto-send,
and require a user's explicit send. Plain/Ask deep-link semantics should remain
unchanged. This helper's isolated tests do not establish those shared-router
requirements, which need their own integration tests and root-owned real Intent
acceptance before claiming the feature is wired.

## Validation performed without devices

```sh
python3 scripts/android-incoming-share-test.py
python3 -m py_compile scripts/android-incoming-share-test.py
```

The runner first compiles the production helper against the actual SDK36
`android.jar` with SDK-local JDK21 `javac --release 8`. It then compiles the same
helper with minimal host Intent/ClipData storage fixtures and runs focused Java
assertions covering bounds, UTF-8 round-trip/query injection, malformed UTF-16,
controls, MIME/parcel/type rejection, no URI/attachment grants, exact-once
consumption, and plain ClipData. Host fixtures are synthetic, not Android API
implementations or device evidence. No adb command is run. The existing build
script includes the helper automatically because it compiles all Java sources.

The helper lane does not edit JcodeActivity, AndroidServices, or the manifest.
Root and the Java integration owner coordinate those small changes and own all
device operations. A built helper alone is not proof of share-sheet discovery,
cold/warm lifecycle delivery, signed-out behavior, or unsent composer behavior.
