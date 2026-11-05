//! Lightweight markdown for assistant replies: paragraphs, headings, bullet
//! lists, fenced code blocks, and inline `code` / **bold** spans.

use gpui::{AnyElement, FontWeight, HighlightStyle, StyledText, div, prelude::*, px};

use crate::theme::Theme;

pub fn render(src: &str, t: &Theme) -> AnyElement {
    let mut col = div().flex().flex_col().gap(px(8.)).text_size(px(13.5)).line_height(px(21.));
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
                    .p(px(10.))
                    .rounded(px(8.))
                    .bg(t.code_bg)
                    .border_1()
                    .border_color(t.border)
                    .font_family(crate::theme::FONT)
                    .text_size(px(12.))
                    .line_height(px(18.))
                    .text_color(t.text)
                    .child(code.join("\n")),
            );
        } else if trimmed.is_empty() {
            col = flush(&mut para, col);
        } else if let Some(h) = trimmed.strip_prefix('#') {
            col = flush(&mut para, col);
            let h = h.trim_start_matches('#').trim();
            col = col.child(div().font_weight(FontWeight::SEMIBOLD).text_size(px(14.)).child(inline(h, t)));
        } else if let Some(item) = trimmed.strip_prefix("- ").or_else(|| trimmed.strip_prefix("* ")) {
            col = flush(&mut para, col);
            col = col.child(
                div()
                    .flex()
                    .gap(px(8.))
                    .pl(px(4.))
                    .child(div().text_color(t.text_muted).child("•"))
                    .child(div().flex_1().min_w_0().child(inline(item, t))),
            );
        } else {
            para.push(trimmed);
        }
    }
    col = flush(&mut para, col);
    col.into_any_element()
}

/// Strips `code` and **bold** markers and highlights those ranges.
fn inline(src: &str, t: &Theme) -> AnyElement {
    let mut out = String::with_capacity(src.len());
    let mut highlights = vec![];
    let mut rest = src;
    while !rest.is_empty() {
        if let Some(r) = rest.strip_prefix('`')
            && let Some(end) = r.find('`')
        {
            let s = out.len();
            out.push_str(&r[..end]);
            highlights.push((
                s..out.len(),
                HighlightStyle { color: Some(t.text_strong), background_color: Some(t.inline_code_bg), ..Default::default() },
            ));
            rest = &r[end + 1..];
        } else if let Some(r) = rest.strip_prefix("**")
            && let Some(end) = r.find("**")
        {
            let s = out.len();
            out.push_str(&r[..end]);
            highlights.push((
                s..out.len(),
                HighlightStyle { font_weight: Some(FontWeight::SEMIBOLD), ..Default::default() },
            ));
            rest = &r[end + 2..];
        } else {
            let c = rest.chars().next().unwrap();
            out.push(c);
            rest = &rest[c.len_utf8()..];
        }
    }
    div()
        .text_color(t.text)
        .child(StyledText::new(out).with_highlights(highlights))
        .into_any_element()
}
