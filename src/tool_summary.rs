//! Tool-call headlines from streamed (possibly partial) JSON input. Port of
//! `JCodeKit/ToolCallSummary.swift`: prefer the agent's stated `intent`, then
//! what the call operates on.

use serde_json::Value;

const SUBJECT_KEYS: [&str; 7] = [
    "command",
    "file_path",
    "path",
    "query",
    "url",
    "pattern",
    "target",
];
const HEADLINE_CHARS: usize = 200;

pub fn headline(input: &str) -> Option<String> {
    if input.is_empty() {
        return None;
    }
    // Parse once, not once per candidate key. Keep fallback behavior for partial
    // JSON (including its intentionally permissive escape handling).
    let parsed = serde_json::from_str::<Value>(input).ok();
    intent_from(input, parsed.as_ref()).or_else(|| subject_from(input, parsed.as_ref()))
}

pub fn intent(input: &str) -> Option<String> {
    let parsed = serde_json::from_str::<Value>(input).ok();
    intent_from(input, parsed.as_ref())
}

pub fn subject(input: &str) -> Option<String> {
    if input.is_empty() {
        return None;
    }
    let parsed = serde_json::from_str::<Value>(input).ok();
    subject_from(input, parsed.as_ref())
}

fn intent_from(input: &str, parsed: Option<&Value>) -> Option<String> {
    complete(parsed, "intent")
        .map(clean)
        .or_else(|| partial(input, "intent"))
}

fn subject_from(input: &str, parsed: Option<&Value>) -> Option<String> {
    for key in SUBJECT_KEYS {
        if let Some(value) = complete(parsed, key) {
            return Some(clean(value));
        }
    }
    for key in SUBJECT_KEYS {
        if let Some(value) = partial(input, key) {
            return Some(value);
        }
    }
    (!input.is_empty() && !input.starts_with('{')).then(|| clean(input))
}

fn complete<'a>(parsed: Option<&'a Value>, key: &str) -> Option<&'a str> {
    parsed?.get(key)?.as_str().filter(|s| !s.is_empty())
}

fn partial(input: &str, key: &str) -> Option<String> {
    // All keys are static and short. No input-sized intermediate string.
    let needle = format!("\"{key}\"");
    let at = input.find(&needle)? + needle.len();
    let rest = input[at..]
        .trim_start()
        .strip_prefix(':')?
        .trim_start()
        .strip_prefix('"')?;
    let mut chars = rest.chars();
    let mut ended = false;
    let decoded = std::iter::from_fn(move || {
        if ended {
            return None;
        }
        match chars.next()? {
            '"' => {
                ended = true;
                None
            }
            '\\' => match chars.next()? {
                'n' => Some('\n'),
                't' => Some('\t'),
                other => Some(other),
            },
            c => Some(c),
        }
    });
    let mut decoded = decoded.peekable();
    // A nonempty whitespace-only value still yields Some("") as before.
    decoded.peek()?;
    Some(clean_chars(decoded))
}

fn clean(s: &str) -> String {
    clean_chars(s.chars())
}

/// Collapse Unicode whitespace while retaining at most 200 scalar values and
/// an ellipsis. Stop at the first additional non-whitespace character, without
/// allocating a flattened copy of the entire input.
fn clean_chars(chars: impl Iterator<Item = char>) -> String {
    let mut out = String::with_capacity(HEADLINE_CHARS + 3);
    let mut count = 0;
    let mut pending_space = false;
    for c in chars {
        if c.is_whitespace() {
            pending_space = count != 0;
            continue;
        }
        if pending_space {
            if count == HEADLINE_CHARS {
                out.push('…');
                return out;
            }
            out.push(' ');
            count += 1;
            pending_space = false;
        }
        if count == HEADLINE_CHARS {
            out.push('…');
            return out;
        }
        out.push(c);
        count += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn old_clean(s: &str) -> String {
        let flat = s.split_whitespace().collect::<Vec<_>>().join(" ");
        if flat.chars().count() > 200 {
            flat.chars().take(200).collect::<String>() + "…"
        } else {
            flat
        }
    }

    fn old_complete(input: &str, key: &str) -> Option<String> {
        let value: Value = serde_json::from_str(input).ok()?;
        value
            .get(key)?
            .as_str()
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    }

    fn old_partial(input: &str, key: &str) -> Option<String> {
        let needle = format!("\"{key}\"");
        let at = input.find(&needle)? + needle.len();
        let rest = input[at..]
            .trim_start()
            .strip_prefix(':')?
            .trim_start()
            .strip_prefix('"')?;
        let mut out = String::new();
        let mut chars = rest.chars();
        while let Some(c) = chars.next() {
            match c {
                '"' => break,
                '\\' => match chars.next() {
                    Some('n') => out.push('\n'),
                    Some('t') => out.push('\t'),
                    Some(other) => out.push(other),
                    None => break,
                },
                c => out.push(c),
            }
        }
        (!out.is_empty()).then_some(out)
    }

    fn old_headline(input: &str) -> Option<String> {
        if input.is_empty() {
            return None;
        }
        if let Some(value) = old_complete(input, "intent").or_else(|| old_partial(input, "intent"))
        {
            return Some(old_clean(&value));
        }
        for key in SUBJECT_KEYS {
            if let Some(value) = old_complete(input, key) {
                return Some(old_clean(&value));
            }
        }
        for key in SUBJECT_KEYS {
            if let Some(value) = old_partial(input, key) {
                return Some(old_clean(&value));
            }
        }
        (!input.starts_with('{')).then(|| old_clean(input))
    }

    #[test]
    fn prefers_intent_and_handles_partial() {
        assert_eq!(
            headline(r#"{"command":"ls","intent":"list files"}"#).unwrap(),
            "list files"
        );
        assert_eq!(headline(r#"{"command":"cargo te"#).unwrap(), "cargo te");
        assert_eq!(
            headline(r#"{"intent":"check the bu"#).unwrap(),
            "check the bu"
        );
        assert_eq!(headline("{").as_deref(), None);
    }

    #[test]
    fn preserves_old_behavior_at_every_stream_boundary() {
        for input in [
            r#"{"command":"ls","intent":"list files"}"#,
            r#"{"intent":"","target":"你好🦀","query":"x"}"#,
            r#"{"intent":"a\n\t\r\u1234\\\"z","command":"ls"}"#,
            r#"{"nested":{"command":"nested"},"target":"top"}"#,
            r#"{"intent":null,"command":42,"path":" p  q "}"#,
            " plain\u{2003}text\n ",
            "{\"intent\":\"   ",
            " {\"intent\":false}",
        ] {
            for end in (0..=input.len()).filter(|&end| input.is_char_boundary(end)) {
                let prefix = &input[..end];
                assert_eq!(headline(prefix), old_headline(prefix), "prefix {prefix:?}");
            }
        }
    }

    #[test]
    fn bounded_clean_matches_unicode_and_space_boundaries() {
        for n in [0, 1, 198, 199, 200, 201, 100_000] {
            for suffix in ["", "  ", " x", "\u{2003}🦀"] {
                let input = format!(" \t{}{suffix}", "界".repeat(n));
                let actual = clean(&input);
                assert_eq!(actual, old_clean(&input));
                assert!(actual.len() <= HEADLINE_CHARS * 4 + 3);
                assert!(actual.capacity() <= 2 * (HEADLINE_CHARS * 4 + 3));
            }
        }
        assert_eq!(intent("{\"intent\":\"   ").as_deref(), Some(""));
    }

    /// Run: justrust test --release benchmark_tool_headlines -- --ignored --nocapture --test-threads=1
    #[test]
    #[ignore = "reproducible old/new microbenchmark, not a timing assertion"]
    fn benchmark_tool_headlines() {
        use std::{hint::black_box, time::Instant};
        let inputs = [
            format!(r#"{{"target":"{}"}}"#, "界 x ".repeat(16_000)),
            format!(r#"{{"intent":"{}"#, "界 x ".repeat(16_000)),
            r#"{"path":"src/app.rs","intent":"inspect app"}"#.to_owned(),
            r#"{"irrelevant":[1,2,3],"target":"last key"}"#.to_owned(),
        ];
        for input in inputs {
            assert_eq!(headline(&input), old_headline(&input));
            for (name, implementation) in [
                ("old", old_headline as fn(&str) -> Option<String>),
                ("new", headline),
            ] {
                let start = Instant::now();
                for _ in 0..1000 {
                    black_box(implementation(black_box(&input)));
                }
                eprintln!(
                    "headline {name}, {} bytes, 1000 iterations: {:?}",
                    input.len(),
                    start.elapsed()
                );
            }
        }
    }
}
