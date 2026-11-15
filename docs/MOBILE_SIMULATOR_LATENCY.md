# Full native mobile simulator latency

## Scope

This harness runs the unmodified release `jcode-mobile` application on its own
390 x 844 Xvfb display. It exercises account loading, actual control-plane HTTP,
pinned russh SSH authentication, the real SDK wire protocol, session creation,
history transfer, reducer application and native GPUI rendering. It does not
replace the app's connection state with a synthetic success.

The server is `crates/jcode-mobile-core/examples/mobile_latency_fixture.rs`.
The driver is `scripts/mobile_simulator_latency.py`. Its ten offline safety
tests are in `scripts/mobile_simulator_latency_test.py`.

Each scenario measures three real app paths:

1. **Cold:** new process with a fresh isolated account, data and cache directory.
2. **Warm tab:** an actual pointer click on the app's + button, reusing the
   authenticated process-local SSH master. The preceding screenshot/OCR check
   gives background spare-channel preparation time to complete. This is not an
   immediate zero-idle-time tab-switch benchmark.
3. **Relaunch:** stop the app and launch a new process with only that scenario's
   own persisted state. Device-key enrollment is handled over the actual SSH
   inspection channel, validated and simulated in memory by the fixture.
   The driver does not fabricate an authorized direct-reconnect cache.

The fixture binds only IPv4 loopback, uses a fake account, validates the exact
built-in enrollment command without executing a shell, and retains keys only
in memory. App XDG data/config/cache/state are isolated. Inherited real-login,
autosend and proxy environment hooks are removed. No real account, production
wake, inference, deployment or production mutation is needed.

## Acceptance and clocks

A completed sample requires all of session-ready, fresh network history,
UI history application, and a transcript-render CPU marker. A cached paint
preceding fresh history does not qualify. The screenshot must also show the
correct newly created `SIMULATOR FRESH fixture-N` session suffix.

- `duration_ms` is the app's own stage duration, often measured from connection
  start. These overlapping spans must not be added together.
- `observed_ms` is the driver's monotonic time from process launch or the actual
  warm-tab input initiation. This includes process startup and scheduling.
- `frame_capture_upper_bound_ms` is the time a screenshot containing that
  session's fresh pixels was captured. It includes capture overhead and is an
  upper bound, not an exact GPU presentation fence.
- `ocr_completed_ms` additionally includes offline OCR work. It is not app
  latency. OCR helper CPU is bounded to one OpenMP thread. A short immutable
  frame burst is captured before OCR, so recognizing an initial pre-presentation
  frame does not postpone the timestamp of a subsequent already-captured frame.

Pillow/XCB captures the private display directly when available. ImageMagick is
a fallback, identified in the result manifest. Earlier ImageMagick trials had
substantial process/capture overhead under host contention. Changing screenshot
helpers is instrumentation improvement, not a product speedup.

The driver records SHA-256 identities and rejects a campaign if either
executable changes. Each result includes injected server parameters. The native
renderer and local scheduler are real, but the server costs are controlled.
This is the Jcode native mobile preview, not Apple's CoreSimulator.
This is not a measurement of cellular RTT, packet loss, iPhone GPU performance,
AWS boot time, or a deployed backend's actual readiness.

## Reproduction

From the repository root, with Xvfb, xdotool, tesseract and either Pillow with
XCB support or ImageMagick installed:

```sh
justrust check --remote -p jcode-mobile-core --example mobile_latency_fixture
justrust build --remote -p jcode-mobile-core --example mobile_latency_fixture
justrust build --remote --release -p jcode-mobile --bin jcode-mobile
python3 -B -W error scripts/mobile_simulator_latency_test.py
python3 -B scripts/mobile_simulator_latency.py \
  --scenario all \
  --output "$HOME/.jcode/scratch/mobile-latency-new-run"
```

The output directory must not already exist. It contains timing JSON, isolated
app logs and cold/warm/relaunch screenshots. Only the processes created by the
driver are terminated. `suspended-peer` sends SIGSTOP to its own fixture for
350 ms while retaining the actual TCP sockets, then resumes it.

Fixture self-tests additionally cover actual create/resume requests, history,
models, runtime metadata, todo, session listing, ping, persisted direct reconnect
without another control-plane poll, and independent delayed SSH API channels.
The accepted socket has TCP_NODELAY set. Server-side delayed API replies run
independently across channels with bounded outstanding work, as distinct remote
API processes would. Initial trials that serialized unrelated delayed channel
replies were corrected and are not evidence of a production optimization.

## Artifact provenance

- Native release app: hosted build `20261115-051237986-1520353`.
- App SHA-256: `a51ca3c1fea5beb5c3647accb70c7c4c93129a45068e96d7aa88fe2369dd7e9b`.
- Final fixture: commit `a8d07c8`, hosted build
  `20261115-053503480-1764259`.
- Fixture SHA-256: `83308c17b938d2fe218965ea54d1266190da8a0cafdc6e86dbdd893ec7525a66`.
- Prior integrated product optimizations and their causal core tests are listed
  in `MOBILE_SUBSECOND_READINESS.md`. This round adds representative full-app
  acceptance, not a new custom production protocol or backend deployment.

## Results

Final immutable campaign: `2026-11-15`, local evidence directory
`~/.jcode/scratch/mobile-sim-latency-20261115/final-burst/`.
All **27 opens** passed ready, fresh network history, render ordering and
session-specific screenshot verification. All **10 driver safety tests** passed.

Times below are rounded milliseconds. Each cell is **cold / warm tab / relaunch**.
CPU denotes the observed fresh transcript CPU render marker, not presentation.
Pixels denote the independently verified screenshot capture upper bound.

| Injected scenario | CPU render (ms) | Fresh pixels upper bound (ms) |
| --- | ---: | ---: |
| Awake, small history | 305 / 83 / 150 | 344 / 121 / 182 |
| Warm peer suspended for 350 ms | 368 / 438 / 152 | 402 / 473 / 184 |
| Control-plane response 1500 ms | 1724 / 59 / 176 | 1756 / 122 / 213 |
| SSH handshake 1200 ms | 1474 / 69 / 1317 | 1506 / 121 / 1349 |
| SSH authentication 1200 ms | 1452 / 67 / 1308 | 1484 / 121 / 1346 |
| API hello 1200 ms | 1521 / 61 / 1395 | 1556 / 122 / 1436 |
| Fresh history response 1500 ms | 1775 / 1517 / 1592 | 1808 / 1550 / 1624 |
| 250 messages, 4096 bytes each | 542 / 121 / 195 | 1520 / 1703 / 1075 |
| Three not-ready boot polls | 1216 / 70 / 186 | 1252 / 125 / 223 |

The awake scenario additionally injects 40 ms control-plane, 20 ms auth,
20 ms hello, 20 ms attach and 40 ms history costs. Full scenario arguments are
recorded in `results.json` and the driver's `SCENARIOS` map.

These results demonstrate the existing optimized real app pipeline, not a
before/after product speedup from this harness. Warm tabs avoid repeated
handshake, authentication and control-plane work. A process relaunch can use
its enrolled direct endpoint without repeating control-plane discovery, but
still pays deliberately delayed new SSH/authentication/hello work. A server
that withholds fresh history for 1500 ms cannot supply that history in 1 second.

**Remaining boundary:** the large-history visible-frame upper bounds exceed
1 second even though CPU markers are much earlier. Frame inspection confirms
actual newly created session suffixes, not cached text. Relaunch's first two
captured frames still show the empty-state UI before its third fresh frame.
The current short capture bursts and intervening offline OCR leave observation
gaps, so these data do not isolate GPU/layout cost from capture scheduling.
Do not report the CPU marker as visible latency or assert this case is subsecond.
A bounded follow-up captured 60 frames at at least 25 ms intervals **before any
OCR**, then independently verified the earliest frame matching the final bottom
transcript crop. Its large-history pixel bounds were 2034 / 1094 / 2073 ms.
Repeating with existing CPU profiling enabled gave 1661 / 1538 / 1302 ms.
These independent observations retain a substantial presentation-side gap, so
OCR alone does not explain it. Evidence directories are `continuous-large/` and
`profile-large/` beside the final campaign. Continuous capture itself adds load,
and the final-crop match is still an upper bound, not a presentation fence.

Existing CPU profiling in the latter run measured maximum `transcript.build`
3.088 ms (cold), 1.672 ms (warm window) and 2.956 ms (relaunch), despite all
250 entries being built on their initial draw. Thus rewriting the element-build
loop is not justified as a fix for the observed second-long gap. Layout/text
shaping, native presentation and this host's graphics scheduling remain
unseparated. The next causal measurement needs layout/presentation profiling;
no speculative geometry approximation or loss of earlier history was introduced.

Host load varied substantially during earlier campaigns, including load averages
above 60. Some earlier awake opens exceeded 1 second. The final table is a
reproducible representative campaign, not a p95 guarantee or evidence that
all cold opens are always subsecond. It does not establish that the original
21.63-second real-device wait has become 344 ms on that device.

Final hosted core regression: **164 passed, 0 failed, 4 ignored**, receipt
`20261115-054449467-1864172`, explicitly verified running on hosted.
No production protocol, authentication budget, pinning policy or backend was
changed by this measurement round.
