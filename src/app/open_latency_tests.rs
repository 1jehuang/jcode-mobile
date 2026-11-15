//! Measurement-only regressions for completed empty opens and abandoned tabs.
use super::navigation_coverage::mount;
use super::*;
use gpui::TestAppContext;
use std::time::Instant;

fn stale_start() -> Instant {
    Instant::now() - Duration::from_secs(476)
}

fn transcript(app: &mut MobileApp) {
    app.session.transcript.push(TranscriptEntry {
        id: 1, role: Role::Assistant, text: "Synthetic fixture".into(),
        reasoning: String::new(), tool_calls: vec![], is_streaming: false,
        is_queued: false,
    });
}

#[gpui::test]
fn empty_history_retires_open_before_a_later_first_message(cx: &mut TestAppContext) {
    let (view, cx) = mount(cx);
    view.update(cx, |app, _| {
        app.screen = Screen::Chat;
        app.open_started = Some(stale_start());
        app.retire_open_latency_for(&ConnectionOutput::Event(jcode_mobile_core::ApiEvent::History {
            session_id: "latency-fixture".into(), messages: vec![], images: vec![],
        }));
        assert!(app.open_started.is_none());
        transcript(app);
        assert!(app.take_transcript_open_latency().is_none());
    });
}

#[gpui::test]
fn empty_history_through_real_pump_completes_empty_open(cx: &mut TestAppContext) {
    let (view, cx) = mount(cx);
    let (tx, rx) = unbounded();
    view.update(cx, |app, cx| {
        app.open_started = Some(stale_start());
        app.pump(app.active_tab, rx, cx);
    });
    tx.unbounded_send(ConnectionOutput::Event(jcode_mobile_core::ApiEvent::History {
        session_id: "latency-fixture".into(), messages: vec![], images: vec![],
    })).unwrap();
    drop(tx);
    cx.run_until_parked();
    view.update(cx, |app, _| {
        assert!(app.open_started.is_none());
        transcript(app);
        assert!(app.take_transcript_open_latency().is_none());
    });
}

#[gpui::test]
fn initial_open_is_consumed_once_on_chat_transcript(cx: &mut TestAppContext) {
    let (view, cx) = mount(cx);
    view.update(cx, |app, _| {
        app.screen = Screen::Chat;
        app.open_started = Some(Instant::now());
        assert!(app.take_transcript_open_latency().is_none());
        assert!(app.open_started.is_some());
        transcript(app);
        assert!(app.take_transcript_open_latency().is_some());
        assert!(app.take_transcript_open_latency().is_none());
    });
}

#[gpui::test]
fn settings_and_signin_cannot_measure_a_hidden_transcript(cx: &mut TestAppContext) {
    let (view, cx) = mount(cx);
    view.update(cx, |app, _| {
        transcript(app);
        for screen in [Screen::Settings, Screen::SignIn] {
            app.screen = screen;
            app.open_started = Some(stale_start());
            assert!(app.take_transcript_open_latency().is_none());
            assert!(app.open_started.is_none());
        }
    });
}

#[gpui::test]
fn retry_failure_and_disconnect_retire_initial_open(cx: &mut TestAppContext) {
    let (view, cx) = mount(cx);
    view.update(cx, |app, _| {
        for phase in [ConnectionPhase::Reconnecting { attempt: 1 },
                      ConnectionPhase::Failed { reason: "fixture".into() },
                      ConnectionPhase::Disconnected] {
            app.open_started = Some(stale_start());
            app.retire_open_latency_for(&ConnectionOutput::Phase(phase));
            assert!(app.open_started.is_none());
        }
        app.open_started = Some(Instant::now());
        app.retire_open_latency_for(&ConnectionOutput::Phase(ConnectionPhase::Connecting));
        app.retire_open_latency_for(&ConnectionOutput::Phase(ConnectionPhase::Connected));
        assert!(app.open_started.is_some());
    });
}

#[gpui::test]
fn parking_active_tab_does_not_carry_its_open_epoch(cx: &mut TestAppContext) {
    let (view, cx) = mount(cx);
    view.update(cx, |app, cx| {
        app.open_started = Some(stale_start());
        app.park_active(cx);
        assert!(app.open_started.is_none());
    });
}
