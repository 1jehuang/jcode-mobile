//! Subscription-backed dictation, using the same endpoint as jcode-voice.
use crate::Account;

pub const MAX_AUDIO_BYTES: usize = 10 * 1024 * 1024;

pub fn transcribe(account: &Account, wav: Vec<u8>) -> Result<String, String> {
    transcribe_with_timeout(account, wav, 120, false)
}

/// A bounded batch preview, not a streaming or websocket transcription session.
/// The final full-recording transcription remains authoritative.
pub fn transcribe_preview(account: &Account, wav: Vec<u8>) -> Result<String, String> {
    transcribe_with_timeout(account, wav, 10, true)
}

fn transcribe_with_timeout(
    account: &Account,
    wav: Vec<u8>,
    seconds: u64,
    preview: bool,
) -> Result<String, String> {
    if wav.len() < 44
        || wav.len() > MAX_AUDIO_BYTES
        || &wav[..4] != b"RIFF"
        || &wav[8..12] != b"WAVE"
    {
        return Err("The microphone did not produce a valid recording.".into());
    }
    let boundary = format!("jcode-{}", uuid::Uuid::new_v4());
    let mut body = format!("--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"recording.wav\"\r\nContent-Type: audio/wav\r\n\r\n").into_bytes();
    body.extend(wav);
    body.extend(format!("\r\n--{boundary}--\r\n").bytes());
    let config =
        ureq::Agent::config_builder().timeout_global(Some(std::time::Duration::from_secs(seconds)));
    // Do not forward the subscription bearer token through redirects for previews.
    let config = if preview {
        config.max_redirects(0)
    } else {
        config
    };
    let agent: ureq::Agent = config.build().into();
    let mut response = agent
        .post(format!(
            "{}/audio/transcriptions",
            account.api_base.trim_end_matches('/')
        ))
        .header("Authorization", &format!("Bearer {}", account.api_key))
        .header(
            "Content-Type",
            &format!("multipart/form-data; boundary={boundary}"),
        )
        .send(body.as_slice())
        .map_err(|_| {
            "Transcription failed. Check your connection and voice subscription, then try again."
                .to_string()
        })?;
    let value: serde_json::Value = response
        .body_mut()
        .with_config()
        .limit(256 * 1024)
        .read_json()
        .map_err(|_| "The transcription service returned an invalid response.".to_string())?;
    let text = value
        .get("text")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .trim();
    if text.is_empty() {
        return Err("No speech detected. Try recording again.".into());
    }
    Ok(text.to_owned())
}

/// Extract the latest at most eight seconds of mono 16-bit 16 kHz PCM.
/// The cursor counts PCM bytes, excluding all RIFF headers. At least one second
/// of new audio is required. Normal polling includes 400 ms of preceding audio.
/// Stale RIFF/data lengths from a still-open recording are tolerated.
pub fn preview_chunk(wav: &[u8], consumed_pcm_bytes: usize) -> Option<(Vec<u8>, usize)> {
    const SECOND: usize = 16_000 * 2;
    const OVERLAP: usize = SECOND * 400 / 1000;
    if wav.get(..4)? != b"RIFF" || wav.get(8..12)? != b"WAVE" {
        return None;
    }
    let mut offset = 12usize;
    let mut valid_format = false;
    let mut pcm = None;
    while offset.checked_add(8)? <= wav.len() {
        let id = &wav[offset..offset + 4];
        let size = u32::from_le_bytes(wav[offset + 4..offset + 8].try_into().ok()?) as usize;
        let start = offset + 8;
        if id == b"fmt " {
            if size < 16 || start.checked_add(size)? > wav.len() {
                return None;
            }
            let fmt = &wav[start..start + 16];
            valid_format = u16::from_le_bytes(fmt[0..2].try_into().ok()?) == 1
                && u16::from_le_bytes(fmt[2..4].try_into().ok()?) == 1
                && u32::from_le_bytes(fmt[4..8].try_into().ok()?) == 16_000
                && u32::from_le_bytes(fmt[8..12].try_into().ok()?) == 32_000
                && u16::from_le_bytes(fmt[12..14].try_into().ok()?) == 2
                && u16::from_le_bytes(fmt[14..16].try_into().ok()?) == 16;
        } else if id == b"data" {
            let declared_end = start.checked_add(size)?;
            let next = declared_end.checked_add(size % 2)?;
            // A recognizable following chunk makes the declared data boundary
            // authoritative. Otherwise capture writers may not have updated it.
            let follows_chunk = wav
                .get(next..next.checked_add(4)?)
                .is_some_and(|id| matches!(id, b"LIST" | b"JUNK" | b"fact" | b"cue " | b"fmt "));
            let end = if follows_chunk {
                declared_end
            } else {
                wav.len()
            };
            pcm = Some(wav.get(start..end)?);
            break;
        }
        offset = start.checked_add(size)?.checked_add(size % 2)?;
    }
    if !valid_format {
        return None;
    }
    let pcm = pcm?;
    let end = pcm.len() & !1;
    if consumed_pcm_bytes > end || consumed_pcm_bytes % 2 != 0 || end - consumed_pcm_bytes < SECOND
    {
        return None;
    }
    let start = consumed_pcm_bytes
        .saturating_sub(OVERLAP)
        .max(end.saturating_sub(8 * SECOND));
    let samples = &pcm[start..end];
    if samples.iter().all(|byte| *byte == 0) {
        return None;
    }
    let mut chunk = Vec::with_capacity(44 + samples.len());
    chunk.extend_from_slice(b"RIFF");
    chunk.extend_from_slice(&(36 + samples.len() as u32).to_le_bytes());
    chunk.extend_from_slice(b"WAVEfmt \x10\0\0\0\x01\0\x01\0");
    chunk.extend_from_slice(&16_000u32.to_le_bytes());
    chunk.extend_from_slice(&32_000u32.to_le_bytes());
    chunk.extend_from_slice(b"\x02\0\x10\0data");
    chunk.extend_from_slice(&(samples.len() as u32).to_le_bytes());
    chunk.extend_from_slice(samples);
    Some((chunk, end))
}

/// Join batch preview text, removing a suffix/prefix overlap of up to 12 words.
pub fn merge_preview(existing: &str, next: &str) -> String {
    let existing = existing.trim();
    let next = next.trim();
    let old: Vec<_> = existing.split_whitespace().collect();
    let new: Vec<_> = next.split_whitespace().collect();
    let normalize = |word: &str| {
        word.trim_matches(|c: char| !c.is_alphanumeric())
            .to_lowercase()
    };
    let overlap = (1..=12.min(old.len()).min(new.len()))
        .rev()
        .find(|&count| {
            old[old.len() - count..]
                .iter()
                .zip(&new[..count])
                .all(|(a, b)| {
                    let a = normalize(a);
                    !a.is_empty() && a == normalize(b)
                })
        })
        .unwrap_or(0);
    let remainder = new[overlap..].join(" ");
    if remainder.is_empty() {
        existing.to_owned()
    } else {
        append(existing, &remainder)
    }
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

    fn recording(bytes: usize) -> Vec<u8> {
        let mut wav = b"RIFF".to_vec();
        wav.extend_from_slice(&(36 + bytes as u32).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt \x10\0\0\0\x01\0\x01\0");
        wav.extend_from_slice(&16_000u32.to_le_bytes());
        wav.extend_from_slice(&32_000u32.to_le_bytes());
        wav.extend_from_slice(b"\x02\0\x10\0data");
        wav.extend_from_slice(&(bytes as u32).to_le_bytes());
        wav.resize(44 + bytes, 1);
        wav
    }

    #[test]
    fn preview_requires_new_non_silent_second() {
        assert!(preview_chunk(&[], 0).is_none());
        assert!(preview_chunk(&recording(31_998), 0).is_none());
        let wav = recording(64_000);
        assert!(preview_chunk(&wav, 32_002).is_none());
        assert!(preview_chunk(&wav, 64_002).is_none());
        assert!(preview_chunk(&wav, 1).is_none());
        let mut silent = recording(32_000);
        silent[44..].fill(0);
        assert!(preview_chunk(&silent, 0).is_none());
        let (chunk, cursor) = preview_chunk(&wav, 32_000).unwrap();
        assert_eq!(cursor, 64_000);
        assert_eq!(chunk.len(), 44 + 32_000 + 12_800);
        assert_eq!(
            u32::from_le_bytes(chunk[40..44].try_into().unwrap()),
            44_800
        );
        assert!(preview_chunk(&wav, cursor).is_none());
    }

    #[test]
    fn preview_is_bounded_and_rewrites_stale_headers() {
        for stale in [0, 32_000, u32::MAX] {
            let mut wav = recording(320_001);
            wav[4..8].copy_from_slice(&stale.to_le_bytes());
            wav[40..44].copy_from_slice(&stale.to_le_bytes());
            let (chunk, cursor) = preview_chunk(&wav, 0).unwrap();
            assert_eq!(cursor, 320_000);
            assert_eq!(chunk.len(), 44 + 256_000);
            assert_eq!(&chunk[..4], b"RIFF");
            assert_eq!(&chunk[8..16], b"WAVEfmt ");
            assert_eq!(&chunk[36..40], b"data");
            assert_eq!(u32::from_le_bytes(chunk[4..8].try_into().unwrap()), 256_036);
            assert_eq!(
                u32::from_le_bytes(chunk[40..44].try_into().unwrap()),
                256_000
            );
        }
    }

    #[test]
    fn preview_parses_padded_metadata_and_validates_format() {
        let original = recording(32_000);
        let mut wav = original[..12].to_vec();
        wav.extend_from_slice(b"JUNK\x03\0\0\0abc\0");
        wav.extend_from_slice(b"LIST\x04\0\0\0INFO");
        wav.extend_from_slice(&original[12..]);
        wav.extend_from_slice(b"LIST\x04\0\0\0INFO");
        let (chunk, cursor) = preview_chunk(&wav, 0).unwrap();
        assert_eq!(cursor, 32_000);
        assert_eq!(chunk, original);
        for (offset, value) in [(20, 3), (22, 2), (24, 1), (28, 1), (32, 4), (34, 8)] {
            let mut invalid = original.clone();
            invalid[offset] = value;
            assert!(preview_chunk(&invalid, 0).is_none());
        }
        let mut malformed = original[..12].to_vec();
        malformed.extend_from_slice(b"JUNK\xff\xff\xff\xff");
        assert!(preview_chunk(&malformed, 0).is_none());
    }

    #[test]
    fn preview_merge_deduplicates_unicode_case_and_punctuation() {
        assert_eq!(
            merge_preview("Hello, WORLD!", "world again."),
            "Hello, WORLD! again."
        );
        assert_eq!(
            merge_preview("An ÉCOLE «東京». action", "école 東京 ACTION follows"),
            "An ÉCOLE «東京». action follows"
        );
        assert_eq!(merge_preview("Hello", "HELLO!"), "Hello");
        assert_eq!(merge_preview("", "  hello  "), "hello");
        assert_eq!(merge_preview("hello", ""), "hello");
        assert_eq!(merge_preview("one two one", "one two"), "one two one two");
        let words = "a b c d e f g h i j k l m";
        assert_eq!(merge_preview(words, words), format!("{words} {words}"));
        assert_eq!(
            merge_preview("x a b c d e f g h i j k l", "a b c d e f g h i j k l tail"),
            "x a b c d e f g h i j k l tail"
        );
    }

    #[test]
    fn preview_does_not_follow_redirects() {
        use std::io::{Read, Write};
        let destination = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        destination.set_nonblocking(true).unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let target = destination.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .unwrap();
            let mut request = Vec::new();
            let mut buf = [0u8; 4096];
            loop {
                let n = stream.read(&mut buf).unwrap();
                assert!(n > 0);
                request.extend_from_slice(&buf[..n]);
                if let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&request[..end]).to_lowercase();
                    let length: usize = headers
                        .lines()
                        .find_map(|l| l.strip_prefix("content-length: "))
                        .unwrap()
                        .parse()
                        .unwrap();
                    if request.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            write!(stream, "HTTP/1.1 307 Temporary Redirect\r\nLocation: http://{target}/leak\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
        });
        let account = Account {
            api_key: "test-key".into(),
            api_base: base,
            email: None,
        };
        assert!(transcribe_preview(&account, recording(32_000)).is_err());
        server.join().unwrap();
        assert_eq!(
            destination.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
    }
    #[test]
    fn preserves_draft() {
        assert_eq!(append("", "Hello"), "Hello");
        assert_eq!(append("Typed", "spoken"), "Typed spoken");
        assert_eq!(append("Typed\n", "spoken"), "Typed\nspoken");
    }
    #[test]
    fn rejects_invalid_audio_before_network() {
        let account = Account {
            api_key: "unused".into(),
            api_base: "http://127.0.0.1:1".into(),
            email: None,
        };
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
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .unwrap();
            let mut request = Vec::new();
            let mut buf = [0; 1024];
            loop {
                let n = stream.read(&mut buf).unwrap();
                assert!(n > 0);
                request.extend_from_slice(&buf[..n]);
                if let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&request[..end]).to_lowercase();
                    let length: usize = headers
                        .lines()
                        .find_map(|l| l.strip_prefix("content-length: "))
                        .unwrap()
                        .parse()
                        .unwrap();
                    if request.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            let request = String::from_utf8_lossy(&request);
            assert!(request.starts_with("POST /audio/transcriptions HTTP/1.1"));
            assert!(request
                .to_lowercase()
                .contains("authorization: bearer test-key"));
            assert!(request.contains("name=\"file\"; filename=\"recording.wav\""));
            assert!(request.contains("Content-Type: audio/wav"));
            let body = r#"{"text":"  Hello from voice  "}"#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )
            .unwrap();
        });
        let account = Account {
            api_key: "test-key".into(),
            api_base: base,
            email: None,
        };
        let mut wav = vec![0; 44];
        wav[..4].copy_from_slice(b"RIFF");
        wav[8..12].copy_from_slice(b"WAVE");
        assert_eq!(transcribe(&account, wav).unwrap(), "Hello from voice");
        server.join().unwrap();
    }
}
