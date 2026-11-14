use super::*;
#[gpui::test]
fn photo_only_local_draft_reaches_computer_http_command(cx: &mut gpui::TestAppContext) {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(std::time::Duration::from_secs(10)))
            .unwrap();
        let mut bytes = Vec::new();
        let end = loop {
            let mut byte = [0];
            socket.read_exact(&mut byte).unwrap();
            bytes.push(byte[0]);
            if bytes.ends_with(b"\r\n\r\n") {
                break bytes.len();
            }
        };
        let headers = String::from_utf8_lossy(&bytes);
        assert!(headers.starts_with("POST /v1/mobile/commands HTTP/1.1"));
        assert!(
            headers
                .to_lowercase()
                .contains("authorization: bearer photo-fixture")
        );
        let length = headers
            .lines()
            .find_map(|line| {
                let (key, value) = line.split_once(':')?;
                key.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().unwrap())
            })
            .unwrap();
        bytes.resize(end + length, 0);
        socket.read_exact(&mut bytes[end..]).unwrap();
        let body: serde_json::Value = serde_json::from_slice(&bytes[end..]).unwrap();
        assert_eq!(
            body,
            serde_json::json!({"kind":"message","session_id":"local-chat","text":"Photo attached.","images":[{"media_type":"image/jpeg","data":"AQ=="}]})
        );
        let body = serde_json::json!({"command":{"command_id":"12345678-1234-1234-1234-123456789abc","kind":"message","session_id":"local-chat","status":"queued"}}).to_string();
        write!(socket, "HTTP/1.1 202 Accepted\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
    });
    cx.update(app::init);
    let (view, cx) = cx.add_window_view(fixture_app);
    view.update(cx, |app, cx| {
        app.account.as_mut().unwrap().api_base = base;
        app.account.as_mut().unwrap().api_key = "photo-fixture".into();
        app.open_local_chat(live(), None, cx);
        app.apply_local_poll(app.active_tab, snapshot(true, false, "Existing"), None);
        app.attach.pending.push(photo(1));
        app.offline = false;
        app.send_local_draft(cx);
        app.attach.pending.push(photo(2));
    });
    cx.run_until_parked();
    view.update(cx, |app, _| {
        let tab = app.active_tab;
        assert!(app.synced.chats[&tab].pending.is_some());
        assert_eq!(app.attach.pending, vec![photo(1), photo(2)]);
        app.apply_local_poll(
            tab,
            snapshot(true, false, "Computer answer"),
            Some(Ok(command("done"))),
        );
        assert_eq!(app.attach.pending, vec![photo(2)]);
        assert_eq!(app.session.transcript[1].text, "Computer answer");
        app.offline = true;
    });
    server.join().unwrap();
}
