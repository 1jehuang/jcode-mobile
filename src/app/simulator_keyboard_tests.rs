//! Drawn GPUI raw-touch campaigns. These do not claim native UIKit coverage.

#[gpui::test]
fn simulator_keyboard_settings_narrow_save_preserves_draft(cx: &mut TestAppContext) {
    let _scale = scale_fixture(2.);
    let (view, cx) = mount(cx);
    resize(&view, cx, 320., 568.);
    view.update(cx, |app, cx| {
        app.composer
            .update(cx, |input, cx| input.set_text("keep draft", cx));
        app.screen = Screen::Settings;
        app.default_dir_input
            .update(cx, |input, cx| input.set_text("/saved", cx));
        cx.notify();
    });
    settle(&view, cx);
    for focused in [false, true] {
        for _ in 0..24 {
            let b = cx.debug_bounds("default-dir-save").unwrap();
            let bottom = if focused {
                cx.debug_bounds("sim-keyboard").unwrap().top()
            } else {
                px(540.)
            };
            if b.top() > px(80.) && b.bottom() <= bottom {
                break;
            }
            cx.simulate_event(gpui::ScrollWheelEvent {
                position: gpui::point(px(150.), px(120.)),
                delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-60.))),
                touch_phase: TouchPhase::Moved,
                modifiers: gpui::Modifiers::default(),
            });
            draw(cx);
        }
        if !focused {
            tap(cx, "default-dir-input");
            settle(&view, cx);
        }
    }
    let save = cx.debug_bounds("default-dir-save").unwrap();
    let kb = cx.debug_bounds("sim-keyboard").unwrap();
    assert!(
        save.top() >= px(68.) && save.bottom() <= kb.top(),
        "settings Save occluded: {save:?} {kb:?}"
    );
    tap(cx, "default-dir-save");
    settle(&view, cx);
    view.read_with(cx, |app, cx| {
        assert_eq!(app.prefs.default_dir.as_deref(), Some("/saved"));
        assert_eq!(app.composer.read(cx).text(), "keep draft");
    });
    tap(cx, "back");
}
use super::navigation_coverage::{draw, mount};
use super::*;
use gpui::{PlatformInput, TestAppContext, TouchEvent, TouchId, TouchPhase, VisualTestContext};

fn scale_fixture(scale: f32) -> ScaleFixture {
    let before = std::env::var_os("JCODE_MOBILE_TEXT_SCALE");
    // This module's campaigns are run with --test-threads=1. Restore even on panic.
    unsafe {
        std::env::set_var("JCODE_MOBILE_TEXT_SCALE", scale.to_string());
    }
    ScaleFixture(before)
}
struct ScaleFixture(Option<std::ffi::OsString>);
impl Drop for ScaleFixture {
    fn drop(&mut self) {
        unsafe {
            if let Some(before) = &self.0 {
                std::env::set_var("JCODE_MOBILE_TEXT_SCALE", before);
            } else {
                std::env::remove_var("JCODE_MOBILE_TEXT_SCALE");
            }
        }
    }
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
fn resize(view: &Entity<MobileApp>, cx: &mut VisualTestContext, w: f32, h: f32) {
    view.update(cx, |app, cx| {
        app.device = Some((w, h));
        cx.notify();
    });
    cx.simulate_resize(gpui::size(px(w), px(h)));
    settle(view, cx);
}
fn tap(cx: &mut VisualTestContext, selector: &'static str) {
    draw(cx);
    let b = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("missing {selector}"));
    let viewport = cx.update(|window, _| window.viewport_size());
    assert!(
        b.left() >= px(0.)
            && b.right() <= viewport.width
            && b.top() >= px(0.)
            && b.bottom() <= viewport.height,
        "{selector} outside viewport: {b:?} {viewport:?}"
    );
    for phase in [TouchPhase::Started, TouchPhase::Ended] {
        cx.update(|window, cx| {
            window.dispatch_event(
                PlatformInput::Touch(TouchEvent {
                    id: TouchId(923),
                    phase,
                    position: b.center(),
                    predicted_position: None,
                    force: Some(0.5),
                }),
                cx,
            );
            window.draw(cx).clear(cx);
        });
    }
}

#[gpui::test]
fn simulator_keyboard_directory_narrow_go_is_above_keys(cx: &mut TestAppContext) {
    let _scale = scale_fixture(1.6);
    let (view, cx) = mount(cx);
    resize(&view, cx, 320., 568.);
    tap(cx, "location");
    tap(cx, "dir-input");
    settle(&view, cx);
    let go = cx.debug_bounds("dir-go").unwrap();
    let kb = cx.debug_bounds("sim-keyboard").expect("keyboard open");
    assert!(
        go.bottom() <= kb.top(),
        "directory Go hidden behind keyboard: {go:?} {kb:?}"
    );
    cx.simulate_input("/other");
    tap(cx, "dir-go");
    view.read_with(cx, |app, _| assert!(!app.dir_picker));
}

#[gpui::test]
fn simulator_keyboard_landscape_directory_go_remains_reachable(cx: &mut TestAppContext) {
    let _scale = scale_fixture(2.);
    let (view, cx) = mount(cx);
    resize(&view, cx, 568., 320.);
    tap(cx, "location");
    tap(cx, "dir-input");
    settle(&view, cx);
    let go = cx.debug_bounds("dir-go").unwrap();
    let kb = cx.debug_bounds("sim-keyboard").expect("keyboard open");
    assert!(
        go.top() >= px(20.) && go.bottom() <= kb.top(),
        "landscape Go occluded: {go:?}, {kb:?}"
    );
    for (index, selector) in [
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
    ]
    .into_iter()
    .enumerate()
    {
        let key = cx.debug_bounds(selector).unwrap();
        assert!(
            key.top() >= kb.top() && key.bottom() <= kb.bottom() && key.bottom() <= px(320.),
            "landscape key {index} out of keyboard: {key:?}"
        );
    }
    let before = view.read_with(cx, |app, cx| app.dir_input.read(cx).text().to_owned());
    tap(cx, "sim-key-0");
    view.read_with(cx, |app, cx| {
        let after = app.dir_input.read(cx).text();
        assert!(after.contains('q'));
        assert_eq!(after.len(), before.len() + 1);
    });
    tap(cx, "sim-key-30");
    settle(&view, cx);
    view.read_with(cx, |app, _| assert!(!app.dir_picker));
}

#[gpui::test]
fn simulator_keyboard_narrow_model_selection_with_composer_focused(cx: &mut TestAppContext) {
    let _scale = scale_fixture(1.6);
    let (view, cx) = mount(cx);
    resize(&view, cx, 320., 568.);
    view.update(cx, |app, cx| {
        app.session.available_models = vec!["fixture-one".into(), "fixture-two".into()];
        cx.notify();
    });
    tap(cx, "composer-input");
    settle(&view, cx);
    tap(cx, "pill-model");
    view.update(cx, |app, cx| {
        assert!(app.model_picker);
        app.feel.g.sheet_y = 0.;
        app.feel.g.edge_x = 0.;
        cx.notify();
    });
    settle(&view, cx);
    tap(cx, "model-row-bare-1");
    view.read_with(cx, |app, _| assert!(!app.model_picker));
}

#[gpui::test]
fn simulator_keyboard_narrow_rename_return_preserves_draft(cx: &mut TestAppContext) {
    let _scale = scale_fixture(2.);
    let (view, cx) = mount(cx);
    resize(&view, cx, 320., 568.);
    view.update(cx, |app, cx| {
        app.composer
            .update(cx, |input, cx| input.set_text("草稿👩🏽‍💻", cx));
        app.feel_rename_tab(app.active_tab, cx);
    });
    settle(&view, cx);
    cx.simulate_input("renamed");
    tap(cx, "sim-key-30");
    cx.run_until_parked();
    settle(&view, cx);
    view.read_with(cx, |app, cx| {
        assert_eq!(app.session.session_title.as_deref(), Some("renamed"));
        assert_eq!(app.composer.read(cx).text(), "草稿👩🏽‍💻");
    });
}
