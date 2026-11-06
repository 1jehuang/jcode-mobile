//! Lightweight markdown for assistant replies: paragraphs, headings, bullet
//! lists, fenced code blocks, and inline `code` / **bold** spans.

use gpui::{AnyElement, FontStyle, FontWeight, HighlightStyle, StyledText, div, prelude::*, px};

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
    let mut col = div()
        .w_full()
        .min_w_0()
        .max_w_full()
        .flex()
        .flex_col()
        .gap(px(8.))
        .text_size(crate::system::fs(13.5))
        .line_height(crate::system::fs(21.));
    let mut lines = src.lines().peekable();
    let mut para: Vec<&str> = vec![];
    let flush = |para: &mut Vec<&str>, col: gpui::Div| -> gpui::Div {
        if para.is_empty() {
            return col;
        }
        let text = para.join(" ");
        para.clear();
        col.child(inline(&text, t))
    };
    while let Some(line) = lines.next() {
        let trimmed = line.trim_start();
        if let Some(_lang) = trimmed.strip_prefix("```") {
            col = flush(&mut para, col);
            let mut code = vec![];
            for l in lines.by_ref() {
                if l.trim_start().starts_with("```") {
                    break;
                }
                code.push(l);
            }
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
                    .child(code.join("\n")),
            );
        } else if trimmed.is_empty() {
            col = flush(&mut para, col);
        } else if let Some(h) = trimmed.strip_prefix('#') {
            col = flush(&mut para, col);
            let h = h.trim_start_matches('#').trim();
            col = col.child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_size(crate::system::fs(14.))
                    .child(inline(h, t)),
            );
        } else if let Some(item) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
        {
            col = flush(&mut para, col);
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
        } else if let Some((num, item)) = ordered_item(trimmed) {
            col = flush(&mut para, col);
            col = col.child(
                div()
                    .flex()
                    .min_w_0()
                    .max_w_full()
                    .gap(px(6.))
                    .pl(px(4.))
                    .child(div().text_color(t.text_muted).child(format!("{num}.")))
                    .child(div().flex_1().min_w_0().child(inline(item, t))),
            );
        } else {
            para.push(trimmed);
        }
    }
    col = flush(&mut para, col);
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
    use super::{InlineStyle, parse_inline};

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
    fn closing(src: &str, marker: &str) -> Option<usize> {
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
                if let Some(end) = code_end(&rest[count..], fence) {
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
            i += rest.chars().next().unwrap().len_utf8();
        }
        None
    }
    fn code_end(src: &str, fence: &str) -> Option<usize> {
        let mut offset = 0;
        while let Some(pos) = src[offset..].find('`') {
            let start = offset + pos;
            let count = src[start..].bytes().take_while(|b| *b == b'`').count();
            if count == fence.len() {
                return Some(start);
            }
            offset = start + count;
        }
        None
    }
    fn parse(
        mut src: &str,
        style: InlineStyle,
        depth: usize,
        out: &mut String,
        spans: &mut Vec<(std::ops::Range<usize>, InlineStyle)>,
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
                if let Some(end) = code_end(&src[count..], &src[..count]) {
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
                && let Some(end) = closing(&src[marker.len()..], marker)
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
                parse(
                    &src[marker.len()..marker.len() + end],
                    nested,
                    depth + 1,
                    out,
                    spans,
                );
                src = &src[marker.len() + end + marker.len()..];
                continue;
            }
            let c = src.chars().next().unwrap();
            append(out, spans, &src[..c.len_utf8()], style);
            src = &src[c.len_utf8()..];
        }
    }
    let mut out = String::with_capacity(src.len());
    let mut spans = vec![];
    parse(src, InlineStyle::default(), 0, &mut out, &mut spans);
    (out, spans)
}

fn inline(src: &str, t: &Theme) -> AnyElement {
    let (out, spans) = parse_inline(src);
    let highlights = spans
        .into_iter()
        .map(|(range, style)| {
            (
                range,
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
        .child(StyledText::new(out).with_highlights(highlights))
        .into_any_element()
}
