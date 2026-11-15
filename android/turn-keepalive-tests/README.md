# Bounded Android turn keepalive

## Design and official rules consulted (2026-11-15)

This is a deliberately limited Android equivalent of `notify::keep_alive(busy)`, not background push delivery. It assists the existing shared GPUI process while an actual aggregate turn is processing. It creates no session, account, token, subscription, network request, wake lock, alarm, job, boot receiver, sticky restart or Java application UI.

Official Android primary references consulted before design:

- API26 foreground-service launch and five-second promotion requirement: https://developer.android.com/about/versions/oreo/background
- Launch/promotion, notification importance and API34 type/permission checks: https://developer.android.com/develop/background-work/services/fgs/launch
- API31+ background-start restrictions and exemptions: https://developer.android.com/develop/background-work/services/fgs/restrictions-bg-start
- API34+ `shortService`, approximately three-minute OS timeout, no sticky support, no type-specific permission, no other foreground-service starts, callback/ANR rules and extension restrictions: https://developer.android.com/develop/background-work/services/fgs/service-types#short-service
- API33 notification permission, including the fact Android itself permits FGS launch without that grant: https://developer.android.com/develop/ui/views/notifications/notification-permission
- API35 dataSync/mediaProcessing six-hour limits: https://developer.android.com/develop/background-work/services/fgs/timeout
- API36 foreground-service execution does not exempt concurrent jobs from quota: https://developer.android.com/about/versions/16/behavior-changes-all

Selection: `shortService` only on API34..36, a normal foreground service on API26..33, with the same **150,000ms nonrenewable application lease on every version**. `FOREGROUND_SERVICE` is the only added permission. No misleading `remoteMessaging`, `dataSync`, microphone, special-use or system-exempt type is claimed. Short-service applicability is limited to briefly completing already-running, nondeferrable interactive turn work. This is not permission for arbitrary long-running agents. Store policy approval is not claimed.

Android busy is recomputed from authenticated, non-offline active and parked SessionState.is_processing only. A genuine user send updates it immediately, both active and parked connection batches update it, synced transcript processing changes update it, and tab close/conversation clear recompute it. Account-wide discovery/agent rows never manufacture processing state. Shared account-workspace discard and explicit signout stop Android runtime before clearing the workspace. The iOS keepalive behavior remains unchanged.

The main-thread pure `TurnLease` policy admits only a false-to-true busy edge observed while the Activity is resumed and notification permission/channel is usable. AndroidServices rechecks its real lifecycle flag on the UI thread. It never starts on pause, resume, a timer, a notification receiver, boot or an existing foreground-service exemption. The OS remains the final authority and a denied/illegal start is caught and consumes that edge. Even when Android would legally allow a service without POST_NOTIFICATIONS, this app intentionally fails closed. Granting permission or resuming does not retry the same busy turn. Repeated stream updates cannot extend the deadline. Another turn can be admitted only after genuine aggregate idle, then a new busy edge while resumed.

A queued service start carries only an ephemeral generation number. The service rejects stale, canceled, expired and null/redelivered starts, promotes immediately inside onStartCommand and returns START_NOT_STICKY. Idle stops it even in background. Signout uses the shared false update plus account-removal cancellation. Activity destruction, task removal, either OS timeout callback and service destruction retire the lease. A canceled/expired busy edge remains consumed until idle. Generation checks keep a stale destroy callback from invalidating a newer lease.

The 150s timer is main-loop scheduled, not a hard real-time guarantee: OS suspension, deep sleep or a stalled UI thread can delay callback delivery, particularly on API26..33, which have no shortService OS timeout. All admission and validity checks use elapsedRealtime, streaming updates retire an expired lease rather than renewing it, and Activity resume checks retire elapsed/denied leases without starting or renewing anything. No wake lock/exact alarm/battery-optimization exemption is introduced to promise wall-clock wakeups. API34+ also has its independent shortService timeout and both timeout overloads immediately stop. Neither the service nor a notification guarantees the socket survives Doze, network loss, user stop, process death or vendor power management. FCM and killed-process delivery remain unavailable.

The ongoing low-importance local-only notification contains generic fixed text, no session/account/prompt/title content, no action or PendingIntent and a generic public version. It uses dedicated channel `turn-keepalive` and untagged ID 4304. It does not cancel agent/turn tags, change their channels, or call cancelAll. Audio, camera, sharing and accessibility paths are unchanged.

## Offline validation

```sh
python3 scripts/android-turn-keepalive-test.py
bash scripts/android-turn-keepalive-test.sh
```

The host suite executes the actual production pure policy using javac/java, including 10,000 sequential leases, exact expiry boundary, repeated busy, background/denied edge consumption, no resume retry, stale tokens, idle-before-queued-start, failed-start retirement and cancellation. Source/manifest contracts cover the narrow JNI route, lifecycle calls, notification privacy, permissions/type and isolation. These source contracts are not runtime proof.

The SDK script compiles **all production Java**, links the production resources/manifest, then compiles the unchanged production TurnLease and TurnKeepAliveService sources together with isolated instrumentation fixtures into a separate signed APK. It records source SHA256 values. Default execution is build-only, no adb, installation, emulator, account, authentication, network or publishing operation. The test-only Activity is not part of production UI. Production remains shared GPUI.

## Coordinator-only isolated OS acceptance

```sh
bash scripts/android-turn-keepalive-test.sh --run emulator-5556
```

The runner requires an emulator API33+ and an installed production package solely to assert a different UID. It never starts that app or loads native GPUI, nativeInit, account storage, authenticated turn state or sockets. The test package has no INTERNET permission. The service and policy are exact production source compiled into that isolated UID, not the installed production APK's Dex. Therefore this validates actual OS service/timer/notification behavior of those sources, **not** the Rust JNI bridge, installed release artifact, genuine-turn wiring, authenticated signout or full GPUI lifecycle. Background/denial policy calls use synthetic boolean inputs in the test UID only. No production session is fabricated.

The runner refuses a pre-existing test notification/channel namespace. It records original test notification permission/enabled state, revokes only TEST permission for a denied phase, grants only TEST permission for granted acceptance, then restores and probes the original state. It never changes production permissions, app ops, channel settings, battery optimizations or OS timeout parameters. Missing prerequisite, denial, crash, early timer expiry, delayed timer beyond the strict awake-emulator window, missing record, cleanup error or restoration mismatch fails, never a passing skip.

Granted assertions inspect a real FLAG_FOREGROUND_SERVICE record, ongoing/local-only/private/generic-public flags, no intents/actions, logical background admission denial and consumed-edge behavior, repeated busy without extension, survival after Activity pause, the **actual unshortened 150s timer**, no expired-background retry, new foreground turn, idle stop, cancellation/no-rearm and fixture Activity-destroy teardown. An unrelated isolated sentinel must survive every teardown. Actual Android background-start exception delivery, blocked-channel behavior, OS-forced shortService timeout callbacks, process-kill/no-restart, task removal, screen-off deep sleep, API26..32 runtime and OEM behavior are not covered by this harness. They remain explicit separate acceptance boundaries, not inferred from source assertions.

Every instrumentation phase must satisfy the existing strict unique terminal-result parser. Instrumentation finally cancels only the isolated service/sentinel and removes its owned test channels. The runner independently repeats cleanup and restores permission/enabled state in its EXIT trap. SIGINT/SIGTERM run that trap. SIGKILL/device loss cannot guarantee cleanup or produce a successful terminal runner result. Existing root font-scale APK/AAB evidence remains earlier-source evidence and is not relabeled as covering this change. Rebuild production after the scoped handoff/freeze before making current-artifact claims.
