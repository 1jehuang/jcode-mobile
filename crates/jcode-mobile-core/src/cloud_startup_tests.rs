//! Offline causal fixtures for the production readiness scheduler. No cloud,
//! account storage, DNS, sockets, or production SSH are contacted.
use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
    mpsc,
};

fn busy() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(false))
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
    // failed probe budget, and server becoming ready 100ms after first reply.
    // Only the scheduling policy differs. Two authenticated requests in each.
    fn run(policy: ProbeScheduling) -> (Duration, usize, Arc<AtomicBool>) {
        let starting = reply(false);
        let mut ready = starting.clone();
        ready.ready = true;
        let permit = busy();
        let start = Instant::now();
        let mut requests = 0;
        let result = schedule_host_readiness(
            || {
                requests += 1;
                Ok(if start.elapsed() >= Duration::from_millis(100) {
                    ready.clone()
                } else {
                    starting.clone()
                })
            },
            Some(|_| {
                std::thread::sleep(Duration::from_millis(1200));
                None::<()>
            }),
            permit.clone(),
            Duration::from_millis(250),
            start + Duration::from_secs(10),
            &|_| {},
            &|| false,
            policy,
        )
        .unwrap();
        assert!(matches!(result, HostWait::Ready(_)));
        (start.elapsed(), requests, permit)
    }
    let (before, before_calls, _) = run(ProbeScheduling::SerialBaseline);
    let (after, after_calls, permit) = run(ProbeScheduling::Concurrent);
    println!(
        "cloud readiness controlled baseline={:.3}ms after={:.3}ms requests={before_calls}/{after_calls}; fixture only, not device startup",
        before.as_secs_f64() * 1000.0,
        after.as_secs_f64() * 1000.0
    );
    assert_eq!((before_calls, after_calls), (2, 2));
    assert!(before >= Duration::from_millis(1450), "{before:?}");
    assert!(after < Duration::from_millis(900), "{after:?}");
    assert!(before.saturating_sub(after) >= Duration::from_millis(900));
    // The losing bounded attempt retains the global permit after return.
    assert!(permit.load(Ordering::Acquire));
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
    let result = schedule_host_readiness(
        || Ok(starting.clone()),
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
        Duration::from_millis(250),
        began + Duration::from_secs(10),
        &|_| {},
        &|| cancel.load(Ordering::Acquire),
        ProbeScheduling::Concurrent,
    );
    assert!(matches!(result, Err(error) if error.kind == jcode_sdk::ErrorKind::Cancelled));
    assert!(began.elapsed() < Duration::from_millis(900));
    assert!(permit.load(Ordering::Acquire));
    // Another caller cannot create an extra speculative worker while the
    // cancelled caller's bounded attempt is still draining.
    let calls = Arc::new(AtomicUsize::new(0));
    let probe_calls = calls.clone();
    let mut polls = 0;
    let result = schedule_host_readiness(
        || {
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
    let mut requests = 0;
    let result = schedule_host_readiness(
        || {
            requests += 1;
            Ok(starting.clone())
        },
        Some(|_| Some(42_u32)),
        busy(),
        Duration::from_millis(250),
        Instant::now() + Duration::from_secs(10),
        &|_| {},
        &|| false,
        ProbeScheduling::Concurrent,
    )
    .unwrap();
    assert!(matches!(result, HostWait::Direct((actual, 42)) if actual == target));
    assert_eq!(requests, 1);
}

#[test]
fn readiness_fixture_invalid_pins_never_start_probe() {
    let mut starting = reply(false);
    starting.host_keys.clear();
    let calls = Arc::new(AtomicUsize::new(0));
    let probe_calls = calls.clone();
    let result = schedule_host_readiness(
        || Ok(starting.clone()),
        Some(move |_| {
            probe_calls.fetch_add(1, Ordering::Relaxed);
            None::<()>
        }),
        busy(),
        Duration::from_millis(1),
        Instant::now(),
        &|_| {},
        &|| false,
        ProbeScheduling::Concurrent,
    );
    assert!(matches!(result, Err(error) if error.kind == jcode_sdk::ErrorKind::StartupTimeout));
    assert_eq!(calls.load(Ordering::Relaxed), 0);
}

#[test]
fn readiness_fixture_cancel_before_request_has_no_control_plane_effect() {
    let mut requests = 0;
    let result = schedule_host_readiness(
        || {
            requests += 1;
            Ok(reply(false))
        },
        None::<fn(jcode_sdk::cloud::CloudTarget) -> Option<()>>,
        busy(),
        Duration::from_millis(250),
        Instant::now() + Duration::from_secs(10),
        &|_| {},
        &|| true,
        ProbeScheduling::Concurrent,
    );
    assert!(matches!(result, Err(error) if error.kind == jcode_sdk::ErrorKind::Cancelled));
    assert_eq!(requests, 0);
}

#[test]
fn readiness_fixture_cancel_during_request_does_not_start_probe() {
    let cancel = AtomicBool::new(false);
    let calls = Arc::new(AtomicUsize::new(0));
    let probe_calls = calls.clone();
    let starting = reply(false);
    let result = schedule_host_readiness(
        || {
            cancel.store(true, Ordering::Release);
            Ok(starting.clone())
        },
        Some(move |_| {
            probe_calls.fetch_add(1, Ordering::Relaxed);
            None::<()>
        }),
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
