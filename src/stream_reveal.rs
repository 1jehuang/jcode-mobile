//! Chunked reveal for live reasoning and response text.
//!
//! Providers deliver text a few tokens at a time. Painting every token, or
//! pacing a word-by-word reveal, keeps the transcript in constant motion: each
//! word nudges the line, each wrap pushes everything below it down. Instead
//! the received text is released in whole blocks at natural boundaries:
//!
//! * as soon as a line or paragraph completes,
//! * after a short hold, up to the last sentence (or word) of a long line,
//! * and immediately once the stream pauses.
//!
//! Releases are rate limited, so the transcript changes a few times per second
//! at most, and each released block fades in as a unit.

use std::time::{Duration, Instant};

use crate::markdown::StreamFade;

/// Minimum time between two releases. Bounds how often the transcript moves.
const MIN_INTERVAL: Duration = Duration::from_millis(140);
/// How long unreleased text without a line break may wait before the reveal
/// settles for a sentence or word boundary instead.
const HOLD: Duration = Duration::from_millis(260);
/// The stream has paused when nothing arrived for this long. Release it all.
const IDLE: Duration = Duration::from_millis(160);
/// How long a released block takes to fade in.
const FADE: Duration = Duration::from_millis(220);
/// Opacity deficit a block starts its fade at.
const FADE_FROM: f32 = 0.85;
/// Larger backlogs (history restores, reconnect recovery) appear immediately.
const SNAP_BACKLOG: usize = 6_000;

#[derive(Debug, Clone, Default)]
pub(crate) struct StreamReveal {
    /// Released prefix length in bytes, always on a char boundary.
    shown: usize,
    /// Start of the newest released block, which fades in.
    fade_from: usize,
    /// Current fade of the newest block, 0 when fully opaque.
    fade: f32,
    target: usize,
    previous: Option<String>,
    released_at: Option<Instant>,
    /// When the oldest unreleased byte arrived.
    pending_since: Option<Instant>,
    /// When the received text last grew.
    grew_at: Option<Instant>,
    /// Only a rendered, animating stream is chunked. Until then (tests,
    /// reduced motion, rows read outside a frame) the full text is shown.
    engaged: bool,
}

impl StreamReveal {
    /// Show `len` bytes immediately, for text that did not arrive as a live stream.
    pub(crate) fn snap(&mut self, len: usize) {
        *self = Self {
            shown: len,
            fade_from: len,
            target: len,
            ..Self::default()
        };
    }

    /// Advance toward the received `text`. Returns true while still animating.
    pub(crate) fn tick(&mut self, text: &str, now: Instant, instant: bool) -> bool {
        let len = text.len();
        if len < self.target
            || len == 0
            || self.shown > len
            || self
                .previous
                .as_ref()
                .is_some_and(|old| !text.starts_with(old))
        {
            // Cleared or replaced: the live row starts over.
            self.snap(0);
        }
        if len > self.target {
            self.grew_at = Some(now);
        }
        self.target = len;
        if self.previous.as_deref() != Some(text) {
            self.previous = Some(text.to_owned());
        }
        if instant || len - self.shown > SNAP_BACKLOG {
            let previous = self.previous.take();
            self.snap(len);
            self.previous = previous;
            return false;
        }
        self.engaged = true;

        if self.shown < len {
            let pending_since = *self.pending_since.get_or_insert(now);
            let rested = self
                .released_at
                .is_none_or(|at| now.saturating_duration_since(at) >= MIN_INTERVAL);
            if rested {
                let paused = self
                    .grew_at
                    .is_none_or(|at| now.saturating_duration_since(at) >= IDLE);
                let held = now.saturating_duration_since(pending_since) >= HOLD;
                if let Some(cut) = release_point(text, self.shown, paused, held) {
                    self.fade_from = self.shown;
                    self.shown = cut;
                    self.released_at = Some(now);
                    self.pending_since = (cut < len).then_some(now);
                }
            }
        }

        let progress = self.released_at.map_or(1.0, |at| {
            now.saturating_duration_since(at).as_secs_f32() / FADE.as_secs_f32()
        });
        self.fade = if progress >= 1.0 || self.fade_from >= self.shown {
            0.0
        } else {
            // Ease out: most of the block is legible almost at once.
            FADE_FROM * (1.0 - progress).powi(2)
        };
        self.shown < len || self.fade > 0.0
    }

    /// The released prefix.
    pub(crate) fn visible<'a>(&self, text: &'a str) -> &'a str {
        if !self.engaged {
            return text;
        }
        &text[..text.floor_char_boundary(self.shown.min(text.len()))]
    }

    /// Bytes at the end of the visible prefix of `text` still fading in.
    pub(crate) fn fading(&self, text: &str) -> usize {
        if !self.engaged || self.fade <= 0.0 {
            return 0;
        }
        self.shown.min(text.len()).saturating_sub(self.fade_from)
    }

    /// The fade applied to the newest block of `text` when painting it.
    pub(crate) fn fade(&self, text: &str) -> StreamFade {
        StreamFade {
            bytes: self.fading(text),
            amount: self.fade,
        }
    }

    #[cfg(test)]
    pub(crate) fn shown_len(&self) -> usize {
        if !self.engaged {
            return self.target;
        }
        self.shown
    }
}

/// Where to cut the next release of `text` beyond `from`, if anywhere yet.
fn release_point(text: &str, from: usize, paused: bool, held: bool) -> Option<usize> {
    let pending = &text[from..];
    if pending.is_empty() {
        return None;
    }
    if paused {
        return Some(text.len());
    }
    // A completed line or paragraph is a block: release it right away.
    if let Some(newline) = pending.rfind('\n') {
        return Some(from + newline + 1);
    }
    if !held {
        return None;
    }
    // A long line is still arriving. Settle for its last full sentence, else
    // its last full word, so the line grows in phrases rather than letters.
    let sentence = pending
        .match_indices([' ', '\t'])
        .rev()
        .find(|(at, _)| {
            pending[..*at]
                .chars()
                .next_back()
                .is_some_and(|c| matches!(c, '.' | '!' | '?' | ':' | ';'))
        })
        .map(|(at, _)| at + 1);
    let word = pending.rfind([' ', '\t']).map(|at| at + 1);
    sentence
        .or(word)
        .map(|cut| from + cut)
        .filter(|&cut| cut > from)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAME: Duration = Duration::from_millis(16);

    #[test]
    fn exact_minimum_release_interval_and_backlog_threshold() {
        let now = Instant::now();
        let mut reveal = StreamReveal::default();
        reveal.tick("one\n", now, false);
        reveal.tick(
            "one\ntwo\n",
            now + MIN_INTERVAL - Duration::from_nanos(1),
            false,
        );
        assert_eq!(reveal.visible("one\ntwo\n"), "one\n");
        reveal.tick("one\ntwo\n", now + MIN_INTERVAL, false);
        assert_eq!(reveal.visible("one\ntwo\n"), "one\ntwo\n");
        assert_eq!(reveal.fade("one\ntwo\n").bytes, 4);

        let mut threshold = StreamReveal::default();
        let text = "x".repeat(SNAP_BACKLOG);
        assert!(threshold.tick(&text, now, false));
        assert_eq!(threshold.visible(&text), "");
        let text = format!("{text}x");
        assert!(!threshold.tick(&text, now + FRAME, false));
        assert_eq!(threshold.visible(&text), text);
        assert_eq!(threshold.fade(&text).amount, 0.0);
    }

    #[test]
    fn same_length_text_replace_resets_and_respects_utf8() {
        let now = Instant::now();
        let mut reveal = StreamReveal::default();
        reveal.tick("abc\nx", now, false);
        assert_eq!(reveal.visible("abc\nx"), "abc\n");
        // Same byte length, but the old cut is now inside a Unicode scalar.
        let replaced = "a🦀";
        assert!(reveal.tick(replaced, now + FRAME, false));
        assert_eq!(reveal.visible(replaced), "");
        reveal.tick(replaced, now + FRAME + IDLE, false);
        assert_eq!(reveal.visible(replaced), replaced);
        assert_eq!(reveal.fade(replaced).bytes, replaced.len());
    }

    #[test]
    fn history_snap_and_reset_do_not_reanimate_old_text() {
        let now = Instant::now();
        let mut reveal = StreamReveal::default();
        reveal.snap("history\n".len());
        assert!(!reveal.tick("history\n", now, false));
        assert_eq!(reveal.fade("history\n"), StreamFade::default());
        reveal.tick("history\nnew\n", now + MIN_INTERVAL, false);
        assert_eq!(reveal.fade("history\nnew\n").bytes, 4);
        reveal.snap(0);
        reveal.tick("next", now + MIN_INTERVAL, false);
        assert_eq!(reveal.visible("next"), "");
        reveal.tick("next", now + MIN_INTERVAL + IDLE, false);
        assert_eq!(reveal.visible("next"), "next");
    }

    #[test]
    fn exact_idle_release_and_fade_deadlines() {
        let now = Instant::now();
        let mut reveal = StreamReveal::default();
        reveal.tick("é", now, false);
        reveal.tick("é", now + IDLE - Duration::from_nanos(1), false);
        assert_eq!(reveal.visible("é"), "");
        assert!(reveal.tick("é", now + IDLE, false));
        assert_eq!(reveal.visible("é"), "é");
        assert!(!reveal.tick("é", now + IDLE + FADE, false));
        assert_eq!(reveal.fade("é").amount, 0.0);
    }

    #[test]
    fn completed_lines_are_released_as_whole_blocks() {
        let mut reveal = StreamReveal::default();
        let start = Instant::now();
        let text = "First paragraph is done.\n\nSecond is still arr";
        assert!(reveal.tick(text, start, false));
        assert_eq!(reveal.visible(text), "First paragraph is done.\n\n");
        assert!(reveal.fading(text) > 0, "the new block fades in");
        assert!(reveal.fade(text).amount > 0.5);
    }

    #[test]
    fn partial_lines_wait_then_release_at_a_sentence() {
        let mut reveal = StreamReveal::default();
        let mut now = Instant::now();
        let mut text = String::new();
        // A steady stream with no line break: tokens every frame.
        let words = "This sentence ends here. And this one keeps going on and on ".repeat(3);
        let mut first_release = None;
        for (frame, word) in words.split_inclusive(' ').enumerate() {
            text.push_str(word);
            reveal.tick(&text, now, false);
            if first_release.is_none() && reveal.shown_len() > 0 {
                first_release = Some((frame, reveal.visible(&text).to_owned()));
            }
            now += FRAME;
        }
        let (frame, shown) = first_release.expect("released while streaming");
        assert!(frame as u32 * 16 >= HOLD.as_millis() as u32, "held first");
        assert!(
            shown.ends_with(". ") || shown.ends_with(' '),
            "cut at a boundary: {shown:?}"
        );
    }

    #[test]
    fn releases_are_rate_limited() {
        let mut reveal = StreamReveal::default();
        let mut now = Instant::now();
        let mut text = String::new();
        let mut releases = 0;
        let mut previous = 0;
        // A new line every frame for a second.
        for n in 0..60 {
            text.push_str(&format!("line {n}\n"));
            reveal.tick(&text, now, false);
            if reveal.shown_len() != previous {
                releases += 1;
                previous = reveal.shown_len();
            }
            now += FRAME;
        }
        assert!(releases <= 8, "{releases} releases in a second");
        assert!(text.len() - reveal.shown_len() < 80, "kept up eagerly");
    }

    #[test]
    fn a_paused_stream_releases_everything_and_settles_opaque() {
        let mut reveal = StreamReveal::default();
        let start = Instant::now();
        let text = "no boundary at all yet";
        assert!(reveal.tick(text, start, false));
        assert_eq!(reveal.shown_len(), 0);
        let mut now = start;
        for _ in 0..60 {
            now += FRAME;
            reveal.tick(text, now, false);
        }
        assert_eq!(reveal.visible(text), text);
        assert_eq!(reveal.fading(text), 0);
        assert!(!reveal.tick(text, now + FRAME, false));
    }

    #[test]
    fn clears_restores_and_reduced_motion_snap() {
        let mut reveal = StreamReveal::default();
        let now = Instant::now();
        reveal.tick(&"x".repeat(100), now, false);
        assert!(!reveal.tick("", now, false));
        assert_eq!(reveal.shown_len(), 0);

        let big = "y".repeat(SNAP_BACKLOG + 1);
        assert!(!reveal.tick(&big, now, false));
        assert_eq!(reveal.shown_len(), SNAP_BACKLOG + 1);

        let mut reduced = StreamReveal::default();
        let text = "x".repeat(50);
        assert!(!reduced.tick(&text, now, true));
        assert_eq!((reduced.shown_len(), reduced.fading(&text)), (50, 0));
        assert_eq!(
            reduced.visible(&text),
            text,
            "unengaged passes text through"
        );
    }

    #[test]
    fn release_points_respect_char_boundaries() {
        let text = "αβ γδ. εζ";
        let cut = release_point(text, 0, false, true).unwrap();
        assert!(text.is_char_boundary(cut));
        assert_eq!(&text[..cut], "αβ γδ. ");
        assert_eq!(release_point("αβγ", 0, false, true), None);
        assert_eq!(release_point("αβγ", 0, true, false), Some("αβγ".len()));
    }
}
