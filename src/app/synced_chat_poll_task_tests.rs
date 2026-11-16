use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
    mpsc,
};

struct GatedPoll {
    account: jcode_mobile_core::Account,
    started: Arc<AtomicUsize>,
    history: mpsc::Sender<()>,
    status: mpsc::Sender<()>,
    server: std::thread::JoinHandle<()>,
}

fn gated_poll() -> GatedPoll {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let account = jcode_mobile_core::Account {
        api_base: format!("http://{}/v1", listener.local_addr().unwrap()),
        api_key: "gated-production-poll".into(),
        given_name: None,
        name: None,
        email: None,
    };
    let started = Arc::new(AtomicUsize::new(0));
    let observed = started.clone();
    let (history, history_gate) = mpsc::channel();
    let (status, status_gate) = mpsc::channel();
    let server = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut gates = [Some(history_gate), Some(status_gate)];
        let mut workers = Vec::new();
        while workers.len() < 2 {
            assert!(
                Instant::now() < deadline,
                "poll loops did not issue both GETs"
            );
            let (mut socket, _) = match listener.accept() {
                Ok(value) => value,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(1));
                    continue;
                }
                Err(error) => panic!("{error}"),
            };
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut bytes = Vec::new();
            while !bytes.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                assert_eq!(socket.read(&mut byte).unwrap(), 1);
                bytes.extend_from_slice(&byte);
            }
            let request = String::from_utf8(bytes).unwrap();
            assert!(
                request.starts_with("GET "),
                "polling must never resend a POST"
            );
            assert!(request.contains("Bearer gated-production-poll"));
            let command_request = request.contains("/mobile/commands/");
            if !command_request {
                assert!(request.contains("after_seq=-1"));
            }
            let index = usize::from(command_request);
            let gate = gates[index]
                .take()
                .expect("duplicate GET while previous request blocked");
            observed.fetch_or(1 << index, Ordering::SeqCst);
            workers.push(std::thread::spawn(move || {
                gate.recv_timeout(Duration::from_secs(10)).expect("response gate not released");
                let body = if command_request {
                    serde_json::json!({"command":{"command_id":command("done").command_id,"kind":"message","session_id":"local-chat","status":"done"}})
                } else {
                    serde_json::json!({"session":{"session_id":"local-chat","source":"local","live":true},"messages":[{"seq":3,"role":"assistant","text":"Network rewrite"}],"more":false})
                }.to_string();
                write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            }));
        }
        for worker in workers {
            worker.join().unwrap();
        }
    });
    GatedPoll {
        account,
        started,
        history,
        status,
        server,
    }
}

fn pump_until(
    cx: &mut gpui::VisualTestContext,
    mut done: impl FnMut(&mut gpui::VisualTestContext) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        // Unlike VisualTestContext::run_until_parked, this does not advance
        // timers and therefore cannot trigger repeated periodic poll GETs.
        cx.dispatcher.run_until_parked();
        if done(cx) {
            return;
        }
        assert!(Instant::now() < deadline, "poll completion did not arrive");
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn finish_tasks(cx: &mut gpui::VisualTestContext, tasks: Vec<gpui::Task<()>>) {
    cx.executor().advance_clock(Duration::from_secs(2));
    cx.executor().allow_parking();
    cx.foreground_executor()
        .clone()
        .block_test(futures::future::join_all(tasks));
    cx.executor().forbid_parking();
}

#[gpui::test]
fn production_poll_tasks_acknowledge_before_gated_history(cx: &mut gpui::TestAppContext) {
    cx.update(app::init);
    // These tests intentionally mix deterministic foreground scheduling with
    // real loopback I/O, so permit cross-thread HTTP completion wakeups.
    cx.executor().allow_parking();
    let fixture = gated_poll();
    let (view, cx) = cx.add_window_view(fixture_app);
    let start = Instant::now();
    let (tab, tasks) = view.update(cx, |app, cx| {
        app.account = Some(fixture.account.clone());
        app.open_local_chat(live(), None, cx);
        let tab = app.active_tab;
        app.apply_local_poll(tab, snapshot(true, false, "Old snapshot"), None);
        let sent = jcode_mobile_core::SessionImage::new("image/jpeg", vec![1]);
        let newer = jcode_mobile_core::SessionImage::new("image/jpeg", vec![2]);
        app.attach.pending = vec![sent.clone(), newer];
        let chat = app.synced.chats.get_mut(&tab).unwrap();
        chat.pending = Some(command("queued").command_id);
        chat.submitted_images = vec![sent];
        app.offline = false;
        // This is the exact task factory used by poll_local_chat. Retain the
        // handles rather than detaching them so termination is also verified.
        (tab, app.local_chat_poll_tasks(tab, cx))
    });
    pump_until(cx, |_| fixture.started.load(Ordering::SeqCst) == 3);
    fixture.status.send(()).unwrap();
    pump_until(cx, |cx| {
        view.update(cx, |app, _| app.synced.chats[&tab].pending.is_none())
    });
    let acknowledged = start.elapsed();
    view.update(cx, |app, _| {
        assert!(app.local_chat_ready(&app.session));
        assert_eq!(app.session.transcript[1].text, "Old snapshot");
        assert!(app.synced.chats[&tab].submitted_images.is_empty());
        assert_eq!(app.attach.pending.len(), 1);
        assert_eq!(app.attach.pending[0].bytes.as_ref(), &vec![2]);
    });
    // History has no response yet. This ordering, rather than a timing
    // threshold, is the regression assertion for the production task loops.
    fixture.history.send(()).unwrap();
    pump_until(cx, |cx| {
        view.update(cx, |app, _| app.session.transcript.len() == 1)
    });
    view.update(cx, |app, _| {
        assert_eq!(app.session.transcript[0].text, "Network rewrite");
        app.auth_generation += 1;
        app.offline = true;
    });
    finish_tasks(cx, tasks);
    fixture.server.join().unwrap();
    eprintln!(
        "local_production_poll_ack_ms={:.3} history_gate_released_after_pending_clear=true",
        acknowledged.as_secs_f64() * 1000.
    );
}

#[gpui::test]
fn offline_open_reconnect_restarts_read_only_delivery_polls(cx: &mut gpui::TestAppContext) {
    cx.update(app::init);
    cx.executor().allow_parking();
    let fixture = gated_poll();
    let (view, cx) = cx.add_window_view(fixture_app);
    let tab = view.update(cx, |app, cx| {
        app.account = Some(fixture.account.clone());
        app.open_local_chat(live(), Some("Keep my draft".into()), cx);
        let tab = app.active_tab;
        let chat = app.synced.chats.get_mut(&tab).unwrap();
        chat.pending = Some(command("queued").command_id);
        assert_eq!(chat.poll_generation, 0, "offline open must not poll");
        app.offline = false;
        app.reconnect(cx);
        assert_eq!(app.synced.chats[&tab].poll_generation, 1);
        tab
    });
    pump_until(cx, |_| fixture.started.load(Ordering::SeqCst) == 3);
    fixture.status.send(()).unwrap();
    fixture.history.send(()).unwrap();
    pump_until(cx, |cx| {
        view.update(cx, |app, _| {
            app.synced.chats[&tab].pending.is_none() && app.session.transcript.len() == 1
        })
    });
    view.update(cx, |app, cx| {
        assert_eq!(app.session.phase, ConnectionPhase::Connected);
        assert_eq!(app.session.transcript[0].text, "Network rewrite");
        assert_eq!(app.composer.read(cx).text(), "Keep my draft");
        assert!(app.connection.is_none(), "local reconnect must not attach SSH");
        app.offline = true;
        app.synced.chats.get_mut(&tab).unwrap().poll_generation += 1;
    });
    cx.executor().advance_clock(Duration::from_secs(2));
    cx.dispatcher.run_until_parked();
    fixture.server.join().unwrap();
}

#[gpui::test]
fn production_poll_tasks_fence_late_auth_account_tab_and_poll_completions(
    cx: &mut gpui::TestAppContext,
) {
    cx.update(app::init);
    // These tests intentionally mix deterministic foreground scheduling with
    // real loopback I/O, so permit cross-thread HTTP completion wakeups.
    cx.executor().allow_parking();
    let (view, cx) = cx.add_window_view(fixture_app);
    for fence in ["auth", "account", "tab", "poll"] {
        let fixture = gated_poll();
        let (tab, tasks) = view.update(cx, |app, cx| {
            app.offline = true;
            app.account = Some(fixture.account.clone());
            app.open_local_chat(live(), None, cx);
            let tab = app.active_tab;
            if !app.tab_order.contains(&tab) {
                app.tab_order.push(tab);
            }
            app.apply_local_poll(tab, snapshot(true, false, "Unchanged"), None);
            let sent = jcode_mobile_core::SessionImage::new("image/jpeg", vec![7]);
            app.attach.pending = vec![sent.clone()];
            let chat = app.synced.chats.get_mut(&tab).unwrap();
            chat.pending = Some(command("queued").command_id);
            chat.submitted_images = vec![sent];
            app.offline = false;
            (tab, app.local_chat_poll_tasks(tab, cx))
        });
        pump_until(cx, |_| fixture.started.load(Ordering::SeqCst) == 3);
        view.update(cx, |app, _| {
            match fence {
                "auth" => app.auth_generation += 1,
                "account" => app.account.as_mut().unwrap().api_key = "different-account".into(),
                "tab" => app.tab_order.retain(|id| *id != tab),
                "poll" => app.synced.chats.get_mut(&tab).unwrap().poll_generation += 1,
                _ => unreachable!(),
            }
            app.offline = true;
        });
        fixture.status.send(()).unwrap();
        fixture.history.send(()).unwrap();
        // Await both real production loop tasks. Both HTTP completions have
        // reached the guarded apply point before checking for stale mutation.
        finish_tasks(cx, tasks);
        fixture.server.join().unwrap();
        view.update(cx, |app, _| {
            assert_eq!(
                app.synced.chats[&tab].pending.as_deref(),
                Some(command("queued").command_id.as_str()),
                "{fence}"
            );
            assert_eq!(app.synced.chats[&tab].submitted_images.len(), 1, "{fence}");
            assert_eq!(app.attach.pending.len(), 1, "{fence}");
            assert_eq!(app.session.transcript[1].text, "Unchanged", "{fence}");
            assert!(app.session.error_banner.is_none(), "{fence}");
            if !app.tab_order.contains(&tab) {
                app.tab_order.push(tab);
            }
        });
    }
}
