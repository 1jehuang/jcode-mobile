//! Measured-height windowing for the existing variable-height transcript.
//!
//! Keep the native ScrollHandle and exact full-document geometry. The first
//! layout measures every row. Subsequent frames replace unchanged offscreen
//! subtrees with equal-height spacers, including during swipe/follow animations.
use gpui::{Pixels, px};
use jcode_mobile_core::{Role, ToolStatus, TranscriptEntry};
use std::{
    cell::RefCell,
    collections::HashMap,
    hash::{Hash, Hasher},
    rc::Rc,
};

const MAX_ROWS: usize = 4096;
const OVERSCAN: f32 = 400.;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Key {
    tab: u64,
    entry: u64,
    fingerprint: u64,
    width: u32,
    scale: u32,
}

pub(super) fn key(
    tab: u64,
    entry: &TranscriptEntry,
    prompt: usize,
    expanded: &std::collections::HashSet<String>,
    width: f32,
) -> Key {
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    entry.text.hash(&mut hash);
    entry.reasoning.hash(&mut hash);
    std::mem::discriminant(&entry.role).hash(&mut hash);
    (if prompt == 0 { 1 } else { prompt.ilog10() + 1 }).hash(&mut hash);
    if entry.role == Role::Assistant {
        for call in &entry.tool_calls {
            call.id.hash(&mut hash);
            call.name.hash(&mut hash);
            call.input.hash(&mut hash);
            let open = expanded.contains(&call.id);
            open.hash(&mut hash);
            // Status text and output only affect height when expanded.
            if open {
                let failure = match &call.status {
                    ToolStatus::Failed(message) => Some(message.as_str()),
                    _ => None,
                };
                let selected = if call.output.is_empty() {
                    failure.unwrap_or("")
                } else {
                    &call.output
                };
                if selected.is_empty() {
                    call.input.hash(&mut hash);
                } else {
                    // The renderer only displays this prefix. Hash it without
                    // allocating a truncated String on every animation frame.
                    let mut end = selected.len().min(crate::tool_output::OUTPUT_BYTES);
                    while !selected.is_char_boundary(end) {
                        end -= 1;
                    }
                    selected[..end].hash(&mut hash);
                    (end < selected.len()).hash(&mut hash);
                }
            }
        }
    }
    Key {
        tab,
        entry: entry.id,
        fingerprint: hash.finish(),
        width: width.to_bits(),
        scale: crate::system::text_scale().to_bits(),
    }
}

#[derive(Clone, Default)]
pub(super) struct Heights(Rc<RefCell<HashMap<(u64, u64), (Key, Pixels)>>>);

impl Heights {
    pub fn get(&self, key: Key) -> Option<Pixels> {
        self.0
            .borrow()
            .get(&(key.tab, key.entry))
            .filter(|(stored, _)| *stored == key)
            .map(|(_, height)| *height)
    }

    pub fn record(&self, key: Key, height: Pixels) {
        if !f32::from(height).is_finite() || height < px(0.) {
            return;
        }
        let mut rows = self.0.borrow_mut();
        if rows.len() >= MAX_ROWS && !rows.contains_key(&(key.tab, key.entry)) {
            rows.clear();
        }
        rows.insert((key.tab, key.entry), (key, height));
    }
}

/// Unknown preceding heights disable culling until layout has measured them.
pub(super) fn offscreen(
    top: Option<Pixels>,
    height: Pixels,
    scroll: Pixels,
    viewport: Pixels,
) -> bool {
    viewport > px(0.)
        && top.is_some_and(|top| {
            top + height < scroll - px(OVERSCAN) || top > scroll + viewport + px(OVERSCAN)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry() -> TranscriptEntry {
        TranscriptEntry {
            id: 1,
            role: Role::Assistant,
            text: "hello".into(),
            reasoning: String::new(),
            tool_calls: vec![],
            is_streaming: false,
            is_queued: false,
        }
    }

    #[test]
    fn measurements_invalidate_on_same_length_edits_resize_scale_and_tab() {
        let heights = Heights::default();
        let mut entry = entry();
        let expanded = Default::default();
        let original = key(1, &entry, 0, &expanded, 362.);
        heights.record(original, px(123.));
        assert_eq!(heights.get(original), Some(px(123.)));
        entry.text = "world".into();
        assert_eq!(heights.get(key(1, &entry, 0, &expanded, 362.)), None);
        entry.text = "hello".into();
        assert_eq!(heights.get(key(2, &entry, 0, &expanded, 362.)), None);
        assert_eq!(heights.get(key(1, &entry, 0, &expanded, 363.)), None);
        let mut scaled = original;
        scaled.scale = 1.3f32.to_bits();
        assert_eq!(heights.get(scaled), None);
    }

    #[test]
    fn unknown_geometry_and_overscan_never_hide_visible_rows() {
        assert!(!offscreen(None, px(50.), px(1000.), px(600.)));
        assert!(!offscreen(Some(px(900.)), px(200.), px(1000.), px(600.)));
        assert!(!offscreen(Some(px(600.)), px(10.), px(1000.), px(600.)));
        assert!(offscreen(Some(px(0.)), px(50.), px(1000.), px(600.)));
        assert!(offscreen(Some(px(2200.)), px(50.), px(1000.), px(600.)));
        assert!(!offscreen(Some(px(2200.)), px(50.), px(1000.), px(0.)));
    }

    #[test]
    fn tools_invalidate_only_when_displayed_geometry_changes() {
        let heights = Heights::default();
        let mut entry = entry();
        entry.tool_calls.push(jcode_mobile_core::ToolCall {
            id: "tool".into(),
            name: "read".into(),
            input: r#"{"path":"fixture.rs"}"#.into(),
            output: "short result".into(),
            status: ToolStatus::Running,
        });
        let mut expanded = std::collections::HashSet::new();
        let original = key(0, &entry, 0, &expanded, 362.);
        heights.record(original, px(123.));
        entry.tool_calls[0].output = "large hidden output".repeat(1000);
        assert_eq!(
            heights.get(key(0, &entry, 0, &expanded, 362.)),
            Some(px(123.))
        );
        expanded.insert("tool".into());
        let open = key(0, &entry, 0, &expanded, 362.);
        assert_eq!(heights.get(open), None);
        heights.record(open, px(456.));
        // Output beyond the displayed prefix cannot change row height.
        entry.tool_calls[0].output.push_str("more hidden bytes");
        assert_eq!(
            heights.get(key(0, &entry, 0, &expanded, 362.)),
            Some(px(456.))
        );
        entry.tool_calls[0].output = String::new();
        entry.tool_calls[0].status = ToolStatus::Failed("failure message".into());
        assert_eq!(heights.get(key(0, &entry, 0, &expanded, 362.)), None);
        expanded.clear();
        entry.tool_calls[0].input.push(' ');
        assert_eq!(heights.get(key(0, &entry, 0, &expanded, 362.)), None);
    }

    #[test]
    fn measurement_storage_is_bounded_and_rejects_invalid_sizes() {
        let heights = Heights::default();
        let mut entry = entry();
        for id in 0..MAX_ROWS as u64 + 5 {
            entry.id = id;
            heights.record(key(0, &entry, 0, &Default::default(), 362.), px(20.));
            assert!(heights.0.borrow().len() <= MAX_ROWS);
        }
        let key = key(0, &entry, 0, &Default::default(), 362.);
        heights.record(key, px(f32::NAN));
        heights.record(key, px(-1.));
        assert_eq!(heights.get(key), Some(px(20.)));
    }
}
