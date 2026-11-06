//! Raw pointer focus and routed keyboard input against offline GPUI app state.
//! These are not native UIKit keyboard or selection-handle tests.
use super::navigation_coverage::{draw, drawer, mount, tap};
use super::*;
use gpui::{TestAppContext, VisualTestContext};

fn mouse_down(cx: &mut VisualTestContext, position: gpui::Point<gpui::Pixels>) {
    cx.simulate_event(gpui::MouseDownEvent {
        button: gpui::MouseButton::Left,
        position,
        modifiers: gpui::Modifiers::default(),
        click_count: 1,
        first_mouse: false,
    });
}

fn settle_keyboard(view: &Entity<MobileApp>, cx: &mut VisualTestContext) {
    draw(cx);
    view.update(cx, |app, cx| {
        app.inset_anim = None;
        cx.notify();
    });
    draw(cx);
}

struct InputHost(Entity<TextInput>);
impl Render for InputHost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .child(div().w(px(300.)).child(self.0.clone()))
    }
}

#[gpui::test]
fn text_input_pointer_release_outside_clears_selection_gesture(cx: &mut TestAppContext) {
    cx.update(init);
    let (host, cx) = cx.add_window_view(|_, cx| InputHost(cx.new(|cx| TextInput::new("", cx))));
    let input = host.read_with(cx, |host, _| host.0.clone());
    draw(cx);
    mouse_down(cx, gpui::point(px(25.), px(20.)));
    cx.simulate_event(gpui::MouseUpEvent {
        button: gpui::MouseButton::Left,
        position: gpui::point(px(25.), px(20.)),
        modifiers: gpui::Modifiers::default(),
        click_count: 1,
    });
    cx.simulate_input("abcdef");
    draw(cx);
    let (start, end) = cx.update(|window, cx| {
        input.update(cx, |input, cx| {
            use gpui::EntityInputHandler;
            let bounds = gpui::Bounds::default();
            let a = input.bounds_for_range(1..1, bounds, window, cx).unwrap();
            let b = input.bounds_for_range(4..4, bounds, window, cx).unwrap();
            (
                a.origin + gpui::point(px(0.), px(8.)),
                b.origin + gpui::point(px(0.), px(8.)),
            )
        })
    });
    mouse_down(cx, start);
    cx.simulate_event(gpui::MouseMoveEvent {
        position: end,
        pressed_button: Some(gpui::MouseButton::Left),
        modifiers: gpui::Modifiers::default(),
    });
    assert_eq!(
        input.read_with(cx, |input, _| input.selected_text().to_owned()),
        "bcd"
    );
    cx.simulate_event(gpui::MouseUpEvent {
        button: gpui::MouseButton::Left,
        position: gpui::point(px(500.), px(200.)),
        modifiers: gpui::Modifiers::default(),
        click_count: 1,
    });
    mouse_down(cx, gpui::point(px(500.), px(200.)));
    cx.simulate_event(gpui::MouseMoveEvent {
        position: start,
        pressed_button: Some(gpui::MouseButton::Left),
        modifiers: gpui::Modifiers::default(),
    });
    assert_eq!(
        input.read_with(cx, |input, _| input.selected_text().to_owned()),
        "bcd"
    );
}

#[gpui::test]
fn composer_desktop_mouse_drag_unicode_selection_and_outside_release(cx: &mut TestAppContext) {
    let (view, cx) = mount(cx);
    // Desktop selection uses raw mouse events, not touch-emulation or UIKit handles.
    cx.update(|window, _| window.set_emulate_touch_from_mouse(false));
    tap(cx, "composer-input");
    cx.simulate_input("A👩🏽‍💻Z");
    // Pin only the keyboard inset animation before measuring caret positions.
    // Focus and text still come exclusively from the actual input events.
    settle_keyboard(&view, cx);
    let (start, end) = cx.update(|window, cx| {
        view.update(cx, |app, cx| {
            use gpui::EntityInputHandler;
            app.composer.update(cx, |input, cx| {
                let bounds = gpui::Bounds::default();
                let start = input.bounds_for_range(1..1, bounds, window, cx).unwrap();
                let end = input.bounds_for_range(8..8, bounds, window, cx).unwrap();
                (
                    start.origin + gpui::point(px(0.), px(8.)),
                    end.origin + gpui::point(px(0.), px(8.)),
                )
            })
        })
    });
    mouse_down(cx, start);
    cx.simulate_event(gpui::MouseMoveEvent {
        position: end,
        pressed_button: Some(gpui::MouseButton::Left),
        modifiers: gpui::Modifiers::default(),
    });
    view.read_with(cx, |app, cx| {
        assert_eq!(app.composer.read(cx).selected_text(), "👩🏽‍💻")
    });
    cx.simulate_event(gpui::MouseUpEvent {
        button: gpui::MouseButton::Left,
        position: gpui::point(px(2.), px(2.)),
        modifiers: gpui::Modifiers::default(),
        click_count: 1,
    });
    // A later pressed move without a new field down must not reuse the old anchor.
    cx.simulate_event(gpui::MouseMoveEvent {
        position: start,
        pressed_button: Some(gpui::MouseButton::Left),
        modifiers: gpui::Modifiers::default(),
    });
    view.read_with(cx, |app, cx| {
        assert_eq!(app.composer.read(cx).selected_text(), "👩🏽‍💻")
    });
    cx.simulate_input("界");
    view.read_with(cx, |app, cx| {
        assert_eq!(app.composer.read(cx).text(), "A界Z")
    });
}

#[gpui::test]
fn drawer_search_pointer_focus_routes_typed_filter_and_result(cx: &mut TestAppContext) {
    let (view, cx) = mount(cx);
    view.update(cx, |app, cx| {
        app.session.session_list = serde_json::from_value(serde_json::json!([
            {"session_id":"alpha", "status":"idle", "title":"Alpha needle", "working_dir":"/alpha"},
            {"session_id":"beta", "status":"idle", "title":"Beta work", "working_dir":"/beta"}
        ]))
        .unwrap();
        cx.notify();
    });
    drawer(&view, cx);
    tap(cx, "drawer-search-input");
    assert_focus(&view, cx, |app| &app.drawer_search);
    cx.simulate_input("needle");
    draw(cx);
    view.read_with(cx, |app, cx| {
        assert_eq!(app.drawer_search.read(cx).text(), "needle");
        assert!(app.composer.read(cx).text().is_empty());
        assert!(app.connection.is_none());
    });
    assert!(cx.debug_bounds("drawer-session-alpha").is_some());
    assert!(cx.debug_bounds("drawer-session-beta").is_none());
    cx.simulate_keystrokes("home delete");
    view.read_with(cx, |app, cx| {
        assert_eq!(app.drawer_search.read(cx).text(), "eedle")
    });
    tap(cx, "drawer-session-alpha");
    view.read_with(cx, |app, _| {
        assert!(!app.drawer_open);
        assert_eq!(app.tab_order.len(), 2);
        assert!(app.connection.is_none());
    });
}

fn assert_focus(
    view: &Entity<MobileApp>,
    cx: &mut VisualTestContext,
    input: impl Fn(&MobileApp) -> &Entity<TextInput>,
) {
    cx.update(|window, cx| {
        view.read_with(cx, |app, _| assert!(input(app).read(cx).is_focused(window)))
    });
}

#[gpui::test]
fn directory_go_keyboard_event_changes_directory_without_transport(cx: &mut TestAppContext) {
    let (view, cx) = mount(cx);
    tap(cx, "location");
    tap(cx, "dir-input");
    assert_focus(&view, cx, |app| &app.dir_input);
    // The directory is prefilled by the route. Select it via a real key binding.
    cx.simulate_keystrokes("ctrl-a");
    cx.simulate_input(" /typed-repo ");
    cx.simulate_keystrokes("enter");
    view.read_with(cx, |app, _| {
        assert!(!app.dir_picker);
        assert_eq!(app.session.working_dir.as_deref(), Some("/typed-repo"));
        assert!(app.connection.is_none());
    });
}

#[gpui::test]
fn composer_pointer_caret_unicode_keyboard_edit_and_focus_transfer(cx: &mut TestAppContext) {
    let (view, cx) = mount(cx);
    tap(cx, "composer-input");
    assert_focus(&view, cx, |app| &app.composer);
    cx.simulate_input("A👩🏽‍💻e\u{301}Z");
    cx.simulate_keystrokes("left backspace");
    view.read_with(cx, |app, cx| {
        assert_eq!(app.composer.read(cx).text(), "A👩🏽‍💻Z")
    });
    cx.simulate_keystrokes("left backspace");
    view.read_with(cx, |app, cx| {
        assert_eq!(app.composer.read(cx).text(), "👩🏽‍💻Z")
    });
    settle_keyboard(&view, cx);
    tap(cx, "location");
    tap(cx, "dir-input");
    assert_focus(&view, cx, |app| &app.dir_input);
    cx.simulate_input("/suffix");
    view.read_with(cx, |app, cx| {
        assert_eq!(app.composer.read(cx).text(), "👩🏽‍💻Z")
    });
    tap(cx, "dir-dismiss");
    settle_keyboard(&view, cx);
    tap(cx, "composer-input");
    assert_focus(&view, cx, |app| &app.composer);
    cx.simulate_keystrokes("home delete");
    view.read_with(cx, |app, cx| assert_eq!(app.composer.read(cx).text(), "Z"));
}

#[gpui::test]
fn onboarding_fields_pointer_focus_typing_submit_and_outside_dismiss(cx: &mut TestAppContext) {
    let (view, cx) = super::account_coverage::mount(cx);
    for code_step in [false, true] {
        view.update(cx, |app, cx| {
            app.login_token = code_step.then(|| "fixture-only-token".into());
            cx.notify();
        });
        let selector = if code_step {
            "code-input"
        } else {
            "email-input"
        };
        super::account_coverage::tap(cx, selector);
        let input = view.read_with(cx, |app, _| {
            if code_step {
                app.code.clone()
            } else {
                app.email.clone()
            }
        });
        let submits = std::rc::Rc::new(std::cell::Cell::new(0));
        let recorded = submits.clone();
        cx.update(|_, cx| {
            cx.subscribe(&input, move |_, event, _| {
                if matches!(event, TextInputEvent::Submit) {
                    recorded.set(recorded.get() + 1);
                }
            })
            .detach();
        });
        cx.update(|window, cx| assert!(input.read(cx).is_focused(window)));
        let text = if code_step {
            "123456"
        } else {
            "fixture@example.test"
        };
        cx.simulate_input(text);
        assert_eq!(
            input.read_with(cx, |input, _| input.text().to_owned()),
            text
        );
        cx.simulate_keystrokes("enter");
        assert_eq!(
            submits.get(),
            1,
            "Return must route through the focused field"
        );
        view.read_with(cx, |app, _| {
            assert!(!app.signing_in && app.account.is_none());
            assert!(app.connection.is_none());
            assert_eq!(app.login_token.is_some(), code_step);
        });
        // Actual button entry is also guarded offline. This does not claim a
        // provider request was started or a native Return key was exercised.
        super::account_coverage::tap(cx, "submit-sign-in");
        view.read_with(cx, |app, _| {
            assert!(!app.signing_in && app.account.is_none())
        });
        cx.update(|window, cx| {
            for phase in [gpui::TouchPhase::Started, gpui::TouchPhase::Ended] {
                window.dispatch_event(
                    gpui::PlatformInput::Touch(gpui::TouchEvent {
                        id: gpui::TouchId(992),
                        phase,
                        position: gpui::point(px(10.), px(70.)),
                        predicted_position: None,
                        force: None,
                    }),
                    cx,
                );
            }
        });
        draw(cx);
        cx.update(|window, cx| assert!(!input.read(cx).is_focused(window)));
        view.read_with(cx, |app, _| assert!(!app.keyboard_up));
    }
}
