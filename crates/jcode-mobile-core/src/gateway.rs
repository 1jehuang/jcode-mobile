//! Gateway endpoints, `jcode://pair` URIs, `POST /pair` and `GET /health`.
//! Port of Gateway.swift + Pairing.swift. The gateway speaks plain HTTP/1.1
//! with `Connection: close`, so a tiny std-only client is enough and keeps the
//! iOS build free of an async runtime and TLS stack.

use serde_json::{Value, json};
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

pub const DEFAULT_PORT: u16 = 7643;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Gateway {
    pub host: String,
    pub port: u16,
}

impl Gateway {
    pub fn new(host: impl Into<String>, port: u16) -> Self {
        Self { host: host.into(), port }
    }
    fn authority(&self) -> String {
        if self.host.contains(':') && !self.host.starts_with('[') {
            format!("[{}]:{}", self.host, self.port)
        } else {
            format!("{}:{}", self.host, self.port)
        }
    }
    pub fn web_socket_url(&self) -> String {
        format!("ws://{}/ws", self.authority())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PairPayload {
    pub gateway: Gateway,
    pub code: String,
}

/// Parses `jcode://pair?host=H&port=P&code=C`.
pub fn parse_pair_uri(s: &str) -> Option<PairPayload> {
    let rest = s.trim().strip_prefix("jcode:")?;
    let rest = rest.trim_start_matches('/');
    let (path, query) = rest.split_once('?')?;
    if path.trim_end_matches('/') != "pair" {
        return None;
    }
    let mut host = None;
    let mut port = None;
    let mut code = None;
    for pair in query.split('&') {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        let v = percent_decode(v);
        match k {
            "host" => host = Some(v),
            "port" => port = v.parse().ok(),
            "code" => code = Some(v),
            _ => {}
        }
    }
    let host = host.filter(|h| !h.is_empty())?;
    let code = code.filter(|c| !c.is_empty())?;
    Some(PairPayload { gateway: Gateway::new(host, port.unwrap_or(DEFAULT_PORT)), code })
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Ok(v) = u8::from_str_radix(std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or(""), 16)
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(if b[i] == b'+' { b' ' } else { b[i] });
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[derive(Debug, Clone, PartialEq)]
pub struct PairResponse {
    pub token: String,
    pub server_name: String,
    pub server_version: String,
    pub working_dir: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PairError {
    InvalidCode(String),
    Server { status: u16, message: String },
    InvalidResponse,
    Network(String),
}

impl std::fmt::Display for PairError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PairError::InvalidCode(m) => write!(f, "Invalid pairing code: {m}"),
            PairError::Server { status, message } => write!(f, "Server error {status}: {message}"),
            PairError::InvalidResponse => write!(f, "The server sent an unexpected response"),
            PairError::Network(m) => write!(f, "Could not reach server: {m}"),
        }
    }
}

/// Minimal HTTP/1.1 request. Returns (status, body).
pub(crate) fn http(
    gateway: &Gateway,
    method: &str,
    path: &str,
    body: Option<&str>,
    timeout: Duration,
) -> std::io::Result<(u16, String)> {
    let addr = (gateway.host.as_str(), gateway.port)
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| std::io::Error::other("host did not resolve"))?;
    let mut stream = TcpStream::connect_timeout(&addr, timeout)?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;
    let body = body.unwrap_or("");
    let req = format!(
        "{method} {path} HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        gateway.authority(),
        body.len()
    );
    stream.write_all(req.as_bytes())?;
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw)?;
    let text = String::from_utf8_lossy(&raw);
    let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    let status = head
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| std::io::Error::other("malformed HTTP response"))?;
    Ok((status, body.to_string()))
}

pub fn pair(
    gateway: &Gateway,
    code: &str,
    device_id: &str,
    device_name: &str,
) -> Result<PairResponse, PairError> {
    let body = json!({"code": code, "device_id": device_id, "device_name": device_name}).to_string();
    let (status, body) = http(gateway, "POST", "/pair", Some(&body), Duration::from_secs(15))
        .map_err(|e| PairError::Network(e.to_string()))?;
    let obj: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
    let get = |k: &str| obj.get(k).and_then(Value::as_str).map(str::to_owned);
    if status != 200 {
        let message = get("error").unwrap_or_else(|| format!("HTTP {status}"));
        return Err(if status == 401 {
            PairError::InvalidCode(message)
        } else {
            PairError::Server { status, message }
        });
    }
    let token = get("token").filter(|t| !t.is_empty()).ok_or(PairError::InvalidResponse)?;
    Ok(PairResponse {
        token,
        server_name: get("server_name").unwrap_or_else(|| "jcode".into()),
        server_version: get("server_version").unwrap_or_else(|| "unknown".into()),
        working_dir: get("working_dir").filter(|w| !w.is_empty()),
    })
}

/// `GET /health`, returning the parsed body when reachable.
pub fn health(gateway: &Gateway) -> Option<Value> {
    let (status, body) = http(gateway, "GET", "/health", None, Duration::from_secs(5)).ok()?;
    (status == 200).then(|| serde_json::from_str(&body).unwrap_or(json!({})))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_pair_uris() {
        let p = parse_pair_uri("jcode://pair?host=10.0.0.5&port=9000&code=123456").unwrap();
        assert_eq!(p.gateway, Gateway::new("10.0.0.5", 9000));
        assert_eq!(p.code, "123456");
        let p = parse_pair_uri("jcode://pair?host=my%2Dbox.ts.net&code=1").unwrap();
        assert_eq!(p.gateway.host, "my-box.ts.net");
        assert_eq!(p.gateway.port, DEFAULT_PORT);
        assert!(parse_pair_uri("jcode://pair?code=1").is_none());
        assert!(parse_pair_uri("https://pair?host=a&code=1").is_none());
    }

    #[test]
    fn pairs_against_a_fake_gateway() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut buf = [0u8; 4096];
            let n = s.read(&mut buf).unwrap();
            let req = String::from_utf8_lossy(&buf[..n]);
            assert!(req.starts_with("POST /pair"));
            assert!(req.contains(r#""code":"42""#));
            let body = r#"{"token":"tok","server_name":"box","server_version":"v1","working_dir":"/home/j"}"#;
            write!(s, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}", body.len()).unwrap();
        });
        let r = pair(&Gateway::new("127.0.0.1", port), "42", "dev", "Phone").unwrap();
        assert_eq!(r.token, "tok");
        assert_eq!(r.working_dir.as_deref(), Some("/home/j"));
    }
}
