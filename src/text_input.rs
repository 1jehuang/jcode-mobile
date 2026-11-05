//! Single-line text field. Adapted from GPUI's `examples/input.rs`, trimmed for
//! touch: tap focuses and places the caret, the platform input handler (IME on
//! desktop, the software keyboard on iOS) inserts text.

use std::ops::Range;

use gpui::{
    App, Bounds, ClipboardItem, Context, CursorStyle, ElementId, ElementInputHandler, Entity,
    EntityInputHandler, EventEmitter, FocusHandle, Focusable, GlobalElementId, LayoutId,
    MouseButton, MouseDownEvent, PaintQuad, Pixels, Point, SharedString, WrappedLine, Style,
    TextRun, UTF16Selection, UnderlineStyle, Window, actions, div, fill, point, prelude::*, px,
    relative, size,
};
use unicode_segmentation::UnicodeSegmentation;

use crate::theme::Theme;

actions!(text_input, [Backspace, Delete, Left, Right, SelectAll, Home, End, Paste, Copy, Cut, Submit]);

pub fn bind_keys(cx: &mut App) {
    use gpui::KeyBinding;
    let ctx = Some("TextInput");
    cx.bind_keys([
        KeyBinding::new("backspace", Backspace, ctx),
        KeyBinding::new("delete", Delete, ctx),
        KeyBinding::new("left", Left, ctx),
        KeyBinding::new("right", Right, ctx),
        KeyBinding::new("home", Home, ctx),
        KeyBinding::new("end", End, ctx),
        KeyBinding::new("enter", Submit, ctx),
        KeyBinding::new("secondary-a", SelectAll, ctx),
        KeyBinding::new("secondary-v", Paste, ctx),
        KeyBinding::new("secondary-c", Copy, ctx),
        KeyBinding::new("secondary-x", Cut, ctx),
    ]);
}

pub enum TextInputEvent {
    Submit,
    Changed,
}

pub struct TextInput {
    focus_handle: FocusHandle,
    content: SharedString,
    placeholder: SharedString,
    selected_range: Range<usize>,
    marked_range: Option<Range<usize>>,
    last_layout: Option<WrappedLine>,
    last_bounds: Option<Bounds<Pixels>>,
    /// Top-left of the text, after scrolling a tall draft.
    last_origin: Point<Pixels>,
    scroll_y: Pixels,
    /// Wrap onto up to this many lines, then scroll. 1 = single line.
    pub max_lines: usize,
    pub secure: bool,
    /// No own chrome: the parent draws the outline (desktop composer).
    pub bare: bool,
    /// Soft keyboard behavior (autocorrect, caps, return key).
    pub config: gpui::TextInputConfiguration,
}

impl EventEmitter<TextInputEvent> for TextInput {}

impl TextInput {
    pub fn new(placeholder: impl Into<SharedString>, cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            content: "".into(),
            placeholder: placeholder.into(),
            selected_range: 0..0,
            marked_range: None,
            last_layout: None,
            last_bounds: None,
            last_origin: Point::default(),
            scroll_y: px(0.),
            max_lines: 1,
            secure: false,
            bare: false,
            config: gpui::TextInputConfiguration::default(),
        }
    }

    pub fn text(&self) -> &str {
        &self.content
    }

    pub fn set_text(&mut self, text: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.content = text.into();
        self.selected_range = self.content.len()..self.content.len();
        self.marked_range = None;
        cx.emit(TextInputEvent::Changed);
        cx.notify();
    }

    pub fn is_focused(&self, window: &Window) -> bool {
        self.focus_handle.is_focused(window)
    }

    /// Simulated soft keyboard (desktop preview): same paths UIKit drives.
    pub fn sim_insert(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.replace_text_in_range(None, text, window, cx);
    }
    pub fn sim_backspace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.backspace(&Backspace, window, cx);
    }
    pub fn sim_submit(&mut self, cx: &mut Context<Self>) {
        cx.emit(TextInputEvent::Submit);
    }

    fn left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
        let to = if self.selected_range.is_empty() {
            self.previous_boundary(self.selected_range.start)
        } else {
            self.selected_range.start
        };
        self.move_to(to, cx);
    }
    fn right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
        let to = if self.selected_range.is_empty() {
            self.next_boundary(self.selected_range.end)
        } else {
            self.selected_range.end
        };
        self.move_to(to, cx);
    }
    fn home(&mut self, _: &Home, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, cx);
    }
    fn end(&mut self, _: &End, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.content.len(), cx);
    }
    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.selected_range = 0..self.content.len();
        cx.notify();
    }
    fn backspace(&mut self, _: &Backspace, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            let prev = self.previous_boundary(self.selected_range.start);
            self.selected_range = prev..self.selected_range.start;
        }
        self.replace_text_in_range(None, "", window, cx)
    }
    fn delete(&mut self, _: &Delete, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            let next = self.next_boundary(self.selected_range.end);
            self.selected_range = self.selected_range.start..next;
        }
        self.replace_text_in_range(None, "", window, cx)
    }
    fn submit(&mut self, _: &Submit, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(TextInputEvent::Submit);
    }
    fn paste(&mut self, _: &Paste, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.replace_text_in_range(None, &text.replace('\n', " "), window, cx);
        }
    }
    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() && !self.secure {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.content[self.selected_range.clone()].to_string(),
            ));
        }
    }
    fn cut(&mut self, a: &Cut, window: &mut Window, cx: &mut Context<Self>) {
        self.copy(&Copy, window, cx);
        let _ = a;
        self.replace_text_in_range(None, "", window, cx)
    }

    fn on_mouse_down(&mut self, e: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        window.focus(&self.focus_handle, cx);
        window.request_virtual_keyboard();
        let i = self.index_for_position(e.position);
        self.move_to(i, cx);
    }

    fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.selected_range = offset..offset;
        cx.notify()
    }

    fn index_for_position(&self, position: Point<Pixels>) -> usize {
        let (Some(lh), Some(line)) = (self.last_line_height(), self.last_layout.as_ref()) else {
            return self.content.len();
        };
        if self.content.is_empty() || self.secure {
            return self.content.len();
        }
        let local = position - self.last_origin;
        let local = point(local.x, local.y.max(px(0.)));
        line.closest_index_for_position(local, lh).unwrap_or_else(|i| i).min(self.content.len())
    }

    fn last_line_height(&self) -> Option<Pixels> {
        self.last_layout.as_ref().map(|_| px(LINE_HEIGHT))
    }

    fn offset_from_utf16(&self, offset: usize) -> usize {
        let mut utf8 = 0;
        let mut utf16 = 0;
        for ch in self.content.chars() {
            if utf16 >= offset {
                break;
            }
            utf16 += ch.len_utf16();
            utf8 += ch.len_utf8();
        }
        utf8
    }
    fn offset_to_utf16(&self, offset: usize) -> usize {
        let mut utf16 = 0;
        let mut utf8 = 0;
        for ch in self.content.chars() {
            if utf8 >= offset {
                break;
            }
            utf8 += ch.len_utf8();
            utf16 += ch.len_utf16();
        }
        utf16
    }
    fn range_to_utf16(&self, r: &Range<usize>) -> Range<usize> {
        self.offset_to_utf16(r.start)..self.offset_to_utf16(r.end)
    }
    fn range_from_utf16(&self, r: &Range<usize>) -> Range<usize> {
        self.offset_from_utf16(r.start)..self.offset_from_utf16(r.end)
    }
    fn previous_boundary(&self, offset: usize) -> usize {
        self.content
            .grapheme_indices(true)
            .rev()
            .find_map(|(i, _)| (i < offset).then_some(i))
            .unwrap_or(0)
    }
    fn next_boundary(&self, offset: usize) -> usize {
        self.content
            .grapheme_indices(true)
            .find_map(|(i, _)| (i > offset).then_some(i))
            .unwrap_or(self.content.len())
    }
}

impl EntityInputHandler for TextInput {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.range_from_utf16(&range_utf16);
        actual_range.replace(self.range_to_utf16(&range));
        Some(self.content[range].to_string())
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection { range: self.range_to_utf16(&self.selected_range), reversed: false })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked_range.as_ref().map(|r| self.range_to_utf16(r))
    }

    fn text_input_configuration(
        &mut self,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> gpui::TextInputConfiguration {
        self.config.clone()
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.marked_range = None;
        cx.notify();
    }

    fn set_selected_text_range(
        &mut self,
        range_utf16: Range<usize>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // UIKit caret moves: taps, spacebar trackpad, selection handles.
        self.selected_range = self.range_from_utf16(&range_utf16);
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .as_ref()
            .map(|r| self.range_from_utf16(r))
            .or(self.marked_range.clone())
            .unwrap_or(self.selected_range.clone());
        self.content =
            (self.content[..range.start].to_owned() + new_text + &self.content[range.end..]).into();
        let caret = range.start + new_text.len();
        self.selected_range = caret..caret;
        self.marked_range = None;
        cx.emit(TextInputEvent::Changed);
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .as_ref()
            .map(|r| self.range_from_utf16(r))
            .or(self.marked_range.clone())
            .unwrap_or(self.selected_range.clone());
        self.content =
            (self.content[..range.start].to_owned() + new_text + &self.content[range.end..]).into();
        self.marked_range =
            (!new_text.is_empty()).then(|| range.start..range.start + new_text.len());
        self.selected_range = new_selected_range_utf16
            .as_ref()
            .map(|r| self.range_from_utf16(r))
            .map(|r| r.start + range.start..r.end + range.start)
            .unwrap_or_else(|| range.start + new_text.len()..range.start + new_text.len());
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let _ = bounds;
        let line = self.last_layout.as_ref()?;
        let lh = px(LINE_HEIGHT);
        let range = self.range_from_utf16(&range_utf16);
        let a = line.position_for_index(range.start, lh)?;
        let b = line.position_for_index(range.end, lh).unwrap_or(a);
        let o = self.last_origin;
        Some(Bounds::from_corners(o + a, o + point(b.x.max(a.x), b.y + lh)))
    }

    fn character_index_for_point(
        &mut self,
        p: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        self.last_bounds?.localize(&p)?;
        // Past the end of the text means "end", not "nowhere".
        let i = self.index_for_position(p);
        Some(self.offset_to_utf16(i))
    }
}

const LINE_HEIGHT: f32 = 20.;

/// What gets shaped: placeholder, bullets, or the draft. Newlines become
/// spaces (same byte length) so the draft shapes as one wrapped paragraph
/// and content offsets stay valid.
fn shaping_text(input: &TextInput) -> SharedString {
    if input.content.is_empty() {
        input.placeholder.clone()
    } else if input.secure {
        "•".repeat(input.content.chars().count()).into()
    } else if input.content.contains('\n') {
        input.content.replace('\n', " ").into()
    } else {
        input.content.clone()
    }
}

struct TextElement {
    input: Entity<TextInput>,
}

struct PrepaintState {
    line: Option<WrappedLine>,
    cursor: Option<PaintQuad>,
    selection: Vec<PaintQuad>,
    origin: Point<Pixels>,
    scroll_y: Pixels,
}

impl IntoElement for TextElement {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Element for TextElement {
    type RequestLayoutState = ();
    type PrepaintState = PrepaintState;

    fn id(&self) -> Option<ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        let lh = px(LINE_HEIGHT);
        let input = self.input.read(cx);
        if input.max_lines <= 1 {
            style.size.height = lh.into();
            return (window.request_layout(style, [], cx), ());
        }
        // Height follows the wrapped draft, capped at max_lines.
        let max_lines = input.max_lines;
        let display = shaping_text(input);
        let text_style = window.text_style();
        let font_size = text_style.font_size.to_pixels(window.rem_size());
        let run = TextRun {
            len: display.len(),
            font: text_style.font(),
            color: text_style.color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let id = window.request_measured_layout(style, move |known, avail, window, _| {
            let width = known.width.or(match avail.width {
                gpui::AvailableSpace::Definite(w) => Some(w),
                _ => None,
            });
            let lines = window
                .text_system()
                .shape_text(display.clone(), font_size, std::slice::from_ref(&run), width, None)
                .ok()
                .and_then(|l| l.into_iter().next())
                .map_or(1, |l| l.wrap_boundaries().len() + 1);
            size(width.unwrap_or_default(), lh * lines.clamp(1, max_lines) as f32)
        });
        (id, ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> PrepaintState {
        let theme = Theme::get(cx);
        let input = self.input.read(cx);
        let style = window.text_style();
        let empty = input.content.is_empty();
        let display = shaping_text(input);
        // Map content offsets to display offsets (secure text uses 3-byte bullets).
        let map = |o: usize| -> usize {
            if empty {
                0
            } else if input.secure {
                input.content[..o].chars().count() * '•'.len_utf8()
            } else {
                o
            }
        };
        let run = TextRun {
            len: display.len(),
            font: style.font(),
            color: if empty { theme.text_muted } else { style.color },
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let runs = match input.marked_range.as_ref().filter(|_| !empty && !input.secure) {
            Some(m) => [
                TextRun { len: m.start, ..run.clone() },
                TextRun {
                    len: m.end - m.start,
                    underline: Some(UnderlineStyle {
                        color: Some(run.color),
                        thickness: px(1.),
                        wavy: false,
                    }),
                    ..run.clone()
                },
                TextRun { len: display.len() - m.end, ..run },
            ]
            .into_iter()
            .filter(|r| r.len > 0)
            .collect(),
            None => vec![run],
        };
        let font_size = style.font_size.to_pixels(window.rem_size());
        let lh = px(LINE_HEIGHT);
        let wrap = (input.max_lines > 1).then_some(bounds.size.width);
        let line = window
            .text_system()
            .shape_text(display, font_size, &runs, wrap, None)
            .ok()
            .and_then(|l| l.into_iter().next())
            .unwrap_or_default();
        let pos = |i: usize| line.position_for_index(map(i), lh).unwrap_or_default();
        let sel = input.selected_range.clone();
        // Keep the caret line in view once the draft is taller than the field.
        let caret = pos(sel.end);
        let content_h = line.size(lh).height;
        let mut scroll_y = input.scroll_y;
        if caret.y < scroll_y {
            scroll_y = caret.y;
        } else if caret.y + lh > scroll_y + bounds.size.height {
            scroll_y = caret.y + lh - bounds.size.height;
        }
        scroll_y = scroll_y.min((content_h - bounds.size.height).max(px(0.))).max(px(0.));
        let origin = bounds.origin - point(px(0.), scroll_y);
        let (selection, cursor) = if sel.is_empty() {
            let c = origin + pos(sel.start);
            (vec![], Some(fill(Bounds::new(c, size(px(2.), lh)), theme.accent)))
        } else {
            let (a, b) = (pos(sel.start), pos(sel.end));
            let right = bounds.size.width;
            let mut rects = vec![];
            if a.y == b.y {
                rects.push(Bounds::from_corners(a, point(b.x, b.y + lh)));
            } else {
                rects.push(Bounds::from_corners(a, point(right, a.y + lh)));
                if b.y > a.y + lh {
                    rects.push(Bounds::from_corners(point(px(0.), a.y + lh), point(right, b.y)));
                }
                rects.push(Bounds::from_corners(point(px(0.), b.y), point(b.x, b.y + lh)));
            }
            let rects = rects
                .into_iter()
                .map(|r| fill(Bounds::new(origin + r.origin, r.size), theme.selection))
                .collect();
            (rects, None)
        };
        PrepaintState { line: Some(line), cursor, selection, origin, scroll_y }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        prepaint: &mut PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus = self.input.read(cx).focus_handle.clone();
        window.handle_input(&focus, ElementInputHandler::new(bounds, self.input.clone()), cx);
        let line = prepaint.line.take().unwrap();
        let origin = prepaint.origin;
        window.with_content_mask(Some(gpui::ContentMask { bounds }), |window| {
            for s in prepaint.selection.drain(..) {
                window.paint_quad(s)
            }
            line.paint(origin, px(LINE_HEIGHT), gpui::TextAlign::Left, None, window, cx).ok();
            if focus.is_focused(window)
                && let Some(c) = prepaint.cursor.take()
            {
                window.paint_quad(c);
            }
        });
        let scroll_y = prepaint.scroll_y;
        self.input.update(cx, |input, _| {
            input.last_layout = Some(line);
            input.last_bounds = Some(bounds);
            input.last_origin = origin;
            input.scroll_y = scroll_y;
        });
    }
}

impl Render for TextInput {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::get(cx);
        let focused = self.focus_handle.is_focused(window);
        div()
            .key_context("TextInput")
            .track_focus(&self.focus_handle)
            .cursor(CursorStyle::IBeam)
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::home))
            .on_action(cx.listener(Self::end))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::submit))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .w_full()
            .when(!self.bare, |d| {
                d.px(px(16.))
                    .py(px(11.))
                    .rounded(px(22.))
                    .bg(theme.field)
                    .border_1()
                    .border_color(if focused { theme.border_focus } else { theme.input_border })
            })
            .py(px(if self.bare { 10. } else { 11. }))
            .text_size(crate::system::fs(14.))
            .line_height(crate::system::fs(LINE_HEIGHT))
            .text_color(theme.text)
            .overflow_hidden()
            .child(TextElement { input: cx.entity() })
    }
}

impl Focusable for TextInput {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

/// Replays the exact call sequences the iOS bridge (gpui_ios window.rs and
/// text_input.rs) makes for soft-keyboard events, against the real widget.
#[cfg(test)]
mod ios_keyboard_tests {
    use super::*;
    use gpui::{
        KeyDownEvent, Keystroke, Modifiers, PlatformInput, TestAppContext, VisualTestContext,
    };

    struct Host {
        input: Entity<TextInput>,
    }
    impl Render for Host {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().size_full().child(self.input.clone())
        }
    }

    fn setup(cx: &mut TestAppContext) -> (Entity<TextInput>, &mut VisualTestContext) {
        cx.update(|cx| {
            bind_keys(cx);
            cx.set_global(crate::theme::Theme::pure_black());
        });
        let (host, cx) = cx.add_window_view(|_, cx| Host {
            input: cx.new(|cx| TextInput::new("", cx)),
        });
        let input = host.read_with(cx, |h, _| h.input.clone());
        cx.update(|window, cx| window.focus(&input.focus_handle(cx), cx));
        cx.run_until_parked();
        INPUT.with(|i| *i.borrow_mut() = Some(input.clone()));
        (input, cx)
    }

    /// gpui_ios `key()`: printable keys prefer text input, others don't.
    fn key(cx: &mut VisualTestContext, key: &str, key_char: Option<&str>) -> bool {
        let ev = PlatformInput::KeyDown(KeyDownEvent {
            keystroke: Keystroke {
                modifiers: Modifiers::default(),
                key: key.into(),
                key_char: key_char.map(Into::into),
            },
            is_held: false,
            prefer_character_input: key_char.is_some(),
        });
        let r = cx.update(|window, cx| window.dispatch_event(ev, cx));
        cx.run_until_parked();
        !r.propagate
    }

    /// The bridge's `with_handler`: ElementInputHandler forwards straight to
    /// the focused entity's EntityInputHandler.
    fn with_handler<R>(
        cx: &mut VisualTestContext,
        f: impl FnOnce(&mut TextInput, &mut Window, &mut Context<TextInput>) -> R,
    ) -> R {
        let input = INPUT.with(|i| i.borrow().clone()).unwrap();
        let r = cx.update(|window, cx| input.update(cx, |t, cx| f(t, window, cx)));
        cx.run_until_parked();
        r
    }

    thread_local! {
        static INPUT: std::cell::RefCell<Option<Entity<TextInput>>> = const { std::cell::RefCell::new(None) };
    }

    /// gpui_ios `insert()` (insertText:).
    fn insert_text(cx: &mut VisualTestContext, text: &str) {
        if text == "\n" {
            key(cx, "enter", None);
            return;
        }
        if text.chars().count() == 1 {
            let k = text.to_lowercase();
            let k = if k == " " { "space".to_string() } else { k };
            if key(cx, &k, Some(text)) {
                return;
            }
        }
        with_handler(cx, |t, w, cx| t.replace_text_in_range(None, text, w, cx));
    }

    /// gpui_ios `delete_backward()` (deleteBackward).
    fn delete_backward(cx: &mut VisualTestContext) {
        key(cx, "backspace", None);
    }

    fn text(input: &Entity<TextInput>, cx: &mut VisualTestContext) -> String {
        input.read_with(cx, |t, _| t.content.to_string())
    }

    #[gpui::test]
    fn typing_and_backspace(cx: &mut TestAppContext) {
        let (input, cx) = setup(cx);
        for c in ["h", "i", " ", "Y", "o"] {
            insert_text(cx, c);
        }
        assert_eq!(text(&input, cx), "hi Yo");
        delete_backward(cx);
        assert_eq!(text(&input, cx), "hi Y");
        delete_backward(cx);
        delete_backward(cx);
        assert_eq!(text(&input, cx), "hi");
    }

    #[gpui::test]
    fn backspace_on_empty_and_held_down(cx: &mut TestAppContext) {
        let (input, cx) = setup(cx);
        delete_backward(cx);
        assert_eq!(text(&input, cx), "");
        insert_text(cx, "abc");
        for _ in 0..10 {
            delete_backward(cx);
        }
        assert_eq!(text(&input, cx), "");
    }

    #[gpui::test]
    fn backspace_removes_whole_emoji_and_accents(cx: &mut TestAppContext) {
        let (input, cx) = setup(cx);
        insert_text(cx, "a👍🏽");
        insert_text(cx, "é");
        delete_backward(cx);
        assert_eq!(text(&input, cx), "a👍🏽");
        delete_backward(cx);
        assert_eq!(text(&input, cx), "a");
    }

    #[gpui::test]
    fn return_key_submits_not_inserts(cx: &mut TestAppContext) {
        let (input, cx) = setup(cx);
        let submitted = std::rc::Rc::new(std::cell::Cell::new(0));
        let s = submitted.clone();
        cx.update(|_, cx| {
            cx.subscribe(&input, move |_, ev, _| {
                if matches!(ev, TextInputEvent::Submit) {
                    s.set(s.get() + 1)
                }
            })
            .detach()
        });
        insert_text(cx, "go");
        insert_text(cx, "\n");
        assert_eq!(text(&input, cx), "go");
        assert_eq!(submitted.get(), 1);
    }

    #[gpui::test]
    fn autocorrect_replaces_word_via_range(cx: &mut TestAppContext) {
        let (input, cx) = setup(cx);
        insert_text(cx, "teh");
        // UIKit: replaceRange:withText: on the misspelled word, then a space.
        with_handler(cx, |t, w, cx| t.replace_text_in_range(Some(0..3), "the", w, cx));
        insert_text(cx, " ");
        assert_eq!(text(&input, cx), "the ");
        let sel = with_handler(cx, |t, w, cx| t.selected_text_range(false, w, cx)).unwrap();
        assert_eq!(sel.range, 4..4);
    }

    #[gpui::test]
    fn caret_move_then_backspace_and_insert(cx: &mut TestAppContext) {
        let (input, cx) = setup(cx);
        insert_text(cx, "hello");
        // Spacebar trackpad / tap: setSelectedTextRange:.
        with_handler(cx, |t, w, cx| t.set_selected_text_range(2..2, w, cx));
        delete_backward(cx);
        assert_eq!(text(&input, cx), "hllo");
        insert_text(cx, "E");
        assert_eq!(text(&input, cx), "hEllo");
        // Selected range then backspace deletes the selection.
        with_handler(cx, |t, w, cx| t.set_selected_text_range(0..2, w, cx));
        delete_backward(cx);
        assert_eq!(text(&input, cx), "llo");
    }

    #[gpui::test]
    fn marked_text_ime_and_utf16_offsets(cx: &mut TestAppContext) {
        let (input, cx) = setup(cx);
        insert_text(cx, "😀");
        // Japanese kana composition: setMarkedText then commit via insertText.
        with_handler(cx, |t, w, cx| t.replace_and_mark_text_in_range(None, "に", Some(1..1), w, cx));
        with_handler(cx, |t, w, cx| t.replace_and_mark_text_in_range(None, "日本", Some(2..2), w, cx));
        assert_eq!(with_handler(cx, |t, w, cx| t.marked_text_range(w, cx)), Some(2..4));
        with_handler(cx, |t, w, cx| t.replace_text_in_range(None, "日本", w, cx));
        assert_eq!(text(&input, cx), "😀日本");
        assert_eq!(with_handler(cx, |t, w, cx| t.marked_text_range(w, cx)), None);
        // Emoji is 2 UTF-16 units, caret after it is utf16 4.
        let sel = with_handler(cx, |t, w, cx| t.selected_text_range(false, w, cx)).unwrap();
        assert_eq!(sel.range, 4..4);
        let mut adj = None;
        assert_eq!(
            with_handler(cx, |t, w, cx| t.text_for_range(2..3, &mut adj, w, cx)).as_deref(),
            Some("日")
        );
    }

    #[gpui::test]
    fn multi_char_paste_and_dictation(cx: &mut TestAppContext) {
        let (input, cx) = setup(cx);
        insert_text(cx, "a");
        insert_text(cx, "pple pie");
        assert_eq!(text(&input, cx), "apple pie");
    }

    #[gpui::test]
    fn keyboard_config_reaches_platform(cx: &mut TestAppContext) {
        let (input, cx) = setup(cx);
        input.update(cx, |t, _| {
            t.config.input_action = gpui::TextInputAction::Send;
            t.config.purpose = gpui::TextInputPurpose::OneTimeCode;
        });
        let c = with_handler(cx, |t, w, cx| t.text_input_configuration(w, cx));
        assert_eq!(c.input_action, gpui::TextInputAction::Send);
        assert_eq!(c.purpose, gpui::TextInputPurpose::OneTimeCode);
    }
}
