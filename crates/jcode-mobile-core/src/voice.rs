//! Subscription-backed dictation, using the same endpoint as jcode-voice.
use crate::Account;

pub const MAX_AUDIO_BYTES: usize = 10 * 1024 * 1024;

pub fn transcribe(account: &Account, wav: Vec<u8>) -> Result<String, String> {
    if wav.len() < 44 || wav.len() > MAX_AUDIO_BYTES || &wav[..4] != b"RIFF" || &wav[8..12] != b"WAVE" {
        return Err("The microphone did not produce a valid recording.".into());
    }
    let boundary = format!("jcode-{}", uuid::Uuid::new_v4());
    let mut body = format!("--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"recording.wav\"\r\nContent-Type: audio/wav\r\n\r\n").into_bytes();
    body.extend(wav);
    body.extend(format!("\r\n--{boundary}--\r\n").bytes());
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(120)))
        .build().into();
    let mut response = agent.post(format!("{}/audio/transcriptions", account.api_base.trim_end_matches('/')))
        .header("Authorization", &format!("Bearer {}", account.api_key))
        .header("Content-Type", &format!("multipart/form-data; boundary={boundary}"))
        .send(body.as_slice())
        .map_err(|_| "Transcription failed. Check your connection and voice subscription, then try again.".to_string())?;
    let value: serde_json::Value = response.body_mut().with_config().limit(256 * 1024).read_json()
        .map_err(|_| "The transcription service returned an invalid response.".to_string())?;
    let text = value.get("text").and_then(|v| v.as_str()).unwrap_or_default().trim();
    if text.is_empty() { return Err("No speech detected. Try recording again.".into()); }
    Ok(text.to_owned())
}

/// Preserve the user's current typing. Dictation never submits a message.
pub fn append(draft: &str, text: &str) -> String {
    if draft.is_empty() || draft.ends_with(char::is_whitespace) {
        format!("{draft}{text}")
    } else {
        format!("{draft} {text}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preserves_draft() {
        assert_eq!(append("", "Hello"), "Hello");
        assert_eq!(append("Typed", "spoken"), "Typed spoken");
        assert_eq!(append("Typed\n", "spoken"), "Typed\nspoken");
    }
    #[test]
    fn rejects_invalid_audio_before_network() {
        let account = Account { api_key: "unused".into(), api_base: "http://127.0.0.1:1".into(), email: None };
        assert!(transcribe(&account, vec![]).is_err());
        assert!(transcribe(&account, vec![0; 44]).is_err());
    }

    #[test]
    fn uploads_authenticated_multipart_and_reads_transcript() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
            let mut request = Vec::new();
            let mut buf = [0; 1024];
            loop {
                let n = stream.read(&mut buf).unwrap();
                assert!(n > 0);
                request.extend_from_slice(&buf[..n]);
                if let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&request[..end]).to_lowercase();
                    let length: usize = headers.lines().find_map(|l| l.strip_prefix("content-length: ")).unwrap().parse().unwrap();
                    if request.len() >= end + 4 + length { break; }
                }
            }
            let request = String::from_utf8_lossy(&request);
            assert!(request.starts_with("POST /audio/transcriptions HTTP/1.1"));
            assert!(request.to_lowercase().contains("authorization: bearer test-key"));
            assert!(request.contains("name=\"file\"; filename=\"recording.wav\""));
            assert!(request.contains("Content-Type: audio/wav"));
            let body = r#"{"text":"  Hello from voice  "}"#;
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
        });
        let account = Account { api_key: "test-key".into(), api_base: base, email: None };
        let mut wav = vec![0; 44];
        wav[..4].copy_from_slice(b"RIFF");
        wav[8..12].copy_from_slice(b"WAVE");
        assert_eq!(transcribe(&account, wav).unwrap(), "Hello from voice");
        server.join().unwrap();
    }
}
