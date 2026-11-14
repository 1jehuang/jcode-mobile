//! Sign the cloud machine back in to OpenAI from the phone.
//!
//! Uses jcode's device-code login (`jcode login --provider openai
//! --device-code`). Every remote command is a fixed string built here, run
//! over the pinned inspection SSH connection, and finishes within the
//! eight-second inspection budget:
//!
//! 1. [`start`] asks the machine for a code (`--print-auth-url --json`).
//! 2. It then detaches `--complete`, which waits for approval on the machine
//!    and saves the tokens there. Tokens never reach the phone.
//! 3. [`status`] reads the detached process's result until it finishes.
//!
//! The person approves the code at [`DeviceCode::url`] in the system sign-in
//! sheet. The running daemon is told about the new login by the CLI itself.

use crate::cloud::{self, Account};
use serde::Deserialize;

const FLOW_ID: &str = "mobile";
const PREFIX: &str = r#"PATH="$HOME/.local/bin:$HOME/.cargo/bin:$PATH"; export PATH; D="$HOME/.jcode/pending-login/flows/mobile"; mkdir -p "$D"; "#;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceCode {
    pub url: String,
    pub user_code: String,
    pub expires_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoginStatus {
    Waiting,
    SignedIn,
    Failed(String),
}

fn start_command() -> String {
    format!(
        "{PREFIX}jcode --no-update login --provider openai --device-code --print-auth-url --json --flow-id {FLOW_ID} 2>/dev/null | tail -n 1 && \
         (rm -f \"$D/result\" \"$D/err\" \"$D/exit\"; nohup sh -c 'jcode --no-update login --provider openai --complete --json --flow-id {FLOW_ID} >\"$0/result.tmp\" 2>\"$0/err\"; echo $? >\"$0/exit\"; mv \"$0/result.tmp\" \"$0/result\"' \"$D\" </dev/null >/dev/null 2>&1 &)"
    )
}

fn status_command() -> String {
    format!(
        "{PREFIX}if [ -f \"$D/exit\" ] && [ -f \"$D/result\" ]; then echo \"exit $(cat \"$D/exit\")\"; tail -n 1 \"$D/result\"; tail -c 300 \"$D/err\" | tail -n 1; else echo waiting; fi"
    )
}

pub(crate) fn parse_start(bytes: &[u8]) -> Result<DeviceCode, String> {
    #[derive(Deserialize)]
    struct Prompt {
        status: String,
        auth_url: String,
        user_code: Option<String>,
        expires_at_ms: i64,
    }
    let text = String::from_utf8_lossy(bytes);
    let line = text.lines().rev().find(|l| l.trim_start().starts_with('{'))
        .ok_or("Your cloud machine did not return a sign-in code. It may need a jcode update.")?;
    let p: Prompt = serde_json::from_str(line).map_err(|_| "Unexpected sign-in reply from your cloud machine.".to_string())?;
    let code = p.user_code.filter(|c| !c.is_empty() && c.len() <= 32 && !c.chars().any(char::is_control));
    if p.status != "pending" || !p.auth_url.starts_with("https://auth.openai.com/") {
        return Err("Unexpected sign-in reply from your cloud machine.".into());
    }
    let user_code = code.ok_or("Your cloud machine did not return a sign-in code. It may need a jcode update.")?;
    Ok(DeviceCode { url: p.auth_url, user_code, expires_at_ms: p.expires_at_ms })
}

pub(crate) fn parse_status(bytes: &[u8]) -> LoginStatus {
    let text = String::from_utf8_lossy(bytes);
    let mut lines = text.lines();
    match lines.next().map(str::trim) {
        Some("waiting") | None => LoginStatus::Waiting,
        Some(first) => {
            let ok = first == "exit 0";
            let rest: Vec<&str> = lines.collect();
            if ok && rest.iter().any(|l| l.contains("\"status\":\"authenticated\"")) {
                return LoginStatus::SignedIn;
            }
            let reason = rest.iter().rev().map(|l| l.trim()).find(|l| !l.is_empty() && !l.starts_with('{'))
                .map(|l| l.trim_start_matches("Error: ").chars().take(200).collect::<String>())
                .unwrap_or_else(|| "Sign-in did not finish. Try again.".into());
            LoginStatus::Failed(reason)
        }
    }
}

/// Ask the cloud machine for a code and start waiting for approval there.
pub fn start(account: &Account) -> Result<DeviceCode, String> {
    let ssh = cloud::inspection_ssh(account)?;
    let out = ssh.run_builtin(start_command())
        .map_err(|_| "Could not start sign-in on your cloud machine. It may need a jcode update.".to_string())?;
    parse_start(&out)
}

/// Read whether the detached login finished.
pub fn status(account: &Account) -> Result<LoginStatus, String> {
    let ssh = cloud::inspection_ssh(account)?;
    let out = ssh.run_builtin(status_command())?;
    Ok(parse_status(&out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_cli_prompt_json() {
        let out = br#"{"status":"pending","provider":"openai","auth_url":"https://auth.openai.com/codex/device","input_kind":"complete","pending_path":"/x","user_code":"ABCD-1234","expires_at_ms":5,"resume_command":"jcode login"}
"#;
        let code = parse_start(out).unwrap();
        assert_eq!(code.user_code, "ABCD-1234");
        assert_eq!(code.url, "https://auth.openai.com/codex/device");
        assert!(parse_start(b"error: unknown flag --device-code\n").is_err());
        assert!(parse_start(br#"{"status":"pending","auth_url":"https://evil.example/","user_code":"A","expires_at_ms":1}"#).is_err());
    }

    #[test]
    fn parses_status() {
        assert_eq!(parse_status(b"waiting\n"), LoginStatus::Waiting);
        assert_eq!(parse_status(b"exit 0\n{\"status\":\"authenticated\",\"provider\":\"openai\"}\n\n"), LoginStatus::SignedIn);
        assert_eq!(
            parse_status(b"exit 1\n\nError: The OpenAI sign-in code expired. Start sign-in again.\n"),
            LoginStatus::Failed("The OpenAI sign-in code expired. Start sign-in again.".into())
        );
    }

    #[test]
    fn commands_are_fixed_strings() {
        assert!(start_command().contains("--device-code --print-auth-url --json --flow-id mobile"));
        assert!(status_command().contains("pending-login/flows/mobile"));
    }
}
