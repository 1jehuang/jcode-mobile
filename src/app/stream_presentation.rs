//! Presentation-only prefixes for live transcript entries. Raw reducer text is
//! never delayed, truncated or mutated by animation.
use std::{collections::HashMap, time::Instant};
use jcode_mobile_core::SessionState;
use crate::{markdown::StreamFade, stream_reveal::StreamReveal};

#[derive(Default)]
struct Pair {
    text: StreamReveal,
    reasoning: StreamReveal,
}

#[derive(Clone, Copy, Default)]
pub(super) struct Row {
    pub text_len: usize,
    pub reasoning_len: usize,
    pub text_fade: StreamFade,
    pub reasoning_fade: StreamFade,
}

#[derive(Default)]
pub(super) struct Frame {
    pub rows: HashMap<u64, Row>,
    pub animating: bool,
}

#[derive(Default)]
pub(super) struct Presentation {
    identity: Option<(u64, Option<String>)>,
    live: HashMap<u64, Pair>,
}

impl Presentation {
    pub fn prepare(&mut self, tab: u64, session: &SessionState, now: Instant, instant: bool) -> Frame {
        let identity = (tab, session.session_id.clone());
        let switched = self.identity.as_ref().is_some_and(|old| old != &identity);
        if self.identity.as_ref() != Some(&identity) {
            self.live.clear();
            self.identity = Some(identity);
        }
        self.live.retain(|id, _| session.transcript.iter().any(|e| e.id == *id && e.is_streaming));
        let mut frame = Frame::default();
        for e in &session.transcript {
            if !e.is_streaming { continue; }
            let pair = self.live.entry(e.id).or_default();
            // Returning to a tab is a restore, not a replay of its unseen output.
            if switched {
                pair.text.snap(e.text.len());
                pair.reasoning.snap(e.reasoning.len());
            }
            frame.animating |= pair.text.tick(&e.text, now, instant || switched);
            frame.animating |= pair.reasoning.tick(&e.reasoning, now, instant || switched);
            frame.rows.insert(e.id, Row {
                text_len: pair.text.visible(&e.text).len(),
                reasoning_len: pair.reasoning.visible(&e.reasoning).len(),
                text_fade: pair.text.fade(&e.text),
                reasoning_fade: pair.reasoning.fade(&e.reasoning),
            });
        }
        frame
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jcode_mobile_core::{ApiEvent, ConnectionOutput};
    use std::time::Duration;

    fn append(state: &mut SessionState, text: &str) {
        state.apply(ConnectionOutput::Event(ApiEvent::TextDelta {
            session_id: "fixture".into(), message_id: None, text: text.into(),
        }));
    }

    #[test]
    fn visible_prefix_does_not_mutate_raw_text_and_completion_flushes() {
        let mut state = SessionState::default();
        append(&mut state, "Done paragraph.\n\nStill arriv");
        let now = Instant::now();
        let mut p = Presentation::default();
        let frame = p.prepare(1, &state, now, false);
        let e = &state.transcript[0];
        assert_eq!(&e.text[..frame.rows[&e.id].text_len], "Done paragraph.\n\n");
        assert_eq!(e.text, "Done paragraph.\n\nStill arriv");
        state.apply(ConnectionOutput::Event(ApiEvent::TextDone {
            session_id: "fixture".into(), message_id: None,
        }));
        let done = p.prepare(1, &state, now + Duration::from_millis(16), false);
        assert!(done.rows.is_empty(), "settled entries paint all received text");
        assert!(!done.animating);
    }

    #[test]
    fn switching_tabs_restores_without_replaying_or_leaking_same_entry_id() {
        let mut a = SessionState::default();
        append(&mut a, "First unfinished");
        let mut b = SessionState::default();
        append(&mut b, "Second unfinished");
        assert_eq!(a.transcript[0].id, b.transcript[0].id);
        let mut p = Presentation::default();
        let now = Instant::now();
        let first = p.prepare(1, &a, now, false);
        assert_eq!(first.rows[&a.transcript[0].id].text_len, 0);
        let restored = p.prepare(2, &b, now, false);
        assert_eq!(restored.rows[&b.transcript[0].id].text_len, b.transcript[0].text.len());
        assert!(!restored.animating);
        assert_eq!(p.prepare(1, &a, now, false).rows[&a.transcript[0].id].text_len, a.transcript[0].text.len());
    }

    #[test]
    fn reduced_motion_and_removed_entries_do_not_keep_animation_alive() {
        let mut state = SessionState::default();
        append(&mut state, "Unfinished output");
        let mut p = Presentation::default();
        let now = Instant::now();
        let frame = p.prepare(1, &state, now, true);
        assert_eq!(frame.rows[&state.transcript[0].id].text_len, state.transcript[0].text.len());
        assert!(!frame.animating);
        state.transcript.clear();
        assert!(p.prepare(1, &state, now, false).rows.is_empty());
        assert!(p.live.is_empty());
    }
}
