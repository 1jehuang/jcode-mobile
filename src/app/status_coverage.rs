//! Local connection presentation branches and bounded phone layout.
//! Service results are fixtures, not actual socket/provisioning acceptance.
use super::navigation_coverage::{draw, mount};
use super::*;
use gpui::TestAppContext;

#[gpui::test]
fn every_connection_phase_has_explicit_status_and_bounded_phone_presentation(
    cx: &mut TestAppContext,
) {
    let (view, cx) = mount(cx);
    for (width, height) in [(320., 568.), (390., 844.), (844., 390.)] {
        cx.simulate_resize(gpui::size(px(width), px(height)));
        for (phase, expected) in [
            (
                ConnectionPhase::Disconnected,
                Some("Disconnected".to_string()),
            ),
            (
                ConnectionPhase::Connecting,
                Some("Connecting to Jcode Cloud…".to_string()),
            ),
            (
                ConnectionPhase::Provisioning {
                    message: "Preparing fixture machine 世界".into(),
                },
                Some("Preparing fixture machine 世界".into()),
            ),
            (
                ConnectionPhase::Provisioning {
                    message: "Long provisioning status 世界 ".repeat(20),
                },
                Some("Long provisioning status 世界 ".repeat(20)),
            ),
            (
                ConnectionPhase::Reconnecting { attempt: 123 },
                Some("Reconnecting (123)…".into()),
            ),
            (ConnectionPhase::Connected, None),
            (
                ConnectionPhase::Failed {
                    reason: "Synthetic failure".into(),
                },
                None,
            ),
        ] {
            view.update(cx, |app, cx| {
                app.device = Some((width, height));
                app.session.phase = phase;
                app.session.transcript.clear();
                app.session.is_processing = false;
                app.session.error_banner = None;
                assert_eq!(app.connection_status_text(), expected);
                cx.notify();
            });
            draw(cx);
            let bounds = cx.debug_bounds("connection-status");
            if expected.is_some() {
                let bounds = bounds.expect("connecting/provisioning status must be rendered");
                assert!(
                    bounds.origin.x >= px(0.) && bounds.right() <= px(width),
                    "status exceeds phone width {width}: {bounds:?}"
                );
                assert!(
                    bounds.size.height > px(0.) && bounds.size.height < px(height),
                    "status consumes whole phone {width}x{height}: {bounds:?}"
                );
            } else {
                assert!(
                    bounds.is_none(),
                    "ready/error state must not retain stale connecting row"
                );
            }
        }
    }
}

#[gpui::test]
fn settings_actions_stay_inside_narrow_phone_width(cx: &mut TestAppContext) {
    let (view, cx) = mount(cx);
    for width in [320., 390.] {
        cx.simulate_resize(gpui::size(px(width), px(844.)));
        view.update(cx, |app, cx| {
            app.device = Some((width, 844.));
            app.screen = Screen::Settings;
            app.account.as_mut().unwrap().email = None;
            app.feel.g.edge_x = 0.;
            cx.notify();
        });
        draw(cx);
        // The first draw observes the screen change and starts the push.
        // Settle that seeded presentation only after recognition, as other
        // Settings fixtures do, before making static containment assertions.
        view.update(cx, |app, cx| {
            app.feel.g.edge_x = 0.;
            cx.notify();
        });
        draw(cx);
        for selector in ["new", "clear", "link-account", "sign-out", "delete-account"] {
            let bounds = cx
                .debug_bounds(selector)
                .unwrap_or_else(|| panic!("missing {selector}"));
            assert!(
                bounds.left() >= px(0.) && bounds.right() <= px(width),
                "{selector} overflows phone {width}: {bounds:?}"
            );
        }
    }
}
