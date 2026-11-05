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
                let out = if self.shift { c.to_uppercase().into() } else { c.clone() };
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
    let accent = t.accent;
    let key = |k: Key, text: SharedString, flex: f32, bg: Hsla| {
        let on_key = on_key.clone();
        div()
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

    let numeric = matches!(config.purpose, TextInputPurpose::OneTimeCode | TextInputPurpose::Number);
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
    let mut bar = div().h(px(40.)).flex().items_center().justify_center().text_size(px(15.));
    if let Some(fill) = fill {
        let on_key = on_key.clone();
        bar = bar.child(
            div()
                .px(px(14.))
                .py(px(4.))
                .rounded(px(6.))
                .text_color(accent)
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
            if state.shift && !state.symbols { c.to_uppercase().to_string().into() } else { c.to_string().into() }
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
                    key(Key::Shift, if state.shift { "⬆" } else { "⇧" }.into(), 1.4, if state.shift { gpui::white().opacity(0.9) } else { fn_bg })
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
        let mut last = row().child(key(Key::Mode, if state.symbols { "ABC" } else { "123" }.into(), 2.5, fn_bg));
        if config.purpose == TextInputPurpose::Email {
            last = last
                .child(key(Key::Space, "space".into(), 5., key_bg))
                .child(key(Key::Char("@".into()), "@".into(), 1.2, key_bg))
                .child(key(Key::Char(".".into()), ".".into(), 1.2, key_bg));
        } else {
            last = last.child(key(Key::Space, "space".into(), 7., key_bg));
        }
        let ret_bg = if config.input_action == TextInputAction::Unspecified { fn_bg } else { accent };
        last = last.child(key(Key::Return, return_label(config.input_action).into(), 2.5, ret_bg));
        rows.push(last);
    }

    div()
        .absolute()
        .bottom_0()
        .left_0()
        .right_0()
        .h(px(HEIGHT))
        .pb(px(bottom_inset))
        .bg(gpui::rgb(0x2b2b2b))
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
