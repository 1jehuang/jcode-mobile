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

use gpui::{AnyElement, Context, Entity, EventEmitter, Focusable as _, Window, div, prelude::*, px};
use jcode_mobile_core::{Account, apple};

use super::{MobileApp, banner, label, outline_button};
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
        match (std::env::var("JCODE_MOBILE_APPLE_TOKEN"), std::env::var("JCODE_MOBILE_APPLE_NONCE")) {
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
        if self.offline { return; }
        if self.signing_in {
            return;
        }
        self.signing_in = true;
        self.sign_in_error = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = match apple_credential().await {
                Ok((token, nonce)) => cx.background_spawn(async move { apple::sign_in(&token, &nonce) }).await,
                Err(e) => Err(e),
            };
            this.update(cx, |this, cx| {
                this.signing_in = false;
                match result {
                    Ok(account) => {
                        if let Err(e) = this.store.save(&account) {
                            this.sign_in_error = Some(format!("Could not save sign-in: {e}"));
                        }
                        this.connect(account, cx);
                    }
                    Err(e) if e.is_empty() => {} // user canceled the sheet
                    Err(e) => {
                        haptics::play(Haptic::Error);
                        this.sign_in_error = Some(e);
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Settings row: "Link existing Jcode account" for Apple accounts.
    pub(super) fn render_apple_link(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.offline { return None; }
        let account = self.account.clone()?;
        if self.apple_link.is_none() && !is_unlinked_apple_account(&account) {
            return None;
        }
        let panel = self.apple_link.get_or_insert_with(|| {
            let panel = cx.new(|cx| AppleLinkPanel::new(account, cx));
            cx.subscribe(&panel, |this, _, ev: &LinkedEvent, cx| {
                if let Some(a) = this.account.as_mut() {
                    a.email = ev.email.clone();
                    let _ = this.store.save(a);
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
    email: Option<String>,
}

enum Step {
    Closed,
    Email,
    Code { link_token: String },
    Done { message: String },
}

pub struct AppleLinkPanel {
    account: Account,
    email: Entity<TextInput>,
    code: Entity<TextInput>,
    step: Step,
    busy: bool,
    error: Option<String>,
    focus_code: bool,
}

impl EventEmitter<LinkedEvent> for AppleLinkPanel {}

impl AppleLinkPanel {
    pub(super) fn focused_input(&self, window: &Window, cx: &gpui::App) -> Option<Entity<TextInput>> {
        [&self.email, &self.code].into_iter()
            .find(|input| input.read(cx).is_focused(window)).cloned()
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
        Self { account, email, code, step: Step::Closed, busy: false, error: None, focus_code: false }
    }

    fn submit(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let account = self.account.clone();
        let email = self.email.read(cx).text().trim().to_string();
        let code = self.code.read(cx).text().trim().to_string();
        enum Out {
            Sent(String),
            Linked(apple::Linked),
        }
        let task = match &self.step {
            Step::Email if email.contains('@') => {
                cx.background_spawn(async move { apple::start_link(&account, &email).map(Out::Sent) })
            }
            Step::Code { link_token } if code.len() >= 6 => {
                let token = link_token.clone();
                cx.background_spawn(async move { apple::verify_link(&account, &token, &code).map(Out::Linked) })
            }
            _ => return,
        };
        self.busy = true;
        haptics::play(Haptic::Light);
        self.error = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                this.busy = false;
                match result {
                    Ok(Out::Sent(link_token)) => {
                        haptics::play(Haptic::Success);
                        this.step = Step::Code { link_token };
                        this.focus_code = true;
                    }
                    Ok(Out::Linked(linked)) => {
                        haptics::play(Haptic::Success);
                        let who = linked.email.clone().unwrap_or_else(|| "your Jcode account".into());
                        this.step = Step::Done {
                            message: if linked.merged {
                                format!("Linked to {who}. Your plan carries over.")
                            } else {
                                format!("Linked {who} to this account.")
                            },
                        };
                        this.code.update(cx, |t, cx| t.set_text("", cx));
                        cx.emit(LinkedEvent { email: linked.email });
                    }
                    Err(e) => {
                        haptics::play(Haptic::Error);
                        this.error = Some(e);
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
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
                outline_button("link-account", "Link existing Jcode account", true, &t).on_click(cx.listener(
                    |this, _, window, cx| {
                        this.step = Step::Email;
                        haptics::play(Haptic::Light);
                        window.focus(&this.email.focus_handle(cx), cx);
                        cx.notify()
                    },
                )),
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
                    .child(if code_step { self.code.clone() } else { self.email.clone() })
                    .child(label(hint, t.text_muted, 12.))
                    .child(
                        div()
                            .flex()
                            .gap(px(10.))
                            .child(
                                outline_button("link-submit", if code_step { "Link" } else { "Send code" }, !self.busy, &t)
                                    .flex_1()
                                    .on_click(cx.listener(|this, _, _, cx| this.submit(cx))),
                            )
                            .child(outline_button("link-cancel", "Not now", true, &t).flex_1().on_click(
                                cx.listener(|this, _, _, cx| {
                                    this.step = Step::Closed;
                                    this.error = None;
                                    cx.notify()
                                }),
                            )),
                    )
            }
        }
    }
}

#[cfg(test)]
mod haptic_tests {
    use super::*;

    #[gpui::test]
    fn invalid_or_busy_link_submissions_stay_silent(cx: &mut gpui::TestAppContext) {
        let panel = cx.new(|cx| AppleLinkPanel::new(Account {
            api_key: "fixture".into(),
            api_base: "http://127.0.0.1:9".into(),
            email: None,
        }, cx));
        panel.update(cx, |panel, cx| {
            let (_, feedback) = haptics::capture(|| panel.submit(cx));
            assert!(feedback.is_empty(), "closed panel is a no-op");
            panel.step = Step::Email;
            let (_, feedback) = haptics::capture(|| panel.submit(cx));
            assert!(feedback.is_empty(), "invalid email is a no-op");
            panel.step = Step::Code { link_token: "fixture".into() };
            let (_, feedback) = haptics::capture(|| panel.submit(cx));
            assert!(feedback.is_empty(), "incomplete code is a no-op");
            panel.busy = true;
            let (_, feedback) = haptics::capture(|| panel.submit(cx));
            assert!(feedback.is_empty(), "busy submission is a no-op");
        });
    }
}
