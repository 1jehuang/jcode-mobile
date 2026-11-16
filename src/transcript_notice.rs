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

/// Approximation marker distinguishes this character heuristic from billed usage.
pub(crate) fn token_badge(tokens: usize) -> String {
    if tokens < 1_000 {
        format!("~{tokens} tok")
    } else if tokens < 1_000_000 {
        format!("~{:.1}k tok", tokens as f64 / 1_000.)
    } else {
        format!("~{:.1}m tok", tokens as f64 / 1_000_000.)
    }
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
    for (prefix, glyph) in [
        ("**Background task progress** `", "◌"),
        ("**Background task stalled** `", "!"),
        ("**Background task started** `", "◌"),
        ("**Background task** `", "✓"),
    ] {
        if let Some(rest) = header.strip_prefix(prefix) {
            let (id, tail) = rest.split_once('`')?;
            if id.is_empty() || !tail.starts_with(" · `") {
                return None;
            }
            let tail = tail.trim_start_matches(" · ");
            let (label, status_text) = tail
                .strip_prefix('`')
                .and_then(|rest| rest.split_once('`'))
                .unwrap_or((id, ""));
            let mut headline = label.to_string();
            if prefix == "**Background task progress** `"
                && let Some(progress) = detail.lines().skip(1).find(|line| !line.trim().is_empty())
            {
                headline.push_str(" · ");
                headline.push_str(progress);
            }
            let glyph = if prefix == "**Background task** `" {
                if status_text.contains("✗ failed") {
                    "✗"
                } else if status_text.contains("✓ completed") {
                    "✓"
                } else if status_text.contains("cancelled") || status_text.contains("canceled") {
                    "⊘"
                } else {
                    "◌"
                }
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
                kind = Some((format!("{sender} · {body}"), "✉"));
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
        let path = header
            .split_once("File activity: ")
            .map(|(_, rest)| rest)
            .unwrap_or(header);
        let path = path.split(['\n', '—']).next().unwrap_or(path).trim();
        let file = path.rsplit('/').next().unwrap_or(path);
        let intent = header.split_once(" — intent: ").map(|(_, intent)| intent);
        let title = match intent {
            Some(intent) => format!("{file} · {intent}"),
            None => file.to_string(),
        };
        kind = Some((title, "▤"));
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
        let rest = header.strip_prefix("Agent ").unwrap_or(header);
        let name = rest.split_whitespace().next().unwrap_or("agent");
        let status = if header.ends_with(" finished with status failed.")
            || header.ends_with(" crashed while working.")
        {
            "failed"
        } else if header.ends_with(" stopped.") {
            "stopped"
        } else {
            "done"
        };
        kind = Some((
            format!("{name} · {status}"),
            if status == "failed" { "✗" } else { "◇" },
        ));
    }
    if kind.is_none() && summary.is_some() {
        kind = Some(("Agent message".into(), "✉"));
    }
    if kind.is_none()
        && header.starts_with(
            "[auto] Your session was interrupted by a server reload while a tool was running.",
        )
    {
        kind = Some(("Session restored after server reload".into(), "↻"));
    }
    if kind.is_none() && system {
        kind = Some((
            if header.is_empty() {
                "Notification".into()
            } else {
                header.to_string()
            },
            "ⓘ",
        ));
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
