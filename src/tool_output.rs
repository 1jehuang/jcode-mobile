//! Bounded expanded tool output. Borrow short values and allocate only the
//! displayed prefix of long values, never a clone of the entire tool result.

use std::borrow::Cow;

pub const OUTPUT_BYTES: usize = 4000;
const MARKER: &str = "\n…";

/// Select output, then failure text, then input. The input fallback remains
/// untruncated to preserve the existing expanded-tool presentation.
pub fn body<'a>(output: &'a str, failure: Option<&'a str>, input: &'a str) -> Cow<'a, str> {
    let selected = if output.is_empty() {
        failure.unwrap_or("")
    } else {
        output
    };
    if selected.is_empty() {
        Cow::Borrowed(input)
    } else {
        prefix(selected, OUTPUT_BYTES)
    }
}

/// Byte-limited UTF-8 prefix with the existing newline/ellipsis marker. The
/// marker is additional to the byte limit. Short strings are borrowed.
pub fn prefix(text: &str, limit: usize) -> Cow<'_, str> {
    if text.len() <= limit {
        return Cow::Borrowed(text);
    }
    let mut end = limit;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    let mut out = String::with_capacity(end + MARKER.len());
    out.push_str(&text[..end]);
    out.push_str(MARKER);
    Cow::Owned(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn old_prefix(text: &str, limit: usize) -> String {
        let mut out = text.to_owned();
        if out.len() > limit {
            let mut end = limit;
            while !out.is_char_boundary(end) {
                end -= 1;
            }
            out.truncate(end);
            out.push_str(MARKER);
        }
        out
    }

    #[test]
    fn matches_old_prefix_at_every_unicode_boundary() {
        for text in ["", "ascii", "a界🦀e\u{301}\n終"] {
            for limit in 0..=text.len() + 1 {
                assert_eq!(prefix(text, limit), old_prefix(text, limit));
                assert_eq!(
                    matches!(prefix(text, limit), Cow::Borrowed(_)),
                    text.len() <= limit
                );
            }
        }
    }

    #[test]
    fn selects_output_failure_then_untruncated_input() {
        assert_eq!(body("ok", Some("error"), "input"), "ok");
        assert_eq!(body("", Some("error"), "input"), "error");
        assert_eq!(body("", Some(""), "input"), "input");
        assert_eq!(body("", None, "input"), "input");
        let long = "界".repeat(100_000);
        assert!(matches!(body("", None, &long), Cow::Borrowed(_)));
        for result in [body(&long, None, ""), body("", Some(&long), "")] {
            assert_eq!(result.len(), 3999 + MARKER.len());
            if let Cow::Owned(result) = result {
                assert_eq!(result.capacity(), result.len());
            } else {
                panic!("long output must allocate its bounded prefix");
            }
        }
    }

    /// Run: justrust test --release benchmark_tool_output -- --ignored --nocapture --test-threads=1
    #[test]
    #[ignore = "reproducible old/new microbenchmark, not a timing assertion"]
    fn benchmark_tool_output() {
        use std::{hint::black_box, time::Instant};
        for text in ["small output".to_owned(), "界 x ".repeat(1_000_000)] {
            let start = Instant::now();
            for _ in 0..1000 {
                black_box(old_prefix(black_box(&text), OUTPUT_BYTES));
            }
            eprintln!(
                "output old, {} bytes, 1000 iterations: {:?}",
                text.len(),
                start.elapsed()
            );
            let start = Instant::now();
            for _ in 0..1000 {
                black_box(prefix(black_box(&text), OUTPUT_BYTES).into_owned());
            }
            eprintln!(
                "output new (owned), {} bytes, 1000 iterations: {:?}",
                text.len(),
                start.elapsed()
            );
        }
    }
}
