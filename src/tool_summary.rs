//! Tool-call headlines from streamed (possibly partial) JSON input. Port of
//! `JCodeKit/ToolCallSummary.swift`: prefer the agent's stated `intent`, then
//! what the call operates on.

use serde_json::Value;
use std::{cell::RefCell, collections::VecDeque};

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
const CACHE_ENTRIES: usize = 256;
const CACHE_SOURCE_BYTES: usize = 1024 * 1024;
const CACHE_INPUT_BYTES: usize = 128 * 1024;
const CACHE_ADMISSION_INTERVAL: usize = 16;

thread_local! {
    static HEADLINES: RefCell<HeadlineCache> = RefCell::new(HeadlineCache::default());
}

/// Reuse unchanged tool inputs across UI frames. Each thread retains at most
/// 256 exact source keys totaling 1 MiB, plus their bounded headlines. Inputs
/// larger than 128 KiB bypass the cache without evicting useful keys.
/// Exact source equality decides identity, including partial streamed edits.
pub fn cached_headline(input: &str) -> Option<String> {
    if input.len() > CACHE_INPUT_BYTES {
        return headline(input);
    }
    HEADLINES.with(|cache| {
        cache
            .borrow_mut()
            .get_or_insert_with(input, || headline(input))
    })
}

#[derive(Default)]
struct HeadlineCache {
    // LRU order keeps repeated sequential frames cheap: their next input is
    // normally at the front. Avoid hashing entire large sources on hot hits.
    entries: VecDeque<(Box<str>, Option<Box<str>>)>,
    source_bytes: usize,
    admission_misses: usize,
}

impl HeadlineCache {
    fn get_or_insert_with(
        &mut self,
        input: &str,
        compute: impl FnOnce() -> Option<String>,
    ) -> Option<String> {
        if let Some(index) = self
            .entries
            .iter()
            .position(|(source, _)| source.as_ref() == input)
        {
            let entry = self.entries.remove(index).expect("matched cache entry");
            let result = entry.1.as_deref().map(str::to_owned);
            self.entries.push_back(entry);
            return result;
        }
        let result = compute();
        if input.len() > CACHE_INPUT_BYTES {
            return result;
        }
        // Admit freely while there is space. Once full, admit only every 16th
        // miss. A sequential scan larger than capacity therefore cannot evict
        // the entire working set before the next frame (as LRU does). This is
        // bounded, deterministic and still gradually adapts to changed inputs.
        if self.entries.len() >= CACHE_ENTRIES
            || self.source_bytes + input.len() > CACHE_SOURCE_BYTES
        {
            self.admission_misses += 1;
            if self.admission_misses < CACHE_ADMISSION_INTERVAL {
                return result;
            }
            self.admission_misses = 0;
        }
        while self.entries.len() >= CACHE_ENTRIES
            || self.source_bytes + input.len() > CACHE_SOURCE_BYTES
        {
            let (source, _) = self.entries.pop_front().expect("nonempty full cache");
            self.source_bytes -= source.len();
        }
        self.source_bytes += input.len();
        self.entries
            .push_back((input.into(), result.as_deref().map(Box::<str>::from)));
        result
    }
}

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

    #[test]
    fn cache_reuses_results_including_none_and_exact_source_edits() {
        let mut cache = HeadlineCache::default();
        for input in ["", "{", r#"{"intent":""}"#, r#"{"intent":"first"#] {
            let expected = headline(input);
            assert_eq!(
                cache.get_or_insert_with(input, || headline(input)),
                expected
            );
            assert_eq!(
                cache.get_or_insert_with(input, || panic!("hit reparsed")),
                expected
            );
        }
        // Same length, shared prefix and changed trailing byte must not alias.
        for input in [r#"{"intent":"first"}"#, r#"{"intent":"other"}"#] {
            assert_eq!(
                cache.get_or_insert_with(input, || headline(input)),
                headline(input)
            );
        }
        let original = r#"{"intent":"first"}"#;
        let mut result = cache
            .get_or_insert_with(original, || panic!("missing original"))
            .unwrap();
        result.clear();
        assert_eq!(
            cache
                .get_or_insert_with(original, || panic!("hit reparsed"))
                .as_deref(),
            Some("first")
        );
    }

    #[test]
    fn cache_admits_periodically_and_evicts_lru_at_entry_limit() {
        let mut cache = HeadlineCache::default();
        for index in 0..CACHE_ENTRIES {
            let input = format!("tool {index}");
            cache.get_or_insert_with(&input, || headline(&input));
        }
        cache.get_or_insert_with("tool 0", || panic!("expected cached first entry"));
        for _ in 0..CACHE_ADMISSION_INTERVAL - 1 {
            cache.get_or_insert_with("new tool", || Some("new".into()));
            assert!(
                cache
                    .entries
                    .iter()
                    .any(|(key, _)| key.as_ref() == "tool 1")
            );
        }
        cache.get_or_insert_with("new tool", || Some("new".into()));
        assert_eq!(cache.entries.len(), CACHE_ENTRIES);
        assert!(
            cache
                .entries
                .iter()
                .any(|(key, _)| key.as_ref() == "tool 0")
        );
        assert!(
            !cache
                .entries
                .iter()
                .any(|(key, _)| key.as_ref() == "tool 1")
        );
        let mut computed = false;
        cache.get_or_insert_with("tool 1", || {
            computed = true;
            Some("tool 1".into())
        });
        assert!(computed);
    }

    #[test]
    fn overcapacity_sequential_frames_keep_hits() {
        for count in [320, 1000] {
            let mut cache = HeadlineCache::default();
            let inputs: Vec<_> = (0..count).map(|i| format!("tool {i} 界")).collect();
            for frame in 0..30 {
                let mut misses = 0;
                for input in &inputs {
                    assert_eq!(
                        cache.get_or_insert_with(input, || {
                            misses += 1;
                            headline(input)
                        }),
                        headline(input)
                    );
                }
                if frame > 0 {
                    assert!(count - misses >= 150, "{count} inputs, {misses} misses");
                }
                assert_eq!(cache.entries.len(), CACHE_ENTRIES);
            }
        }
    }

    #[test]
    fn cache_bounds_source_bytes_and_bypasses_oversized_inputs() {
        let mut cache = HeadlineCache::default();
        let sources: Vec<_> = (0..CACHE_SOURCE_BYTES / CACHE_INPUT_BYTES)
            .map(|index| {
                char::from(b'a' + index as u8)
                    .to_string()
                    .repeat(CACHE_INPUT_BYTES)
            })
            .collect();
        for input in &sources {
            cache.get_or_insert_with(input, || headline(input));
        }
        assert_eq!(cache.source_bytes, CACHE_SOURCE_BYTES);
        for _ in 0..CACHE_ADMISSION_INTERVAL {
            cache.get_or_insert_with("small", || headline("small"));
        }
        assert_eq!(cache.entries.len(), CACHE_SOURCE_BYTES / CACHE_INPUT_BYTES);
        assert!(
            !cache
                .entries
                .iter()
                .any(|(key, _)| key.as_ref() == sources[0])
        );
        assert_eq!(
            cache.source_bytes,
            CACHE_SOURCE_BYTES - CACHE_INPUT_BYTES + 5
        );
        let oversized = "🦀".repeat(CACHE_INPUT_BYTES / 4 + 1);
        assert_eq!(
            cache.get_or_insert_with(&oversized, || headline(&oversized)),
            headline(&oversized)
        );
        assert_eq!(
            cache.source_bytes,
            CACHE_SOURCE_BYTES - CACHE_INPUT_BYTES + 5
        );
        assert_eq!(cache.entries.len(), CACHE_SOURCE_BYTES / CACHE_INPUT_BYTES);
        for index in 0..1000 {
            let input = format!("{index}{}", "界".repeat(index));
            cache.get_or_insert_with(&input, || headline(&input));
            assert!(cache.entries.len() <= CACHE_ENTRIES);
            assert!(cache.source_bytes <= CACHE_SOURCE_BYTES);
            assert_eq!(
                cache.source_bytes,
                cache
                    .entries
                    .iter()
                    .map(|(key, _)| key.len())
                    .sum::<usize>()
            );
        }
    }

    #[test]
    fn cached_api_matches_uncached_at_stream_and_unicode_boundaries() {
        for input in [
            r#"{"command":"ls","intent":"你好🦀\ntext\u1234"}"#,
            r#"{"intent":null,"target":"subject"}"#,
            r#"{"nested":{"command":"inner"},"target":"outer"}"#,
            " raw\u{2003}text ",
        ] {
            for end in (0..=input.len()).filter(|&end| input.is_char_boundary(end)) {
                for _ in 0..2 {
                    assert_eq!(cached_headline(&input[..end]), headline(&input[..end]));
                }
            }
        }
        let oversized = format!(r#"{{"intent":"{}"}}"#, "界".repeat(CACHE_INPUT_BYTES));
        assert_eq!(cached_headline(&oversized), headline(&oversized));
    }

    /// Run: justrust test --release benchmark_cached_headline_frames -- --ignored --nocapture --test-threads=1
    #[test]
    #[ignore = "repeated-frame bounded-cache benchmark, not a timing assertion"]
    fn benchmark_cached_headline_frames() {
        use std::{hint::black_box, time::Instant};
        let frames: usize = std::env::var("HEADLINE_BENCH_FRAMES")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(1000);
        assert!(frames > 0);
        // Previous implementation, kept here only as a reproducible baseline.
        #[derive(Default)]
        struct LinearLru {
            entries: VecDeque<(Box<str>, Option<Box<str>>)>,
            bytes: usize,
        }
        impl LinearLru {
            fn get(&mut self, input: &str) -> Option<String> {
                if let Some(index) = self.entries.iter().position(|(s, _)| s.as_ref() == input) {
                    let entry = self.entries.remove(index).unwrap();
                    let result = entry.1.as_deref().map(str::to_owned);
                    self.entries.push_back(entry);
                    return result;
                }
                let result = headline(input);
                if input.len() <= CACHE_INPUT_BYTES {
                    while self.entries.len() >= CACHE_ENTRIES
                        || self.bytes + input.len() > CACHE_SOURCE_BYTES
                    {
                        self.bytes -= self.entries.pop_front().unwrap().0.len();
                    }
                    self.bytes += input.len();
                    self.entries
                        .push_back((input.into(), result.as_deref().map(Box::from)));
                }
                result
            }
        }
        for (tools, padding) in [
            (1, 0),
            (32, 0),
            (32, 2048),
            (80, 0),
            (80, 2048),
            (CACHE_ENTRIES, 0),
            (CACHE_ENTRIES + 64, 0),
            (1000, 0),
            (320, 2048),
            (32, 32 * 1024),
            (32, CACHE_INPUT_BYTES),
        ] {
            let inputs: Vec<_> = (0..tools).map(|index| {
                format!(r#"{{"padding":"{}","path":"src/tool_{index}.rs","intent":"Inspect tool {index}"}}"#, "x".repeat(padding))
            }).collect();
            HEADLINES.with(|cache| *cache.borrow_mut() = HeadlineCache::default());
            for input in &inputs {
                assert_eq!(cached_headline(input), headline(input));
            }
            let mut baseline = LinearLru::default();
            for input in &inputs {
                baseline.get(input);
            }
            for name in ["uncached", "linear-lru", "cached"] {
                let start = Instant::now();
                for _ in 0..frames {
                    for input in &inputs {
                        let input = black_box(input.as_str());
                        black_box(match name {
                            "uncached" => headline(input),
                            "linear-lru" => baseline.get(input),
                            _ => cached_headline(input),
                        });
                    }
                }
                let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
                eprintln!(
                    "frames {name}, {tools} tools, {padding} padding bytes: {elapsed_ms:.3} ms/{frames} frames, {:.6} ms/frame",
                    elapsed_ms / frames as f64
                );
            }
        }
    }

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
