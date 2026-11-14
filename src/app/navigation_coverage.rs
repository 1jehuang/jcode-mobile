//! Offline, hit-tested navigation workflows. Fixtures never load accounts or start transport.
use super::*;
use gpui::{
    Entity, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, TestAppContext, VisualTestContext,
};

pub(super) fn mount(cx: &mut TestAppContext) -> (Entity<MobileApp>, &mut VisualTestContext) {
    cx.update(init);
    let (view, cx) = cx.add_window_view(|window, cx| {
        let mut app = MobileApp::new_offline(
            SafeArea {
                top: 20.,
                bottom: 0.,
            },
            window,
            cx,
        );
        app.screen = Screen::Chat;
        app.device = Some((390., 844.));
        app.account = Some(Account {
            given_name: None,
            name: None,
            api_key: "navigation-fixture".into(),
            api_base: "https://offline.invalid".into(),
            email: Some("fixture@example.test".into()),
        });
        app.session.working_dir = Some("/repo".into());
        app
    });
    cx.simulate_resize(gpui::size(px(390.), px(844.)));
    cx.update(|window, _| window.set_emulate_touch_from_mouse(true));
    (view, cx)
}

pub(super) fn draw(cx: &mut VisualTestContext) {
    cx.update(|window, cx| window.draw(cx).clear(cx));
}

/// Raw mouse down/up is translated by GPUI into the phone touch path.
pub(super) fn tap(cx: &mut VisualTestContext, selector: &'static str) {
    draw(cx);
    let bounds = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("missing navigation control {selector}"));
    assert!(
        bounds.size.width > px(0.) && bounds.size.height > px(0.),
        "empty hit target {selector}"
    );
    let position = bounds.center();
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

pub(super) fn drawer(view: &Entity<MobileApp>, cx: &mut VisualTestContext) {
    // Animation time is not advanced by headless frames. Pin the fixture at its
    // settled position after testing the actual menu control.
    tap(cx, "menu");
    view.update(cx, |app, cx| {
        assert!(app.drawer_open);
        app.drawer_t = 1.;
        cx.notify();
    });
}

#[gpui::test]
fn directory_sheet_hit_test_invalid_same_valid_and_dismiss(cx: &mut TestAppContext) {
    let (view, cx) = mount(cx);
    tap(cx, "location");
    view.update(cx, |app, cx| {
        assert!(app.dir_picker);
        app.dir_input
            .update(cx, |input, cx| input.set_text("   ", cx));
    });
    tap(cx, "dir-go");
    view.read_with(cx, |app, _| {
        assert!(app.dir_picker);
        assert_eq!(app.session.working_dir.as_deref(), Some("/repo"));
    });
    view.update(cx, |app, cx| {
        app.dir_input
            .update(cx, |input, cx| input.set_text(" /repo/ ", cx))
    });
    tap(cx, "dir-go");
    view.read_with(cx, |app, _| {
        assert!(!app.dir_picker);
        assert_eq!(app.tab_order, [0]);
    });
    tap(cx, "location");
    view.update(cx, |app, cx| {
        app.dir_input
            .update(cx, |input, cx| input.set_text(" /other ", cx))
    });
    tap(cx, "dir-go");
    view.read_with(cx, |app, _| {
        assert!(!app.dir_picker);
        assert_eq!(app.session.working_dir.as_deref(), Some("/other"));
        assert!(app.connection.is_none());
    });
    tap(cx, "location");
    tap(cx, "dir-dismiss");
    view.read_with(cx, |app, _| {
        assert!(!app.dir_picker)
    });
}

#[gpui::test]
fn directory_recent_row_and_start_suggestions_hit_test(cx: &mut TestAppContext) {
    let (view, cx) = mount(cx);
    view.update(cx, |app, cx| {
        app.session.session_list = serde_json::from_value(serde_json::json!([
            {"session_id":"session_fixture_1700000001000", "title":"Continue fixture", "status":"running", "working_dir":"/other"}
        ])).unwrap();
        cx.notify();
    });
    draw(cx);
    assert!(cx.debug_bounds("start-suggestions").is_some());
    tap(cx, "location");
    tap(cx, "dir-row-0");
    view.read_with(cx, |app, _| {
        assert_eq!(app.session.working_dir.as_deref(), Some("/other"));
        assert!(!app.dir_picker);
    });
    view.update(cx, |app, cx| {
        app.session.working_dir = Some("/repo".into());
        cx.notify();
    });
    tap(cx, "recent-dir-0");
    view.read_with(cx, |app, _| {
        assert_eq!(app.session.working_dir.as_deref(), Some("/other"));
        assert!(app.connection.is_none());
    });
    tap(cx, "suggested-session-0");
    view.read_with(cx, |app, _| {
        assert_eq!(app.tab_order.len(), 2);
        assert!(app.connection.is_none());
    });
}

#[gpui::test]
fn right_edge_new_session_has_no_directory_sheet_and_preserves_draft(cx: &mut TestAppContext) {
    let (view, cx) = mount(cx);
    view.update(cx, |app, cx| {
        app.composer
            .update(cx, |input, cx| input.set_text("Original draft", cx))
    });
    view.update(cx, |app, cx| {
        app.swipe.feed(-crate::swipe::NEW_SESSION_COMMIT, 0., std::time::Instant::now());
        app.end_swipe(cx);
    });
    draw(cx);
    assert!(cx.debug_bounds("dir-go").is_none());
    assert!(cx.debug_bounds("dir-input").is_none());
    view.read_with(cx, |app, _| {
        assert!(!app.dir_picker);
        assert_eq!(app.tab_order.len(), 2);
        assert!(app.session.working_dir.is_none());
        assert!(app.connection.is_none());
    });
    tap(cx, "tab-0");
    view.read_with(cx, |app, cx| {
        assert_eq!(app.active_tab, 0);
        assert_eq!(app.composer.read(cx).text(), "Original draft");
    });
}

#[gpui::test]
fn directory_change_with_history_opens_tab_and_preserves_conversation(cx: &mut TestAppContext) {
    let (view, cx) = mount(cx);
    view.update(cx, |app, cx| {
        app.session.transcript.push(TranscriptEntry {
            id: 1,
            role: Role::User,
            text: "Keep this conversation".into(),
            reasoning: String::new(),
            tool_calls: vec![],
            is_streaming: false,
            is_queued: false,
        });
        cx.notify();
    });
    tap(cx, "location");
    view.update(cx, |app, cx| {
        app.dir_input
            .update(cx, |input, cx| input.set_text("/new-repo", cx))
    });
    tap(cx, "dir-go");
    view.read_with(cx, |app, _| {
        assert_eq!(app.tab_order.len(), 2);
        assert_eq!(app.session.working_dir.as_deref(), Some("/new-repo"));
        assert!(app.session.transcript.is_empty());
        assert!(app.connection.is_none());
    });
    tap(cx, "tab-0");
    view.read_with(cx, |app, _| {
        assert_eq!(app.session.working_dir.as_deref(), Some("/repo"));
        assert_eq!(app.session.transcript[0].text, "Keep this conversation");
    });
}

#[gpui::test]
fn drawer_new_switch_close_refresh_and_account_hit_test(cx: &mut TestAppContext) {
    let (view, cx) = mount(cx);
    view.update(cx, |app, cx| {
        app.device = Some((1024., 844.));
        cx.notify();
    });
    cx.simulate_resize(gpui::size(px(1024.), px(844.)));
    // These quick actions belong to the docked wide sidebar, not the phone drawer.
    tap(cx, "drawer-refresh");
    view.read_with(cx, |app, _| assert!(app.connection.is_none()));
    tap(cx, "drawer-new-thread");
    view.update(cx, |app, cx| {
        assert_eq!(app.tab_order.len(), 2);
        assert!(!app.drawer_open);
        app.drawer_t = 0.;
        app.device = Some((390., 844.));
        cx.notify();
    });
    cx.simulate_resize(gpui::size(px(390.), px(844.)));
    drawer(&view, cx);
    tap(cx, "drawer-tab-0");
    view.update(cx, |app, cx| {
        assert_eq!(app.active_tab, 0);
        assert!(!app.drawer_open);
        app.drawer_t = 0.;
        cx.notify();
    });
    drawer(&view, cx);
    tap(cx, "drawer-tab-close-1");
    view.read_with(cx, |app, _| {
        assert_eq!(app.tab_order, [0]);
        assert!(app.drawer_open, "close must not bubble into tab switch");
    });
    tap(cx, "drawer-account");
    view.read_with(cx, |app, _| {
        assert!(matches!(app.screen, Screen::Settings));
        assert!(!app.drawer_open);
        assert!(app.connection.is_none());
    });
}

#[gpui::test]
fn drawer_directory_groups_and_search_hit_test(cx: &mut TestAppContext) {
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
    tap(cx, "drawer-directory-/beta");
    draw(cx);
    assert!(cx.debug_bounds("drawer-session-beta").is_some());
    assert!(cx.debug_bounds("drawer-session-alpha").is_none());
    tap(cx, "drawer-directory-/beta");
    draw(cx);
    assert!(cx.debug_bounds("drawer-session-beta").is_none());
    // Text is supplied synthetically, filtering and result selection are real.
    view.update(cx, |app, cx| {
        app.drawer_search
            .update(cx, |input, cx| input.set_text("needle", cx));
        cx.notify();
    });
    draw(cx);
    assert!(cx.debug_bounds("drawer-session-alpha").is_some());
    assert!(cx.debug_bounds("drawer-session-beta").is_none());
    tap(cx, "drawer-session-alpha");
    view.read_with(cx, |app, _| {
        assert_eq!(app.tab_order.len(), 2);
        assert!(!app.drawer_open);
        assert!(app.connection.is_none());
    });
}

#[gpui::test]
fn drawer_directory_new_button_does_not_toggle_group(cx: &mut TestAppContext) {
    let (view, cx) = mount(cx);
    view.update(cx, |app, cx| {
        app.session.session_list = serde_json::from_value(serde_json::json!([
            {"session_id":"alpha", "status":"idle", "title":"Alpha", "working_dir":"/alpha"}
        ]))
        .unwrap();
        cx.notify();
    });
    drawer(&view, cx);
    tap(cx, "drawer-new-in-/alpha");
    view.read_with(cx, |app, _| {
        assert_eq!(app.tab_order.len(), 2);
        assert_eq!(app.session.working_dir.as_deref(), Some("/alpha"));
        assert!(!app.drawer_open);
        assert!(
            matches!(app.drawer_directory, sidebar::DirectorySelection::Automatic),
            "nested + must not bubble into group toggle"
        );
        assert!(app.connection.is_none());
    });
}

#[gpui::test]
fn tab_menu_switch_close_others_and_disabled_fork_hit_test(cx: &mut TestAppContext) {
    let (view, cx) = mount(cx);
    view.update(cx, |app, cx| {
        app.new_tab(cx);
        app.new_tab(cx);
        app.tab_menu = Some(0);
        cx.notify();
    });
    tap(cx, "tm-open");
    view.read_with(cx, |app, _| {
        assert_eq!(app.active_tab, 0);
        assert!(app.tab_menu.is_none());
    });
    view.update(cx, |app, cx| {
        app.tab_menu = Some(0);
        cx.notify();
    });
    tap(cx, "tm-fork");
    view.read_with(cx, |app, _| {
        assert_eq!(app.tab_order.len(), 3);
        assert!(app.tab_menu.is_some());
        assert!(app.connection.is_none());
    });
    tap(cx, "tm-others");
    view.read_with(cx, |app, _| {
        assert_eq!(app.tab_order, [0]);
        assert!(app.tab_menu.is_none());
    });
    view.update(cx, |app, cx| {
        app.tab_menu = Some(0);
        cx.notify();
    });
    tap(cx, "tm-close");
    view.read_with(cx, |app, _| assert_eq!(app.tab_order, [0]));
    tap(cx, "tm-dismiss");
    view.read_with(cx, |app, _| assert!(app.tab_menu.is_none()));
}

fn open_model(view: &Entity<MobileApp>, cx: &mut VisualTestContext) {
    tap(cx, "pill-model");
    draw(cx);
    // Settle presentation only. The model opening and selection still pass
    // through their real hit-tested controls, not direct action invocation.
    view.update(cx, |app, cx| {
        assert!(app.model_picker);
        app.feel.g.sheet_y = 0.;
        app.feel.g.edge_x = 0.;
        cx.notify();
    });
}

#[gpui::test]
fn model_picker_bare_selection_same_selection_and_dismiss_hit_test(cx: &mut TestAppContext) {
    let (view, cx) = mount(cx);
    view.update(cx, |app, cx| {
        app.session.model_name = Some("fixture-one".into());
        app.session.available_models = vec!["fixture-one".into(), "fixture-two".into()];
        cx.notify();
    });
    open_model(&view, cx);
    let (_, feedback) = haptics::capture(|| tap(cx, "model-row-bare-0"));
    assert!(
        feedback.is_empty(),
        "selecting the current model stays quiet"
    );
    view.read_with(cx, |app, _| assert!(!app.model_picker));
    open_model(&view, cx);
    let (_, feedback) = haptics::capture(|| tap(cx, "model-row-bare-1"));
    assert_eq!(feedback, [Haptic::Selection]);
    view.read_with(cx, |app, _| {
        assert!(!app.model_picker);
        assert!(app.connection.is_none());
    });
    open_model(&view, cx);
    tap(cx, "model-dismiss");
    view.update(cx, |app, cx| {
        // Advance to the dismissal target rather than depending on wall time.
        app.feel.g.sheet_y = app
            .feel
            .g
            .sheet
            .map_or(520., |bounds| f32::from(bounds.size.height))
            .max(1.);
        cx.notify();
    });
    draw(cx);
    view.read_with(cx, |app, _| assert!(!app.model_picker));
}

#[gpui::test]
fn model_picker_unavailable_route_is_inert_and_available_route_selects(cx: &mut TestAppContext) {
    let (view, cx) = mount(cx);
    view.update(cx, |app, cx| {
        app.session.routes = serde_json::from_value(serde_json::json!([
            {"model":"fixture-model", "provider":"Unavailable", "api_method":"openai-api", "available":false, "detail":"No fixture credential"},
            {"model":"fixture-model", "provider":"Available", "api_method":"openai-api", "available":true, "detail":""}
        ])).unwrap();
        cx.notify();
    });
    open_model(&view, cx);
    let (_, feedback) = haptics::capture(|| tap(cx, "model-row-route-0"));
    assert!(feedback.is_empty());
    view.read_with(cx, |app, _| assert!(app.model_picker));
    let (_, feedback) = haptics::capture(|| tap(cx, "model-row-route-1"));
    assert_eq!(feedback, [Haptic::Selection]);
    view.read_with(cx, |app, _| {
        assert!(!app.model_picker);
        assert!(app.connection.is_none());
    });
}
