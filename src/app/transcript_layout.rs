//! Measured-height windowing for the existing variable-height transcript.
//!
//! Keep the native ScrollHandle and exact full-document geometry. The first
//! layout measures every row. Subsequent frames replace unchanged offscreen
//! subtrees with equal-height spacers, including during swipe/follow animations.
use gpui::{
    AnyElement, App, Bounds, Element, ElementId, GlobalElementId, InspectorElementId, IntoElement,
    LayoutId, Pixels, Window, px,
};
use jcode_mobile_core::{Role, ToolStatus, TranscriptEntry};
use std::{
    cell::RefCell,
    collections::HashMap,
    hash::{Hash, Hasher},
    rc::Rc,
};

const MAX_ROWS: usize = 4096;
const MAX_SHARED_ROWS: usize = 4096;
const OVERSCAN: f32 = 400.;

/// `height_after` is a lower bound because unmeasured suffix rows count zero.
/// The full window height safely bounds the transcript even during a resize.
pub(super) fn before_tail(height_after: Pixels, viewport: Pixels) -> bool {
    viewport > px(0.) && height_after > viewport + px(OVERSCAN)
}

/// Collapse contiguous hidden rows into one layout node. Preserve the internal
/// row gaps here, while the parent flex gap supplies gaps between runs.
#[derive(Default)]
pub(super) struct Rows {
    elements: Vec<gpui::AnyElement>,
    hidden: Option<Pixels>,
    #[cfg(test)]
    pub uncoalesced: bool,
}

impl Rows {
    pub fn hide(&mut self, height: Pixels) {
        #[cfg(test)]
        if self.uncoalesced {
            self.hidden = Some(height);
            self.flush();
            return;
        }
        self.hidden = Some(self.hidden.map_or(height, |sum| sum + px(14.) + height));
    }

    fn flush(&mut self) {
        use gpui::prelude::*;
        if let Some(height) = self.hidden.take() {
            self.elements
                .push(gpui::div().h(height).flex_none().into_any_element());
        }
    }

    pub fn push(&mut self, element: gpui::AnyElement) {
        self.flush();
        self.elements.push(element);
    }

    pub fn finish(mut self) -> Vec<gpui::AnyElement> {
        self.flush();
        self.elements
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Key {
    tab: u64,
    entry: u64,
    fingerprint: u64,
    width: u32,
    scale: u32,
    density: u32,
    shareable: bool,
}

impl Key {
    /// Native text measurement is snapped at the window's device scale.
    pub(super) fn with_window_scale(mut self, scale: f32) -> Self {
        self.density = scale.to_bits();
        self
    }

    pub(super) fn with_images(mut self, images: &[jcode_mobile_core::SessionImage]) -> Self {
        if images.is_empty() {
            return self;
        }
        // Image loading and dimensions are not determined by prose content.
        self.shareable = false;
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        self.fingerprint.hash(&mut hash);
        for image in images {
            image.id.hash(&mut hash);
        }
        self.fingerprint = hash.finish();
        self
    }
}

pub(super) fn key(
    tab: u64,
    entry: &TranscriptEntry,
    prompt: usize,
    expanded: &std::collections::HashSet<String>,
    width: f32,
) -> Key {
    key_visible(
        tab,
        entry,
        prompt,
        expanded,
        width,
        &entry.text,
        &entry.reasoning,
    )
}

pub(super) fn key_visible(
    tab: u64,
    entry: &TranscriptEntry,
    prompt: usize,
    expanded: &std::collections::HashSet<String>,
    width: f32,
    text: &str,
    reasoning: &str,
) -> Key {
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hash);
    reasoning.hash(&mut hash);
    std::mem::discriminant(&entry.role).hash(&mut hash);
    (if prompt == 0 { 1 } else { prompt.ilog10() + 1 }).hash(&mut hash);
    let activity = crate::transcript_notice::for_entry(entry).is_some();
    if activity {
        expanded.contains(&crate::transcript_notice::expansion_key(entry.id)).hash(&mut hash);
    }
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
        density: 1f32.to_bits(),
        // Limit cross-identity reuse to settled prose. Other row types can
        // depend on identity-specific expansion or asynchronous resources.
        shareable: !activity && matches!(entry.role, Role::User | Role::Assistant)
            && entry.tool_calls.is_empty()
            && entry.reasoning.is_empty()
            && !entry.is_streaming,
    }
}

type Measurements = HashMap<(u64, u64), (Key, Pixels)>;

/// Content-derived geometry only, with no row/account identifiers or raw text.
/// The fingerprint includes rendered text, role, and prompt badge digit width.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct GeometryKey {
    fingerprint: u64,
    width: u32,
    scale: u32,
    density: u32,
}

impl Key {
    fn geometry(self) -> Option<GeometryKey> {
        self.shareable.then_some(GeometryKey {
            fingerprint: self.fingerprint,
            width: self.width,
            scale: self.scale,
            density: self.density,
        })
    }
}

#[derive(Clone, Default)]
pub(super) struct Heights(
    Rc<RefCell<Measurements>>,
    Rc<RefCell<HashMap<GeometryKey, Pixels>>>,
);

impl Heights {
    pub fn get(&self, key: Key) -> Option<Pixels> {
        self.0
            .borrow()
            .get(&(key.tab, key.entry))
            .filter(|(stored, _)| *stored == key)
            .map(|(_, height)| *height)
            .or_else(|| {
                key.geometry()
                    .and_then(|geometry| self.1.borrow().get(&geometry).copied())
            })
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
        if let Some(geometry) = key.geometry() {
            let mut shared = self.1.borrow_mut();
            if shared.len() >= MAX_SHARED_ROWS && !shared.contains_key(&geometry) {
                // Bounded retention, just like the primary row cache. Eviction
                // only causes a fresh native measurement, never an estimate.
                shared.clear();
            }
            shared.insert(geometry, height);
        }
    }
}

/// Observe the row's native geometry without adding a measurement canvas to
/// its layout tree. The child keeps its original layout node and constraints.
pub(super) fn measured_row(child: AnyElement, key: Key, heights: Heights) -> MeasuredRow {
    MeasuredRow {
        child,
        key,
        heights,
    }
}

pub(super) struct MeasuredRow {
    child: AnyElement,
    key: Key,
    heights: Heights,
}

impl IntoElement for MeasuredRow {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Element for MeasuredRow {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        (self.child.request_layout(window, cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        self.heights.record(self.key, bounds.size.height);
        self.child.prepaint(window, cx);
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        self.child.paint(window, cx);
    }
}

#[cfg(test)]
#[path = "cold_layout_tests.rs"]
mod cold_layout_tests;

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

    #[test]
    fn hidden_runs_collapse_and_preserve_internal_gaps() {
        use gpui::prelude::*;
        let mut rows = Rows::default();
        rows.hide(px(20.));
        rows.hide(px(30.));
        assert_eq!(rows.hidden, Some(px(64.)));
        rows.push(gpui::div().into_any_element());
        rows.hide(px(40.));
        rows.hide(px(50.));
        assert_eq!(rows.hidden, Some(px(104.)));
        assert_eq!(rows.finish().len(), 3);
        assert!(Rows::default().finish().is_empty());
    }

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
    fn pending_raw_text_preserves_measurements_until_visible_prefix_changes() {
        let heights = Heights::default();
        let mut entry = entry();
        entry.is_streaming = true;
        entry.text = "Visible paragraph.\n\nPending".into();
        let expanded = Default::default();
        let visible = "Visible paragraph.\n\n";
        let original = key_visible(1, &entry, 0, &expanded, 362., visible, "");
        heights.record(original, px(123.));
        entry.text.push_str(" more arriving tokens");
        assert_eq!(
            heights.get(key_visible(1, &entry, 0, &expanded, 362., visible, "")),
            Some(px(123.))
        );
        assert_eq!(
            heights.get(key_visible(1, &entry, 0, &expanded, 362., &entry.text, "")),
            None,
            "idle release must invalidate cached geometry even with no new raw event"
        );
        assert_eq!(
            heights.get(key_visible(
                1,
                &entry,
                0,
                &expanded,
                362.,
                "Changed paragraph.\n\n",
                ""
            )),
            None,
            "same-length visible replacements must invalidate measurements"
        );
    }

    #[test]
    fn measurements_invalidate_on_edits_resize_scale_but_reuse_identical_prose_across_tabs() {
        let heights = Heights::default();
        let mut entry = entry();
        let expanded = Default::default();
        let original = key(1, &entry, 0, &expanded, 362.);
        heights.record(original, px(123.));
        assert_eq!(heights.get(original), Some(px(123.)));
        entry.text = "world".into();
        assert_eq!(heights.get(key(1, &entry, 0, &expanded, 362.)), None);
        entry.text = "hello".into();
        assert_eq!(
            heights.get(key(2, &entry, 0, &expanded, 362.)),
            Some(px(123.))
        );
        assert_eq!(heights.get(key(1, &entry, 0, &expanded, 363.)), None);
        let mut scaled = original;
        scaled.scale = 1.3f32.to_bits();
        assert_eq!(heights.get(scaled), None);
    }

    #[test]
    fn unknown_geometry_and_overscan_never_hide_visible_rows() {
        assert!(!before_tail(px(1000.), px(600.)));
        assert!(before_tail(px(1001.), px(600.)));
        assert!(!before_tail(px(1001.), px(0.)));
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
    fn shared_geometry_is_bounded_and_never_records_invalid_measurements() {
        let heights = Heights::default();
        let mut entry = entry();
        let expanded = Default::default();
        let oldest = key(1, &entry, 1, &expanded, 362.);
        heights.record(oldest, px(20.));
        for index in 0..MAX_SHARED_ROWS {
            entry.text = format!("distinct measured prose {index}");
            heights.record(key(1, &entry, 1, &expanded, 362.), px(30.));
            assert!(heights.1.borrow().len() <= MAX_SHARED_ROWS);
        }
        assert_eq!(
            heights.1.borrow().len(),
            1,
            "full cache is evicted before insertion"
        );
        assert_eq!(
            heights.get(oldest),
            None,
            "eviction requires fresh measurement"
        );
        let latest = key(2, &entry, 1, &expanded, 362.);
        assert_eq!(heights.get(latest), Some(px(30.)));
        heights.record(latest, px(f32::NAN));
        heights.record(latest, px(f32::INFINITY));
        heights.record(latest, px(-1.));
        assert_eq!(heights.get(latest), Some(px(30.)));
        entry.text = "never measured".into();
        let unknown = key(3, &entry, 1, &expanded, 362.);
        heights.record(unknown, px(f32::NAN));
        assert_eq!(heights.get(unknown), None);
        assert_eq!(heights.1.borrow().len(), 1);
    }

    #[test]
    fn activity_disclosure_invalidates_height_and_never_shares_identity_state() {
        let heights = Heights::default();
        let mut entry = entry();
        entry.role = Role::System;
        entry.text = "DM from fox: Details\n\nMany more details".into();
        let mut expanded = std::collections::HashSet::new();
        let collapsed = key(0, &entry, 0, &expanded, 292.);
        heights.record(collapsed, px(24.));
        expanded.insert(crate::transcript_notice::expansion_key(entry.id));
        let open = key(0, &entry, 0, &expanded, 292.);
        assert_ne!(collapsed, open);
        assert_eq!(heights.get(open), None, "expanded activity requires fresh measurement");
        heights.record(open, px(200.));
        entry.id += 1;
        assert_eq!(heights.get(key(0, &entry, 0, &expanded, 292.)), None,
            "identical notices must not reuse another row's disclosure geometry");
        entry.id -= 1;
        assert_eq!(heights.get(collapsed), None,
            "collapsing after expansion requires fresh measurement");
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
