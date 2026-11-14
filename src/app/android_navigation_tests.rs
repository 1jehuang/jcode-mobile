//! Android back navigation uses the same GPUI state as touch dismissals.
use super::*;
use gpui::{Focusable, TestAppContext};

#[gpui::test]
fn system_back_dismisses_nested_surfaces_before_leaving_root(cx: &mut TestAppContext) {
    cx.update(super::super::init);
    let (view, cx) = cx.add_window_view(super::super::lifecycle_coverage::fixture_app);
    cx.update(|window, cx| {
        view.update(cx, |app, cx| {
            window.blur(cx);
            app.tab_menu = Some(app.active_tab);
            app.dir_picker = true;
            app.drawer_open = true;
            assert!(app.system_back(window, cx));
            assert!(app.tab_menu.is_none());
            assert!(app.dir_picker && app.drawer_open);
            assert!(app.system_back(window, cx));
            assert!(!app.dir_picker && app.drawer_open);
            assert!(app.system_back(window, cx));
            assert!(!app.drawer_open);
            assert!(!app.system_back(window, cx), "root back belongs to Android");
        });
    });
}

#[gpui::test]
fn system_back_blurs_input_without_discarding_draft(cx: &mut TestAppContext) {
    cx.update(super::super::init);
    let (view, cx) = cx.add_window_view(super::super::lifecycle_coverage::fixture_app);
    cx.update(|window, cx| {
        view.update(cx, |app, cx| {
            app.composer.update(cx, |input, cx| input.set_text("keep draft 🦀", cx));
            window.focus(&app.composer.read(cx).focus_handle(cx), cx);
            app.drawer_open = true;
            assert!(app.system_back(window, cx));
            assert!(!app.composer.read(cx).is_focused(window));
            assert_eq!(app.composer.read(cx).text(), "keep draft 🦀");
            assert!(app.drawer_open, "keyboard dismissal must not also close drawer");
        });
    });
}

#[gpui::test]
fn system_back_cancels_auth_generation_and_clears_verification_code(cx: &mut TestAppContext) {
    cx.update(super::super::init);
    let (view, cx) = cx.add_window_view(super::super::lifecycle_coverage::fixture_app);
    cx.update(|window, cx| {
        view.update(cx, |app, cx| {
            window.blur(cx);
            app.screen = Screen::SignIn;
            app.login_token = Some("fixture-token".into());
            app.code.update(cx, |input, cx| input.set_text("123456", cx));
            let generation = app.auth_generation;
            app.signing_in = true;
            assert!(app.system_back(window, cx));
            assert!(app.login_token.is_none());
            assert!(!app.signing_in);
            assert_ne!(generation, app.auth_generation);
            assert!(app.code.read(cx).text().is_empty());
            assert!(!app.system_back(window, cx));
        });
    });
}
