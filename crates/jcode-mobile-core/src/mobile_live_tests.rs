use super::*;
use std::io::{Read, Write};

const COMMAND: &str = "12345678-1234-1234-1234-123456789abc";

fn session(live: bool, busy: bool) -> SyncedSession {
    serde_json::from_value(
        serde_json::json!({"session_id":"local:work","source":"local","live":live,"busy":busy}),
    )
    .unwrap()
}

fn serve(
    responses: Vec<(String, u16, serde_json::Value)>,
) -> (Account, std::thread::JoinHandle<Vec<String>>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let account = Account {
        api_base: format!("http://{}/v1", listener.local_addr().unwrap()),
        api_key: "local-chat-account".into(),
        given_name: None,
        name: None,
        email: None,
    };
    let task = std::thread::spawn(move || {
        responses.into_iter().map(|(path, status, body)| {
            let (mut stream, _) = listener.accept().unwrap();
            stream.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
            let mut bytes = vec![];
            loop {
                let mut buf = [0; 4096];
                let n = stream.read(&mut buf).unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&buf[..n]);
                if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&bytes[..end]);
                    let length = headers.lines().find_map(|line| {
                        let (key, value) = line.split_once(':')?;
                        key.eq_ignore_ascii_case("content-length").then(|| value.trim().parse::<usize>().unwrap())
                    }).unwrap_or(0);
                    if bytes.len() >= end + 4 + length { break; }
                }
            }
            let request = String::from_utf8(bytes).unwrap();
            assert!(request.starts_with(&path), "{request}");
            assert!(request.to_lowercase().contains("authorization: bearer local-chat-account"));
            let body = body.to_string();
            write!(stream, "HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            request
        }).collect()
    });
    (account, task)
}

fn command_value(status: &str) -> serde_json::Value {
    serde_json::json!({"command_id": COMMAND, "kind":"message", "session_id":"local:work", "status":status})
}

#[test]
fn live_message_roundtrip_never_calls_cloud_attach_or_transfer() {
    let (account, server) = serve(vec![
        (
            "POST /v1/mobile/commands HTTP/1.1".into(),
            202,
            serde_json::json!({"command":command_value("queued"),"host":{"state":"local"}}),
        ),
        (
            format!("GET /v1/mobile/commands/{COMMAND} HTTP/1.1"),
            200,
            serde_json::json!({"command":command_value("running")}),
        ),
        (
            "GET /v1/mobile/sessions/local%3Awork?after_seq=-1 HTTP/1.1".into(),
            200,
            serde_json::json!({"session":{"session_id":"local:work","source":"local","live":true,"busy":true},"messages":[{"seq":0,"role":"user","text":"Continue here"}],"more":true}),
        ),
        (
            "GET /v1/mobile/sessions/local%3Awork?after_seq=0 HTTP/1.1".into(),
            200,
            serde_json::json!({"session":{"session_id":"local:work","source":"local","live":true,"busy":false},"messages":[{"seq":5,"role":"assistant","text":"Done on computer"}],"more":false}),
        ),
        (
            format!("GET /v1/mobile/commands/{COMMAND} HTTP/1.1"),
            200,
            serde_json::json!({"command":command_value("done")}),
        ),
    ]);
    let queued = message(&account, &session(true, true), " Continue here ").unwrap();
    assert!(!queued.message_complete("local:work").unwrap());
    assert!(
        !command(&account, COMMAND)
            .unwrap()
            .message_complete("local:work")
            .unwrap()
    );
    let history = transcript(&account, "local:work").unwrap();
    assert_eq!(history.messages.len(), 2);
    assert_eq!(history.messages[1].content, "Done on computer");
    assert!(history.session.live);
    assert!(!history.session.busy);
    assert!(
        command(&account, COMMAND)
            .unwrap()
            .message_complete("local:work")
            .unwrap()
    );
    let requests = server.join().unwrap();
    let body: serde_json::Value =
        serde_json::from_str(requests[0].split_once("\r\n\r\n").unwrap().1).unwrap();
    assert_eq!(
        body,
        serde_json::json!({"kind":"message","session_id":"local:work","text":"Continue here"})
    );
    assert!(
        requests
            .iter()
            .all(|request| !request.contains("/cloud/") && !request.contains("\"transfer\""))
    );
}

#[test]
fn missing_presence_is_read_only_and_invalid_messages_never_request() {
    let old: SyncedSession =
        serde_json::from_value(serde_json::json!({"session_id":"old","source":"local"})).unwrap();
    assert!(!old.live && !old.busy && !old.can_message());
    let account = Account {
        api_base: "http://127.0.0.1:1".into(),
        api_key: "unused".into(),
        given_name: None,
        name: None,
        email: None,
    };
    assert!(
        message(&account, &old, "hello")
            .unwrap_err()
            .contains("not live")
    );
    assert!(
        message(&account, &session(true, false), "  ")
            .unwrap_err()
            .contains("Enter a message")
    );
    let mut cloud = session(true, false);
    cloud.source = "cloud".into();
    assert!(!cloud.can_message());
    assert!(message(&account, &cloud, "hello").is_err());
}

#[test]
fn message_status_requires_same_session_and_kind_not_new_destination() {
    let mut value = command_value("done");
    let command: Command = serde_json::from_value(value.clone()).unwrap();
    assert!(command.message_complete("local:work").unwrap());
    assert!(command.message_complete("another").is_err());
    assert!(command.destination("local:work").is_err());
    value["kind"] = "transfer".into();
    assert!(
        serde_json::from_value::<Command>(value)
            .unwrap()
            .message_complete("local:work")
            .is_err()
    );
    for status in ["failed", "unexpected"] {
        let c: Command = serde_json::from_value(command_value(status)).unwrap();
        assert!(c.message_complete("local:work").is_err());
    }
}

#[test]
fn stale_presence_rejection_surfaces_without_reposting() {
    let (account, server) = serve(vec![(
        "POST /v1/mobile/commands HTTP/1.1".into(),
        409,
        serde_json::json!({"error":{"code":"device_offline","message":"computer is offline"}}),
    )]);
    assert_eq!(
        message(&account, &session(true, false), "hello").unwrap_err(),
        "computer is offline"
    );
    assert_eq!(server.join().unwrap().len(), 1);
}
