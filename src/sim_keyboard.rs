//! Desktop preview only: an on-screen iPhone-style software keyboard that
//! appears when a text field is focused, follows the field's
//! `TextInputConfiguration` (email / number pad / return key label), and types
//! into the field through the same input-handler path UIKit uses.

use std::rc::Rc;

use gpui::{
    AnyElement, App, Hsla, MouseButton, SharedString, TextInputAction, TextInputConfiguration,
    TextInputPurpose, Window, div, prelude::*, px,
};

use crate::theme::Theme;

/// Height of the keyboard incl. suggestion bar and home-indicator area.
pub const HEIGHT: f32 = 336.;

#[derive(Clone, Debug, PartialEq)]
pub enum Key {
    Char(SharedString),
    Shift,
    Backspace,
    Space,
    Return,
    /// Toggle letters / numbers+symbols.
    Mode,
}

#[derive(Default)]
pub struct SimKeyboard {
    pub shift: bool,
    pub symbols: bool,
}

impl SimKeyboard {
    /// What a key press means for this config, updating shift/mode state.
    /// Returns text to insert, or the special key to apply.
    pub fn press(&mut self, key: &Key) -> Option<Key> {
        match key {
            Key::Shift => {
                self.shift = !self.shift;
                None
            }
            Key::Mode => {
                self.symbols = !self.symbols;
                None
            }
            Key::Char(c) => {
                let out = if self.shift {
                    c.to_uppercase().into()
                } else {
                    c.clone()
                };
                self.shift = false;
                Some(Key::Char(out))
            }
            k => Some(k.clone()),
        }
    }
}

fn return_label(a: TextInputAction) -> &'static str {
    match a {
        TextInputAction::Go => "go",
        TextInputAction::Next => "next",
        TextInputAction::Done => "done",
        TextInputAction::Search => "search",
        TextInputAction::Send => "send",
        _ => "return",
    }
}

pub fn render(
    state: &SimKeyboard,
    config: &TextInputConfiguration,
    bottom_inset: f32,
    on_key: Rc<dyn Fn(Key, &mut Window, &mut App)>,
    t: &Theme,
) -> AnyElement {
    let key_bg: Hsla = gpui::rgb(0x6b6b6b).into();
    let fn_bg: Hsla = gpui::rgb(0x464646).into();
    let accent: Hsla = gpui::rgb(0x0a84ff).into();
    let _ = t;
    // Position disambiguates repeated Mode keys and email punctuation.
    let key_index = std::cell::Cell::new(0usize);
    let key = |k: Key, text: SharedString, flex: f32, bg: Hsla| {
        let index = key_index.get();
        key_index.set(index + 1);
        let on_key = on_key.clone();
        div()
            .debug_selector(move || format!("sim-key-{index}"))
            .flex_grow(1.)
            .flex_basis(px(flex * 32.))
            .h(px(42.))
            .rounded(px(5.))
            .bg(bg)
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(if text.chars().count() > 1 { 15. } else { 22. }))
            .text_color(gpui::white())
            .child(text)
            .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                cx.stop_propagation();
                window.prevent_default();
                on_key(k.clone(), window, cx)
            })
    };
    let row = || div().flex().gap(px(6.)).px(px(3.)).w_full();

    let numeric = matches!(
        config.purpose,
        TextInputPurpose::OneTimeCode | TextInputPurpose::Number
    );
    let mut rows: Vec<gpui::Div> = Vec::new();
    // Suggestion bar: iOS offers the mailed code here for one-time-code fields.
    let suggestion = match config.purpose {
        TextInputPurpose::OneTimeCode => "From Mail: 123456",
        TextInputPurpose::Email => "you@example.com",
        _ if config.suggestions => "\u{201c}the\u{201d}   I   and",
        _ => "",
    };
    let suggestion_text: SharedString = suggestion.into();
    let fill: Option<SharedString> = match config.purpose {
        TextInputPurpose::OneTimeCode => Some("123456".into()),
        TextInputPurpose::Email => Some("you@example.com".into()),
        _ => None,
    };
    let mut bar = div()
        .h(px(40.))
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(15.));
    if let Some(fill) = fill {
        let on_key = on_key.clone();
        bar = bar.child(
            div()
                .debug_selector(|| "sim-suggestion".into())
                .px(px(14.))
                .py(px(4.))
                .rounded(px(6.))
                .text_color(gpui::white())
                .child(suggestion_text)
                .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                    cx.stop_propagation();
                    window.prevent_default();
                    on_key(Key::Char(fill.clone()), window, cx)
                }),
        );
    } else {
        bar = bar.text_color(gpui::rgb(0xbbbbbb)).child(suggestion_text);
    }

    if numeric {
        for r in [["1", "2", "3"], ["4", "5", "6"], ["7", "8", "9"]] {
            let mut d = row();
            for c in r {
                d = d.child(key(Key::Char(c.into()), c.into(), 3., key_bg));
            }
            rows.push(d);
        }
        rows.push(
            row()
                .child(div().flex_grow(1.).flex_basis(px(96.)))
                .child(key(Key::Char("0".into()), "0".into(), 3., key_bg))
                .child(key(Key::Backspace, "⌫".into(), 3., fn_bg)),
        );
    } else {
        let letters: [&str; 3] = if state.symbols {
            ["1234567890", "-/:;()$&@\"", ".,?!'"]
        } else {
            ["qwertyuiop", "asdfghjkl", "zxcvbnm"]
        };
        let shown = |c: char| -> SharedString {
            if state.shift && !state.symbols {
                c.to_uppercase().to_string().into()
            } else {
                c.to_string().into()
            }
        };
        for (i, r) in letters.iter().enumerate() {
            let mut d = row();
            if i == 1 && !state.symbols {
                d = d.px(px(19.));
            }
            if i == 2 {
                d = d.child(if state.symbols {
                    key(Key::Mode, "ABC".into(), 1.4, fn_bg)
                } else {
                    key(
                        Key::Shift,
                        if state.shift { "⬆" } else { "⇧" }.into(),
                        1.4,
                        if state.shift {
                            gpui::white().opacity(0.9)
                        } else {
                            fn_bg
                        },
                    )
                });
            }
            for c in r.chars() {
                d = d.child(key(Key::Char(c.to_string().into()), shown(c), 1., key_bg));
            }
            if i == 2 {
                d = d.child(key(Key::Backspace, "⌫".into(), 1.4, fn_bg));
            }
            rows.push(d);
        }
        let mut last = row().child(key(
            Key::Mode,
            if state.symbols { "ABC" } else { "123" }.into(),
            2.5,
            fn_bg,
        ));
        if config.purpose == TextInputPurpose::Email {
            last = last
                .child(key(Key::Space, "space".into(), 5., key_bg))
                .child(key(Key::Char("@".into()), "@".into(), 1.2, key_bg))
                .child(key(Key::Char(".".into()), ".".into(), 1.2, key_bg));
        } else {
            last = last.child(key(Key::Space, "space".into(), 7., key_bg));
        }
        let ret_bg = if config.input_action == TextInputAction::Unspecified {
            fn_bg
        } else {
            accent
        };
        last = last.child(key(
            Key::Return,
            return_label(config.input_action).into(),
            2.5,
            ret_bg,
        ));
        rows.push(last);
    }

    div()
        .debug_selector(|| "sim-keyboard".into())
        .absolute()
        .bottom_0()
        .left_0()
        .right_0()
        .h(px(HEIGHT))
        .pb(px(bottom_inset))
        .bg(gpui::rgb(0x2b2b2b))
        .rounded_b(px(47.))
        .flex()
        .flex_col()
        .gap(px(11.))
        .child(bar)
        .children(rows)
        // Swallow taps between keys so they don't blur the field.
        .on_mouse_down(MouseButton::Left, |_, window, cx| {
            cx.stop_propagation();
            window.prevent_default();
        })
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    // A standalone preview host: records the renderer callback and applies only
    // the local shift/mode reducer. It deliberately does not claim native UIKit,
    // TextInput bridge, service, or transport coverage.
    struct PointerHost {
        keyboard: SimKeyboard,
        config: TextInputConfiguration,
        received: Vec<Key>,
        output: Vec<Key>,
        outside_down: usize,
        focus: gpui::FocusHandle,
    }

    impl Render for PointerHost {
        fn render(&mut self, _: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
            let entity = cx.entity().downgrade();
            let callback = Rc::new(move |key: Key, _: &mut Window, cx: &mut App| {
                entity
                    .update(cx, |host, cx| {
                        host.received.push(key.clone());
                        if let Some(output) = host.keyboard.press(&key) {
                            host.output.push(output);
                        }
                        cx.notify();
                    })
                    .unwrap();
            });
            div()
                .size_full()
                .relative()
                .child(div().h(px(40.)).track_focus(&self.focus))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|host, _, window, cx| {
                        host.outside_down += 1;
                        window.prevent_default();
                        window.blur(cx);
                    }),
                )
                .child(super::render(
                    &self.keyboard,
                    &self.config,
                    20.,
                    callback,
                    &Theme::pure_black(),
                ))
        }
    }

    fn mount(
        cx: &mut gpui::TestAppContext,
    ) -> (gpui::Entity<PointerHost>, &mut gpui::VisualTestContext) {
        let (host, cx) = cx.add_window_view(|_, cx| PointerHost {
            keyboard: SimKeyboard::default(),
            config: TextInputConfiguration::default(),
            received: Vec::new(),
            output: Vec::new(),
            outside_down: 0,
            focus: cx.focus_handle(),
        });
        cx.simulate_resize(gpui::size(px(390.), px(844.)));
        cx.update(|window, cx| {
            let focus = host.read(cx).focus.clone();
            focus.focus(window, cx);
        });
        (host, cx)
    }

    fn draw(cx: &mut gpui::VisualTestContext) {
        cx.update(|window, cx| window.draw(cx).clear(cx));
    }

    fn tap(cx: &mut gpui::VisualTestContext, selector: &'static str) {
        draw(cx);
        let bounds = cx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("missing {selector}"));
        assert!(bounds.size.width > px(0.) && bounds.size.height > px(0.));
        pointer(cx, bounds.center());
    }

    fn pointer(cx: &mut gpui::VisualTestContext, position: gpui::Point<gpui::Pixels>) {
        cx.simulate_event(gpui::MouseDownEvent {
            button: MouseButton::Left,
            position,
            modifiers: Default::default(),
            click_count: 1,
            first_mouse: false,
        });
        cx.simulate_event(gpui::MouseUpEvent {
            button: MouseButton::Left,
            position,
            modifiers: Default::default(),
            click_count: 1,
        });
    }

    fn assert_owned(host: &gpui::Entity<PointerHost>, cx: &mut gpui::VisualTestContext) {
        cx.update(|window, cx| {
            let host = host.read(cx);
            assert_eq!(host.outside_down, 0, "keyboard tap escaped to blur handler");
            assert!(
                host.focus.is_focused(window),
                "keyboard tap stole field focus"
            );
        });
    }

    #[gpui::test]
    fn rendered_pointer_routes_every_key_in_all_layouts(cx: &mut gpui::TestAppContext) {
        let (host, cx) = mount(cx);
        for purpose in [
            TextInputPurpose::Text,
            TextInputPurpose::Email,
            TextInputPurpose::Number,
            TextInputPurpose::OneTimeCode,
        ] {
            for (symbols, shift) in [(false, false), (false, true), (true, false)] {
                let numeric = matches!(
                    purpose,
                    TextInputPurpose::Number | TextInputPurpose::OneTimeCode
                );
                let mut expected = Vec::new();
                if numeric {
                    expected.extend(
                        "1234567890"
                            .chars()
                            .map(|c| Key::Char(c.to_string().into())),
                    );
                    expected.push(Key::Backspace);
                } else {
                    let rows = if symbols {
                        ["1234567890", "-/:;()$&@\"", ".,?!'"]
                    } else {
                        ["qwertyuiop", "asdfghjkl", "zxcvbnm"]
                    };
                    for (index, row) in rows.iter().enumerate() {
                        if index == 2 {
                            expected.push(if symbols { Key::Mode } else { Key::Shift });
                        }
                        expected.extend(row.chars().map(|c| Key::Char(c.to_string().into())));
                        if index == 2 {
                            expected.push(Key::Backspace);
                        }
                    }
                    expected.extend([Key::Mode, Key::Space]);
                    if purpose == TextInputPurpose::Email {
                        expected.extend([Key::Char("@".into()), Key::Char(".".into())]);
                    }
                    expected.push(Key::Return);
                }
                for (index, key) in expected.iter().enumerate() {
                    host.update(cx, |host, cx| {
                        host.config.purpose = purpose;
                        host.keyboard.symbols = symbols;
                        host.keyboard.shift = shift;
                        host.received.clear();
                        host.output.clear();
                        cx.notify();
                    });
                    const SELECTORS: &[&str] = &[
                        "sim-key-0",
                        "sim-key-1",
                        "sim-key-2",
                        "sim-key-3",
                        "sim-key-4",
                        "sim-key-5",
                        "sim-key-6",
                        "sim-key-7",
                        "sim-key-8",
                        "sim-key-9",
                        "sim-key-10",
                        "sim-key-11",
                        "sim-key-12",
                        "sim-key-13",
                        "sim-key-14",
                        "sim-key-15",
                        "sim-key-16",
                        "sim-key-17",
                        "sim-key-18",
                        "sim-key-19",
                        "sim-key-20",
                        "sim-key-21",
                        "sim-key-22",
                        "sim-key-23",
                        "sim-key-24",
                        "sim-key-25",
                        "sim-key-26",
                        "sim-key-27",
                        "sim-key-28",
                        "sim-key-29",
                        "sim-key-30",
                        "sim-key-31",
                        "sim-key-32",
                    ];
                    tap(cx, SELECTORS[index]);
                    host.read_with(cx, |host, _| {
                        assert_eq!(
                            host.received,
                            vec![key.clone()],
                            "{purpose:?}/{symbols}/{shift}/{index}"
                        );
                        let output = match key {
                            Key::Shift | Key::Mode => vec![],
                            Key::Char(text) if shift => vec![Key::Char(text.to_uppercase().into())],
                            key => vec![key.clone()],
                        };
                        assert_eq!(host.output, output);
                        assert_eq!(
                            host.keyboard.symbols,
                            if *key == Key::Mode { !symbols } else { symbols }
                        );
                        assert_eq!(
                            host.keyboard.shift,
                            match key {
                                Key::Shift => !shift,
                                Key::Char(_) => false,
                                _ => shift,
                            }
                        );
                    });
                    assert_owned(&host, cx);
                }
            }
        }
    }

    #[gpui::test]
    fn rendered_shift_and_mode_transitions_survive_redraw(cx: &mut gpui::TestAppContext) {
        let (host, cx) = mount(cx);
        tap(cx, "sim-key-19"); // shift in letters
        tap(cx, "sim-key-0"); // Q, consumes shift
        tap(cx, "sim-key-0"); // q after redraw
        tap(cx, "sim-key-28"); // bottom Mode in letters
        tap(cx, "sim-key-0"); // 1 in symbols
        tap(cx, "sim-key-20"); // upper ABC in symbols
        tap(cx, "sim-key-0"); // q in letters again
        host.read_with(cx, |host, _| {
            assert_eq!(
                host.output,
                vec![
                    Key::Char("Q".into()),
                    Key::Char("q".into()),
                    Key::Char("1".into()),
                    Key::Char("q".into())
                ]
            );
            assert!(!host.keyboard.shift && !host.keyboard.symbols);
            assert_eq!(host.received.len(), 7);
        });
        assert_owned(&host, cx);
    }

    #[gpui::test]
    fn rendered_suggestions_and_blank_area_preserve_focus(cx: &mut gpui::TestAppContext) {
        let (host, cx) = mount(cx);
        for (purpose, fill) in [
            (TextInputPurpose::Email, "you@example.com"),
            (TextInputPurpose::OneTimeCode, "123456"),
        ] {
            host.update(cx, |host, cx| {
                host.config.purpose = purpose;
                host.received.clear();
                cx.notify();
            });
            tap(cx, "sim-suggestion");
            host.read_with(cx, |host, _| {
                assert_eq!(host.received, vec![Key::Char(fill.into())])
            });
            assert_owned(&host, cx);
            draw(cx);
            let bounds = cx.debug_bounds("sim-keyboard").unwrap();
            // Bottom inset below the key rows is keyboard background, not a key.
            pointer(cx, gpui::point(bounds.center().x, bounds.bottom() - px(8.)));
            host.read_with(cx, |host, _| assert_eq!(host.received.len(), 1));
            assert_owned(&host, cx);
        }
        // Positive control: the same pointer path outside the keyboard really
        // does reach the ancestor and blur, so the preservation checks matter.
        pointer(cx, gpui::point(px(10.), px(10.)));
        cx.update(|window, cx| {
            assert_eq!(host.read(cx).outside_down, 1);
            assert!(!host.read(cx).focus.is_focused(window));
        });
    }

    #[test]
    fn shift_is_one_shot() {
        let mut k = SimKeyboard::default();
        k.press(&Key::Shift);
        assert_eq!(k.press(&Key::Char("a".into())), Some(Key::Char("A".into())));
        assert_eq!(k.press(&Key::Char("a".into())), Some(Key::Char("a".into())));
    }

    #[test]
    fn return_label_follows_action() {
        assert_eq!(return_label(TextInputAction::Go), "go");
        assert_eq!(return_label(TextInputAction::Send), "send");
    }
}
