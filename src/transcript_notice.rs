//! Conservative recognition of reserved machine envelopes, never substring matching.
use jcode_mobile_core::{Role, TranscriptEntry};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Notice<'a> {
    pub title: String,
    pub detail: &'a str,
    pub glyph: &'static str,
    pub initially_expanded: bool,
}

impl Notice<'_> {
    /// Character heuristic only, not provider usage or billed tokens.
    pub fn estimated_tokens(&self) -> usize {
        self.detail.chars().count().div_ceil(4)
    }
}

pub(crate) fn expansion_key(id: u64) -> String {
    format!("notice:{id}")
}

pub(crate) fn for_entry(entry: &TranscriptEntry) -> Option<Notice<'_>> {
    if entry.role == Role::Assistant {
        return None;
    }
    parse(&entry.text, entry.role == Role::System)
}

pub(crate) fn is_prompt(entry: &TranscriptEntry) -> bool {
    entry.role == Role::User && for_entry(entry).is_none()
}

pub(crate) fn parse(text: &str, system: bool) -> Option<Notice<'_>> {
    // Do not trim the start: quoted/indented examples are ordinary prose.
    let mut detail = text;
    let mut summary = None;
    let mut initially_expanded = false;
    if let Some(rest) = text.strip_prefix("\u{200b}\u{200b}") {
        let (state, rest) = rest.split_once("\u{200b}\u{200b}")?;
        initially_expanded = match state {
            "swarm-tldr:collapsed" => false,
            "swarm-tldr:expanded" => true,
            _ => return None,
        };
        let (tldr, body) = rest.split_once('\n').unwrap_or((rest, ""));
        summary = Some(tldr);
        detail = if body.is_empty() { tldr } else { body };
    }
    let header = detail.lines().next().unwrap_or("").trim_end_matches('\r');
    let mut kind = None;
    for (prefix, title, glyph) in [
        ("**Background task progress** `", "Background progress", "◌"),
        ("**Background task stalled** `", "Background stalled", "!"),
        ("**Background task started** `", "Background started", "◌"),
        ("**Background task** `", "Background task", "✓"),
    ] {
        if let Some(rest) = header.strip_prefix(prefix) {
            let (id, tail) = rest.split_once('`')?;
            if id.is_empty() || !tail.starts_with(" · `") {
                return None;
            }
            let tail = tail.trim_start_matches(" · ");
            let mut headline = format!("{title} · {id} · {}", tail.replace('`', ""));
            if prefix == "**Background task progress** `"
                && let Some(progress) = detail.lines().skip(1).find(|line| !line.trim().is_empty())
            {
                headline.push_str(" · ");
                headline.push_str(progress);
            }
            let glyph = if prefix == "**Background task** `"
                && (tail.contains("✗") || tail.contains("failed"))
            {
                "✗"
            } else {
                glyph
            };
            kind = Some((headline, glyph));
            break;
        }
    }
    if kind.is_none() {
        for prefix in ["DM from ", "Cross-swarm DM from ", "broadcast from "] {
            if let Some(rest) = header.strip_prefix(prefix)
                && let Some((sender, body)) = rest.split_once(": ")
                && !sender.trim().is_empty()
                && (prefix == "Cross-swarm DM from " || !sender.contains(char::is_whitespace))
            {
                kind = Some((format!("{prefix}{sender} · {body}"), "✉"));
                break;
            }
        }
    }
    if kind.is_none()
        && let Some(rest) = header.strip_prefix('#')
        && let Some((head, body)) = rest.split_once(": ")
        && let Some((channel, sender)) = head.split_once(" from ")
        && !channel.is_empty()
        && !channel.contains(char::is_whitespace)
        && !sender.is_empty()
        && !sender.contains(char::is_whitespace)
    {
        kind = Some((format!("#{channel} · {sender} · {body}"), "✉"));
    }
    if kind.is_none() && header == "🐝 **Swarm await finished**" {
        kind = Some(("Swarm await finished".into(), "◇"));
    }
    if kind.is_none()
        && (header.starts_with("⚠ File activity: ") || header.starts_with("⚠️ File activity: "))
    {
        kind = Some((header.to_string(), "!"));
    }
    if kind.is_none()
        && header.starts_with("Agent ")
        && [
            " finished their work and is ready for more.",
            " finished with status failed.",
            " stopped.",
            " crashed while working.",
            " completed their work.",
        ]
        .iter()
        .any(|suffix| header.ends_with(suffix))
        && detail.contains("\n\nLane plan:\n")
        && detail.contains("\n\nFinal response:\n")
    {
        kind = Some((header.to_string(), "◇"));
    }
    if kind.is_none() && summary.is_some() {
        kind = Some(("Agent message".into(), "✉"));
    }
    if kind.is_none() && system {
        kind = Some((format!("Notification · {header}"), "·"));
    }
    let (title, glyph) = kind?;
    Some(Notice {
        title: summary
            .filter(|s| !s.is_empty())
            .map_or(title, str::to_string),
        detail,
        glyph,
        initially_expanded,
    })
}
