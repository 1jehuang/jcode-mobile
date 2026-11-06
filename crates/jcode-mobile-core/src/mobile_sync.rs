//! Account-scoped synced transcript browsing and explicit local-to-cloud continuation.
use crate::cloud::{Account, CLIENT_NAME};
use serde::{Deserialize, Serialize};

pub const SYNC_UNAVAILABLE: &str = "Synced sessions are not available on this server yet";

#[derive(Clone, Debug, Deserialize)]
pub struct SyncedSession {
    pub session_id: String,
    pub title: Option<String>,
    pub source: String,
    pub host: Option<String>,
    pub last_message_preview: Option<String>,
}

impl SyncedSession {
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
    path: &str,
    body: Option<serde_json::Value>,
) -> Result<serde_json::Value, String> {
    let url = format!("{}{path}", account.api_base.trim_end_matches('/'));
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(30)))
        .http_status_as_error(false)
        .user_agent(CLIENT_NAME)
        .build()
        .into();
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
    Ok(value)
}

pub fn list(account: &Account) -> Result<Vec<SyncedSession>, String> {
    let value = request(account, "/mobile/sessions?limit=500", None)?;
    serde_json::from_value(value["sessions"].clone()).map_err(|e| e.to_string())
}

pub fn transfer(account: &Account, session: &SyncedSession, text: &str) -> Result<Command, String> {
    let body = serde_json::to_value(transfer_body(session, text)?).map_err(|e| e.to_string())?;
    let value = request(account, "/mobile/commands", Some(body))?;
    serde_json::from_value(value["command"].clone()).map_err(|e| e.to_string())
}

pub fn command(account: &Account, id: &str) -> Result<Command, String> {
    if id.len() != 36 || !id.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-') {
        return Err("Invalid command id".into());
    }
    let value = request(account, &format!("/mobile/commands/{id}"), None)?;
    serde_json::from_value(value["command"].clone()).map_err(|e| e.to_string())
}

pub fn commands(account: &Account) -> Result<Vec<Command>, String> {
    let value = request(account, "/mobile/commands", None)?;
    serde_json::from_value(value["commands"].clone()).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
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
