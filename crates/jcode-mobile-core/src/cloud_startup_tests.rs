//! Offline causal fixtures for the production readiness scheduler. No cloud,
//! account storage, DNS, sockets, or production SSH are contacted.
use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
    mpsc,
};

#[test]
fn connection_lock_cancellation_does_not_wait_for_or_interrupt_owner() {
    let mutex = Arc::new(Mutex::new(7usize));
    let owner = mutex.lock().unwrap();
    let (attempted, observed) = mpsc::channel();
    let (completed, result) = mpsc::channel();
    let worker_mutex = mutex.clone();
    let worker = std::thread::spawn(move || {
        let result = lock_with_cancellation(&worker_mutex, &|| {
            attempted.send(()).unwrap();
            true
        });
        completed.send(result.err().map(|error| error.kind)).unwrap();
    });
    observed.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(result.recv_timeout(Duration::from_secs(5)).unwrap(), Some(jcode_sdk::ErrorKind::Cancelled));
    assert_eq!(*owner, 7, "cancelled waiter changed the active opener");
    assert!(mutex.try_lock().is_err(), "owner must still hold provisioning lock");
    drop(owner);
    worker.join().unwrap();
}

#[test]
fn connection_lock_waiter_observes_cancellation_after_contention() {
    let mutex = Arc::new(Mutex::new(()));
    let owner = mutex.lock().unwrap();
    let (attempted, observed) = mpsc::channel();
    let (completed, result) = mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let worker_cancel = cancel.clone();
    let worker_mutex = mutex.clone();
    let worker = std::thread::spawn(move || {
        let checks = AtomicUsize::new(0);
        let result = lock_with_cancellation(&worker_mutex, &|| {
            if checks.fetch_add(1, Ordering::Relaxed) == 0 {
                attempted.send(()).unwrap();
                return false;
            }
            worker_cancel.load(Ordering::Acquire)
        });
        completed.send(result.err().map(|error| error.kind)).unwrap();
    });
    // The held owner is the causal barrier. Cancellation must complete before
    // releasing it, regardless of how long real provisioning takes.
    observed.recv_timeout(Duration::from_secs(5)).unwrap();
    cancel.store(true, Ordering::Release);
    assert_eq!(result.recv_timeout(Duration::from_secs(5)).unwrap(), Some(jcode_sdk::ErrorKind::Cancelled));
    drop(owner);
    worker.join().unwrap();
    assert!(lock_with_cancellation(&mutex, &|| false).is_ok());
}

fn busy() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(false))
}

#[test]
fn warm_cache_reuse_renews_idle_deadline_not_authentication() {
    let account = Account { given_name: None, name: None, api_key: "fixture".into(), email: None, api_base: "https://fixture.invalid".into() };
    let now = Instant::now();
    let identity = Arc::new(());
    let master = Arc::new(());
    let mut cache = Some(WarmTarget { account: account.clone(), ssh: master.clone(), expires: now + TARGET_TTL, identity: identity.clone() });
    for minute in [4, 8, 12] {
        let at = now + Duration::from_secs(minute * 60);
        let reused = cache.as_mut().unwrap().for_account(&account, at).expect("active master was expired by creation age");
        assert!(Arc::ptr_eq(&master, &reused), "reuse replaced the authenticated master");
        assert_eq!(expire_idle_master(&mut cache, &identity, at), Some(at + TARGET_TTL));
    }
    let idle_end = now + Duration::from_secs(17 * 60);
    assert!(cache.as_mut().unwrap().for_account(&account, idle_end).is_none(), "expired cache must not revive");
    assert_eq!(expire_idle_master(&mut cache, &identity, idle_end), None);
    assert!(cache.is_none());
}

#[test]
fn warm_cache_wrong_account_and_old_timer_cannot_extend_or_evict_replacement() {
    let account = Account { given_name: None, name: None, api_key: "fixture".into(), email: None, api_base: "https://fixture.invalid".into() };
    let now = Instant::now();
    let identity = Arc::new(());
    let mut cache = Some(WarmTarget { account: account.clone(), ssh: 7, expires: now + TARGET_TTL, identity: identity.clone() });
    for other in [Account { api_key: "different".into(), ..account.clone() }, Account { api_base: "https://other.invalid".into(), ..account.clone() }] {
        assert_eq!(cache.as_mut().unwrap().for_account(&other, now + Duration::from_secs(240)), None);
        assert_eq!(cache.as_ref().unwrap().expires, now + TARGET_TTL);
    }
    let retired_identity = Arc::new(());
    assert_eq!(expire_idle_master(&mut cache, &retired_identity, now + TARGET_TTL), None);
    assert!(cache.is_some(), "old generation evicted its replacement");
    cache = None; // sign-out: a late timer must not restore anything.
    assert_eq!(expire_idle_master(&mut cache, &identity, now + TARGET_TTL), None);
    assert!(cache.is_none());
}

#[test]
fn authenticated_api_open_releases_provisioning_before_blocked_successful_hello() {
    let mutex = Arc::new(Mutex::new(0usize));
    let worker_mutex = mutex.clone();
    let (started, opening) = mpsc::channel();
    let (release, gate) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        let mut guard = worker_mutex.lock().unwrap();
        *guard = 7; // authenticated direct master is published before unlock
        open_after_provisioning_unlock(guard, || {
            started.send(()).unwrap();
            gate.recv_timeout(Duration::from_secs(5)).unwrap();
            42
        })
    });
    opening.recv_timeout(Duration::from_secs(5)).unwrap();
    // A simulated API hello is still blocked. Other callers can acquire the
    // real production handoff's lock and recheck the authenticated cache now.
    let mut guard = mutex.try_lock().expect("API hello retained provisioning lock");
    assert_eq!(*guard, 7, "authenticated master must be visible before hello");
    *guard = 1;
    drop(guard);
    release.send(()).unwrap();
    assert_eq!(worker.join().unwrap(), 42);
    assert_eq!(*mutex.lock().unwrap(), 1);
}

#[test]
fn direct_hello_failure_reacquires_before_rechecking_replacement_or_provisioning() {
    for replacement in [None, Some(2usize)] {
        let provisioning = Arc::new(Mutex::new(()));
        let cache = Arc::new(Mutex::new(None));
        let checks = Arc::new(AtomicUsize::new(0));
        let (hello_started, hello_observed) = mpsc::channel();
        let (fail_hello, hello_gate) = mpsc::channel();
        let (reacquiring, reacquire_observed) = mpsc::channel();
        let (completed, result) = mpsc::channel();
        let worker_provisioning = provisioning.clone();
        let worker_cache = cache.clone();
        let worker_checks = checks.clone();
        let worker = std::thread::spawn(move || {
            let guard = worker_provisioning.lock().unwrap();
            *worker_cache.lock().unwrap() = Some(1); // publish authenticated direct master
            let hello: Result<(), ()> = open_after_provisioning_unlock(guard, || {
                hello_started.send(()).unwrap();
                hello_gate.recv_timeout(Duration::from_secs(5)).unwrap();
                Err(())
            });
            assert!(hello.is_err());
            {
                let mut cache = worker_cache.lock().unwrap();
                if *cache == Some(1) {
                    *cache = None;
                }
            }
            let outcome = acquire_provisioning(
                || lock_with_cancellation(&worker_provisioning, &|| {
                    reacquiring.send(()).unwrap();
                    false
                }),
                |_| {
                    worker_checks.fetch_add(1, Ordering::SeqCst);
                    *worker_cache.lock().unwrap()
                },
                |master| {
                    assert!(worker_provisioning.try_lock().is_ok(), "replacement hello kept provisioning lock");
                    Ok(Some(master))
                },
            ).unwrap();
            let value = match outcome {
                Provisioning::Client(client) => Some(client),
                Provisioning::Guard(guard) => {
                    assert!(worker_provisioning.try_lock().is_err(), "fallback provisioning is not serialized");
                    assert!(worker_cache.lock().unwrap().is_none());
                    drop(guard);
                    None
                }
            };
            completed.send(value).unwrap();
        });
        hello_observed.recv_timeout(Duration::from_secs(5)).unwrap();
        let guard = provisioning.try_lock().expect("remembered-direct hello retained cold lock");
        // The other opener owns provisioning through the failed hello. No
        // replacement check or fallback may run before this owner publishes.
        fail_hello.send(()).unwrap();
        reacquire_observed.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(checks.load(Ordering::SeqCst), 0);
        assert!(matches!(result.try_recv(), Err(mpsc::TryRecvError::Empty)));
        *cache.lock().unwrap() = replacement;
        drop(guard);
        assert_eq!(result.recv_timeout(Duration::from_secs(5)).unwrap(), replacement);
        worker.join().unwrap();
        assert_eq!(checks.load(Ordering::SeqCst), 1, "fallback repeated direct authentication or missed shared cache recheck");
    }
}

#[test]
fn provisioning_rechecks_again_after_failed_replacement_hello() {
    let provisioning = Mutex::new(());
    let cache = Mutex::new(Some(1usize));
    let attempts = AtomicUsize::new(0);
    let outcome = acquire_provisioning(
        || lock_with_cancellation(&provisioning, &|| false),
        |_| *cache.lock().unwrap(),
        |master| {
            assert_eq!(master, 1);
            assert!(provisioning.try_lock().is_ok());
            attempts.fetch_add(1, Ordering::SeqCst);
            *cache.lock().unwrap() = None;
            Ok(None::<usize>)
        },
    ).unwrap();
    assert!(matches!(outcome, Provisioning::Guard(_)));
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
    assert!(provisioning.try_lock().is_err());
}

fn await_permit_release(permit: &AtomicBool) {
    let watchdog = Instant::now() + Duration::from_secs(10);
    while permit.load(Ordering::Acquire) {
        assert!(
            Instant::now() < watchdog,
            "worker permit leaked after fixture drain"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn reply(ready: bool) -> jcode_sdk::cloud::ConnectReply {
    jcode_sdk::cloud::ConnectReply {
        state: if ready { "ready" } else { "starting" }.into(),
        ready,
        address: Some("127.0.0.1".into()),
        port: Some(22),
        user: Some("fixture".into()),
        host_keys: vec![
            crate::cloud_ssh::CloudSshKey::generate("offline-fixture")
                .unwrap()
                .public_openssh()
                .split_whitespace()
                .take(2)
                .collect::<Vec<_>>()
                .join(" "),
        ],
        ..Default::default()
    }
}

#[test]
fn readiness_fixture_measures_serial_probe_delay_removed() {
    // Both runs use the same production loop, 250ms poll budget, 1200ms
    // failed probe budget, and readiness in the second control-plane reply.
    // Only the scheduling policy differs. Timing is diagnostic, not a gate.
    fn run(policy: ProbeScheduling) -> (Duration, usize) {
        let starting = reply(false);
        let mut ready = starting.clone();
        ready.ready = true;
        let permit = busy();
        let (finished, drained) = mpsc::channel();
        let drain_notice = Arc::new(ProbeDrained(finished));
        let start = Instant::now();
        let requests = Arc::new(AtomicUsize::new(0));
        let request_count = requests.clone();
        let result = schedule_host_readiness(
            move || {
                let requests = request_count.fetch_add(1, Ordering::AcqRel) + 1;
                Ok(if requests == 2 {
                    ready.clone()
                } else {
                    starting.clone()
                })
            },
            Some(move |_| {
                let _retain_until_worker_exit = &drain_notice;
                std::thread::sleep(Duration::from_millis(1200));
                None::<()>
            }),
            permit.clone(),
            busy(),
            Duration::from_millis(250),
            start + Duration::from_secs(10),
            &|_| {},
            &|| false,
            policy,
        )
        .unwrap();
        assert!(matches!(result, HostWait::Ready(_)));
        let elapsed = start.elapsed();
        drained.recv_timeout(Duration::from_secs(10)).unwrap();
        (elapsed, requests.load(Ordering::Acquire))
    }
    let (before, before_calls) = run(ProbeScheduling::SerialBaseline);
    let (after, after_calls) = run(ProbeScheduling::Concurrent);
    println!(
        "cloud readiness controlled baseline={:.3}ms after={:.3}ms requests={before_calls}/{after_calls}; fixture only, not device startup",
        before.as_secs_f64() * 1000.0,
        after.as_secs_f64() * 1000.0
    );
    assert_eq!((before_calls, after_calls), (2, 2));
}

/// Notify only after every copy of the probe closure has been dropped. This
/// drains the detached fixture worker, not just its callback's final statement.
struct ProbeDrained(mpsc::Sender<()>);

impl Drop for ProbeDrained {
    fn drop(&mut self) {
        let _ = self.0.send(());
    }
}

struct ActiveProbe(Arc<AtomicUsize>);

impl Drop for ActiveProbe {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Keep the speculative transport behind an explicit gate until the actual
/// production scheduler returns the requested control-plane ready response.
/// Channel timeouts are deadlock watchdogs, never latency acceptance limits.
fn gated_readiness_fixture(pending_replies: usize, poll_interval: Duration) -> Vec<Instant> {
    let permit = busy();
    let active = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let probes = Arc::new(AtomicUsize::new(0));
    let (release, gate) = mpsc::channel();
    let gate = Arc::new(Mutex::new(gate));
    let (entered, started) = mpsc::channel();
    let (finished, drained) = mpsc::channel();
    let drain_notice = Arc::new(ProbeDrained(finished));
    let (complete, completed) = mpsc::channel();
    let worker_permit = permit.clone();
    let probe_active = active.clone();
    let probe_peak = peak.clone();
    let probe_count = probes.clone();
    let starting = reply(false);
    let worker = std::thread::spawn(move || {
        let starts = Arc::new(Mutex::new(Vec::new()));
        let request_starts = starts.clone();
        let result = schedule_host_readiness(
            move || {
                let mut starts = request_starts.lock().unwrap();
                starts.push(Instant::now());
                if starts.len() == 2 {
                    // Establish that the probe is genuinely executing before
                    // the second control-plane response can be returned.
                    started.recv_timeout(Duration::from_secs(10)).unwrap();
                }
                let mut response = starting.clone();
                response.ready = starts.len() > pending_replies;
                Ok(response)
            },
            Some(move |_| {
                let _retain_until_worker_exit = &drain_notice;
                let count = probe_active.fetch_add(1, Ordering::AcqRel) + 1;
                let _active = ActiveProbe(probe_active.clone());
                probe_peak.fetch_max(count, Ordering::AcqRel);
                probe_count.fetch_add(1, Ordering::AcqRel);
                entered.send(()).unwrap();
                gate.lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(30))
                    .unwrap();
                None::<()>
            }),
            worker_permit,
            busy(),
            poll_interval,
            Instant::now() + Duration::from_secs(30),
            &|_| {},
            &|| false,
            ProbeScheduling::Concurrent,
        );
        complete
            .send((
                matches!(result, Ok(HostWait::Ready(_))),
                starts.lock().unwrap().clone(),
            ))
            .unwrap();
    });

    // A serialized regression cannot return readiness until the gate opens.
    // Always release and drain before asserting, including on watchdog expiry.
    let outcome = completed.recv_timeout(Duration::from_secs(10));
    let active_before_release = active.load(Ordering::Acquire);
    let busy_before_release = permit.load(Ordering::Acquire);
    release.send(()).unwrap();
    drained.recv_timeout(Duration::from_secs(10)).unwrap();
    worker.join().unwrap();
    let (ready, starts) = outcome.expect("readiness was blocked behind the gated probe");
    assert!(ready);
    assert_eq!(
        active_before_release, 1,
        "probe must still be gated when readiness returns"
    );
    assert!(busy_before_release);
    assert_eq!(probes.load(Ordering::Acquire), 1);
    assert_eq!(peak.load(Ordering::Acquire), 1);
    assert_eq!(active.load(Ordering::Acquire), 0);
    assert!(!permit.load(Ordering::Acquire));
    assert_eq!(starts.len(), pending_replies + 1);
    starts
}

#[test]
fn readiness_fixture_second_control_plane_ready_returns_while_probe_gated() {
    gated_readiness_fixture(1, Duration::from_millis(10));
}

#[test]
fn readiness_fixture_prolonged_not_ready_preserves_cadence_and_single_probe() {
    let interval = Duration::from_millis(20);
    let starts = gated_readiness_fixture(6, interval);
    // Lower bound only: slow or descheduled CI must not fail this regression.
    // The six not-ready responses must not cause a busy poll or extra probes.
    for pair in starts.windows(2) {
        assert!(pair[1].duration_since(pair[0]) >= interval);
    }
}

fn probe_wins_blocked_control_plane_fixture(late_error: bool) {
    let request_permit = busy();
    let worker_request_permit = request_permit.clone();
    let (release, gate) = mpsc::channel();
    let (second_started, second_request) = mpsc::channel();
    let second_request = Arc::new(Mutex::new(second_request));
    let (finished, drained) = mpsc::channel();
    let request_drain = Arc::new(ProbeDrained(finished));
    let (complete, completed) = mpsc::channel();
    let requests = Arc::new(AtomicUsize::new(0));
    let request_count = requests.clone();
    let starting = reply(false);
    let target = jcode_sdk::cloud::CloudTarget::from_reply(&starting).unwrap();
    let worker = std::thread::spawn(move || {
        let result = schedule_host_readiness(
            move || {
                let _retain_until_request_exits = &request_drain;
                let call = request_count.fetch_add(1, Ordering::AcqRel) + 1;
                if call == 1 {
                    return Ok(starting.clone());
                }
                assert_eq!(call, 2, "only one control-plane request may be in flight");
                second_started.send(()).unwrap();
                gate.recv_timeout(Duration::from_secs(30)).unwrap();
                if late_error {
                    Err(jcode_sdk::Error::new(
                        jcode_sdk::ErrorKind::Disconnected,
                        "offline late control-plane error",
                    ))
                } else {
                    let mut ready = starting.clone();
                    ready.ready = true;
                    Ok(ready)
                }
            },
            Some(move |_| {
                second_request
                    .lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(10))
                    .unwrap();
                Some(42_u32)
            }),
            busy(),
            worker_request_permit,
            Duration::from_millis(20),
            Instant::now() + Duration::from_secs(30),
            &|_| {},
            &|| false,
            ProbeScheduling::Concurrent,
        );
        complete
            .send(matches!(result, Ok(HostWait::Direct((actual, 42))) if actual == target))
            .unwrap();
    });
    // Success must be returned while the second HTTP response is still gated.
    // Release and drain even if a serialized regression hits the watchdog.
    let outcome = completed.recv_timeout(Duration::from_secs(10));
    let busy_before_release = request_permit.load(Ordering::Acquire);
    release.send(()).unwrap();
    drained.recv_timeout(Duration::from_secs(10)).unwrap();
    worker.join().unwrap();
    assert!(outcome.expect("successful SSH was blocked behind the control-plane response"));
    assert!(
        busy_before_release,
        "the losing HTTP request must still hold its permit"
    );
    await_permit_release(&request_permit);
    assert!(!request_permit.load(Ordering::Acquire));
    assert_eq!(requests.load(Ordering::Acquire), 2);
}

#[test]
fn readiness_fixture_successful_probe_beats_blocked_second_control_plane_response() {
    probe_wins_blocked_control_plane_fixture(false);
}

#[test]
fn readiness_fixture_successful_probe_discards_late_control_plane_error() {
    probe_wins_blocked_control_plane_fixture(true);
}

fn control_plane_error_waits_for_probe_fixture(probe_succeeds: bool) {
    let request_permit = busy();
    let worker_request_permit = request_permit.clone();
    let requests = Arc::new(AtomicUsize::new(0));
    let request_count = requests.clone();
    let observed_requests = requests.clone();
    let observed_permit = request_permit.clone();
    let (release, gate) = mpsc::channel();
    let gate = Arc::new(Mutex::new(gate));
    let (entered, probe_started) = mpsc::channel();
    let (observed, error_consumed) = mpsc::channel();
    let (complete, completed) = mpsc::channel();
    let (finished, drained) = mpsc::channel();
    let probe_drain = Arc::new(ProbeDrained(finished));
    let starting = reply(false);
    let worker = std::thread::spawn(move || {
        let observation_sent = AtomicBool::new(false);
        let result = schedule_host_readiness(
            move || {
                let call = request_count.fetch_add(1, Ordering::AcqRel) + 1;
                if call == 1 {
                    return Ok(starting.clone());
                }
                assert_eq!(call, 2);
                probe_started.recv_timeout(Duration::from_secs(10)).unwrap();
                Err(jcode_sdk::Error::new(
                    jcode_sdk::ErrorKind::Disconnected,
                    "offline control-plane error before probe completion",
                ))
            },
            Some(move |_| {
                let _retain_until_worker_exit = &probe_drain;
                entered.send(()).unwrap();
                gate.lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(30))
                    .unwrap();
                probe_succeeds.then_some(42_u32)
            }),
            busy(),
            worker_request_permit,
            Duration::from_millis(20),
            Instant::now() + Duration::from_secs(30),
            &|_| {},
            &|| {
                // The permit travels with the HTTP result. Its release after
                // request two proves the production loop consumed the error,
                // while the main fixture still holds the SSH completion gate.
                if observed_requests.load(Ordering::Acquire) == 2
                    && !observed_permit.load(Ordering::Acquire)
                    && !observation_sent.swap(true, Ordering::AcqRel)
                {
                    observed.send(()).unwrap();
                }
                false
            },
            ProbeScheduling::Concurrent,
        );
        let expected = if probe_succeeds {
            matches!(result, Ok(HostWait::Direct((_, 42))))
        } else {
            matches!(result, Err(error) if error.kind == jcode_sdk::ErrorKind::Disconnected && error.message == "offline control-plane error before probe completion")
        };
        complete.send(expected).unwrap();
    });
    let observed = error_consumed.recv_timeout(Duration::from_secs(10));
    release.send(()).unwrap();
    let result = completed.recv_timeout(Duration::from_secs(10));
    drained.recv_timeout(Duration::from_secs(10)).unwrap();
    worker.join().unwrap();
    observed.expect("control-plane error did not remain observable while the probe was gated");
    assert!(result.unwrap());
    assert_eq!(requests.load(Ordering::Acquire), 2);
    assert!(!request_permit.load(Ordering::Acquire));
}

#[test]
fn readiness_fixture_control_plane_error_cannot_mask_active_probe_success() {
    control_plane_error_waits_for_probe_fixture(true);
}

#[test]
fn readiness_fixture_control_plane_error_surfaces_after_active_probe_fails() {
    control_plane_error_waits_for_probe_fixture(false);
}

#[test]
fn readiness_fixture_cancelled_request_retains_global_bound_until_drained() {
    let request_permit = busy();
    let worker_permit = request_permit.clone();
    let cancel = Arc::new(AtomicBool::new(false));
    let worker_cancel = cancel.clone();
    let (entered, started) = mpsc::channel();
    let (release, gate) = mpsc::channel();
    let (finished, drained) = mpsc::channel();
    let request_drain = Arc::new(ProbeDrained(finished));
    let (complete, completed) = mpsc::channel();
    let starting = reply(false);
    let worker = std::thread::spawn(move || {
        let result = schedule_host_readiness(
            move || {
                let _retain_until_request_exits = &request_drain;
                entered.send(()).unwrap();
                gate.recv_timeout(Duration::from_secs(30)).unwrap();
                Ok(starting.clone())
            },
            None::<fn(jcode_sdk::cloud::CloudTarget) -> Option<()>>,
            busy(),
            worker_permit,
            Duration::from_millis(20),
            Instant::now() + Duration::from_secs(30),
            &|_| {},
            &|| worker_cancel.load(Ordering::Acquire),
            ProbeScheduling::Concurrent,
        );
        complete
            .send(matches!(result, Err(error) if error.kind == jcode_sdk::ErrorKind::Cancelled))
            .unwrap();
    });
    let entered = started.recv_timeout(Duration::from_secs(10));
    cancel.store(true, Ordering::Release);
    let cancelled = completed.recv_timeout(Duration::from_secs(10));
    let retained = request_permit.load(Ordering::Acquire);

    // A replacement foreground call is allowed several scheduler turns but
    // cannot launch another request while the abandoned worker owns the slot.
    let calls = Arc::new(AtomicUsize::new(0));
    let request_calls = calls.clone();
    let turns = AtomicUsize::new(0);
    let replacement = schedule_host_readiness(
        move || {
            request_calls.fetch_add(1, Ordering::AcqRel);
            Ok(reply(false))
        },
        None::<fn(jcode_sdk::cloud::CloudTarget) -> Option<()>>,
        busy(),
        request_permit.clone(),
        Duration::from_millis(20),
        Instant::now() + Duration::from_secs(30),
        &|_| {},
        &|| turns.fetch_add(1, Ordering::AcqRel) >= 4,
        ProbeScheduling::Concurrent,
    );
    release.send(()).unwrap();
    drained.recv_timeout(Duration::from_secs(10)).unwrap();
    worker.join().unwrap();
    await_permit_release(&request_permit);
    entered.unwrap();
    assert!(cancelled.unwrap());
    assert!(retained);
    assert!(matches!(replacement, Err(error) if error.kind == jcode_sdk::ErrorKind::Cancelled));
    assert_eq!(calls.load(Ordering::Acquire), 0);
}

#[test]
fn readiness_fixture_cancellation_retains_bound_and_drops_late_success() {
    struct Transport(mpsc::Sender<()>);
    impl Drop for Transport {
        fn drop(&mut self) {
            let _ = self.0.send(());
        }
    }
    let permit = busy();
    let cancel = Arc::new(AtomicBool::new(false));
    let (release, released) = mpsc::channel();
    let released = Arc::new(Mutex::new(released));
    let (dropped, drop_notice) = mpsc::channel();
    let probe_cancel = cancel.clone();
    let starting = reply(false);
    let began = Instant::now();
    let first_starting = starting.clone();
    let result = schedule_host_readiness(
        move || Ok(first_starting.clone()),
        Some(move |_| {
            probe_cancel.store(true, Ordering::Release);
            released
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(5))
                .unwrap();
            Some(Transport(dropped.clone()))
        }),
        permit.clone(),
        busy(),
        Duration::from_millis(250),
        began + Duration::from_secs(10),
        &|_| {},
        &|| cancel.load(Ordering::Acquire),
        ProbeScheduling::Concurrent,
    );
    assert!(matches!(result, Err(error) if error.kind == jcode_sdk::ErrorKind::Cancelled));
    assert!(permit.load(Ordering::Acquire));
    // Another caller cannot create an extra speculative worker while the
    // cancelled caller's bounded attempt is still draining.
    let calls = Arc::new(AtomicUsize::new(0));
    let probe_calls = calls.clone();
    let mut polls = 0;
    let result = schedule_host_readiness(
        move || {
            polls += 1;
            let mut response = starting.clone();
            response.ready = polls == 2;
            Ok(response)
        },
        Some(move |_| {
            probe_calls.fetch_add(1, Ordering::Relaxed);
            None::<()>
        }),
        permit.clone(),
        busy(),
        Duration::from_millis(10),
        Instant::now() + Duration::from_secs(10),
        &|_| {},
        &|| false,
        ProbeScheduling::Concurrent,
    )
    .unwrap();
    assert!(matches!(result, HostWait::Ready(_)));
    assert_eq!(calls.load(Ordering::Relaxed), 0);
    release.send(()).unwrap();
    drop_notice.recv_timeout(Duration::from_secs(2)).unwrap();
}

#[test]
fn readiness_fixture_direct_success_uses_validated_target_without_second_poll() {
    let starting = reply(false);
    let target = jcode_sdk::cloud::CloudTarget::from_reply(&starting).unwrap();
    let requests = Arc::new(AtomicUsize::new(0));
    let request_count = requests.clone();
    let result = schedule_host_readiness(
        move || {
            request_count.fetch_add(1, Ordering::AcqRel);
            Ok(starting.clone())
        },
        Some(|_| Some(42_u32)),
        busy(),
        busy(),
        // A generous interval avoids imposing subsecond thread scheduling
        // through this request-count assertion.
        Duration::from_secs(10),
        Instant::now() + Duration::from_secs(30),
        &|_| {},
        &|| false,
        ProbeScheduling::Concurrent,
    )
    .unwrap();
    assert!(matches!(result, HostWait::Direct((actual, 42)) if actual == target));
    assert_eq!(requests.load(Ordering::Acquire), 1);
}

#[test]
fn readiness_fixture_invalid_pins_never_start_probe() {
    let mut starting = reply(false);
    let mut ready = starting.clone();
    ready.ready = true;
    starting.host_keys.clear();
    let calls = Arc::new(AtomicUsize::new(0));
    let probe_calls = calls.clone();
    let mut requests = 0;
    let result = schedule_host_readiness(
        move || {
            requests += 1;
            Ok(if requests == 1 {
                starting.clone()
            } else {
                ready.clone()
            })
        },
        Some(move |_| {
            probe_calls.fetch_add(1, Ordering::Relaxed);
            None::<()>
        }),
        busy(),
        busy(),
        Duration::from_millis(1),
        Instant::now() + Duration::from_secs(10),
        &|_| {},
        &|| false,
        ProbeScheduling::Concurrent,
    );
    assert!(matches!(result, Ok(HostWait::Ready(_))));
    assert_eq!(calls.load(Ordering::Relaxed), 0);
}

#[test]
fn readiness_fixture_cancel_before_request_has_no_control_plane_effect() {
    let requests = Arc::new(AtomicUsize::new(0));
    let request_count = requests.clone();
    let result = schedule_host_readiness(
        move || {
            request_count.fetch_add(1, Ordering::AcqRel);
            Ok(reply(false))
        },
        None::<fn(jcode_sdk::cloud::CloudTarget) -> Option<()>>,
        busy(),
        busy(),
        Duration::from_millis(250),
        Instant::now() + Duration::from_secs(10),
        &|_| {},
        &|| true,
        ProbeScheduling::Concurrent,
    );
    assert!(matches!(result, Err(error) if error.kind == jcode_sdk::ErrorKind::Cancelled));
    assert_eq!(requests.load(Ordering::Acquire), 0);
}

#[test]
fn readiness_fixture_cancel_during_request_does_not_start_probe() {
    let cancel = Arc::new(AtomicBool::new(false));
    let request_cancel = cancel.clone();
    let calls = Arc::new(AtomicUsize::new(0));
    let probe_calls = calls.clone();
    let starting = reply(false);
    let result = schedule_host_readiness(
        move || {
            request_cancel.store(true, Ordering::Release);
            Ok(starting.clone())
        },
        Some(move |_| {
            probe_calls.fetch_add(1, Ordering::Relaxed);
            None::<()>
        }),
        busy(),
        busy(),
        Duration::from_millis(250),
        Instant::now() + Duration::from_secs(10),
        &|_| {},
        &|| cancel.load(Ordering::Acquire),
        ProbeScheduling::Concurrent,
    );
    assert!(matches!(result, Err(error) if error.kind == jcode_sdk::ErrorKind::Cancelled));
    assert_eq!(calls.load(Ordering::Relaxed), 0);
}
