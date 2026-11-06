//! Sign in with Apple: the HIG-style button on the sign-in screen and the
//! Settings "Link existing Jcode account" flow.
//!
//! On iOS the button opens the system sheet (`gpui_platform::ios`), then trades
//! Apple's identity token for an account key (`jcode_mobile_core::apple`). The
//! desktop preview shows the same button; tapping it is mocked (a banner, or a
//! real exchange when `JCODE_MOBILE_APPLE_TOKEN` + `JCODE_MOBILE_APPLE_NONCE`
//! are set for testing against a dev worker).
//!
//! Linking never blocks the app: an Apple account works on its own, and the
//! Settings row lets the user merge it into an existing email account so a
//! subscription there carries over.

use gpui::{
    AnyElement, Context, Entity, EventEmitter, Focusable as _, Window, div, prelude::*, px,
};
use jcode_mobile_core::{Account, apple};

use super::{MobileApp, SignInOutcome, banner, label, outline_button};
use crate::haptics::{self, Haptic};
use crate::text_input::{TextInput, TextInputEvent};
use crate::theme::Theme;

fn apple_logo() -> std::sync::Arc<gpui::Image> {
    static LOGO: std::sync::OnceLock<std::sync::Arc<gpui::Image>> = std::sync::OnceLock::new();
    LOGO.get_or_init(|| {
        std::sync::Arc::new(gpui::Image::from_bytes(
            gpui::ImageFormat::Png,
            include_bytes!("../../assets/brand/apple-logo.png").to_vec(),
        ))
    })
    .clone()
}

/// Accounts created by Sign in with Apple carry a relay or placeholder email
/// until linked, so that is when the Settings link row is offered.
pub(super) fn is_unlinked_apple_account(account: &Account) -> bool {
    match account.email.as_deref() {
        None => true,
        Some(e) => e.ends_with("@privaterelay.appleid.com") || e.ends_with("@apple.invalid"),
    }
}

/// Runs the platform sheet. Resolves to (identity_token, raw_nonce).
async fn apple_credential() -> Result<(String, String), String> {
    #[cfg(target_os = "ios")]
    {
        let nonce = apple::new_nonce();
        let (tx, rx) = futures::channel::oneshot::channel();
        gpui_platform::ios::sign_in_with_apple(&apple::sha256_hex(&nonce), move |r| {
            let _ = tx.send(r);
        });
        match rx.await {
            Ok(Ok(cred)) => Ok((cred.identity_token, nonce)),
            Ok(Err(gpui_platform::ios::AppleSignInError::Canceled)) | Err(_) => Err(String::new()),
            Ok(Err(e)) => Err(e.to_string()),
        }
    }
    #[cfg(not(target_os = "ios"))]
    {
        match (
            std::env::var("JCODE_MOBILE_APPLE_TOKEN"),
            std::env::var("JCODE_MOBILE_APPLE_NONCE"),
        ) {
            (Ok(token), Ok(nonce)) => Ok((token, nonce)),
            _ => Err("Preview: Sign in with Apple opens the system sheet on iPhone.".into()),
        }
    }
}

impl MobileApp {
    /// White Apple button (HIG: white style on dark backgrounds).
    pub(super) fn render_apple_button(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let busy = self.signing_in;
        div()
            .w_full()
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(
                div()
                    .id("sign-in-with-apple")
                    .debug_selector(|| "sign-in-with-apple".into())
                    .w_full()
                    .h(px(44.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap(px(6.))
                    .rounded(px(10.))
                    .bg(gpui::white())
                    .text_color(gpui::black())
                    .text_size(crate::system::fs(17.))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .cursor_pointer()
                    .active(|s| s.opacity(0.85))
                    .when(busy, |d| d.opacity(0.6))
                    .child(gpui::img(apple_logo()).size(px(17.)).mb(px(2.)).flex_none())
                    .child("Sign in with Apple")
                    .on_click(cx.listener(|this, _, _, cx| this.apple_sign_in(cx))),
            )
            .into_any_element()
    }

    fn apple_sign_in(&mut self, cx: &mut Context<Self>) {
        if self.offline {
            return;
        }
        if self.signing_in {
            return;
        }
        let generation = self.begin_auth();
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = match apple_credential().await {
                Ok((token, nonce)) => {
                    cx.background_spawn(async move { apple::sign_in(&token, &nonce) })
                        .await
                }
                Err(e) => Err(e),
            };
            this.update(cx, |this, cx| {
                this.complete_sign_in(generation, result.map(SignInOutcome::Authenticated), cx);
            })
            .ok();
        })
        .detach();
    }

    /// Settings row: "Link existing Jcode account" for Apple accounts.
    pub(super) fn render_apple_link(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let account = self.account.clone()?;
        if self.apple_link.as_ref().is_some_and(|panel| {
            let source = &panel.read(cx).account;
            source.api_key != account.api_key || source.api_base != account.api_base
        }) {
            self.apple_link = None;
        }
        if self.apple_link.is_none() && !is_unlinked_apple_account(&account) {
            return None;
        }
        let offline = self.offline;
        let auth_generation = self.auth_generation;
        let panel = self.apple_link.get_or_insert_with(|| {
            let panel = cx.new(|cx| {
                let mut panel = AppleLinkPanel::new(account, cx);
                panel.offline |= offline;
                panel
            });
            cx.subscribe(&panel, move |this, _, ev: &LinkedEvent, cx| {
                if this.auth_generation != auth_generation {
                    return;
                }
                if let Some(a) = this.account.as_mut() {
                    if a.api_key != ev.api_key || a.api_base != ev.api_base {
                        return;
                    }
                    a.email = ev.email.clone();
                    if !this.offline {
                        let _ = this.store.save(a);
                    }
                }
                cx.notify();
            })
            .detach();
            panel
        });
        Some(panel.clone().into_any_element())
    }
}

pub struct LinkedEvent {
    api_key: String,
    api_base: String,
    email: Option<String>,
}

enum Step {
    Closed,
    Email,
    Code { link_token: String },
    Done { message: String },
}

enum LinkResult {
    Sent(String),
    Linked(apple::Linked),
}

enum LinkRequest {
    Send { email: String },
    Verify { token: String, code: String },
}

pub struct AppleLinkPanel {
    account: Account,
    email: Entity<TextInput>,
    code: Entity<TextInput>,
    step: Step,
    busy: bool,
    error: Option<String>,
    focus_code: bool,
    offline: bool,
    generation: u64,
    pending: Option<(u64, bool)>,
}

impl EventEmitter<LinkedEvent> for AppleLinkPanel {}

impl AppleLinkPanel {
    pub(super) fn focused_input(
        &self,
        window: &Window,
        cx: &gpui::App,
    ) -> Option<Entity<TextInput>> {
        [&self.email, &self.code]
            .into_iter()
            .find(|input| input.read(cx).is_focused(window))
            .cloned()
    }

    fn new(account: Account, cx: &mut Context<Self>) -> Self {
        use gpui::{TextInputAction, TextInputConfiguration, TextInputPurpose};
        let email = cx.new(|cx| {
            let mut t = TextInput::new("Existing account email", cx);
            t.config = TextInputConfiguration {
                input_action: TextInputAction::Next,
                purpose: TextInputPurpose::Email,
                ..Default::default()
            };
            t
        });
        let code = cx.new(|cx| {
            let mut t = TextInput::new("6-digit code", cx);
            t.config = TextInputConfiguration {
                input_action: TextInputAction::Go,
                purpose: TextInputPurpose::OneTimeCode,
                ..Default::default()
            };
            t
        });
        for input in [&email, &code] {
            cx.subscribe(input, |this, _, ev, cx| match ev {
                TextInputEvent::Submit => this.submit(cx),
                TextInputEvent::Changed => cx.notify(),
            })
            .detach();
        }
        Self {
            account,
            email,
            code,
            step: Step::Closed,
            busy: false,
            error: None,
            focus_code: false,
            offline: cfg!(test),
            generation: 0,
            pending: None,
        }
    }

    fn submit(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let account = self.account.clone();
        let email = self.email.read(cx).text().trim().to_string();
        let code = self.code.read(cx).text().trim().to_string();
        let request = match &self.step {
            Step::Email if email.contains('@') => LinkRequest::Send { email },
            Step::Code { link_token } if code.len() >= 6 => LinkRequest::Verify {
                token: link_token.clone(),
                code,
            },
            _ => return,
        };
        self.generation += 1;
        let generation = self.generation;
        self.pending = Some((generation, matches!(&request, LinkRequest::Verify { .. })));
        self.busy = true;
        haptics::play(Haptic::Light);
        self.error = None;
        cx.notify();
        // Tests exercise real controls but supply completions explicitly. No
        // platform sheet, HTTP task, or persistence is permitted in this mode.
        if self.offline {
            return;
        }
        let task = cx.background_spawn(async move {
            match request {
                LinkRequest::Send { email } => {
                    apple::start_link(&account, &email).map(LinkResult::Sent)
                }
                LinkRequest::Verify { token, code } => {
                    apple::verify_link(&account, &token, &code).map(LinkResult::Linked)
                }
            }
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                this.complete(generation, result, cx);
            })
            .ok();
        })
        .detach();
    }

    fn cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.generation += 1;
        self.pending = None;
        self.busy = false;
        self.error = None;
        self.focus_code = false;
        self.step = Step::Closed;
        self.code.update(cx, |t, cx| t.set_text("", cx));
        if self.focused_input(window, cx).is_some() {
            window.blur(cx);
        }
        cx.notify();
    }

    fn complete(
        &mut self,
        generation: u64,
        result: Result<LinkResult, String>,
        cx: &mut Context<Self>,
    ) {
        let Some((active, verifying)) = self.pending else {
            return;
        };
        if generation != active || generation != self.generation || !self.busy {
            return;
        }
        // A result for a different phase must not mutate or finish this request.
        if matches!(&result, Ok(LinkResult::Linked(_))) && !verifying
            || matches!(&result, Ok(LinkResult::Sent(_))) && verifying
        {
            return;
        }
        self.pending = None;
        self.busy = false;
        match result {
            Ok(LinkResult::Sent(link_token)) => {
                haptics::play(Haptic::Success);
                self.step = Step::Code { link_token };
                self.focus_code = true;
            }
            Ok(LinkResult::Linked(linked)) => {
                haptics::play(Haptic::Success);
                let who = linked
                    .email
                    .clone()
                    .unwrap_or_else(|| "your Jcode account".into());
                self.step = Step::Done {
                    message: if linked.merged {
                        format!("Linked to {who}. Your plan carries over.")
                    } else {
                        format!("Linked {who} to this account.")
                    },
                };
                self.code.update(cx, |t, cx| t.set_text("", cx));
                cx.emit(LinkedEvent {
                    api_key: self.account.api_key.clone(),
                    api_base: self.account.api_base.clone(),
                    email: linked.email,
                });
            }
            Err(e) => {
                haptics::play(Haptic::Error);
                self.error = Some(e);
            }
        }
        cx.notify();
    }
}

impl Render for AppleLinkPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::get(cx);
        if std::mem::take(&mut self.focus_code) {
            _window.focus(&self.code.focus_handle(cx), cx);
        }
        let col = div().w_full().flex().flex_col().gap(px(10.));
        match &self.step {
            Step::Closed => col.child(
                outline_button("link-account", "Link existing Jcode account", true, &t).on_click(
                    cx.listener(|this, _, window, cx| {
                        this.step = Step::Email;
                        haptics::play(Haptic::Light);
                        window.focus(&this.email.focus_handle(cx), cx);
                        cx.notify()
                    }),
                ),
            ),
            Step::Done { message } => col.child(label(message.clone(), t.text_secondary, 12.)),
            step => {
                let code_step = matches!(step, Step::Code { .. });
                let hint = match (code_step, self.busy) {
                    (_, true) => "Working…".to_string(),
                    (false, false) => {
                        "Already have a Jcode account? Enter its email and we'll send a code. Your subscription moves to this sign-in.".into()
                    }
                    (true, false) => format!("We emailed a code to {}.", self.email.read(cx).text().trim()),
                };
                col.child(label("Link existing Jcode account", t.text_strong, 13.))
                    .when_some(self.error.clone(), |d, e| d.child(banner(&e, t.error, &t)))
                    .child(if code_step {
                        self.code.clone()
                    } else {
                        self.email.clone()
                    })
                    .child(label(hint, t.text_muted, 12.))
                    .child(
                        div()
                            .flex()
                            .gap(px(10.))
                            .child(
                                outline_button(
                                    "link-submit",
                                    if code_step { "Link" } else { "Send code" },
                                    !self.busy,
                                    &t,
                                )
                                .flex_1()
                                .on_click(cx.listener(|this, _, _, cx| this.submit(cx))),
                            )
                            .child(
                                outline_button("link-cancel", "Not now", true, &t)
                                    .flex_1()
                                    .on_click(
                                        cx.listener(|this, _, window, cx| this.cancel(window, cx)),
                                    ),
                            ),
                    )
            }
        }
    }
}

#[cfg(test)]
mod haptic_tests {
    use super::*;

    fn account() -> Account {
        Account {
            api_key: "apple-fixture".into(),
            api_base: "https://offline.invalid".into(),
            email: None,
        }
    }

    fn tap(cx: &mut gpui::VisualTestContext, selector: &str) {
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let at = cx
            .debug_bounds(selector)
            .expect("rendered Apple control")
            .center();
        for phase in [gpui::TouchPhase::Started, gpui::TouchPhase::Ended] {
            cx.update(|window, cx| {
                window.dispatch_event(
                    gpui::PlatformInput::Touch(gpui::TouchEvent {
                        id: gpui::TouchId(81),
                        phase,
                        position: at,
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
    fn rendered_link_flow_error_retry_busy_code_done(cx: &mut gpui::TestAppContext) {
        cx.update(super::super::init);
        let (panel, cx) = cx.add_window_view(|_, cx| AppleLinkPanel::new(account(), cx));
        tap(cx, "link-account");
        panel.update(cx, |p, cx| {
            assert!(matches!(p.step, Step::Email));
            assert!(p.offline);
            p.email
                .update(cx, |t, cx| t.set_text(" existing@example.test ", cx));
        });
        tap(cx, "link-submit");
        let first = panel.read_with(cx, |p, _| {
            assert!(p.busy);
            p.generation
        });
        tap(cx, "link-submit");
        panel.update(cx, |p, cx| {
            assert_eq!(
                p.generation, first,
                "busy control cannot duplicate a request"
            );
            p.complete(first, Err("Fixture send failure".into()), cx);
            assert_eq!(p.error.as_deref(), Some("Fixture send failure"));
            assert!(!p.busy);
        });
        tap(cx, "link-submit");
        panel.update(cx, |p, cx| {
            assert!(p.error.is_none());
            p.complete(first, Ok(LinkResult::Sent("stale".into())), cx);
            assert!(matches!(p.step, Step::Email));
            p.complete(
                p.generation,
                Ok(LinkResult::Sent("fixture-token".into())),
                cx,
            );
        });
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
            let p = panel.read(cx);
            assert!(p.code.read(cx).is_focused(window));
        });
        panel.update(cx, |p, cx| {
            p.code.update(cx, |t, cx| t.set_text("123456", cx));
        });
        tap(cx, "link-submit");
        panel.update(cx, |p, cx| {
            p.complete(p.generation, Err("Fixture bad code".into()), cx);
            assert!(matches!(p.step, Step::Code { .. }));
        });
        tap(cx, "link-submit");
        panel.update(cx, |p, cx| {
            let generation = p.generation;
            p.complete(generation, Ok(LinkResult::Linked(apple::Linked {
                email: Some("existing@example.test".into()), merged: true,
            })), cx);
            assert!(matches!(&p.step, Step::Done { message } if message.contains("Your plan carries over")));
            assert_eq!(p.code.read(cx).text(), "");
            p.complete(generation, Err("duplicate".into()), cx);
            assert!(p.error.is_none());
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert!(cx.debug_bounds("link-submit").is_none());
    }

    #[gpui::test]
    fn rendered_cancel_reopen_rejects_stale_send_and_verify(cx: &mut gpui::TestAppContext) {
        cx.update(super::super::init);
        let (panel, cx) = cx.add_window_view(|_, cx| AppleLinkPanel::new(account(), cx));
        tap(cx, "link-account");
        panel.update(cx, |p, cx| {
            p.email
                .update(cx, |t, cx| t.set_text("fixture@example.test", cx));
        });
        tap(cx, "link-submit");
        let old = panel.read_with(cx, |p, _| p.generation);
        tap(cx, "link-cancel");
        cx.update(|window, cx| {
            let p = panel.read(cx);
            assert!(matches!(p.step, Step::Closed));
            assert!(
                p.focused_input(window, cx).is_none(),
                "cancel dismisses input focus/keyboard"
            );
        });
        panel.update(cx, |p, cx| {
            p.complete(old, Ok(LinkResult::Sent("late".into())), cx);
            assert!(matches!(p.step, Step::Closed));
            assert!(!p.busy && !p.focus_code);
        });
        tap(cx, "link-account");
        tap(cx, "link-submit");
        panel.update(cx, |p, cx| {
            p.complete(old, Err("late failure".into()), cx);
            assert!(p.error.is_none() && p.busy);
            p.complete(p.generation, Ok(LinkResult::Sent("new-token".into())), cx);
            p.code.update(cx, |t, cx| t.set_text("654321", cx));
        });
        tap(cx, "link-submit");
        let old = panel.read_with(cx, |p, _| p.generation);
        tap(cx, "link-cancel");
        panel.update(cx, |p, cx| {
            p.complete(
                old,
                Ok(LinkResult::Linked(apple::Linked {
                    email: Some("wrong@example.test".into()),
                    merged: true,
                })),
                cx,
            );
            assert!(matches!(p.step, Step::Closed));
            assert_eq!(p.code.read(cx).text(), "");
        });
        tap(cx, "link-account");
        panel.read_with(cx, |p, _| assert!(matches!(p.step, Step::Email)));
    }

    #[gpui::test]
    fn offline_link_events_cannot_update_a_different_account(cx: &mut gpui::TestAppContext) {
        cx.update(super::super::init);
        let (app, cx) = cx.add_window_view(|window, cx| {
            MobileApp::new_offline(
                super::super::SafeArea {
                    top: 0.,
                    bottom: 0.,
                },
                window,
                cx,
            )
        });
        let panel = app.update(cx, |app, cx| {
            app.account = Some(account());
            assert!(
                app.render_apple_link(cx).is_some(),
                "synthetic unlinked account renders offline"
            );
            app.apple_link.clone().unwrap()
        });
        app.update(cx, |app, _| {
            app.account.as_mut().unwrap().api_key = "other-account".into();
        });
        panel.update(cx, |_, cx| {
            cx.emit(LinkedEvent {
                api_key: account().api_key,
                api_base: account().api_base,
                email: Some("wrong@example.test".into()),
            })
        });
        cx.run_until_parked();
        app.update(cx, |app, cx| {
            assert!(app.account.as_ref().unwrap().email.is_none());
            assert!(app.render_apple_link(cx).is_some());
            assert_ne!(
                app.apple_link.as_ref().unwrap().entity_id(),
                panel.entity_id()
            );
        });
        let replacement = app.read_with(cx, |app, _| app.apple_link.clone().unwrap());
        replacement.update(cx, |_, cx| {
            cx.emit(LinkedEvent {
                api_key: "other-account".into(),
                api_base: account().api_base,
                email: Some("right@example.test".into()),
            })
        });
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert_eq!(
                app.account.as_ref().unwrap().email.as_deref(),
                Some("right@example.test")
            )
        });
        app.update(cx, |app, _| app.invalidate_auth());
        replacement.update(cx, |_, cx| {
            cx.emit(LinkedEvent {
                api_key: "other-account".into(),
                api_base: account().api_base,
                email: Some("late-same-key@example.test".into()),
            })
        });
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert_eq!(
                app.account.as_ref().unwrap().email.as_deref(),
                Some("right@example.test")
            );
        });
    }

    #[gpui::test]
    fn invalid_or_busy_link_submissions_stay_silent(cx: &mut gpui::TestAppContext) {
        let panel = cx.new(|cx| {
            AppleLinkPanel::new(
                Account {
                    api_key: "fixture".into(),
                    api_base: "http://127.0.0.1:9".into(),
                    email: None,
                },
                cx,
            )
        });
        panel.update(cx, |panel, cx| {
            let (_, feedback) = haptics::capture(|| panel.submit(cx));
            assert!(feedback.is_empty(), "closed panel is a no-op");
            panel.step = Step::Email;
            let (_, feedback) = haptics::capture(|| panel.submit(cx));
            assert!(feedback.is_empty(), "invalid email is a no-op");
            panel.step = Step::Code {
                link_token: "fixture".into(),
            };
            let (_, feedback) = haptics::capture(|| panel.submit(cx));
            assert!(feedback.is_empty(), "incomplete code is a no-op");
            panel.busy = true;
            let (_, feedback) = haptics::capture(|| panel.submit(cx));
            assert!(feedback.is_empty(), "busy submission is a no-op");
        });
    }
}
