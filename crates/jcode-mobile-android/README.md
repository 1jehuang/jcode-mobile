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

Normal responses have this shape:

```json
{
  "ok": true,
  "error": null,
  "status": "connected",
  "offline_demo": false,
  "selected_session_id": "session-id",
  "session_title": "Session title",
  "model": "model-name",
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

## Validation

```sh
justrust check -p jcode-mobile-android
justrust test --lib -p jcode-mobile-android
```

Tests exercise a real TCP socket with typed SDK harness frames: handshake,
session creation/list/history, prompt delivery, streamed response reduction,
selection of a second session, and disconnect. Other tests cover malformed
requests, endpoint validation, token rejection, and explicitly offline demo.
They do not claim a live production model response or Android packaging coverage.
