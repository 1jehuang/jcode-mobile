use super::*;

#[gpui::test]
fn http_outage_recovers_all_pages_and_replaces_rewritten_rows(cx: &mut gpui::TestAppContext) {
    use crate::app::navigation_coverage::{draw, mount};
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let api_base = format!("http://{}/v1", listener.local_addr().unwrap());
    let page = |rows: serde_json::Value, more| {
        serde_json::json!({
            "session":{"session_id":"local-chat","source":"local","live":true},
            "messages":rows,"more":more
        })
    };
    let responses = [
        (
            -1,
            503,
            serde_json::json!({"error":{"message":"fixture outage"}}),
        ),
        (
            -1,
            200,
            page(
                serde_json::json!([
                    {"seq":0,"role":"user","text":"Question"},
                    {"seq":3,"role":"assistant","text":"Partial answer"}
                ]),
                true,
            ),
        ),
        (
            3,
            200,
            page(
                serde_json::json!([
                    {"seq":8,"role":"tool","text":"Recovered tool output"}
                ]),
                false,
            ),
        ),
        (
            -1,
            200,
            page(
                serde_json::json!([
                    {"seq":0,"role":"user","text":"Question"},
                    {"seq":3,"role":"assistant","text":"Rewritten final answer"}
                ]),
                false,
            ),
        ),
    ];
    let server = std::thread::spawn(move || {
        for (cursor, status, body) in responses {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .unwrap();
            let mut bytes = Vec::new();
            while !bytes.windows(4).any(|w| w == b"\r\n\r\n") {
                let mut buf = [0; 4096];
                let n = stream.read(&mut buf).unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&buf[..n]);
            }
            let request = String::from_utf8(bytes).unwrap();
            assert!(request.starts_with(&format!(
                "GET /v1/mobile/sessions/local-chat?after_seq={cursor} HTTP/1.1"
            )));
            assert!(
                request
                    .to_lowercase()
                    .contains("authorization: bearer recovery-fixture")
            );
            let body = body.to_string();
            write!(stream, "HTTP/1.1 {status} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        }
    });
    let (view, cx) = mount(cx);
    view.update(cx, |app, cx| {
        // Offline mode suppresses automatic polling. Only these explicit GETs
        // reach the loopback fixture, never a production account or POST.
        app.account.as_mut().unwrap().api_base = api_base;
        app.account.as_mut().unwrap().api_key = "recovery-fixture".into();
        app.open_local_chat(live(), Some("Retained draft".into()), cx);
        app.apply_local_poll(
            app.active_tab,
            snapshot(true, true, "Last good answer"),
            None,
        );
        let account = app.account.as_ref().unwrap();
        let result = mobile_sync::transcript(account, "local-chat");
        app.apply_local_poll(app.active_tab, result, None);
        assert_eq!(app.session.phase, ConnectionPhase::Disconnected);
        assert_eq!(app.session.transcript[1].text, "Last good answer");
        assert_eq!(app.composer.read(cx).text(), "Retained draft");
        cx.notify();
    });
    draw(cx);
    assert!(cx.debug_bounds("composer-connection-status").is_some());
    view.update(cx, |app, cx| {
        let result = mobile_sync::transcript(app.account.as_ref().unwrap(), "local-chat");
        app.apply_local_poll(app.active_tab, result, None);
        assert_eq!(app.session.transcript.len(), 3);
        assert_eq!(app.session.transcript[1].text, "Partial answer");
        assert!(
            app.session.transcript[2].tool_calls[0]
                .output
                .contains("Recovered tool output")
        );
        assert_eq!(app.session.phase, ConnectionPhase::Connected);
        assert!(app.session.error_banner.is_none());
        let result = mobile_sync::transcript(app.account.as_ref().unwrap(), "local-chat");
        app.apply_local_poll(app.active_tab, result, None);
        assert_eq!(app.session.transcript.len(), 2);
        assert_eq!(app.session.transcript[1].text, "Rewritten final answer");
        assert_eq!(app.composer.read(cx).text(), "Retained draft");
        assert!(app.connection.is_none());
        cx.notify();
    });
    draw(cx);
    assert!(cx.debug_bounds("composer-connection-status").is_none());
    server.join().unwrap();
}

#[gpui::test]
fn closed_local_delivery_state_survives_reopen_and_late_completion(cx: &mut gpui::TestAppContext) {
    cx.update(app::init);
    let (view, cx) = cx.add_window_view(fixture_app);
    view.update(cx, |app, cx| {
        for outcome in [
            "queued",
            "uncertain",
            "submitting",
            "late_ack",
            "late_error",
        ] {
            app.open_local_chat(live(), None, cx);
            let tab = app.active_tab;
            app.apply_local_poll(tab, snapshot(true, false, "Existing"), None);
            let account = app.account.clone().unwrap();
            let generation = app.auth_generation;
            let chat = app.synced.chats.get_mut(&tab).unwrap();
            match outcome {
                "queued" => chat.pending = Some(command("queued").command_id),
                "uncertain" => chat.uncertain = true,
                _ => chat.submitting = true,
            }
            app.close_tab(tab, cx);
            if outcome == "submitting" {
                app.complete_local_submission(
                    tab,
                    &account,
                    generation,
                    "Submitted",
                    Ok(command("queued")),
                    cx,
                );
            }
            app.open_local_chat(live(), None, cx);
            if matches!(outcome, "late_ack" | "late_error") {
                assert_eq!(
                    app.active_tab, tab,
                    "reopening must retain the in-flight owner"
                );
                let result = if outcome == "late_ack" {
                    Ok(command("queued"))
                } else {
                    Err("fixture lost POST response".into())
                };
                app.complete_local_submission(tab, &account, generation, "Submitted", result, cx);
            }
            app.apply_local_poll(app.active_tab, snapshot(true, false, "Recovered"), None);
            assert!(
                !app.local_chat_ready(&app.session),
                "closing {outcome} must not unlock duplicate sends"
            );
            if matches!(outcome, "uncertain" | "late_error") {
                assert!(app.local_delivery_uncertain(&app.session));
                app.acknowledge_local_delivery(cx);
            } else {
                assert!(app.synced.chats[&app.active_tab].pending.is_some());
                app.apply_local_poll(
                    app.active_tab,
                    snapshot(true, false, "Done"),
                    Some(Ok(command("done"))),
                );
            }
            assert!(app.local_chat_ready(&app.session));
            app.close_tab(app.active_tab, cx);
        }
    });
}

#[gpui::test]
fn composer_sends_local_command_and_receives_synced_answer(cx: &mut gpui::TestAppContext) {
    use crate::app::navigation_coverage::{mount, tap};
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let api_base = format!("http://{}/v1", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();
        let mut bytes = Vec::new();
        loop {
            let mut buf = [0; 4096];
            let n = stream.read(&mut buf).unwrap();
            assert!(n > 0);
            bytes.extend_from_slice(&buf[..n]);
            if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&bytes[..end]);
                let length = headers
                    .lines()
                    .find_map(|line| {
                        let (key, value) = line.split_once(':')?;
                        key.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().unwrap())
                    })
                    .unwrap_or(0);
                if bytes.len() >= end + 4 + length {
                    break;
                }
            }
        }
        let request = String::from_utf8(bytes).unwrap();
        assert!(request.starts_with("POST /v1/mobile/commands HTTP/1.1"));
        assert!(
            request
                .to_lowercase()
                .contains("authorization: bearer local-composer-test")
        );
        let body: serde_json::Value =
            serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
        assert_eq!(
            body,
            serde_json::json!({"kind":"message","session_id":"local-chat","text":"From phone"})
        );
        let body = serde_json::json!({"command":{"command_id":"12345678-1234-1234-1234-123456789abc","kind":"message","session_id":"local-chat","status":"queued"},"host":{"state":"local"}}).to_string();
        write!(stream, "HTTP/1.1 202 Accepted\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
    });
    let (view, cx) = mount(cx);
    view.update(cx, |app, cx| {
        app.account.as_mut().unwrap().api_base = api_base;
        app.account.as_mut().unwrap().api_key = "local-composer-test".into();
        // Open without a poller so this test owns exactly one real HTTP call.
        app.open_local_chat(live(), None, cx);
        app.apply_local_poll(app.active_tab, snapshot(true, false, "Existing"), None);
        app.composer
            .update(cx, |input, cx| input.set_text(" From phone ", cx));
        app.offline = false;
        cx.notify();
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    tap(cx, "send");
    cx.run_until_parked();
    view.update(cx, |app, cx| {
        let local = app.active_tab;
        assert!(app.synced.chats[&local].pending.is_some());
        assert_eq!(app.composer.read(cx).text(), "");
        assert!(app.connection.is_none());
        app.apply_local_poll(
            local,
            snapshot(true, true, "Working"),
            Some(Ok(command("running"))),
        );
        assert!(!app.local_chat_ready(&app.session));
        app.apply_local_poll(
            local,
            snapshot(true, false, "Answer from computer"),
            Some(Ok(command("done"))),
        );
        assert!(app.local_chat_ready(&app.session));
        assert_eq!(app.session.session_id.as_deref(), Some("local-chat"));
        assert_eq!(app.session.transcript[1].text, "Answer from computer");
        assert_eq!(app.tab_order.len(), 2);
        app.offline = true;
    });
    server.join().unwrap();
}

#[gpui::test]
fn live_drawer_row_opens_chat_instead_of_cloud_transfer(cx: &mut gpui::TestAppContext) {
    use crate::app::navigation_coverage::{drawer, mount, tap};
    let (view, cx) = mount(cx);
    view.update(cx, |app, cx| {
        app.synced.sessions = vec![live()];
        app.synced.selected = Some("local-chat".into());
        cx.notify();
    });
    drawer(&view, cx);
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("transfer-local-chat").is_none());
    tap(cx, "drawer-local-session-local-chat");
    view.read_with(cx, |app, _| {
        assert!(app.is_local_chat());
        assert_eq!(app.session.session_id.as_deref(), Some("local-chat"));
        assert!(app.connection.is_none());
        assert!(!app.drawer_open);
        assert_eq!(app.tab_order.len(), 2);
    });
}
#[gpui::test]
fn submission_completion_preserves_new_typing_and_parked_drafts(cx: &mut gpui::TestAppContext) {
    cx.update(app::init);
    let (view, cx) = cx.add_window_view(fixture_app);
    view.update(cx, |app, cx| {
        let cloud = app.active_tab;
        app.open_local_chat(live(), None, cx);
        let local = app.active_tab;
        let account = app.account.clone().unwrap();
        let generation = app.auth_generation;
        app.apply_local_poll(local, snapshot(true, false, "Existing"), None);
        app.composer
            .update(cx, |input, cx| input.set_text("New typing", cx));
        app.complete_local_submission(
            local,
            &account,
            generation,
            "Submitted text",
            Ok(command("queued")),
            cx,
        );
        assert_eq!(app.composer.read(cx).text(), "New typing");
        assert!(app.synced.chats[&local].pending.is_some());
        app.synced.chats.get_mut(&local).unwrap().pending = None;
        app.focus_tab(cloud, cx);
        app.composer
            .update(cx, |input, cx| input.set_text("Cloud is untouched", cx));
        app.complete_local_submission(
            local,
            &account,
            generation,
            "New typing",
            Ok(command("queued")),
            cx,
        );
        assert_eq!(app.composer.read(cx).text(), "Cloud is untouched");
        app.focus_tab(local, cx);
        assert_eq!(app.composer.read(cx).text(), "");
        app.composer
            .update(cx, |input, cx| input.set_text("Keep on network error", cx));
        app.synced.chats.get_mut(&local).unwrap().pending = None;
        app.complete_local_submission(
            local,
            &account,
            generation,
            "Keep on network error",
            Err("response lost".into()),
            cx,
        );
        assert!(app.local_delivery_uncertain(&app.session));
        assert_eq!(app.composer.read(cx).text(), "Keep on network error");
        app.acknowledge_local_delivery(cx);
        assert!(!app.local_delivery_uncertain(&app.session));
        assert!(app.synced.chats[&local].pending.is_none());
        assert_eq!(app.composer.read(cx).text(), "Keep on network error");
        app.complete_local_submission(
            local,
            &account,
            generation.wrapping_add(1),
            "Keep on network error",
            Ok(command("queued")),
            cx,
        );
        assert!(app.synced.chats[&local].pending.is_none());
        assert_eq!(app.composer.read(cx).text(), "Keep on network error");
    });
}

#[gpui::test]
fn late_pre_close_submission_does_not_clear_identical_reopened_draft(
    cx: &mut gpui::TestAppContext,
) {
    cx.update(app::init);
    let (view, cx) = cx.add_window_view(fixture_app);
    view.update(cx, |app, cx| {
        app.open_local_chat(live(), None, cx);
        let tab = app.active_tab;
        let account = app.account.clone().unwrap();
        let generation = app.auth_generation;
        app.synced.chats.get_mut(&tab).unwrap().submitting = true;
        app.close_tab(tab, cx);
        app.open_local_chat(live(), Some("Same text".into()), cx);
        assert_eq!(app.active_tab, tab);
        app.complete_local_submission(
            tab, &account, generation, "Same text", Ok(command("queued")), cx,
        );
        assert_eq!(app.composer.read(cx).text(), "Same text");
        assert!(app.synced.chats[&tab].pending.is_some());
        assert!(!app.local_chat_ready(&app.session));
    });
}

#[gpui::test]
fn already_open_local_notification_reply_is_draft_not_ssh_message(cx: &mut gpui::TestAppContext) {
    cx.update(app::init);
    let (view, cx) = cx.add_window_view(fixture_app);
    view.update(cx, |app, cx| {
        app.open_local_chat(live(), None, cx);
        let local = app.active_tab;
        app.apply_local_poll(local, snapshot(true, false, "Existing"), None);
        app.open_external_session("local-chat".into(), Some("Reply from push".into()), cx);
        assert_eq!(app.active_tab, local);
        assert_eq!(app.tab_order.len(), 2);
        assert_eq!(app.session.transcript.len(), 3);
        assert_eq!(app.composer.read(cx).text(), "Reply from push");
        assert!(app.connection.is_none());
        assert!(app.auto_send.is_none());
    });
}
