use super::*;
use std::io::{Read, Write};
use std::time::Instant;

// Identical two-page snapshots in both runs. Each page stalls for 200 ms.
// The server services connections independently, as the real API does.
fn delayed_history() -> (jcode_mobile_core::Account, std::thread::JoinHandle<()>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let account = jcode_mobile_core::Account {
        api_base: format!("http://{}/v1", listener.local_addr().unwrap()),
        api_key: "latency-fixture".into(),
        given_name: None,
        name: None,
        email: None,
    };
    let server = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut workers = Vec::new();
        while workers.len() < 3 {
            assert!(Instant::now() < deadline, "missing fixture request");
            let (mut socket, _) = match listener.accept() {
                Ok(value) => value,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(1));
                    continue;
                }
                Err(error) => panic!("{error}"),
            };
            workers.push(std::thread::spawn(move || {
                socket.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
                let mut bytes = Vec::new();
                while !bytes.ends_with(b"\r\n\r\n") {
                    let mut byte = [0];
                    assert_eq!(socket.read(&mut byte).unwrap(), 1);
                    bytes.extend_from_slice(&byte);
                }
                let request = String::from_utf8(bytes).unwrap();
                assert!(request.starts_with("GET "), "no duplicate POST: {request}");
                let body = if request.contains("/mobile/commands/") {
                    serde_json::json!({"command":{"command_id":command("done").command_id,"kind":"message","session_id":"local-chat","status":"done"}})
                } else {
                    std::thread::sleep(Duration::from_millis(200));
                    let first = request.contains("after_seq=-1");
                    assert!(first || request.contains("after_seq=3"));
                    serde_json::json!({"session":{"session_id":"local-chat","source":"local","live":true}, "messages":[
                        {"seq":if first {3} else {8},"role":"assistant","text":if first {"Rewritten"} else {"Tail"}}
                    ],"more":first})
                }.to_string();
                write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            }));
        }
        for worker in workers {
            worker.join().unwrap();
        }
    });
    (account, server)
}

#[gpui::test]
fn delayed_history_does_not_gate_pending_command_ack(cx: &mut gpui::TestAppContext) {
    cx.update(app::init);
    let (view, cx) = cx.add_window_view(fixture_app);
    let mut durations = Vec::new();
    for independent in [false, true] {
        let (account, server) = delayed_history();
        view.update(cx, |app, cx| {
            app.open_local_chat(live(), None, cx);
            let tab = app.active_tab;
            app.apply_local_poll(tab, snapshot(true, false, "Old snapshot"), None);
            app.synced.chats.get_mut(&tab).unwrap().pending = Some(command("queued").command_id);
            let start = Instant::now();
            let transcript = if independent {
                let history_account = account.clone();
                let history = std::thread::spawn(move || {
                    mobile_sync::transcript(&history_account, "local-chat")
                });
                let status = mobile_sync::command(&account, &command("queued").command_id);
                app.apply_local_command(tab, Some(status));
                assert!(app.synced.chats[&tab].pending.is_none());
                assert!(app.local_chat_ready(&app.session));
                assert_eq!(app.session.transcript[1].text, "Old snapshot");
                durations.push(start.elapsed());
                history.join().unwrap()
            } else {
                let transcript = mobile_sync::transcript(&account, "local-chat");
                assert!(app.synced.chats[&tab].pending.is_some());
                let status = mobile_sync::command(&account, &command("queued").command_id);
                app.apply_local_poll(tab, transcript.clone(), Some(status));
                assert!(app.synced.chats[&tab].pending.is_none());
                durations.push(start.elapsed());
                transcript
            };
            app.apply_local_poll(tab, transcript, None);
            assert_eq!(app.session.transcript.len(), 2);
            assert_eq!(app.session.transcript[0].text, "Rewritten");
            assert_eq!(app.session.transcript[1].text, "Tail");
        });
        server.join().unwrap();
    }
    eprintln!(
        "local_pending_ack serial_ms={:.3} independent_ms={:.3} history_delay_ms=400",
        durations[0].as_secs_f64() * 1000.,
        durations[1].as_secs_f64() * 1000.
    );
    assert!(durations[0] >= Duration::from_millis(400));
    assert!(durations[1] < durations[0] / 2);
}
