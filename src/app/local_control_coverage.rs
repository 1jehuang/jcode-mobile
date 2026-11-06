//! Local rendered controls only. Synthetic voice flags never acquire a microphone.
//! Sign-in taps prove the offline early return, not email/code validation or authentication.
use super::account_coverage::{draw, tap};
use super::*;
use gpui::TestAppContext;

const DRAFT: &str = "Keep draft 👩🏽‍💻 e\u{301}";

#[gpui::test]
fn cancel_dictation_touch_clears_synthetic_busy_start_and_recovers(cx: &mut TestAppContext) {
    let (view, cx) = super::navigation_coverage::mount(cx);
    for width in [320., 390.] {
        cx.simulate_resize(gpui::size(px(width), px(844.)));
        view.update(cx, |app, cx| {
            app.device = Some((width, 844.));
            app.composer
                .update(cx, |input, cx| input.set_text(DRAFT, cx));
            cx.notify();
        });
        for starting in [true, false, true] {
            view.update(cx, |app, cx| {
                // No Recording, pending task, or native capture object exists.
                assert!(app.voice.recording.is_none());
                app.voice.busy = true;
                app.voice.starting = starting;
                app.voice.locked = true;
                app.voice.auto_send = true;
                app.voice.began = Some(std::time::Instant::now());
                cx.notify();
            });
            draw(cx);
            assert!(cx.debug_bounds("cancel-dictation").is_some());
            tap(cx, "cancel-dictation");
            draw(cx);
            assert!(cx.debug_bounds("cancel-dictation").is_none());
            view.read_with(cx, |app, cx| {
                assert!(
                    !app.voice.busy
                        && !app.voice.starting
                        && !app.voice.locked
                        && !app.voice.auto_send
                );
                assert!(app.voice.recording.is_none() && app.voice.began.is_none());
                assert!(app.voice.status().is_none());
                assert_eq!(app.composer.read(cx).text(), DRAFT);
                assert!(app.session.transcript.is_empty() && app.connection.is_none());
                assert_eq!(app.active_tab, 0);
                assert_eq!(app.tab_order, [0]);
                assert!(!app.model_picker && !app.dir_picker && !app.drawer_open);
            });
        }
    }
}

#[gpui::test]
fn sign_in_submit_touch_preserves_email_code_and_busy_at_offline_boundary(cx: &mut TestAppContext) {
    let (view, cx) = super::account_coverage::mount(cx);
    for width in [320., 390.] {
        cx.simulate_resize(gpui::size(px(width), px(844.)));
        for (code_step, busy, email, code) in [
            (false, false, "", ""),
            (false, false, "not-an-email", ""),
            (false, false, "fixture@example.test", ""),
            (false, true, "fixture@example.test", ""),
            (true, false, "fixture@example.test", "12"),
            (true, false, "fixture@example.test", "123456"),
            (true, true, "fixture@example.test", "123456"),
        ] {
            let generation = view.update(cx, |app, cx| {
                assert!(app.offline && app.account.is_none());
                app.device = Some((width, 844.));
                app.screen = Screen::SignIn;
                // Pin the keyboard presentation to a settled, reachable form.
                app.kb_lift = 220.;
                app.login_token = code_step.then(|| "offline-control-token".into());
                app.signing_in = busy;
                app.sign_in_error = Some("Preserved fixture error".into());
                app.email.update(cx, |input, cx| input.set_text(email, cx));
                app.code.update(cx, |input, cx| input.set_text(code, cx));
                app.composer
                    .update(cx, |input, cx| input.set_text(DRAFT, cx));
                cx.notify();
                app.auth_generation
            });
            draw(cx);
            assert_eq!(
                cx.debug_bounds("use-other-email").is_some(),
                code_step && !busy
            );
            for _ in 0..2 {
                // The production handler returns before validation or service
                // startup in offline fixtures, even for ready-looking inputs.
                tap(cx, "submit-sign-in");
                view.read_with(cx, |app, cx| {
                    assert!(app.screen == Screen::SignIn);
                    assert_eq!(app.auth_generation, generation);
                    assert_eq!(app.signing_in, busy);
                    assert_eq!(app.email.read(cx).text(), email);
                    assert_eq!(app.code.read(cx).text(), code);
                    assert_eq!(
                        app.login_token.as_deref(),
                        code_step.then_some("offline-control-token")
                    );
                    assert_eq!(
                        app.sign_in_error.as_deref(),
                        Some("Preserved fixture error")
                    );
                    assert!(
                        app.account.is_none()
                            && app.connection.is_none()
                            && app.browser_login.is_none()
                    );
                    assert_eq!(app.composer.read(cx).text(), DRAFT);
                    assert!(app.session.transcript.is_empty());
                });
            }
            if code_step && !busy {
                tap(cx, "use-other-email");
                view.read_with(cx, |app, cx| {
                    assert!(app.login_token.is_none() && app.sign_in_error.is_none());
                    assert_eq!(app.code.read(cx).text(), "");
                    assert_eq!(app.email.read(cx).text(), email);
                    assert_eq!(app.composer.read(cx).text(), DRAFT);
                    assert!(app.account.is_none() && app.connection.is_none());
                });
            }
        }
    }
}
