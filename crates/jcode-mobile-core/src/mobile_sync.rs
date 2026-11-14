//! Account-scoped synced transcript browsing and explicit local-to-cloud continuation.
use crate::cloud::{Account, CLIENT_NAME};
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;
use std::time::Instant;

#[cfg(test)]
#[path = "mobile_sync_http_tests.rs"]
mod http_tests;

#[cfg(test)]
#[path = "mobile_live_tests.rs"]
mod live_tests;

pub const SYNC_UNAVAILABLE: &str = "Synced sessions are not available on this server yet";

fn agent() -> &'static ureq::Agent {
    // The agent stores no account headers or transcript results. Its transport
    // pool is origin-scoped, while every request sets its own bearer credential.
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        ureq::Agent::config_builder()
            .timeout_global(Some(std::time::Duration::from_secs(30)))
            .http_status_as_error(false)
            .user_agent(CLIENT_NAME)
            .build()
            .into()
    })
}

struct RequestTiming {
    operation: &'static str,
    started: Instant,
    outcome: &'static str,
}

impl Drop for RequestTiming {
    fn drop(&mut self) {
        // Fixed operation labels only. Never include keys, URLs, IDs or bodies.
        eprintln!(
            "[CLOUD_TIMING] stage=mobile_sync_request operation={} outcome={} duration_ms={:.3}",
            self.operation,
            self.outcome,
            self.started.elapsed().as_secs_f64() * 1000.0
        );
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct SyncedSession {
    #[serde(default)]
    pub live: bool,
    #[serde(default)]
    pub busy: bool,
    pub session_id: String,
    pub title: Option<String>,
    pub source: String,
    pub host: Option<String>,
    pub last_message_preview: Option<String>,
}

/// A read-only account-scoped transcript, in ascending message sequence order.
#[derive(Clone, Debug)]
pub struct SyncedTranscript {
    pub session: SyncedSession,
    pub messages: Vec<SyncedMessage>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct SyncedMessage {
    pub seq: u64,
    pub role: String,
    /// The sync API calls this field `text`.
    #[serde(rename = "text")]
    pub content: String,
    pub created_at: Option<String>,
}

#[derive(Deserialize)]
struct TranscriptPage {
    session: SyncedSession,
    messages: Vec<SyncedMessage>,
    more: bool,
}

fn transcript_path(id: &str) -> Result<String, String> {
    // Match the server's ID grammar, also rejecting URL dot-segment aliases.
    if id.is_empty()
        || id.len() > 128
        || matches!(id, "." | "..")
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._:-".contains(&b))
    {
        return Err("Invalid synced session id".into());
    }
    // Encode even the allowed ':' so the ID remains a single URL path segment.
    Ok(format!("/mobile/sessions/{}", id.replace(':', "%3A")))
}

/// Read one synced session's metadata without fetching the full transcript.
pub fn lookup(account: &Account, id: &str) -> Result<SyncedSession, String> {
    let path = transcript_path(id)?;
    let value = request(
        account,
        "lookup",
        &format!("{path}?limit=1&after_seq=-1"),
        None,
    )?;
    let session: SyncedSession =
        serde_json::from_value(value["session"].clone()).map_err(|e| e.to_string())?;
    if session.session_id != id {
        return Err("Synced transcript returned a different session".into());
    }
    Ok(session)
}

/// Fetch every page using only this account's bearer credentials. This never
/// wakes a cloud host, issues a command, or continues the synced session.
pub fn transcript(account: &Account, id: &str) -> Result<SyncedTranscript, String> {
    let path = transcript_path(id)?;
    let mut messages = Vec::new();
    let mut after_seq: i64 = -1;
    loop {
        let value = request(
            account,
            "transcript",
            &format!("{path}?after_seq={after_seq}"),
            None,
        )?;
        let page: TranscriptPage = serde_json::from_value(value).map_err(|e| e.to_string())?;
        if page.session.session_id != id {
            return Err("Synced transcript returned a different session".into());
        }
        let previous = after_seq;
        for message in page.messages {
            let seq = i64::try_from(message.seq)
                .map_err(|_| "Invalid synced message sequence".to_string())?;
            if seq <= after_seq {
                return Err("Synced transcript pagination did not advance".into());
            }
            after_seq = seq;
            messages.push(message);
        }
        if !page.more {
            return Ok(SyncedTranscript {
                session: page.session,
                messages,
            });
        }
        if after_seq == previous {
            return Err("Synced transcript pagination did not advance".into());
        }
    }
}

impl SyncedSession {
    pub fn can_message(&self) -> bool {
        self.source == "local" && self.live
    }
    pub fn can_transfer(&self) -> bool {
        self.source == "local"
    }
    pub fn label(&self) -> &str {
        self.title.as_deref().unwrap_or(&self.session_id)
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct Command {
    pub command_id: String,
    pub kind: String,
    pub session_id: Option<String>,
    pub status: String,
    pub result_session_id: Option<String>,
    pub error: Option<String>,
}

impl Command {
    /// Local completion never navigates or requires a new cloud session.
    pub fn message_complete(&self, session: &str) -> Result<bool, String> {
        if self.kind != "message" || self.session_id.as_deref() != Some(session) {
            return Err("Message command returned a different session or kind".into());
        }
        match self.status.as_str() {
            "done" => Ok(true),
            "queued" | "running" => Ok(false),
            "failed" => Err(self
                .error
                .clone()
                .unwrap_or_else(|| "Computer message failed".into())),
            _ => Err("Unknown message command status".into()),
        }
    }
    /// Never navigate to the original local transcript or a missing result.
    pub fn destination(&self, source: &str) -> Result<Option<String>, String> {
        match self.status.as_str() {
            "done" => self
                .result_session_id
                .as_ref()
                .filter(|id| !id.is_empty() && id.as_str() != source)
                .cloned()
                .map(Some)
                .ok_or_else(|| "Transfer completed without a new cloud session".into()),
            "failed" => Err(self
                .error
                .clone()
                .unwrap_or_else(|| "Cloud continuation failed".into())),
            "queued" | "running" => Ok(None),
            _ => Err("Unknown cloud command status".into()),
        }
    }
}

#[derive(Serialize)]
struct Transfer<'a> {
    kind: &'static str,
    session_id: &'a str,
    text: &'a str,
}

fn transfer_body<'a>(session: &'a SyncedSession, text: &'a str) -> Result<Transfer<'a>, String> {
    if !session.can_transfer() {
        return Err("Only local synced sessions can be transferred".into());
    }
    if text.trim().is_empty() {
        return Err("Enter a message to continue in cloud".into());
    }
    Ok(Transfer {
        kind: "transfer",
        session_id: &session.session_id,
        text: text.trim(),
    })
}

fn request(
    account: &Account,
    operation: &'static str,
    path: &str,
    body: Option<serde_json::Value>,
) -> Result<serde_json::Value, String> {
    let mut timing = RequestTiming {
        operation,
        started: Instant::now(),
        outcome: "error",
    };
    let url = format!("{}{path}", account.api_base.trim_end_matches('/'));
    let agent = agent();
    let auth = format!("Bearer {}", account.api_key);
    let mut response = match body {
        Some(body) => agent
            .post(&url)
            .header("Authorization", &auth)
            .send_json(body),
        None => agent.get(&url).header("Authorization", &auth).call(),
    }
    .map_err(|e| e.to_string())?;
    let status = response.status().as_u16();
    let value: serde_json::Value = response.body_mut().read_json().map_err(|e| e.to_string())?;
    if status == 404
        && value["error"]["code"] == "not_found"
        && value["error"]["message"]
            .as_str()
            .is_some_and(|m| m.starts_with("no route for "))
    {
        return Err(SYNC_UNAVAILABLE.into());
    }
    if status >= 400 {
        return Err(value["error"]["message"]
            .as_str()
            .unwrap_or("Mobile sync request failed")
            .into());
    }
    timing.outcome = "success";
    Ok(value)
}

pub fn list(account: &Account) -> Result<Vec<SyncedSession>, String> {
    let value = request(account, "list", "/mobile/sessions?limit=500", None)?;
    serde_json::from_value(value["sessions"].clone()).map_err(|e| e.to_string())
}

pub fn transfer(account: &Account, session: &SyncedSession, text: &str) -> Result<Command, String> {
    let body = serde_json::to_value(transfer_body(session, text)?).map_err(|e| e.to_string())?;
    let value = request(account, "transfer", "/mobile/commands", Some(body))?;
    serde_json::from_value(value["command"].clone()).map_err(|e| e.to_string())
}

pub fn message(account: &Account, session: &SyncedSession, text: &str) -> Result<Command, String> {
    if !session.can_message() {
        return Err("This session is not live on your computer".into());
    }
    transcript_path(&session.session_id)?;
    if text.trim().is_empty() {
        return Err("Enter a message".into());
    }
    let value = request(
        account,
        "message",
        "/mobile/commands",
        Some(serde_json::json!({
            "kind": "message", "session_id": session.session_id, "text": text.trim()
        })),
    )?;
    let command: Command =
        serde_json::from_value(value["command"].clone()).map_err(|e| e.to_string())?;
    command.message_complete(&session.session_id)?;
    Ok(command)
}

pub fn command(account: &Account, id: &str) -> Result<Command, String> {
    if id.len() != 36 || !id.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-') {
        return Err("Invalid command id".into());
    }
    let value = request(account, "command", &format!("/mobile/commands/{id}"), None)?;
    serde_json::from_value(value["command"].clone()).map_err(|e| e.to_string())
}

pub fn commands(account: &Account) -> Result<Vec<Command>, String> {
    let value = request(account, "commands", "/mobile/commands", None)?;
    serde_json::from_value(value["commands"].clone()).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lookup_reads_account_scoped_metadata_and_validates_session() {
        let (account, server) = mock_response(
            200,
            serde_json::json!({"session":{"session_id":"local:1","source":"local","title":"Laptop"}}),
        );
        assert_eq!(lookup(&account, "local:1").unwrap().label(), "Laptop");
        let sent = server.join().unwrap();
        assert!(
            sent.starts_with("GET /v1/mobile/sessions/local%3A1?limit=1&after_seq=-1 HTTP/1.1")
        );
        assert!(
            sent.to_lowercase()
                .contains("authorization: bearer test-account-key")
        );
        let (account, server) = mock_response(
            200,
            serde_json::json!({"session":{"session_id":"other","source":"local"}}),
        );
        assert!(lookup(&account, "local").is_err());
        server.join().unwrap();
        assert!(lookup(&account, "../unsafe").is_err());
    }

    #[test]
    fn transcript_pages_are_read_only_account_scoped_and_complete() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let account = Account {
            given_name: None,
            name: None,
            api_base: format!("http://{}/v1", listener.local_addr().unwrap()),
            api_key: "test-account-key".into(),
            email: None,
        };
        let server = std::thread::spawn(move || {
            for (cursor, rows, more) in [
                (
                    -1,
                    serde_json::json!([{"seq":0,"role":"user","text":"hello"},{"seq":7,"role":"assistant","text":"answer"}]),
                    true,
                ),
                (
                    7,
                    serde_json::json!([{"seq":12,"role":"tool","text":"result"},{"seq":15,"role":"system","text":"context"}]),
                    false,
                ),
            ] {
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
                let sent = String::from_utf8(bytes).unwrap();
                assert!(sent.starts_with(&format!(
                    "GET /v1/mobile/sessions/local%3A1?after_seq={cursor} HTTP/1.1"
                )));
                assert!(
                    sent.to_lowercase()
                        .contains("authorization: bearer test-account-key")
                );
                let body = serde_json::json!({"session":{"session_id":"local:1","source":"local"},"messages":rows,"more":more}).to_string();
                write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            }
        });
        let result = transcript(&account, "local:1").unwrap();
        assert_eq!(result.session.session_id, "local:1");
        assert_eq!(result.messages.len(), 4);
        assert_eq!(result.messages[1].content, "answer");
        assert_eq!(result.messages[2].role, "tool");
        assert_eq!(result.messages[3].seq, 15);
        server.join().unwrap();
    }

    #[test]
    fn transcript_rejects_unsafe_ids_and_invalid_pages() {
        for id in ["", ".", "..", "a/b", "a?x=y", "a#b", "%2f", "非"] {
            assert!(transcript_path(id).is_err(), "{id}");
        }
        assert!(transcript_path(&"a".repeat(129)).is_err());
        for body in [
            serde_json::json!({"session":{"session_id":"local","source":"local"},"messages":[],"more":true}),
            serde_json::json!({"session":{"session_id":"other","source":"local"},"messages":[],"more":false}),
            serde_json::json!({"session":{"session_id":"local","source":"local"},"messages":[{"seq":1,"role":"user","text":"x"},{"seq":1,"role":"assistant","text":"y"}],"more":false}),
        ] {
            let (account, server) = mock_response(200, body);
            assert!(transcript(&account, "local").is_err());
            server.join().unwrap();
        }
        let (account, server) = mock_response(
            404,
            serde_json::json!({"error":{"message":"no such session"}}),
        );
        assert_eq!(
            transcript(&account, "local").unwrap_err(),
            "no such session"
        );
        server.join().unwrap();
    }

    fn mock_response(
        status: u16,
        body: serde_json::Value,
    ) -> (Account, std::thread::JoinHandle<String>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let api_base = format!("http://{}/v1", listener.local_addr().unwrap());
        let task = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .unwrap();
            let mut bytes = Vec::new();
            let mut buf = [0; 4096];
            loop {
                let n = stream.read(&mut buf).unwrap();
                assert!(n > 0, "request ended before body");
                bytes.extend_from_slice(&buf[..n]);
                if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&bytes[..end]);
                    let length = headers
                        .lines()
                        .find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    if bytes.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            let body = body.to_string();
            write!(stream, "HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            String::from_utf8(bytes).unwrap()
        });
        (
            Account {
                given_name: None,
                name: None,
                api_base,
                api_key: "test-account-key".into(),
                email: None,
            },
            task,
        )
    }

    #[test]
    fn http_transfer_enqueues_explicit_kind_and_reads_new_destination() {
        let source: SyncedSession =
            serde_json::from_value(serde_json::json!({"session_id":"local-1","source":"local"}))
                .unwrap();
        let id = "12345678-1234-1234-1234-123456789abc";
        let (account, request) = mock_response(
            202,
            serde_json::json!({"command":{
            "command_id":id,"kind":"transfer","session_id":"local-1","status":"queued","result_session_id":null,"error":null
        },"host":{"state":"starting"}}),
        );
        let queued = transfer(&account, &source, " next ").unwrap();
        assert_eq!(queued.destination("local-1").unwrap(), None);
        let sent = request.join().unwrap();
        assert!(sent.starts_with("POST /v1/mobile/commands HTTP/1.1"));
        assert!(
            sent.to_lowercase()
                .contains("authorization: bearer test-account-key")
        );
        let body: serde_json::Value =
            serde_json::from_str(sent.split_once("\r\n\r\n").unwrap().1).unwrap();
        assert_eq!(
            body,
            serde_json::json!({"kind":"transfer","session_id":"local-1","text":"next"})
        );

        let (account, request) = mock_response(
            200,
            serde_json::json!({"command":{
                "command_id":id,"kind":"transfer","session_id":"local-1","status":"done","result_session_id":"new-cloud","error":null
            }}),
        );
        assert_eq!(
            command(&account, id)
                .unwrap()
                .destination("local-1")
                .unwrap(),
            Some("new-cloud".into())
        );
        assert!(
            request
                .join()
                .unwrap()
                .starts_with(&format!("GET /v1/mobile/commands/{id} HTTP/1.1"))
        );
    }

    #[test]
    fn http_list_and_errors_use_account_scoped_api() {
        let (account, request) = mock_response(
            200,
            serde_json::json!({"sessions":[{"session_id":"local","source":"local"},{"session_id":"cloud","source":"cloud"}]}),
        );
        let sessions = list(&account).unwrap();
        assert!(sessions[0].can_transfer());
        assert!(!sessions[1].can_transfer());
        assert!(
            request
                .join()
                .unwrap()
                .starts_with("GET /v1/mobile/sessions?limit=500 HTTP/1.1")
        );
        let (account, request) = mock_response(
            402,
            serde_json::json!({"error":{"code":"subscription_required","message":"Subscription required"}}),
        );
        assert_eq!(list(&account).unwrap_err(), "Subscription required");
        request.join().unwrap();
    }
    #[test]
    fn missing_sync_route_has_actionable_error() {
        let (account, request) = mock_response(
            404,
            serde_json::json!({"error":{"code":"not_found","message":"no route for GET /v1/mobile/sessions"}}),
        );
        assert_eq!(
            list(&account).unwrap_err(),
            "Synced sessions are not available on this server yet"
        );
        request.join().unwrap();

        let (account, request) = mock_response(
            404,
            serde_json::json!({"error":{"code":"not_found","message":"Command not found"}}),
        );
        assert_eq!(commands(&account).unwrap_err(), "Command not found");
        request.join().unwrap();
    }

    #[test]
    fn transfer_is_explicit_and_local_only() {
        let mut s: SyncedSession =
            serde_json::from_value(serde_json::json!({"session_id":"local-1","source":"local"}))
                .unwrap();
        assert_eq!(
            serde_json::to_value(transfer_body(&s, " next ").unwrap()).unwrap(),
            serde_json::json!({"kind":"transfer","session_id":"local-1","text":"next"})
        );
        assert!(transfer_body(&s, " ").is_err());
        s.source = "cloud".into();
        assert!(transfer_body(&s, "next").is_err());
    }
    #[test]
    fn completion_navigates_only_to_new_cloud_id() {
        let mut c = Command {
            command_id: "id".into(),
            kind: "transfer".into(),
            session_id: Some("local".into()),
            status: "queued".into(),
            result_session_id: None,
            error: None,
        };
        assert_eq!(c.destination("local").unwrap(), None);
        c.status = "running".into();
        assert_eq!(c.destination("local").unwrap(), None);
        c.status = "done".into();
        assert!(c.destination("local").is_err());
        c.result_session_id = Some("local".into());
        assert!(c.destination("local").is_err());
        c.result_session_id = Some("cloud-new".into());
        assert_eq!(c.destination("local").unwrap(), Some("cloud-new".into()));
        c.status = "failed".into();
        c.error = Some("runner failed".into());
        assert_eq!(c.destination("local").unwrap_err(), "runner failed");
    }
}
