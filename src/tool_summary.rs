//! Tool-call headlines from streamed (possibly partial) JSON input. Port of
//! `JCodeKit/ToolCallSummary.swift`: prefer the agent's stated `intent`, then
//! what the call operates on.

use serde_json::Value;

const SUBJECT_KEYS: [&str; 7] = ["command", "file_path", "path", "query", "url", "pattern", "target"];

pub fn headline(input: &str) -> Option<String> {
    intent(input).or_else(|| subject(input))
}

pub fn intent(input: &str) -> Option<String> {
    if input.is_empty() {
        return None;
    }
    complete(input, "intent").or_else(|| partial(input, "intent")).map(|s| clean(&s))
}

pub fn subject(input: &str) -> Option<String> {
    if input.is_empty() {
        return None;
    }
    for k in SUBJECT_KEYS {
        if let Some(v) = complete(input, k) {
            return Some(clean(&v));
        }
    }
    for k in SUBJECT_KEYS {
        if let Some(v) = partial(input, k) {
            return Some(clean(&v));
        }
    }
    (!input.starts_with('{')).then(|| clean(input))
}

fn complete(input: &str, key: &str) -> Option<String> {
    let v: Value = serde_json::from_str(input).ok()?;
    v.get(key)?.as_str().filter(|s| !s.is_empty()).map(str::to_owned)
}

fn partial(input: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\"");
    let at = input.find(&needle)? + needle.len();
    let rest = input[at..].trim_start().strip_prefix(':')?.trim_start().strip_prefix('"')?;
    let mut out = String::new();
    let mut chars = rest.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => break,
            '\\' => match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some(o) => out.push(o),
                None => break,
            },
            c => out.push(c),
        }
    }
    (!out.is_empty()).then_some(out)
}

fn clean(s: &str) -> String {
    let flat: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() > 200 { flat.chars().take(200).collect::<String>() + "…" } else { flat }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn prefers_intent_and_handles_partial() {
        assert_eq!(headline(r#"{"command":"ls","intent":"list files"}"#).unwrap(), "list files");
        assert_eq!(headline(r#"{"command":"cargo te"#).unwrap(), "cargo te");
        assert_eq!(headline(r#"{"intent":"check the bu"#).unwrap(), "check the bu");
        assert_eq!(headline("{").as_deref(), None);
    }
}
