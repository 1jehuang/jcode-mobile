//! Rendered preview keyboard -> MobileApp -> focused TextInput bridge.
//! Offline GPUI touch emulation, not native UIKit keyboard coverage.
use super::navigation_coverage::{draw, drawer, mount};
use super::*;
use gpui::{
    Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, TestAppContext, VisualTestContext,
};

fn disabled() -> bool {
    std::env::var("JCODE_MOBILE_KEYBOARD").ok().as_deref() == Some("0")
}

fn settle(view: &Entity<MobileApp>, cx: &mut VisualTestContext) {
    draw(cx);
    view.update(cx, |app, cx| {
        app.inset_anim = None;
        app.feel.g.edge_x = 0.;
        cx.notify();
    });
    draw(cx);
}

fn raw_tap(cx: &mut VisualTestContext, position: gpui::Point<gpui::Pixels>) {
    let viewport = cx.update(|window, _| window.viewport_size());
    assert!(
        position.x > px(0.) && position.x < viewport.width,
        "tap {position:?}, viewport {viewport:?}"
    );
    assert!(
        position.y > px(0.) && position.y < viewport.height,
        "tap {position:?}, viewport {viewport:?}"
    );
    cx.simulate_event(MouseDownEvent {
        button: MouseButton::Left,
        position,
        modifiers: Modifiers::default(),
        click_count: 1,
        first_mouse: false,
    });
    cx.simulate_event(MouseUpEvent {
        button: MouseButton::Left,
        position,
        modifiers: Modifiers::default(),
        click_count: 1,
    });
}

fn tap(cx: &mut VisualTestContext, selector: &'static str) {
    draw(cx);
    let bounds = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("missing {selector}"));
    let viewport = cx.update(|window, _| window.viewport_size());
    assert!(bounds.size.width > px(0.) && bounds.size.height > px(0.));
    assert!(
        bounds.left() >= px(0.) && bounds.right() <= viewport.width,
        "{selector}: {bounds:?}"
    );
    assert!(
        bounds.top() >= px(0.) && bounds.bottom() <= viewport.height,
        "{selector}: {bounds:?}"
    );
    raw_tap(cx, bounds.center());
}

fn focused(input: &Entity<TextInput>, cx: &mut VisualTestContext) {
    cx.update(|window, cx| assert!(input.read(cx).is_focused(window)));
}

fn text(input: &Entity<TextInput>, cx: &VisualTestContext) -> String {
    input.read_with(cx, |input, _| input.text().to_owned())
}

fn submissions(
    input: &Entity<TextInput>,
    cx: &mut VisualTestContext,
) -> std::rc::Rc<std::cell::Cell<usize>> {
    let count = std::rc::Rc::new(std::cell::Cell::new(0));
    let observed = count.clone();
    cx.update(|_, cx| {
        cx.subscribe(input, move |_, event, _| {
            if matches!(event, TextInputEvent::Submit) {
                observed.set(observed.get() + 1);
            }
        })
        .detach();
    });
    count
}

fn tap_input(input: &Entity<TextInput>, cx: &mut VisualTestContext) {
    draw(cx);
    let caret = cx.update(|window, cx| {
        input.update(cx, |input, cx| {
            use gpui::EntityInputHandler;
            input
                .bounds_for_range(0..0, gpui::Bounds::default(), window, cx)
                .unwrap()
        })
    });
    raw_tap(cx, caret.origin + gpui::point(px(2.), px(8.)));
    focused(input, cx);
}

fn reveal_input(
    view: &Entity<MobileApp>,
    input: &Entity<TextInput>,
    clip: Option<&'static str>,
    cx: &mut VisualTestContext,
) {
    for _ in 0..24 {
        settle(view, cx);
        let caret = cx.update(|window, cx| {
            input.update(cx, |input, cx| {
                use gpui::EntityInputHandler;
                input
                    .bounds_for_range(0..0, gpui::Bounds::default(), window, cx)
                    .unwrap()
            })
        });
        let viewport = cx.update(|window, _| window.viewport_size());
        let keyboard = view.read_with(cx, |app, _| app.keyboard_up);
        let mut top = px(22.);
        let mut bottom = viewport.height
            - px(if keyboard {
                crate::sim_keyboard::HEIGHT
            } else {
                8.
            });
        let mut x = viewport.width / 2.;
        if let Some(selector) = clip {
            let bounds = cx.debug_bounds(selector).unwrap();
            top = top.max(bounds.top());
            bottom = bottom.min(bounds.bottom());
            x = bounds.center().x;
        }
        let point = caret.origin + gpui::point(px(2.), px(8.));
        if point.y > top + px(4.) && point.y < bottom - px(4.) {
            raw_tap(cx, point);
            focused(input, cx);
            return;
        }
        assert!(bottom > top, "input clip collapsed");
        cx.simulate_event(gpui::ScrollWheelEvent {
            position: gpui::point(x, (top + bottom) / 2.),
            delta: gpui::ScrollDelta::Pixels(gpui::point(
                px(0.),
                px(if point.y < top + px(4.) { 80. } else { -80. }),
            )),
            touch_phase: gpui::TouchPhase::Moved,
            modifiers: Modifiers::default(),
        });
    }
    panic!("input never became reachable inside its visible clip");
}

#[gpui::test]
fn keyboard_bridge_composer_keys_blank_background_and_offline_return(cx: &mut TestAppContext) {
    if disabled() {
        return;
    }
    let (view, cx) = mount(cx);
    let composer = view.read_with(cx, |app, _| app.composer.clone());
    tap(cx, "composer-input");
    cx.simulate_input("界👩🏽‍💻e\u{301}");
    settle(&view, cx);
    tap(cx, "sim-key-0"); // q
    tap(cx, "sim-key-19"); // shift
    tap(cx, "sim-key-0"); // Q, then one-shot shift resets
    tap(cx, "sim-key-0");
    assert_eq!(text(&composer, cx), "界👩🏽‍💻e\u{301}qQq");
    tap(cx, "sim-key-27"); // backspace
    tap(cx, "sim-key-29"); // space
    assert_eq!(text(&composer, cx), "界👩🏽‍💻e\u{301}qQ ");
    draw(cx);
    let keyboard = cx.debug_bounds("sim-keyboard").unwrap();
    // Empty home-indicator area, below the final row, must swallow blur.
    raw_tap(
        cx,
        gpui::point(keyboard.center().x, keyboard.bottom() - px(5.)),
    );
    focused(&composer, cx);
    let sent = submissions(&composer, cx);
    tap(cx, "sim-key-30"); // send
    assert_eq!(sent.get(), 1);
    focused(&composer, cx);
    assert_eq!(text(&composer, cx), "界👩🏽‍💻e\u{301}qQ ");
    view.read_with(cx, |app, _| {
        assert!(app.connection.is_none());
        assert!(app.session.transcript.is_empty());
    });
}

#[gpui::test]
fn keyboard_bridge_focus_transfer_search_return_preserves_unicode(cx: &mut TestAppContext) {
    if disabled() {
        return;
    }
    let (view, cx) = mount(cx);
    let composer = view.read_with(cx, |app, _| app.composer.clone());
    tap(cx, "composer-input");
    cx.simulate_input("草稿👩🏽‍💻e\u{301}");
    settle(&view, cx);
    drawer(&view, cx);
    tap(cx, "drawer-search-input");
    settle(&view, cx);
    tap(cx, "sim-key-0");
    let search = view.read_with(cx, |app, _| app.drawer_search.clone());
    focused(&search, cx);
    assert_eq!(text(&search, cx), "q");
    assert_eq!(text(&composer, cx), "草稿👩🏽‍💻e\u{301}");
    // Search Return must not submit the unrelated composer.
    let searched = submissions(&search, cx);
    let sent = submissions(&composer, cx);
    tap(cx, "sim-key-30");
    assert_eq!(searched.get(), 1);
    assert_eq!(sent.get(), 0);
    assert_eq!(text(&composer, cx), "草稿👩🏽‍💻e\u{301}");
    assert_eq!(text(&search, cx), "q");
}

#[gpui::test]
fn keyboard_bridge_directory_go_preserves_unicode_composer(cx: &mut TestAppContext) {
    if disabled() {
        return;
    }
    let (view, cx) = mount(cx);
    let composer = view.read_with(cx, |app, _| app.composer.clone());
    tap(cx, "composer-input");
    cx.simulate_input("草稿👩🏽‍💻e\u{301}");
    settle(&view, cx);
    tap(cx, "location");
    tap(cx, "dir-input");
    settle(&view, cx);
    let dir = view.read_with(cx, |app, _| app.dir_input.clone());
    cx.simulate_keystrokes("ctrl-a");
    cx.simulate_input("/界");
    tap(cx, "sim-key-0");
    assert_eq!(text(&dir, cx), "/界q");
    focused(&dir, cx);
    tap(cx, "sim-key-30");
    view.read_with(cx, |app, _| {
        assert!(!app.dir_picker);
        assert_eq!(app.session.working_dir.as_deref(), Some("/界q"));
        assert!(app.connection.is_none());
    });
    assert_eq!(text(&composer, cx), "草稿👩🏽‍💻e\u{301}");
}

#[gpui::test]
fn keyboard_bridge_email_code_suggestions_and_return_are_field_local(cx: &mut TestAppContext) {
    if disabled() {
        return;
    }
    let (view, cx) = super::account_coverage::mount(cx);
    cx.update(|window, _| window.set_emulate_touch_from_mouse(true));
    let (email, code, composer) = view.read_with(cx, |app, _| {
        (app.email.clone(), app.code.clone(), app.composer.clone())
    });
    view.update(cx, |app, cx| {
        app.composer
            .update(cx, |input, cx| input.set_text("未送信👩🏽‍💻", cx))
    });
    tap(cx, "email-input");
    settle(&view, cx);
    tap(cx, "sim-suggestion");
    tap(cx, "sim-key-31"); // email punctuation '.'
    assert_eq!(text(&email, cx), "you@example.com.");
    focused(&email, cx);
    let submits = std::rc::Rc::new(std::cell::Cell::new(0));
    let observed = submits.clone();
    cx.update(|_, cx| {
        cx.subscribe(&email, move |_, event, _| {
            if matches!(event, TextInputEvent::Submit) {
                observed.set(observed.get() + 1);
            }
        })
        .detach();
    });
    tap(cx, "sim-key-32"); // next
    assert_eq!(submits.get(), 1);
    view.update(cx, |app, cx| {
        app.login_token = Some("offline-fixture-token".into());
        cx.notify();
    });
    settle(&view, cx);
    tap(cx, "code-input");
    settle(&view, cx);
    tap(cx, "sim-suggestion");
    assert_eq!(text(&code, cx), "123456");
    tap(cx, "sim-key-10"); // numeric backspace
    tap(cx, "sim-key-0"); // numeric 1
    assert_eq!(text(&code, cx), "123451");
    focused(&code, cx);
    assert_eq!(text(&email, cx), "you@example.com.");
    assert_eq!(text(&composer, cx), "未送信👩🏽‍💻");
    view.read_with(cx, |app, _| {
        assert!(app.connection.is_none());
        assert!(app.account.is_none());
        assert!(!app.signing_in);
    });
}

#[gpui::test]
fn keyboard_bridge_transient_rename_done_preserves_composer(cx: &mut TestAppContext) {
    if disabled() {
        return;
    }
    let (view, cx) = mount(cx);
    let composer = view.read_with(cx, |app, _| app.composer.clone());
    tap(cx, "composer-input");
    cx.simulate_input("草稿👩🏽‍💻e\u{301}");
    view.update(cx, |app, cx| app.feel_rename_tab(app.active_tab, cx));
    settle(&view, cx);
    let rename = cx.update(|window, cx| {
        view.read_with(cx, |app, cx| app.focused_rename_input(window, cx).unwrap())
    });
    tap_input(&rename, cx);
    cx.simulate_input("会話👋");
    tap(cx, "sim-key-0");
    assert_eq!(text(&rename, cx), "会話👋q");
    assert_eq!(text(&composer, cx), "草稿👩🏽‍💻e\u{301}");
    tap(cx, "sim-key-30");
    cx.run_until_parked();
    settle(&view, cx);
    view.read_with(cx, |app, _| {
        assert_eq!(app.session.session_title.as_deref(), Some("会話👋q"));
        assert!(app.connection.is_none());
    });
    assert_eq!(text(&composer, cx), "草稿👩🏽‍💻e\u{301}");
    assert!(cx.debug_bounds("rename-card").is_none());
    cx.update(|window, cx| assert!(!rename.read(cx).is_focused(window)));
}

#[gpui::test]
fn keyboard_bridge_transfer_editor_preserves_unrelated_draft(cx: &mut TestAppContext) {
    if disabled() {
        return;
    }
    let (view, cx) = mount(cx);
    let (composer, transfer) = view.read_with(cx, |app, _| {
        (app.composer.clone(), app.transfer_input.clone())
    });
    view.update(cx, |app, cx| {
        app.synced = super::synced_sessions::State::fixture();
        app.composer
            .update(cx, |input, cx| input.set_text("草稿👩🏽‍💻e\u{301}", cx));
        cx.notify();
    });
    drawer(&view, cx);
    reveal_input(&view, &transfer, Some("drawer-overview"), cx);
    settle(&view, cx);
    tap(cx, "sim-key-0");
    assert_eq!(text(&transfer, cx), "q");
    focused(&transfer, cx);
    let submitted = submissions(&transfer, cx);
    tap(cx, "sim-key-30");
    assert_eq!(submitted.get(), 1);
    assert_eq!(text(&transfer, cx), "q");
    assert_eq!(text(&composer, cx), "草稿👩🏽‍💻e\u{301}");
    view.read_with(cx, |app, _| assert!(app.connection.is_none()));
}

#[gpui::test]
fn keyboard_bridge_apple_link_email_suggestion_and_next_preserve_draft(cx: &mut TestAppContext) {
    if disabled() {
        return;
    }
    let (view, cx) = mount(cx);
    let composer = view.read_with(cx, |app, _| app.composer.clone());
    view.update(cx, |app, cx| {
        app.screen = Screen::Settings;
        app.account.as_mut().unwrap().email = None;
        app.composer
            .update(cx, |input, cx| input.set_text("草稿👩🏽‍💻e\u{301}", cx));
        cx.notify();
    });
    // A forced preview keyboard is already visible before any field focuses.
    // Scroll the settings action above it rather than tapping an occluded bound.
    for _ in 0..16 {
        settle(&view, cx);
        let bounds = cx.debug_bounds("link-account").unwrap();
        let limit = view.read_with(cx, |app, _| {
            if app.keyboard_up {
                844. - crate::sim_keyboard::HEIGHT
            } else {
                844.
            }
        });
        if bounds.top() > px(20.) && bounds.bottom() < px(limit) {
            break;
        }
        cx.simulate_event(gpui::ScrollWheelEvent {
            position: gpui::point(px(195.), px(300.)),
            delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-160.))),
            touch_phase: gpui::TouchPhase::Moved,
            modifiers: Modifiers::default(),
        });
    }
    draw(cx);
    let bounds = cx.debug_bounds("link-account").unwrap();
    let limit = view.read_with(cx, |app, _| {
        if app.keyboard_up {
            844. - crate::sim_keyboard::HEIGHT
        } else {
            844.
        }
    });
    assert!(
        bounds.top() > px(20.) && bounds.bottom() < px(limit),
        "Apple-link action occluded: {bounds:?}"
    );
    tap(cx, "link-account");
    settle(&view, cx);
    let input = cx.update(|window, cx| {
        view.read_with(cx, |app, cx| {
            app.apple_link
                .as_ref()
                .unwrap()
                .read(cx)
                .focused_input(window, cx)
                .unwrap()
        })
    });
    reveal_input(&view, &input, None, cx);
    tap(cx, "sim-suggestion");
    assert_eq!(text(&input, cx), "you@example.com");
    let submitted = submissions(&input, cx);
    tap(cx, "sim-key-32");
    assert_eq!(submitted.get(), 1);
    assert_eq!(text(&composer, cx), "草稿👩🏽‍💻e\u{301}");
    view.read_with(cx, |app, _| assert!(app.connection.is_none()));
}
