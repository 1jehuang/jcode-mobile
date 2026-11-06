//! Lightweight markdown for assistant replies: paragraphs, headings, bullet
//! lists, fenced code blocks, and inline `code` / **bold** spans.

use gpui::{
    AnyElement, FontStyle, FontWeight, HighlightStyle, SharedString, StyledText, div, prelude::*,
    px,
};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex, OnceLock},
};

use crate::theme::Theme;

/// Render already-decoded reasoning with muted ink, without decoding synthetic wrappers.
pub fn render_reasoning(src: &str, t: &Theme) -> AnyElement {
    let muted = Theme {
        text: t.text_muted,
        text_strong: t.text_muted,
        ..*t
    };
    render(src, &muted)
}

pub fn render(src: &str, t: &Theme) -> AnyElement {
    render_parsed(&cached_parse(src), t)
}

#[derive(Debug, PartialEq, Eq)]
struct ParsedInline {
    text: SharedString,
    spans: Vec<(std::ops::Range<usize>, InlineStyle)>,
}

impl ParsedInline {
    fn new(src: &str) -> Self {
        let (text, spans) = parse_inline(src);
        Self {
            text: text.into(),
            spans,
        }
    }

    fn bytes(&self) -> usize {
        self.text.len()
            + 64
            + self.spans.capacity() * std::mem::size_of::<(std::ops::Range<usize>, InlineStyle)>()
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Block {
    Paragraph(ParsedInline),
    Heading(ParsedInline),
    Bullet(ParsedInline),
    Ordered(SharedString, ParsedInline),
    Code(SharedString),
}

#[derive(Debug, PartialEq, Eq)]
struct ParsedMarkdown(Vec<Block>);

impl ParsedMarkdown {
    fn bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + 64
            + self.0.capacity() * std::mem::size_of::<Block>()
            + self
                .0
                .iter()
                .map(|block| match block {
                    Block::Paragraph(text) | Block::Heading(text) | Block::Bullet(text) => {
                        text.bytes()
                    }
                    Block::Ordered(number, text) => number.len() + 64 + text.bytes(),
                    Block::Code(text) => text.len() + 64,
                })
                .sum::<usize>()
    }
}

// Bound both retained allocation bytes (including vector capacities and allocation
// overhead allowances) and entry count. Oversized replies render without retention.
const CACHE_BYTES: usize = 2 * 1024 * 1024;
const CACHE_ENTRIES: usize = 128;
// Large unchanged replies still benefit from reuse. The total allocation budget,
// not just this source limit, governs admission, including span-heavy documents.
const MAX_CACHED_SOURCE: usize = CACHE_BYTES / 4;

#[derive(Default)]
struct ParseCache {
    entries: VecDeque<(Box<str>, Arc<ParsedMarkdown>, usize)>,
    bytes: usize,
}

impl ParseCache {
    fn get(&mut self, src: &str) -> Arc<ParsedMarkdown> {
        if let Some(index) = self
            .entries
            .iter()
            .position(|(key, _, _)| key.as_ref() == src)
        {
            let entry = self.entries.remove(index).unwrap();
            let parsed = Arc::clone(&entry.1);
            self.entries.push_back(entry);
            return parsed;
        }
        let parsed = Arc::new(parse_markdown(src));
        let bytes = src.len()
            + parsed.bytes()
            + std::mem::size_of::<(Box<str>, Arc<ParsedMarkdown>, usize)>()
            + 64;
        if src.len() <= MAX_CACHED_SOURCE && bytes <= CACHE_BYTES {
            while self.entries.len() >= CACHE_ENTRIES || self.bytes + bytes > CACHE_BYTES {
                self.bytes -= self.entries.pop_front().unwrap().2;
            }
            self.entries
                .push_back((src.into(), Arc::clone(&parsed), bytes));
            self.bytes += bytes;
        }
        parsed
    }
}

fn cached_parse(src: &str) -> Arc<ParsedMarkdown> {
    if src.len() > MAX_CACHED_SOURCE {
        return Arc::new(parse_markdown(src));
    }
    static CACHE: OnceLock<Mutex<ParseCache>> = OnceLock::new();
    CACHE
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(src)
}

fn parse_markdown(src: &str) -> ParsedMarkdown {
    let mut blocks = Vec::new();
    let mut lines = src.lines().peekable();
    let mut para = Vec::new();
    let flush = |para: &mut Vec<&str>, blocks: &mut Vec<Block>| {
        if !para.is_empty() {
            blocks.push(Block::Paragraph(ParsedInline::new(&para.join(" "))));
            para.clear();
        }
    };
    while let Some(line) = lines.next() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") {
            flush(&mut para, &mut blocks);
            let mut code = Vec::new();
            for line in lines.by_ref() {
                if line.trim_start().starts_with("```") {
                    break;
                }
                code.push(line);
            }
            blocks.push(Block::Code(code.join("\n").into()));
        } else if trimmed.is_empty() {
            flush(&mut para, &mut blocks);
        } else if let Some(h) = trimmed.strip_prefix('#') {
            flush(&mut para, &mut blocks);
            blocks.push(Block::Heading(ParsedInline::new(
                h.trim_start_matches('#').trim(),
            )));
        } else if let Some(item) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
        {
            flush(&mut para, &mut blocks);
            blocks.push(Block::Bullet(ParsedInline::new(item)));
        } else if let Some((num, item)) = ordered_item(trimmed) {
            flush(&mut para, &mut blocks);
            blocks.push(Block::Ordered(
                format!("{num}.").into(),
                ParsedInline::new(item),
            ));
        } else {
            para.push(trimmed);
        }
    }
    flush(&mut para, &mut blocks);
    ParsedMarkdown(blocks)
}

fn render_parsed(parsed: &ParsedMarkdown, t: &Theme) -> AnyElement {
    let mut col = div()
        .w_full()
        .min_w_0()
        .max_w_full()
        .flex()
        .flex_col()
        .gap(px(8.))
        .text_size(crate::system::fs(13.5))
        .line_height(crate::system::fs(21.));
    for block in &parsed.0 {
        match block {
            Block::Paragraph(text) => {
                col = col.child(inline(text, t));
            }
            Block::Code(code) => {
                col = col.child(
                    div()
                        .min_w_0()
                        .max_w_full()
                        .overflow_hidden()
                        .p(px(10.))
                        .rounded(px(8.))
                        .bg(t.code_bg)
                        .border_1()
                        .border_color(t.border)
                        .font_family(crate::theme::FONT)
                        .text_size(crate::system::fs(12.))
                        .line_height(crate::system::fs(18.))
                        .text_color(t.text)
                        .child(code.clone()),
                );
            }
            Block::Heading(h) => {
                col = col.child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_size(crate::system::fs(14.))
                        .child(inline(h, t)),
                );
            }
            Block::Bullet(item) => {
                col = col.child(
                    div()
                        .flex()
                        .min_w_0()
                        .max_w_full()
                        .gap(px(8.))
                        .pl(px(4.))
                        .child(div().text_color(t.text_muted).child("•"))
                        .child(div().flex_1().min_w_0().child(inline(item, t))),
                );
            }
            Block::Ordered(num, item) => {
                col = col.child(
                    div()
                        .flex()
                        .min_w_0()
                        .max_w_full()
                        .gap(px(6.))
                        .pl(px(4.))
                        .child(div().text_color(t.text_muted).child(num.clone()))
                        .child(div().flex_1().min_w_0().child(inline(item, t))),
                );
            }
        }
    }
    col.into_any_element()
}

/// `12. item` or `12) item` -> ("12", "item").
fn ordered_item(line: &str) -> Option<(&str, &str)> {
    let digits = line.bytes().take_while(u8::is_ascii_digit).count();
    if digits == 0 || digits > 3 {
        return None;
    }
    let rest = &line[digits..];
    let item = rest
        .strip_prefix(". ")
        .or_else(|| rest.strip_prefix(") "))?;
    Some((&line[..digits], item))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunked_inline_matches_scalar_reference() {
        fn check(source: &str) {
            assert_eq!(
                parse_inline(source),
                parse_inline_impl::<false>(source),
                "{source:?}"
            );
        }
        // Exhaustive short combinations exercise adjacent runs and escaped markers.
        let tokens = [
            "a", "é", "中", "🦀", "*", "**", "***", "`", "``", "\\", " ", "\n",
        ];
        for a in tokens {
            for b in tokens {
                for c in tokens {
                    for d in tokens {
                        check(&[a, b, c, d].concat());
                    }
                }
            }
        }
        // Longer deterministic mixtures exercise nesting and code skipping.
        let mut seed = 0x431b_782a_u64;
        for _ in 0..4_000 {
            let mut source = String::new();
            for _ in 0..80 {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                source.push_str(tokens[(seed >> 32) as usize % tokens.len()]);
            }
            check(&source);
        }
        for _ in 0..100 {
            let mut source = String::from("````` unmatched ");
            for _ in 0..1_200 {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                source.push_str(tokens[(seed >> 32) as usize % tokens.len()]);
            }
            check(&source);
        }
        for source in [
            "**é *中 `🦀*`* tail**".repeat(100),
            "\\*\\`é\\中 trailing\\".repeat(100),
            "* ".repeat(1_000),
            (1..100).map(|n| format!("{}é ", "`".repeat(n))).collect(),
            format!(
                "`````{} **é `x*y`** \\`z` *a ``é`` b* `tail",
                "plain é ".repeat(300)
            ),
            format!("*`````{}* **a `x`** ``two``", "plain é ".repeat(300)),
        ] {
            check(&source);
            for end in (0..source.len())
                .step_by(37)
                .filter(|end| source.is_char_boundary(*end))
            {
                check(&source[..end]);
            }
        }
    }

    /// Parsing-only timings in milliseconds per operation, including allocating output.
    /// Run with --release -- --ignored --nocapture and compare on the same machine.
    #[test]
    #[ignore = "before/after inline workload microbenchmark"]
    fn benchmark_inline_workloads() {
        use std::{hint::black_box, time::Instant};
        let prose = "Plain English with UTF-8 é 中 🦀 and no formatting. ".repeat(6_000);
        let mixed = "A **bold** and *italic é* paragraph with `literal * code` and \\* escapes. "
            .repeat(3_000);
        let unmatched = (1..250)
            .map(|n| format!("{}é ", "`".repeat(n)))
            .collect::<String>();
        for (label, source, iterations) in [
            ("plain-large", prose.as_str(), 100),
            ("mixed-large", mixed.as_str(), 100),
            ("unmatched-backticks", unmatched.as_str(), 50),
            (
                "unmatched-emphasis",
                "* long plain unmatched é text",
                10_000,
            ),
        ] {
            assert_eq!(parse_inline(source), parse_inline_impl::<false>(source));
            let start = Instant::now();
            for _ in 0..iterations {
                black_box(parse_inline_impl::<false>(black_box(source)));
            }
            let before = start.elapsed().as_secs_f64() * 1_000. / iterations as f64;
            let start = Instant::now();
            for _ in 0..iterations {
                black_box(parse_inline(black_box(source)));
            }
            let after = start.elapsed().as_secs_f64() * 1_000. / iterations as f64;
            eprintln!(
                "{label}: {} bytes, before {before:.6} ms/op, after {after:.6} ms/op",
                source.len()
            );
        }
        let start = Instant::now();
        for end in (1_024..=mixed.len()).step_by(1_024) {
            let end = (end..=mixed.len())
                .find(|end| mixed.is_char_boundary(*end))
                .unwrap();
            black_box(parse_inline_impl::<false>(black_box(&mixed[..end])));
        }
        let before = start.elapsed().as_secs_f64() * 1_000.;
        let start = Instant::now();
        for end in (1_024..=mixed.len()).step_by(1_024) {
            let end = (end..=mixed.len())
                .find(|end| mixed.is_char_boundary(*end))
                .unwrap();
            black_box(parse_inline(black_box(&mixed[..end])));
        }
        let after = start.elapsed().as_secs_f64() * 1_000.;
        let operations = mixed.len() / 1_024;
        eprintln!(
            "streaming mixed 1KiB updates: {operations} prefixes, before {:.6} ms/op, after {:.6} ms/op",
            before / operations as f64,
            after / operations as f64
        );
    }

    #[test]
    fn block_formatting_preserves_existing_semantics() {
        let parsed = parse_markdown(
            "  first  \r\n\tsecond\n\n ### **é** ##  \n - `x`\n * *y*\n 007) z\n2026. literal\n3.5 literal\n ```rust\n  **raw**\n\tcode\n ```closing\nlast\nline\n```\nunclosed\n",
        );
        assert_eq!(
            parsed,
            ParsedMarkdown(vec![
                Block::Paragraph(ParsedInline::new("first   second")),
                Block::Heading(ParsedInline::new("**é** ##")),
                Block::Bullet(ParsedInline::new("`x`")),
                Block::Bullet(ParsedInline::new("*y*")),
                Block::Ordered("007.".into(), ParsedInline::new("z")),
                Block::Paragraph(ParsedInline::new("2026. literal 3.5 literal")),
                Block::Code("  **raw**\n\tcode".into()),
                Block::Paragraph(ParsedInline::new("last line")),
                Block::Code("unclosed".into()),
            ])
        );
        assert_eq!(parse_markdown("\n \n"), ParsedMarkdown(vec![]));
        assert_eq!(
            parse_markdown("#\n```\n```"),
            ParsedMarkdown(vec![
                Block::Heading(ParsedInline::new("")),
                Block::Code("".into()),
            ])
        );
    }

    #[test]
    fn cache_reuses_unchanged_parse_and_invalidates_every_stream_prefix() {
        let mut cache = ParseCache::default();
        let src = "# é\n\n**bold** and `code`\n- item\n```rust\nfn x() {}\n```\n";
        let mut previous = None;
        for end in (0..=src.len()).filter(|end| src.is_char_boundary(*end)) {
            let prefix = &src[..end];
            let parsed = cache.get(prefix);
            assert_eq!(*parsed, parse_markdown(prefix));
            assert!(Arc::ptr_eq(&parsed, &cache.get(prefix)));
            if let Some(previous) = previous {
                assert!(!Arc::ptr_eq(&parsed, &previous));
            }
            previous = Some(parsed);
        }
        // No theme or GPUI element is retained, including for reasoning rendering.
        assert!(Arc::ptr_eq(&cached_parse(src), &cached_parse(src)));
    }

    #[test]
    fn cache_evicts_least_recently_used_entries_and_releases_them() {
        let mut cache = ParseCache::default();
        let oldest = cache.get("oldest");
        let evicted = Arc::downgrade(&cache.get("evict me"));
        for i in 2..CACHE_ENTRIES {
            cache.get(&format!("entry {i}"));
        }
        assert!(Arc::ptr_eq(&oldest, &cache.get("oldest")));
        cache.get("new entry");
        assert!(evicted.upgrade().is_none());
        assert!(Arc::ptr_eq(&oldest, &cache.get("oldest")));
        assert_eq!(cache.entries.len(), CACHE_ENTRIES);
        assert!(cache.bytes <= CACHE_BYTES);
    }

    #[test]
    fn cache_bounds_bytes_and_bypasses_oversized_or_amplified_input() {
        let mut cache = ParseCache::default();
        for i in 0..100 {
            cache.get(&format!("{i} {}", "x".repeat(40_000)));
            assert!(cache.bytes <= CACHE_BYTES);
            assert_eq!(
                cache.bytes,
                cache.entries.iter().map(|entry| entry.2).sum::<usize>()
            );
        }
        assert!(cache.entries.len() < CACHE_ENTRIES);
        let before = (cache.entries.len(), cache.bytes);
        for src in [
            "x".repeat(MAX_CACHED_SOURCE + 1),
            "- \n".repeat(MAX_CACHED_SOURCE / 3),
        ] {
            let first = cache.get(&src);
            assert!(!Arc::ptr_eq(&first, &cache.get(&src)));
            assert_eq!((cache.entries.len(), cache.bytes), before);
        }
    }

    #[test]
    fn cache_reuses_large_replies_within_the_same_byte_budget() {
        let mut cache = ParseCache::default();
        let source = "é plain reply ".repeat(20_000);
        assert!(source.len() > 64 * 1024);
        let first = cache.get(&source);
        assert!(Arc::ptr_eq(&first, &cache.get(&source)));
        assert_eq!(*first, parse_markdown(&source));
        for n in 0..20 {
            let source = format!("{n} {}", "a".repeat(200_000));
            cache.get(&source);
            assert!(cache.bytes <= CACHE_BYTES);
        }
    }

    /// Run: justrust test -p jcode-mobile markdown::tests::benchmark_parse_cache --release -- --ignored --nocapture
    /// Reports parsing only, not GPUI layout/paint. No timing assertion on shared runners.
    #[test]
    #[ignore = "reproducible parsing microbenchmark"]
    fn benchmark_parse_cache() {
        use std::{hint::black_box, time::Instant};
        for (label, src, iterations) in [
            ("small", "# Heading\n\nA **bold** and *italic* é paragraph with `code`.\n- item\n12) ordered\n```rust\nfn main() {}\n```\n\n".repeat(40), 2_000),
            ("large-plain", "Plain English with UTF-8 é 中 🦀 and no formatting. ".repeat(6_000), 100),
            ("large-mixed", "A **bold** and *italic é* paragraph with `code`. ".repeat(5_000), 100),
        ] {
            assert_eq!(*cached_parse(&src), parse_markdown(&src));
            let start = Instant::now();
            for _ in 0..iterations {
                black_box(parse_markdown(black_box(&src)));
            }
            let uncached = start.elapsed();
            let start = Instant::now();
            for _ in 0..iterations {
                black_box(cached_parse(black_box(&src)));
            }
            let cached = start.elapsed();
            eprintln!(
                "markdown {label}: {iterations} iterations, {} source bytes, cold {:.6} ms/op, warm {:.6} ms/op",
                src.len(),
                uncached.as_secs_f64() * 1_000. / iterations as f64,
                cached.as_secs_f64() * 1_000. / iterations as f64,
            );
        }
    }

    #[test]
    fn escapes_stay_literal_not_formatting() {
        let (text, spans) = parse_inline(r"\*\*literal\*\* and C:\Users\me \\");
        assert_eq!(text, r"**literal** and C:\Users\me \");
        assert!(spans.is_empty());
        let (text, spans) = parse_inline(r"*\*\*Adding TODO initialization\*\**");
        assert_eq!(text, "**Adding TODO initialization**");
        assert_eq!(
            spans,
            vec![(
                0..text.len(),
                InlineStyle {
                    italic: true,
                    ..Default::default()
                }
            )]
        );
    }

    #[test]
    fn code_is_literal_and_matches_exact_backtick_runs() {
        let (text, spans) = parse_inline(r"`` `x` \* **é** ``");
        assert_eq!(text, r"`x` \* **é**");
        assert_eq!(
            spans,
            vec![(
                0..text.len(),
                InlineStyle {
                    code: true,
                    ..Default::default()
                }
            )]
        );
        assert_eq!(parse_inline("`unterminated").0, "`unterminated");
        assert_eq!(parse_inline("``one`two``").0, "one`two");
        assert_eq!(parse_inline("``unmatched`tail").0, "``unmatched`tail");
    }

    #[test]
    fn nested_styles_have_sorted_nonoverlapping_utf8_ranges() {
        for (source, expected) in [
            ("**é `x*y`**", "é x*y"),
            ("*a **é** c*", "a é c"),
            ("**a *é***", "a é"),
            ("***é***", "é"),
        ] {
            let (text, spans) = parse_inline(source);
            assert_eq!(text, expected, "{source}");
            assert!(!text.contains('`'));
            let mut end = 0;
            for (range, _) in spans {
                assert!(range.start >= end);
                assert!(text.get(range.clone()).is_some());
                end = range.end;
            }
        }
    }

    #[test]
    fn incomplete_and_non_delimiter_asterisks_survive() {
        for source in [
            "**unfinished",
            "*unfinished",
            "2 * 3",
            "trailing\\",
            "é\\中",
        ] {
            assert_eq!(parse_inline(source).0, source);
        }
    }

    #[test]
    fn ordered_items() {
        assert_eq!(super::ordered_item("3. ls: list"), Some(("3", "ls: list")));
        assert_eq!(super::ordered_item("12) x"), Some(("12", "x")));
        assert_eq!(super::ordered_item("2026. was a year"), None);
        assert_eq!(super::ordered_item("3.5 GiB"), None);
    }
}

/// Parses escapes before delimiters, so escaped markers never become formatting.
/// Code contents are literal. Ranges use UTF-8 byte offsets, as GPUI requires.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct InlineStyle {
    bold: bool,
    italic: bool,
    code: bool,
}

fn parse_inline(src: &str) -> (String, Vec<(std::ops::Range<usize>, InlineStyle)>) {
    parse_inline_impl::<true>(src)
}

// The scalar path is retained as a differential oracle in tests. Const specialization
// removes it from the production parser.
fn parse_inline_impl<const CHUNKED: bool>(
    src: &str,
) -> (String, Vec<(std::ops::Range<usize>, InlineStyle)>) {
    struct CodeSearch<'a> {
        root: &'a str,
        // Built only after a failed long search. Ordinary prose and matched code
        // keep the allocation-free search, while adversarial distinct fence lengths
        // cannot repeatedly scan the remaining document. Never retained in the cache.
        runs: Option<std::collections::HashMap<usize, Vec<usize>>>,
    }
    fn append(
        out: &mut String,
        spans: &mut Vec<(std::ops::Range<usize>, InlineStyle)>,
        text: &str,
        style: InlineStyle,
    ) {
        let start = out.len();
        out.push_str(text);
        if let Some((range, _)) = spans
            .last_mut()
            .filter(|(range, previous)| range.end == start && *previous == style)
        {
            range.end = out.len();
        } else if start != out.len() && style != InlineStyle::default() {
            spans.push((start..out.len(), style));
        }
    }
    fn closing<const CHUNKED: bool>(
        src: &str,
        marker: &str,
        code: &mut CodeSearch<'_>,
    ) -> Option<usize> {
        let mut i = 0;
        while i < src.len() {
            let rest = &src[i..];
            if rest.starts_with('\\') {
                i += 1;
                if let Some(c) = src[i..].chars().next() {
                    i += c.len_utf8();
                }
                continue;
            }
            // Do not mistake delimiters inside a code span for emphasis closers.
            if rest.starts_with('`') && marker != "`" {
                let count = rest.bytes().take_while(|b| *b == b'`').count();
                let fence = &rest[..count];
                if let Some(end) = code_end::<CHUNKED>(&rest[count..], fence, code) {
                    i += count + end + count;
                    continue;
                }
            }
            if rest.starts_with(marker) {
                let count = rest.bytes().take_while(|b| *b == b'*').count();
                if marker == "*" && count == 2 {
                    i += count;
                    continue;
                }
                return Some(if marker == "**" && count == 3 {
                    i + 1
                } else {
                    i
                });
            }
            i += if CHUNKED {
                // ASCII marker bytes cannot occur inside a UTF-8 scalar.
                rest.as_bytes()[1..]
                    .iter()
                    .position(|b| matches!(b, b'\\' | b'`' | b'*'))
                    .map_or(rest.len(), |next| next + 1)
            } else {
                rest.chars().next().unwrap().len_utf8()
            };
        }
        None
    }
    fn code_end<const CHUNKED: bool>(
        src: &str,
        fence: &str,
        code: &mut CodeSearch<'_>,
    ) -> Option<usize> {
        if CHUNKED && let Some(runs) = &code.runs {
            let start = src.as_ptr() as usize - code.root.as_ptr() as usize;
            let candidates = runs.get(&fence.len())?;
            let next = *candidates.get(candidates.partition_point(|offset| *offset < start))?;
            return (next + fence.len() <= start + src.len()).then_some(next - start);
        }
        let mut offset = 0;
        while let Some(pos) = src[offset..].find('`') {
            let start = offset + pos;
            let count = src[start..].bytes().take_while(|b| *b == b'`').count();
            if count == fence.len() {
                return Some(start);
            }
            offset = start + count;
        }
        if CHUNKED && src.len() > 1_024 {
            let mut runs = std::collections::HashMap::<usize, Vec<usize>>::new();
            let mut offset = 0;
            while let Some(pos) = code.root[offset..].find('`') {
                let start = offset + pos;
                let count = code.root[start..]
                    .bytes()
                    .take_while(|b| *b == b'`')
                    .count();
                runs.entry(count).or_default().push(start);
                offset = start + count;
            }
            code.runs = Some(runs);
        }
        None
    }
    fn parse<const CHUNKED: bool>(
        mut src: &str,
        style: InlineStyle,
        depth: usize,
        out: &mut String,
        spans: &mut Vec<(std::ops::Range<usize>, InlineStyle)>,
        code: &mut CodeSearch<'_>,
    ) {
        while !src.is_empty() {
            if let Some(rest) = src.strip_prefix('\\')
                && let Some(c) = rest.chars().next()
                && c.is_ascii_punctuation()
            {
                append(out, spans, &rest[..c.len_utf8()], style);
                src = &rest[c.len_utf8()..];
                continue;
            }
            if src.starts_with('`') {
                let count = src.bytes().take_while(|b| *b == b'`').count();
                if let Some(end) = code_end::<CHUNKED>(&src[count..], &src[..count], code) {
                    let content = src[count..count + end].replace('\n', " ");
                    let content = if content.starts_with(' ')
                        && content.ends_with(' ')
                        && !content.chars().all(|c| c == ' ')
                    {
                        &content[1..content.len() - 1]
                    } else {
                        &content
                    };
                    append(
                        out,
                        spans,
                        content,
                        InlineStyle {
                            code: true,
                            ..style
                        },
                    );
                    src = &src[count + end + count..];
                    continue;
                }
                append(out, spans, &src[..count], style);
                src = &src[count..];
                continue;
            }
            let marker = if src.starts_with("***") {
                "***"
            } else if src.starts_with("**") {
                "**"
            } else {
                "*"
            };
            if depth < 32
                && src.starts_with(marker)
                // A whitespace opener can never format, regardless of its closer.
                && (!CHUNKED || !src[marker.len()..].starts_with(char::is_whitespace))
                && let Some(end) = closing::<CHUNKED>(&src[marker.len()..], marker, code)
                && end > 0
                && !src[marker.len()..].starts_with(char::is_whitespace)
                && !src[..marker.len() + end].ends_with(char::is_whitespace)
            {
                let nested = match marker {
                    "***" => InlineStyle {
                        bold: true,
                        italic: true,
                        ..style
                    },
                    "**" => InlineStyle {
                        bold: true,
                        ..style
                    },
                    _ => InlineStyle {
                        italic: true,
                        ..style
                    },
                };
                parse::<CHUNKED>(
                    &src[marker.len()..marker.len() + end],
                    nested,
                    depth + 1,
                    out,
                    spans,
                    code,
                );
                src = &src[marker.len() + end + marker.len()..];
                continue;
            }
            let count = if CHUNKED {
                // Consume the current literal scalar plus the entire following plain
                // run. Stop only at ASCII syntax, preserving byte-based span ranges.
                src.as_bytes()[1..]
                    .iter()
                    .position(|b| matches!(b, b'\\' | b'`' | b'*'))
                    .map_or(src.len(), |next| next + 1)
            } else {
                src.chars().next().unwrap().len_utf8()
            };
            append(out, spans, &src[..count], style);
            src = &src[count..];
        }
    }
    let mut out = String::with_capacity(src.len());
    let mut spans = vec![];
    let mut code = CodeSearch {
        root: src,
        runs: None,
    };
    parse::<CHUNKED>(
        src,
        InlineStyle::default(),
        0,
        &mut out,
        &mut spans,
        &mut code,
    );
    (out, spans)
}

fn inline(parsed: &ParsedInline, t: &Theme) -> AnyElement {
    let highlights = parsed
        .spans
        .iter()
        .map(|(range, style)| {
            (
                range.clone(),
                HighlightStyle {
                    color: style.code.then_some(t.text_strong),
                    background_color: style.code.then_some(t.inline_code_bg),
                    font_weight: style.bold.then_some(FontWeight::SEMIBOLD),
                    font_style: style.italic.then_some(FontStyle::Italic),
                    ..Default::default()
                },
            )
        })
        .collect::<Vec<_>>();
    div()
        .min_w_0()
        .max_w_full()
        .overflow_hidden()
        .text_color(t.text)
        .child(StyledText::new(parsed.text.clone()).with_highlights(highlights))
        .into_any_element()
}
