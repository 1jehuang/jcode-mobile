//! Rendered touch ownership, not service or platform integration coverage.
use super::navigation_coverage::{draw, mount, tap};
use super::*;
use gpui::{
    Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, Pixels, Point, TestAppContext,
    VisualTestContext,
};

fn touch(cx: &mut VisualTestContext, position: Point<Pixels>) {
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
    draw(cx);
}

fn unchanged(app: &MobileApp, cx: &App) {
    assert!(!app.model_picker && !app.drawer_open && app.tab_menu.is_none());
    assert_eq!(app.tab_order, [0]);
    assert_eq!(app.session.working_dir.as_deref(), Some("/repo"));
    assert_eq!(app.composer.read(cx).text(), "Preserved draft");
    assert!(app.connection.is_none());
}

#[gpui::test]
fn directory_overlay_owns_metadata_and_header_touches(cx: &mut TestAppContext) {
    let (view, cx) = mount(cx);
    for width in [320., 390.] {
        cx.simulate_resize(gpui::size(px(width), px(844.)));
        view.update(cx, |app, cx| {
            app.device = Some((width, 844.));
            app.feel.g.sheet_y = 0.;
            app.feel.g.edge_x = 0.;
            app.composer
                .update(cx, |input, cx| input.set_text("Preserved draft", cx));
            app.session.session_list = serde_json::from_value(serde_json::json!([
                {"session_id":"fixture_1700000000000", "working_dir":"/repo", "status":"idle"}
            ]))
            .unwrap();
            cx.notify();
        });
        for underlying in ["pill-account", "location", "menu", "new-session"] {
            tap(cx, "location");
            draw(cx);
            let target = cx.debug_bounds(underlying).unwrap().center();
            let scrim = cx.debug_bounds("dir-dismiss").unwrap();
            // Metadata lies in the sheet, header controls in the scrim. Both
            // are visible points in the actual overlay, not blind coordinates.
            assert!(target.x >= px(0.) && target.x < px(width));
            assert!(target.y >= px(0.) && target.y < px(844.));
            let dismisses = scrim.contains(&target);
            let selects = ["dir-row-0", "dir-go"]
                .into_iter()
                .any(|selector| cx.debug_bounds(selector).unwrap().contains(&target));
            assert_eq!(
                dismisses,
                underlying == "menu" || underlying == "new-session",
                "fixture must overlap sheet metadata or header scrim as intended"
            );
            touch(cx, target);
            view.read_with(cx, |app, cx| {
                unchanged(app, cx);
                assert_eq!(
                    app.dir_picker,
                    !(dismisses || selects),
                    "{underlying} must belong exclusively to the overlay"
                );
            });
            if !dismisses && !selects {
                tap(cx, "dir-dismiss");
            }
        }
        tap(cx, "location");
        draw(cx);
        let row = cx.debug_bounds("dir-row-0").unwrap();
        let clip = cx.debug_bounds("dir-list").unwrap();
        assert!(
            clip.contains(&row.center()),
            "row center must be inside the actual scroll clip"
        );
        let scrim = cx.debug_bounds("dir-dismiss").unwrap();
        assert!(
            row.center().y > scrim.bottom() && row.center().y < px(844.),
            "row center must be reachable inside the sheet"
        );
        tap(cx, "dir-row-0");
        view.read_with(cx, |app, cx| {
            unchanged(app, cx);
            assert!(!app.dir_picker);
        });
        tap(cx, "location");
        draw(cx);
        let go = cx.debug_bounds("dir-go").unwrap().center();
        assert!(go.y > cx.debug_bounds("dir-dismiss").unwrap().bottom() && go.y < px(844.));
        tap(cx, "dir-go");
        view.read_with(cx, |app, cx| {
            unchanged(app, cx);
            assert!(!app.dir_picker);
        });
    }
}

#[gpui::test]
fn git_overlay_owns_metadata_header_and_controls(cx: &mut TestAppContext) {
    let (view, cx) = mount(cx);
    for width in [320., 390.] {
        cx.simulate_resize(gpui::size(px(width), px(844.)));
        view.update(cx, |app, cx| {
            app.device = Some((width, 844.));
            app.composer
                .update(cx, |input, cx| input.set_text("Preserved draft", cx));
            cx.notify();
        });
        tap(cx, "git-pull-handle");
        for underlying in ["pill-account", "location", "menu", "new-session"] {
            draw(cx);
            let target = cx.debug_bounds(underlying).unwrap().center();
            let panel = cx.debug_bounds("git-info-panel").unwrap();
            let closes = cx.debug_bounds("close-git").unwrap().contains(&target);
            assert!(target.x >= px(0.) && target.x < px(width));
            assert!(target.y >= px(0.) && target.y < px(844.));
            if underlying == "pill-account" || underlying == "location" {
                assert!(
                    panel.contains(&target),
                    "metadata point must actually overlap the panel viewport"
                );
            }
            touch(cx, target);
            view.read_with(cx, |app, cx| {
                unchanged(app, cx);
                assert_eq!(
                    app.git_info.open, !closes,
                    "{underlying} must only invoke the overlapping Git control"
                );
            });
            if closes {
                tap(cx, "git-pull-handle");
            }
        }
        draw(cx);
        let refresh = cx.debug_bounds("refresh-git").unwrap().center();
        assert!(
            cx.debug_bounds("git-info-panel")
                .unwrap()
                .contains(&refresh)
        );
        tap(cx, "refresh-git");
        view.read_with(cx, |app, cx| {
            unchanged(app, cx);
            assert!(app.git_info.open);
        });
        tap(cx, "close-git");
        view.read_with(cx, |app, cx| {
            unchanged(app, cx);
            assert!(!app.git_info.open);
        });
    }
}

#[gpui::test]
fn tab_menu_owns_overlaps_and_local_actions(cx: &mut TestAppContext) {
    let (view, cx) = mount(cx);
    for width in [320., 390.] {
        cx.simulate_resize(gpui::size(px(width), px(844.)));
        view.update(cx, |app, cx| {
            app.device = Some((width, 844.));
            app.session.session_id = Some("clipboard-fixture".into());
            app.composer
                .update(cx, |input, cx| input.set_text("Preserved draft", cx));
            cx.notify();
        });
        for underlying in ["pill-account", "location", "menu", "new-session"] {
            view.update(cx, |app, cx| {
                app.tab_menu = Some(0);
                cx.notify();
            });
            draw(cx);
            let target = cx.debug_bounds(underlying).unwrap().center();
            let dismiss = cx.debug_bounds("tm-dismiss").unwrap();
            assert!(target.x >= px(0.) && target.x < px(width));
            assert!(target.y >= px(0.) && target.y < px(844.));
            touch(cx, target);
            view.read_with(cx, |app, cx| {
                assert!(!app.model_picker && !app.drawer_open && !app.dir_picker);
                assert_eq!(app.tab_order, [0]);
                assert_eq!(app.composer.read(cx).text(), "Preserved draft");
                if dismiss.contains(&target) {
                    assert!(app.tab_menu.is_none());
                }
            });
        }
        view.update(cx, |app, cx| {
            app.tab_menu = Some(0);
            cx.notify();
        });
        tap(cx, "tm-dismiss");
        view.read_with(cx, |app, _| assert!(app.tab_menu.is_none()));
        view.update(cx, |app, cx| {
            app.tab_menu = Some(0);
            cx.notify();
        });
        tap(cx, "tm-close");
        view.read_with(cx, |app, cx| {
            assert_eq!(app.tab_order, [0]);
            assert_eq!(app.tab_menu, Some(0));
            assert_eq!(app.composer.read(cx).text(), "Preserved draft");
        });
        tap(cx, "tm-copy");
        cx.update(|_, cx| {
            assert_eq!(
                cx.read_from_clipboard()
                    .and_then(|item| item.text())
                    .as_deref(),
                Some("clipboard-fixture")
            )
        });
        view.update(cx, |app, cx| {
            assert!(app.tab_menu.is_none());
            app.new_tab(cx);
            app.composer.update(cx, |input, cx| {
                input.set_text("Discard closing tab only", cx)
            });
            app.tab_menu = Some(app.active_tab);
            cx.notify();
        });
        tap(cx, "tm-close");
        view.read_with(cx, |app, cx| {
            assert_eq!(app.tab_order, [0]);
            assert_eq!(app.active_tab, 0);
            assert!(app.tab_menu.is_none() && app.connection.is_none());
            assert_eq!(app.composer.read(cx).text(), "Preserved draft");
        });
    }
}
