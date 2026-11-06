//! Deterministic, offline interaction campaigns against GPUI's *raw* touch path.
//!
//! Replay with JCODE_MOBILE_FUZZ_SEED=<u64>, JCODE_MOBILE_FUZZ_SEEDS=1..8,
//! JCODE_MOBILE_FUZZ_STEPS=1..512 (steps per seed). Invalid values use defaults.
//! No environment variables are written. The offline constructor is mandatory:
//! random hit testing deliberately reaches authentication and destructive UI.
use super::*;
use gpui::{
    Bounds, Pixels, PlatformInput, Point, TestAppContext, TouchEvent, TouchId, TouchPhase,
    VisualTestContext, point, size,
};
use std::panic::{AssertUnwindSafe, catch_unwind};

const DRAFT: &str = "Offline draft 👩🏽‍💻 e\u{301}\nDo not lose this text";
const SIZES: [(f32, f32); 4] = [(320., 568.), (390., 844.), (844., 390.), (568., 320.)];

#[derive(Clone, Copy, Debug)]
enum Surface {
    Email,
    Code,
    Chat,
    Keyboard,
    Settings,
    Drawer,
    Models,
    Directory,
}
const SURFACES: [Surface; 8] = [
    Surface::Email,
    Surface::Code,
    Surface::Chat,
    Surface::Keyboard,
    Surface::Settings,
    Surface::Drawer,
    Surface::Models,
    Surface::Directory,
];

struct Random(u64);
impl Random {
    fn next(&mut self) -> u64 {
        // SplitMix64 has no absorbing zero seed and is independent of libc/Rust RNG versions.
        self.0 = self.0.wrapping_add(0x9e3779b97f4a7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
        z ^ (z >> 31)
    }
    fn fraction(&mut self) -> f32 {
        (self.next() % 10001) as f32 / 10000.
    }
    fn position(&mut self, width: f32, height: f32) -> Point<Pixels> {
        point(px(self.fraction() * width), px(self.fraction() * height))
    }
}

fn bounded_env(name: &str, default: usize, max: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .filter(|&n| (1..=max).contains(&n))
        .unwrap_or(default)
}

fn replay<T>(
    seed: u64,
    step: usize,
    surface: Surface,
    dimensions: (f32, f32),
    operation: impl FnOnce() -> T,
) -> T {
    match catch_unwind(AssertUnwindSafe(operation)) {
        Ok(value) => value,
        Err(error) => {
            let message = error
                .downcast_ref::<String>()
                .map(String::as_str)
                .or_else(|| error.downcast_ref::<&str>().copied())
                .unwrap_or("non-string panic");
            panic!(
                "raw-touch replay seed={seed} step={step} surface={surface:?} size={dimensions:?}: {message}"
            );
        }
    }
}

fn draw(cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
}

fn touch(cx: &mut VisualTestContext, id: u64, phase: TouchPhase, position: Point<Pixels>) {
    cx.update(|window, cx| {
        window.dispatch_event(
            PlatformInput::Touch(TouchEvent {
                id: TouchId(id),
                phase,
                position,
                predicted_position: None,
                force: Some(0.5),
            }),
            cx,
        );
        // Real drawing between phases catches render-only calls made by input listeners.
        window.draw(cx).clear(cx);
    });
}

fn tap(cx: &mut VisualTestContext, id: u64, at: Point<Pixels>) {
    touch(cx, id, TouchPhase::Started, at);
    touch(cx, id, TouchPhase::Ended, at);
}

fn resize(view: &Entity<MobileApp>, cx: &mut VisualTestContext, (width, height): (f32, f32)) {
    view.update(cx, |app, cx| {
        app.device = Some((width, height));
        cx.notify();
    });
    cx.simulate_resize(size(px(width), px(height)));
    draw(cx);
}

fn install_surface(view: &Entity<MobileApp>, cx: &mut VisualTestContext, surface: Surface) {
    view.update(cx, |app, cx| {
        app.screen = match surface {
            Surface::Email | Surface::Code => Screen::SignIn,
            Surface::Settings => Screen::Settings,
            _ => Screen::Chat,
        };
        app.login_token = matches!(surface, Surface::Code).then(|| "offline-code-fixture".into());
        app.signing_in = false;
        app.focus_code = false;
        app.model_picker = matches!(surface, Surface::Models);
        app.dir_picker = matches!(surface, Surface::Directory);
        app.tab_menu = None;
        app.drawer_open = matches!(surface, Surface::Drawer);
        app.drawer_t = if app.drawer_open { 1. } else { 0. };
        app.drawer_drag = 0.;
        app.drawer_drag_last = None;
        app.feel = Default::default();
        app.swipe = Default::default();
        app.account = if matches!(surface, Surface::Email | Surface::Code) {
            None
        } else {
            Some(Account {
                api_key: "offline-fixture".into(),
                api_base: "https://offline.invalid/v1".into(),
                email: Some("touch@example.test".into()),
            })
        };
        app.session.phase = ConnectionPhase::Connecting;
        app.session.available_models = vec!["offline-model-a".into(), "offline-model-b".into()];
        app.session.working_dir = Some("/offline/project".into());
        app.synced = synced_sessions::State::fixture();
        app.email
            .update(cx, |input, cx| input.set_text("touch@example.test", cx));
        app.code
            .update(cx, |input, cx| input.set_text("123456", cx));
        app.dir_input
            .update(cx, |input, cx| input.set_text("/offline/project", cx));
        cx.notify();
    });
    let input = view.read_with(cx, |app, _| match surface {
        Surface::Email => Some(app.email.clone()),
        Surface::Code => Some(app.code.clone()),
        Surface::Keyboard => Some(app.composer.clone()),
        Surface::Directory => Some(app.dir_input.clone()),
        _ => None,
    });
    cx.update(|window, cx| {
        window.blur(cx);
        if let Some(input) = input {
            window.focus(&input.focus_handle(cx), cx);
        }
        window.draw(cx).clear(cx);
    });
    draw(cx);
}

fn assert_invariants(view: &Entity<MobileApp>, cx: &mut VisualTestContext) {
    view.read_with(cx, |app, cx| {
        assert!(
            app.connection.is_none(),
            "offline fixture created a connection"
        );
        assert!(
            app.browser_login.is_none(),
            "offline fixture started browser login"
        );
        assert!(
            (0.0..=1.0).contains(&app.drawer_t),
            "drawer progress {}",
            app.drawer_t
        );
        assert!(app.drawer_drag.is_finite() && app.drawer_drag <= 0.);
        assert!(app.kb_lift.is_finite() && app.kb_lift >= 0.);
        let (width, height) = app.device.unwrap();
        let (top, left, bottom) = app.feel_sheet_offsets();
        for offset in [top, left, bottom, app.swipe.dx] {
            assert!(offset.is_finite(), "nonfinite gesture offset");
            assert!(
                offset.abs() <= width.max(height) * 16.,
                "runaway gesture offset {offset}"
            );
        }
        assert!((0.0..=0.5).contains(&app.feel_backdrop()));
        let offset = app.transcript_scroll.offset();
        assert!(f32::from(offset.x).is_finite() && f32::from(offset.y).is_finite());
        assert!(
            app.tab_order.contains(&app.active_tab),
            "active tab unreachable"
        );
        let mut ids = app.tab_order.clone();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), app.tab_order.len(), "duplicate tab identity");
        // selected_text slices the editor's actual byte range, checking ordering,
        // UTF-8 boundaries and bounds without exposing private TextInput fields.
        for input in [
            &app.email,
            &app.code,
            &app.composer,
            &app.dir_input,
            &app.drawer_search,
            &app.transfer_input,
        ] {
            let input = input.read(cx);
            assert!(input.text().contains(input.selected_text()));
        }
    });
}

fn reachable(
    cx: &mut VisualTestContext,
    selector: &'static str,
    (width, height): (f32, f32),
) -> Bounds<Pixels> {
    let bounds = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("missing control {selector}"));
    assert!(
        bounds.size.width >= px(44.) && bounds.size.height >= px(44.),
        "undersized {selector}: {bounds:?}"
    );
    assert!(
        bounds.left() >= px(-0.5)
            && bounds.right() <= px(width + 0.5)
            && bounds.top() >= px(-0.5)
            && bounds.bottom() <= px(height + 0.5),
        "unreachable {selector}: {bounds:?}"
    );
    bounds
}

#[gpui::test]
fn deterministic_raw_touch_campaigns(cx: &mut TestAppContext) {
    cx.update(init);
    let seed = std::env::var("JCODE_MOBILE_FUZZ_SEED")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0x544f554348);
    let seeds = bounded_env("JCODE_MOBILE_FUZZ_SEEDS", 2, 8);
    let steps = bounded_env("JCODE_MOBILE_FUZZ_STEPS", 48, 512);
    for campaign in 0..seeds {
        let seed = seed.wrapping_add(campaign as u64);
        let (view, cx) = cx.add_window_view(|window, cx| {
            let mut app = MobileApp::new_offline(
                SafeArea {
                    top: 20.,
                    bottom: 0.,
                },
                window,
                cx,
            );
            app.synced = synced_sessions::State::fixture();
            app.composer
                .update(cx, |input, cx| input.set_text(DRAFT, cx));
            app
        });
        eprintln!("raw-touch campaign start seed={seed} steps={steps} surfaces=8 sizes={SIZES:?}");
        let mut random = Random(seed);
        let mut contacts = 0;
        let mut events = 0;
        for step in 0..steps {
            // Every default campaign visits all surfaces and all device sizes.
            let surface = SURFACES[(step / 6) % SURFACES.len()];
            let dimensions = SIZES[(step / 3 + campaign) % SIZES.len()];
            replay(seed, step, surface, dimensions, || {
                if step % 6 == 0 {
                    install_surface(&view, cx, surface);
                }
                if step % 3 == 0 {
                    resize(&view, cx, dimensions);
                }
                let (width, height) = dimensions;
                let start = random.position(width, height);
                let end = point(
                    px((random.fraction() * 3. - 1.) * width),
                    px((random.fraction() * 3. - 1.) * height),
                );
                let id = 10 + step as u64 * 3;
                match step % 6 {
                    0 => tap(cx, id, start),
                    1 | 2 => {
                        touch(cx, id, TouchPhase::Started, start);
                        for fraction in [0.25, 0.5, 1.] {
                            touch(cx, id, TouchPhase::Moved, start + (end - start) * fraction);
                        }
                        touch(
                            cx,
                            id,
                            if step % 6 == 1 {
                                TouchPhase::Ended
                            } else {
                                TouchPhase::Cancelled
                            },
                            end,
                        );
                    }
                    3 => {
                        // Resize while a captured contact is active.
                        touch(cx, id, TouchPhase::Started, start);
                        resize(&view, cx, SIZES[(step / 3 + campaign + 1) % SIZES.len()]);
                        touch(cx, id, TouchPhase::Moved, end);
                        touch(cx, id, TouchPhase::Cancelled, end);
                        resize(&view, cx, dimensions);
                    }
                    4 => {
                        touch(cx, id, TouchPhase::Started, start);
                        touch(cx, id + 1, TouchPhase::Started, end);
                        touch(cx, id, TouchPhase::Moved, end);
                        touch(cx, id + 1, TouchPhase::Moved, start);
                        touch(cx, id, TouchPhase::Cancelled, end);
                        touch(cx, id + 1, TouchPhase::Ended, start);
                    }
                    _ => {
                        for n in 0..3 {
                            tap(cx, id + n, start);
                        }
                    }
                }
                draw(cx);
                assert_invariants(&view, cx);
            });
            contacts += [1, 1, 1, 1, 2, 3][step % 6];
            events += [2, 5, 5, 3, 6, 6][step % 6];
        }
        eprintln!(
            "raw-touch campaign complete seed={seed} cases={steps} contacts={contacts} events={events}"
        );
    }
}

#[gpui::test]
fn raw_touch_offline_draft_and_overlay_recovery(cx: &mut TestAppContext) {
    cx.update(init);
    for dimensions in SIZES {
        let (view, cx) = cx.add_window_view(|window, cx| {
            MobileApp::new_offline(
                SafeArea {
                    top: 20.,
                    bottom: 0.,
                },
                window,
                cx,
            )
        });
        replay(0x5245434f564552, 0, Surface::Chat, dimensions, || {
            install_surface(&view, cx, Surface::Chat);
            resize(&view, cx, dimensions);
            view.update(cx, |app, cx| {
                app.composer
                    .update(cx, |input, cx| input.set_text(DRAFT, cx));
                cx.notify();
            });
            draw(cx);
            let before = view.read_with(cx, |app, _| app.session.transcript.len());
            let send = reachable(cx, "send", dimensions);
            tap(cx, 1, send.center());
            view.read_with(cx, |app, cx| {
                assert_eq!(
                    app.composer.read(cx).text(),
                    DRAFT,
                    "offline touch-send lost draft"
                );
                assert_eq!(
                    app.session.transcript.len(),
                    before,
                    "offline touch-send fabricated history"
                );
            });
            // A cancelled button contact must not activate the drawer.
            let menu = reachable(cx, "menu", dimensions);
            touch(cx, 2, TouchPhase::Started, menu.center());
            touch(cx, 2, TouchPhase::Cancelled, menu.center());
            view.read_with(cx, |app, _| {
                assert!(!app.drawer_open, "cancel activated menu")
            });
            tap(cx, 3, menu.center());
            view.read_with(cx, |app, _| {
                assert!(app.drawer_open, "raw menu tap did not open drawer")
            });
            // Settle only the presentation clock. Do not alter open state or hit testing.
            view.update(cx, |app, cx| {
                app.drawer_t = 1.;
                cx.notify();
            });
            draw(cx);
            let (width, height) = dimensions;
            tap(cx, 4, point(px(width - 2.), px(height * 0.5)));
            view.read_with(cx, |app, _| {
                assert!(!app.drawer_open, "scrim did not dismiss drawer")
            });
            view.update(cx, |app, cx| {
                app.drawer_t = 0.;
                cx.notify();
            });
            draw(cx);
            reachable(cx, "menu", dimensions);
            reachable(cx, "new-session", dimensions);
            reachable(cx, "send", dimensions);
            assert_invariants(&view, cx);
        });
    }
}

#[gpui::test]
fn raw_touch_onboarding_outside_taps_release_focus(cx: &mut TestAppContext) {
    cx.update(init);
    for (step, surface) in [Surface::Email, Surface::Code].into_iter().enumerate() {
        let dimensions = (390., 844.);
        let (view, cx) = cx.add_window_view(|window, cx| {
            MobileApp::new_offline(
                SafeArea {
                    top: 47.,
                    bottom: 34.,
                },
                window,
                cx,
            )
        });
        replay(0x464f435553, step, surface, dimensions, || {
            resize(&view, cx, dimensions);
            install_surface(&view, cx, surface);
            let input = view.read_with(cx, |app, _| {
                if matches!(surface, Surface::Code) {
                    app.code.clone()
                } else {
                    app.email.clone()
                }
            });
            cx.update(|window, cx| {
                assert!(input.read(cx).is_focused(window), "fixture not focused")
            });
            tap(cx, 1, point(px(10.), px(70.)));
            cx.update(|window, cx| {
                assert!(
                    !input.read(cx).is_focused(window),
                    "outside raw tap retained focus"
                )
            });
            assert_invariants(&view, cx);
        });
    }
    eprintln!("raw-touch focus complete cases=2 contacts=2 events=4");
}

fn wheel(cx: &mut VisualTestContext, phase: TouchPhase, at: Point<Pixels>, dx: f32) {
    cx.update(|window, cx| {
        window.dispatch_event(
            PlatformInput::ScrollWheel(gpui::ScrollWheelEvent {
                position: at,
                delta: gpui::ScrollDelta::Pixels(point(px(dx), px(0.))),
                modifiers: Default::default(),
                touch_phase: phase,
            }),
            cx,
        );
        window.draw(cx).clear(cx);
    });
}

#[gpui::test]
fn cancelled_pan_does_not_commit_drawer_or_tab_navigation(cx: &mut TestAppContext) {
    cx.update(init);
    let dimensions = (390., 844.);
    let (view, cx) = cx.add_window_view(|window, cx| {
        MobileApp::new_offline(
            SafeArea {
                top: 20.,
                bottom: 0.,
            },
            window,
            cx,
        )
    });
    replay(0x43414e43454c, 0, Surface::Drawer, dimensions, || {
        resize(&view, cx, dimensions);
        install_surface(&view, cx, Surface::Drawer);
        // The drawer's blank top padding avoids scrollable descendants.
        let at = point(px(100.), px(10.));
        wheel(cx, TouchPhase::Started, at, 0.);
        wheel(cx, TouchPhase::Moved, at, -220.);
        view.read_with(cx, |app, _| {
            assert!(app.drawer_drag < -100., "drawer pan fixture did not engage")
        });
        wheel(cx, TouchPhase::Cancelled, at, 0.);
        view.read_with(cx, |app, _| {
            assert!(app.drawer_open, "cancelled pan dismissed drawer");
            assert!(
                app.drawer_drag_last.is_none(),
                "cancelled drawer can still idle-commit"
            );
            assert_eq!(app.drawer_drag, 0., "cancelled drawer retained travel");
        });
    });
    replay(0x43414e43454c, 1, Surface::Chat, dimensions, || {
        install_surface(&view, cx, Surface::Chat);
        view.update(cx, |app, cx| {
            app.composer
                .update(cx, |input, cx| input.set_text("First offline tab", cx));
            app.new_tab(cx);
            app.composer
                .update(cx, |input, cx| input.set_text(DRAFT, cx));
            app.session.phase = ConnectionPhase::Connecting;
            cx.notify();
        });
        draw(cx);
        let active = view.read_with(cx, |app, _| {
            assert_eq!(app.tab_order.len(), 2, "tab fixture not created");
            app.active_tab
        });
        let at = point(px(190.), px(270.));
        wheel(cx, TouchPhase::Started, at, 0.);
        wheel(cx, TouchPhase::Moved, at, 200.);
        view.read_with(cx, |app, _| {
            assert!(app.swipe.active, "tab pan fixture did not engage")
        });
        wheel(cx, TouchPhase::Cancelled, at, 0.);
        view.read_with(cx, |app, cx| {
            assert_eq!(app.active_tab, active, "cancelled swipe switched tabs");
            assert!(
                !app.swipe.active && app.swipe.last_event.is_none(),
                "cancelled tab swipe can idle-commit"
            );
            assert_eq!(
                app.composer.read(cx).text(),
                DRAFT,
                "cancelled tab swipe changed draft"
            );
        });
        assert_invariants(&view, cx);
    });
    eprintln!("cancelled-pan regressions complete cases=2 wheel_events=6");
}

#[gpui::test]
fn auxiliary_fields_show_keyboard_and_receive_typing(cx: &mut TestAppContext) {
    // This regression describes automatic keyboard presentation, not the
    // developer's explicit force-on/force-off preview override.
    if std::env::var_os("JCODE_MOBILE_KEYBOARD").is_some() {
        eprintln!("auxiliary keyboard regression skipped: explicit keyboard override");
        return;
    }
    cx.update(init);
    for (step, surface) in [Surface::Drawer, Surface::Directory, Surface::Drawer]
        .into_iter()
        .enumerate()
    {
        let dimensions = (390., 844.);
        let (view, cx) = cx.add_window_view(|window, cx| {
            MobileApp::new_offline(
                SafeArea {
                    top: 20.,
                    bottom: 0.,
                },
                window,
                cx,
            )
        });
        replay(0x4b4559424f415244, step, surface, dimensions, || {
            resize(&view, cx, dimensions);
            install_surface(&view, cx, surface);
            let input = view.read_with(cx, |app, _| match step {
                0 => app.drawer_search.clone(),
                1 => app.dir_input.clone(),
                _ => app.transfer_input.clone(),
            });
            view.update(cx, |app, cx| {
                app.composer
                    .update(cx, |input, cx| input.set_text(DRAFT, cx));
                input.update(cx, |input, cx| input.set_text("", cx));
                cx.notify();
            });
            cx.update(|window, cx| {
                window.focus(&input.focus_handle(cx), cx);
                window.draw(cx).clear(cx);
            });
            view.read_with(cx, |app, _| {
                assert!(app.keyboard_up, "auxiliary field did not show keyboard")
            });
            cx.update(|window, cx| {
                view.update(cx, |app, cx| {
                    app.sim_key(crate::sim_keyboard::Key::Char("x".into()), window, cx)
                });
                window.draw(cx).clear(cx);
            });
            view.read_with(cx, |app, cx| {
                assert_eq!(
                    input.read(cx).text(),
                    "x",
                    "keyboard typing missed focused auxiliary field"
                );
                assert_eq!(
                    app.composer.read(cx).text(),
                    DRAFT,
                    "auxiliary typing changed composer"
                );
            });
            assert_invariants(&view, cx);
        });
    }
    eprintln!("auxiliary keyboard regression complete cases=3 keypresses=3");
}
