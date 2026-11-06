//! Glue between `notify` push events and the app: token registration with the
//! account, badge reset on foreground, tap-to-open, and inline replies.

use futures::StreamExt;
use gpui::{AppContext as _, Context};
use jcode_mobile_core::cloud::{self, PushDevice};
use jcode_mobile_core::{ConnectionPhase, LocalIntent, Request, SessionState};

use super::MobileApp;
use crate::notify::{self, PushEvent};

impl MobileApp {
    pub(super) fn install_push(&mut self, cx: &mut Context<Self>) {
        let mut rx = notify::install();
        cx.spawn(async move |this, cx| {
            while let Some(event) = rx.next().await {
                if this.update(cx, |this, cx| this.on_push(event, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    fn on_push(&mut self, event: PushEvent, cx: &mut Context<Self>) {
        match event {
            PushEvent::Token(token) => {
                self.push_token = Some(token);
                self.register_push(cx);
            }
            PushEvent::Foreground => {
                notify::clear_badge();
                if let Some(account) = self.account.clone() {
                    cx.background_spawn(async move {
                        if let Err(e) = cloud::reset_badge(&account) {
                            log::debug!("push: badge reset failed: {e}");
                        }
                    })
                    .detach();
                }
            }
            PushEvent::Open { session_id } => self.open_session(session_id, None, cx),
            PushEvent::Reply { session_id, text } => self.open_session(session_id, Some(text), cx),
        }
    }

    /// Register the current APNs token with the signed-in account. Called on
    /// token arrival and after sign-in.
    pub(super) fn register_push(&mut self, cx: &mut Context<Self>) {
        if self.offline { return; }
        let (Some(account), Some(token)) = (self.account.clone(), self.push_token.clone()) else {
            return;
        };
        let device = PushDevice {
            token,
            environment: notify::push_environment(),
            bundle_id: notify::BUNDLE_ID.into(),
            device_name: Some("iPhone".into()),
        };
        cx.background_spawn(async move {
            match cloud::register_push(&account, &device) {
                Ok(()) => log::info!("push: device registered"),
                Err(e) => log::warn!("push: register failed: {e}"),
            }
        })
        .detach();
    }

    /// Forget the token server-side (sign-out). Fire and forget.
    pub(super) fn unregister_push(&mut self, cx: &mut Context<Self>) {
        if self.offline { return; }
        let (Some(account), Some(token)) = (self.account.clone(), self.push_token.clone()) else {
            return;
        };
        cx.background_spawn(async move {
            let _ = cloud::unregister_push(&account, &token);
        })
        .detach();
    }

    /// Focus the tab showing `session_id`, or resume it in a new tab. With
    /// `reply`, send it to that session as soon as it is connected.
    pub(super) fn open_session(&mut self, session_id: String, reply: Option<String>, cx: &mut Context<Self>) {
        if self.account.is_none() {
            return;
        }
        if self.session.session_id.as_deref() != Some(session_id.as_str()) {
            let parked = self
                .parked
                .iter()
                .find(|p| p.session.session_id.as_deref() == Some(session_id.as_str()))
                .map(|p| p.id);
            match parked {
                Some(tab) => self.focus_tab(tab, cx),
                None => {
                    let id = self.next_tab;
                    self.next_tab += 1;
                    self.tab_order.push(id);
                    self.park_active(cx);
                    self.active_tab = id;
                    self.session = SessionState::default();
                    self.start_connection(Some(session_id), cx);
                }
            }
        }
        if let Some(text) = reply {
            if self.session.phase != ConnectionPhase::Connected {
                // pump sends it once the resumed session connects
                self.auto_send = Some(text);
            } else if self.session.is_processing {
                self.session.apply_intent(LocalIntent::UserQueuedInterrupt(text.clone()));
                self.send(|id| Request::SoftInterrupt { id, content: text, urgent: false });
            } else {
                self.session.apply_intent(LocalIntent::UserSentMessage(text.clone()));
                self.send(|id| Request::Message { id, content: text });
            }
        }
        self.follow = true;
        cx.notify();
    }
}
