# Android native bridge

`libjcode_mobile_android.so` exposes the static Java JNI method
`org.jcode.mobile.NativeBridge.request(String): String` using JNI 0.21.
The process owns one shared bridge behind a mutex. Call it from a worker
executor, not the Android main thread. Requests return promptly while the shared
core connects and receives events on its own connection thread. Poll at roughly
250 ms while visible. Each response contains a complete current snapshot, not an
event delta. `disconnect` releases the connection and clears the transcript.

## Request and response contract

Requests are JSON objects with an `action`:

| Action | Additional fields | Behavior |
| --- | --- | --- |
| `status`, `poll` | none | Drain available core events and return snapshot |
| `connect` | `endpoint`, optional `session_id`, optional empty `token` | Start SDK handshake, create a session or resume the supplied ID |
| `sessions` | none | Queue refresh of server session list |
| `select_session` | `session_id` | Stop previous connection and attach exactly this session |
| `send` | `text` | Send to a connected idle session, optimistically add user entry |
| `disconnect` | none | Stop connection and clear local snapshot |
| `demo` | none | Clearly labeled offline demo, no server/network |
| `start_login` | `email` | Asynchronously ask Jcode Cloud to email a six-digit code |
| `verify_login` | `code` | Asynchronously exchange the emailed code for an in-memory account |
| `connect_cloud` | optional `session_id` | Connect through the shared core's authenticated, host-key-pinned native SSH |
| `logout` | none | Stop connection, discard account/pending login, clear transcript |
| `wake` | none | Ask shared connection to probe liveness/retry immediately |
| `cancel` | none | Cancel generation in the connected session |
| `set_model` | `model` | Ask the shared SDK to change model, updated by server events |

Normal responses have this shape:

```json
{
  "ok": true,
  "error": null,
  "status": "connected",
  "offline_demo": false,
  "login_state": "signed_out",
  "login_busy": false,
  "login_error": null,
  "selected_session_id": "session-id",
  "session_title": "Session title",
  "model": "model-name",
  "models": ["model-name", "another-model"],
  "is_processing": false,
  "sessions": [
    {"id":"session-id", "session_id":"session-id", "title":"Session title", "status":"idle"}
  ],
  "messages": [
    {"id":1, "role":"assistant", "text":"Hello", "reasoning":"", "streaming":false, "queued":false, "tools":[]}
  ]
}
```

Status values: `disconnected`, `connecting`, `provisioning`, `connected`,
`reconnecting`, `failed`, `demo`. Nullable strings are JSON null when absent.
`ok` describes acceptance of the current action. An asynchronous connection or
server failure can be reflected by `status`/`error` even when a poll is accepted.
Tools contain `id`, `name`, `input`, `output`. IDs are opaque, not array offsets.
A JNI failure or caught panic returns at least `ok:false,error:string`. JVM
allocation failure may return null with a pending Java exception.

## Jcode Cloud sign-in

Use `start_login` with the person's email. While the request is running,
`login_busy` is true. Poll until it becomes false and `login_state` becomes
`code_sent`, then submit `verify_login` with the six-digit code. Poll until
`login_state` is `signed_in`, then call `connect_cloud`. The shared core handles
host provisioning, pinned native SSH, SDK negotiation and session reduction.
There is no custom cloud/auth protocol in this adapter. Failures return a
sanitized `login_error`, never raw HTTP replies or credentials.

The login token and `Account` live only in Rust process memory and are never
included in JSON, logs, preferences or `AccountStore`. App process death requires
sign-in again. `disconnect` preserves the current in-memory sign-in, whereas
`logout` forgets it and invalidates pending auth results. Logout is local, not
server-side key revocation. The existing shared cloud core may cache transcripts
and pinned SSH device state in its app-data location. Android packaging should
set a private app-data HOME/XDG location before cloud connection.

## Real server connectivity

The existing SDK speaks the **harness API**, not the historical mobile gateway
WebSocket protocol. This bridge reuses `jcode-mobile-core::connection::Connection`
and `SessionState` directly. It does not implement another protocol.

Supported endpoints:

* `tcp://host:port`: a byte-for-byte TCP forward of the harness API socket.
* `unix:///absolute/path`: native harness socket, useful for host integration.

The TCP connection is **unencrypted and unauthenticated**. Use only a trusted
loopback forward, emulator host route, or a secure tunnel. Never expose it on a
public network. Nonempty `token` is explicitly rejected rather than silently
ignored: gateway bearer-token authentication is not a supported SDK transport.
Credential-bearing URLs, query strings, fragments, unsupported schemes and
missing/zero TCP ports are rejected without echoing the input. The bridge does
not persist credentials or endpoints and does not log request bodies.

For emulator testing, forward a running harness API Unix socket onto the host
loopback interface, for example with `socat`:

```sh
socat TCP-LISTEN:7644,bind=127.0.0.1,reuseaddr,fork UNIX-CONNECT:/absolute/path/to/harness-api.sock
adb reverse tcp:7644 tcp:7644
```

Connect Android to `tcp://127.0.0.1:7644` and leave token blank. Substitute the
actual socket path from your running harness API bridge. Do not forward the old
`/ws` gateway, daemon internal protocol socket, or HTTP control-plane port.
`adb reverse` must remain active for the emulator/device session.

## Emulator transport fixture

For deterministic tests without an account or live model:

```sh
python3 crates/jcode-mobile-android/scripts/harness_fixture.py --port 7644
adb reverse tcp:7644 tcp:7644
```

Connect to `tcp://127.0.0.1:7644` with an empty token. This is a **local SDK
protocol test fixture, not a live model**. The session title and model metadata
identify the fixture. Stable instrumentation data: session IDs `s1` and `s2`,
initial assistant history `earlier`, completed streamed answer `actual streamed reply`.
It supports SDK hello, sessions, attach/create, history, model/runtime metadata,
ping/wake, prompts, streamed text, cancellation, model switching and turn completion.
Models are `fixture-not-a-live-model` and `fixture-alternate`. Two named sessions allow
selection testing. It retains history in memory across reconnects. No request
payloads are logged and its default listener is host loopback only.

`--port 0` selects an ephemeral port and prints one JSON readiness line with the
chosen `port`. `--delay 0.03` controls seconds between streamed text chunks.
Python 3 standard library is the only dependency. The Rust test launches the
script and uses the real SDK and shared reducer against it, not a second Python
client that could share the same protocol mistake.

## Validation

```sh
justrust check -p jcode-mobile-android
justrust test --lib -p jcode-mobile-android
```

Tests exercise real TCP sockets with typed SDK harness frames and the Python fixture: handshake,
session creation/list/history, prompt delivery, streamed response reduction,
selection of a second session, and disconnect. Other tests cover malformed
requests, endpoint validation, token rejection, explicitly offline demo, actual
SDK HTTP email auth against a local fake control plane, credential non-disclosure,
and logout invalidation of pending results.
They do not claim a live production model response or Android packaging coverage.
