//! Crash capture: a panic hook writes a report (message, location, backtrace,
//! build number, OS) to the app data dir, and the next launch uploads it to
//! `POST /v1/mobile/crash` and deletes it. The hook runs before unwinding, so
//! it also works with `panic = "abort"` release profiles.

use std::path::{Path, PathBuf};

use crate::cloud::{Account, CLIENT_NAME, DEFAULT_API_BASE};

/// Server cap is 64 KB for the whole JSON body; keep the report under it.
pub const MAX_REPORT_BYTES: usize = 60 * 1024;
const FILE_NAME: &str = "crash-report.txt";

/// Where the report lives: `~/Documents` on iOS (the app container), the XDG
/// data dir elsewhere.
pub fn default_path() -> PathBuf {
    #[cfg(target_os = "ios")]
    let base = std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Documents"));
    #[cfg(not(target_os = "ios"))]
    let base = dirs::data_dir().map(|d| d.join("jcode-mobile"));
    base.unwrap_or_else(std::env::temp_dir).join(FILE_NAME)
}

/// CFBundleVersion from the app bundle's Info.plist next to the executable.
pub fn bundle_version() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    let text = std::fs::read_to_string(exe.parent()?.join("Info.plist")).ok()?;
    plist_string(&text, "CFBundleVersion")
}

fn plist_string(text: &str, key: &str) -> Option<String> {
    let rest = &text[text.find(&format!("<key>{key}</key>"))?..];
    let start = rest.find("<string>")? + "<string>".len();
    let end = rest[start..].find("</string>")?;
    Some(rest[start..start + end].trim().to_string())
}

pub fn os_description() -> String {
    format!("{} {}", std::env::consts::OS, std::env::consts::ARCH)
}

/// Formats one report. Truncates the backtrace so the file fits the server cap.
pub fn format_report(message: &str, location: Option<&str>, backtrace: &str, build: &str, os: &str) -> String {
    let thread = std::thread::current();
    let mut out = format!(
        "panic: {message}\nlocation: {}\nthread: {}\nbuild: {build}\nos: {os}\ntime_unix: {}\n\nbacktrace:\n",
        location.unwrap_or("unknown"),
        thread.name().unwrap_or("<unnamed>"),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
    );
    out.push_str(backtrace);
    truncate_utf8(&mut out, MAX_REPORT_BYTES);
    out
}

fn truncate_utf8(s: &mut String, max: usize) {
    if s.len() <= max {
        return;
    }
    let mut cut = max;
    while !s.is_char_boundary(cut) {
        cut -= 1;
    }
    s.truncate(cut);
}

fn payload_message(info: &std::panic::PanicHookInfo<'_>) -> String {
    let p = info.payload();
    p.downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| p.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "<non-string panic payload>".into())
}

/// Installs the hook writing to `path`, then chains to the previous hook.
pub fn install_hook(path: PathBuf) {
    let build = bundle_version().unwrap_or_else(|| "dev".into());
    let os = os_description();
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let loc = info.location().map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()));
        let bt = std::backtrace::Backtrace::force_capture().to_string();
        let report = format_report(&payload_message(info), loc.as_deref(), &bt, &build, &os);
        let _ = write_report(&path, &report);
        previous(info);
    }));
}

/// Keeps the first panic of a run (later ones are usually fallout).
fn write_report(path: &Path, report: &str) -> std::io::Result<()> {
    if path.exists() {
        return Ok(());
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, report)
}

/// Uploads a pending report, deleting it on success (or on a permanent 4xx
/// rejection, so a bad file cannot retry forever).
pub fn upload_pending(path: &Path, account: Option<&Account>) -> Result<bool, String> {
    let Ok(report) = std::fs::read_to_string(path) else { return Ok(false) };
    let build = field(&report, "build: ").unwrap_or_default();
    let os = field(&report, "os: ").unwrap_or_default();
    let base = account.map(|a| a.api_base.as_str()).unwrap_or(DEFAULT_API_BASE).trim_end_matches('/');
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(20)))
        .http_status_as_error(false)
        .user_agent(CLIENT_NAME)
        .build()
        .into();
    let mut req = agent.post(&format!("{base}/mobile/crash"));
    if let Some(a) = account {
        req = req.header("Authorization", &format!("Bearer {}", a.api_key));
    }
    let status = req
        .send_json(serde_json::json!({ "report": report, "build": build, "os": os, "app_version": env!("CARGO_PKG_VERSION") }))
        .map_err(|e| e.to_string())?
        .status()
        .as_u16();
    if (200..300).contains(&status) || (400..500).contains(&status) && status != 429 {
        let _ = std::fs::remove_file(path);
    }
    if (200..300).contains(&status) { Ok(true) } else { Err(format!("HTTP {status}")) }
}

fn field(report: &str, prefix: &str) -> Option<String> {
    report.lines().find_map(|l| l.strip_prefix(prefix)).map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plist_parsing() {
        let p = "<dict>\n  <key>CFBundleShortVersionString</key><string>1.0</string>\n  <key>CFBundleVersion</key><string>1793919915</string>\n</dict>";
        assert_eq!(plist_string(p, "CFBundleVersion").as_deref(), Some("1793919915"));
        assert_eq!(plist_string(p, "Missing"), None);
    }

    #[test]
    fn report_is_capped_and_parsable() {
        let bt = "é".repeat(MAX_REPORT_BYTES);
        let r = format_report("boom", Some("src/a.rs:1:2"), &bt, "42", "ios aarch64");
        assert!(r.len() <= MAX_REPORT_BYTES);
        assert!(r.starts_with("panic: boom\nlocation: src/a.rs:1:2\n"));
        assert_eq!(field(&r, "build: ").as_deref(), Some("42"));
        assert_eq!(field(&r, "os: ").as_deref(), Some("ios aarch64"));
    }

    #[test]
    fn hook_writes_report_and_chains() {
        let dir = std::env::temp_dir().join(format!("jcode-crash-{}", std::process::id()));
        let path = dir.join(FILE_NAME);
        let _ = std::fs::remove_dir_all(&dir);
        install_hook(path.clone());
        let r = std::thread::spawn(|| panic!("test panic 123")).join();
        let _ = std::panic::take_hook();
        assert!(r.is_err());
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("panic: test panic 123"), "{text}");
        assert!(text.contains("crash.rs"), "{text}");
        assert!(text.contains("backtrace:\n"));
        // A second panic does not overwrite the first report.
        write_report(&path, "other").unwrap();
        assert!(std::fs::read_to_string(&path).unwrap().contains("test panic 123"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn upload_without_file_is_noop() {
        assert_eq!(upload_pending(Path::new("/nonexistent/crash.txt"), None), Ok(false));
    }
}
