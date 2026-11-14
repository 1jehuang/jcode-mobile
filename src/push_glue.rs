//! Glue between `notify` push events and the app: token registration with the
//! account, badge reset on foreground, tap-to-open, and inline replies.

use futures::StreamExt;
use gpui::{AppContext as _, Context};
use jcode_mobile_core::agents;
use jcode_mobile_core::cloud::{self, PushDevice};
use jcode_mobile_core::{ConnectionPhase, LocalIntent, Request, SessionState};

use super::MobileApp;
use crate::notify::{self, PushEvent};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{init, lifecycle_coverage::fixture_app};
    use gpui::TestAppContext;

    #[gpui::test]
    fn offline_token_and_foreground_are_local_only_even_with_account(cx: &mut TestAppContext) {
        cx.update(init);
        let (view, cx) = cx.add_window_view(fixture_app);
        view.update(cx, |app, cx| {
            app.on_push(PushEvent::Token("fixture-token".into()), cx);
            assert_eq!(app.push_token.as_deref(), Some("fixture-token"));
            app.on_push(PushEvent::Foreground, cx);
            assert!(app.connection.is_none());
            assert!(app.session.transcript.is_empty());
            app.sign_out(cx);
            app.on_push(PushEvent::Foreground, cx);
            app.on_push(PushEvent::Token("rotated-fixture-token".into()), cx);
            assert_eq!(app.push_token.as_deref(), Some("rotated-fixture-token"));
            assert!(app.account.is_none());
        });
    }

    #[gpui::test]
    fn mocked_connected_and_processing_inline_replies_preserve_draft(cx: &mut TestAppContext) {
        cx.update(init);
        let (view, cx) = cx.add_window_view(fixture_app);
        view.update(cx, |app, cx| {
            // Fake connection state. No socket exists, send only applies local intent.
            app.session.phase = ConnectionPhase::Connected;
            app.session.session_id = Some("push-fixture".into());
            app.composer
                .update(cx, |input, cx| input.set_text("unrelated typing", cx));
            let tab = app.active_tab;
            app.on_push(
                PushEvent::Open {
                    session_id: "push-fixture".into(),
                },
                cx,
            );
            assert_eq!(app.active_tab, tab);
            app.on_push(
                PushEvent::Reply {
                    session_id: "push-fixture".into(),
                    text: "first reply".into(),
                },
                cx,
            );
            assert_eq!(app.session.transcript.last().unwrap().text, "first reply");
            assert!(!app.session.transcript.last().unwrap().is_queued);
            assert!(app.session.is_processing);
            app.on_push(
                PushEvent::Reply {
                    session_id: "push-fixture".into(),
                    text: "interrupt reply".into(),
                },
                cx,
            );
            assert_eq!(
                app.session.transcript.last().unwrap().text,
                "interrupt reply"
            );
            assert!(app.session.transcript.last().unwrap().is_queued);
            assert_eq!(app.composer.read(cx).text(), "unrelated typing");
            assert!(app.auto_send.is_none() && app.connection.is_none());
        });
    }

    #[gpui::test]
    fn offline_disconnected_reply_waits_and_parked_session_is_reused(cx: &mut TestAppContext) {
        cx.update(init);
        let (view, cx) = cx.add_window_view(fixture_app);
        view.update(cx, |app, cx| {
            let first = app.active_tab;
            app.session.session_id = Some("parked-fixture".into());
            app.composer
                .update(cx, |input, cx| input.set_text("parked draft", cx));
            app.new_tab(cx);
            let count = app.tab_order.len();
            app.on_push(
                PushEvent::Reply {
                    session_id: "parked-fixture".into(),
                    text: "deferred reply".into(),
                },
                cx,
            );
            assert_eq!(app.active_tab, first);
            assert_eq!(app.tab_order.len(), count);
            assert_eq!(app.auto_send.as_deref(), Some("deferred reply"));
            assert!(app.session.transcript.is_empty());
            assert_eq!(app.composer.read(cx).text(), "parked draft");
            app.on_push(
                PushEvent::Open {
                    session_id: "new-fixture".into(),
                },
                cx,
            );
            assert_eq!(app.tab_order.len(), count + 1);
            assert!(app.connection.is_none());
            app.sign_out(cx);
            let tab = app.active_tab;
            for event in [
                PushEvent::Open {
                    session_id: "ignored".into(),
                },
                PushEvent::Reply {
                    session_id: "ignored".into(),
                    text: "not authorized".into(),
                },
            ] {
                app.on_push(event, cx);
                assert_eq!(app.active_tab, tab);
                assert!(app.session.transcript.is_empty());
                assert!(app.parked.is_empty());
            }
        });
    }
}

impl MobileApp {
    pub(super) fn install_push(&mut self, cx: &mut Context<Self>) {
        let mut tokens = crate::ios_ext::install_live_tokens();
        cx.spawn(async move |this, cx| {
            while let Some((kind, token)) = tokens.next().await {
                if this.update(cx, |this, cx| this.register_live_token(kind, token, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
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
                self.refresh_agents_status(cx);
            }
            PushEvent::Foreground => {
                if self.offline {
                    return;
                }
                notify::clear_badge();
                jcode_mobile_core::timing::event("app foreground", &format!("tabs={}", 1 + self.parked.len()));
                // Sockets often die silently while suspended. Probe every
                // live tab now and cut any pending reconnect backoff short.
                for c in self.connection.iter().chain(self.parked.iter().filter_map(|p| p.connection.as_ref())) {
                    c.wake();
                }
                if let Some(account) = self.account.clone() {
                    cloud::prewarm(&account);
                    self.refresh_agents_status(cx);
                    cx.background_spawn(async move {
                        if let Err(e) = cloud::reset_badge(&account) {
                            log::debug!("push: badge reset failed: {e}");
                        }
                    })
                    .detach();
                }
            }
            PushEvent::Open { session_id } => self.open_external_session(session_id, None, cx),
            PushEvent::Reply { session_id, text } => self.open_external_session(session_id, Some(text), cx),
        }
    }

    /// Register the current APNs token with the signed-in account. Called on
    /// token arrival and after sign-in.
    /// Fetch the account-wide agent view so the Live Activity covers agents
    /// on every host, not only this app's open tabs.
    pub(super) fn refresh_agents_status(&mut self, cx: &mut Context<Self>) {
        if self.offline {
            return;
        }
        let Some(account) = self.account.clone() else { return };
        let task = cx.background_spawn(async move {
            jcode_mobile_core::timing::timed("agents status", || agents::fetch_status(&account))
        });
        cx.spawn(async move |this, cx| {
            match task.await {
                Ok(state) => {
                    let _ = this.update(cx, |this, _| {
                        this.agents_status = Some(state);
                        this.sync_live_activity();
                    });
                }
                Err(e) => log::debug!("agents: status fetch failed: {e}"),
            }
        })
        .detach();
    }

    fn register_live_token(&mut self, kind: agents::LiveTokenKind, token: String, cx: &mut Context<Self>) {
        if self.offline {
            return;
        }
        let Some(account) = self.account.clone() else { return };
        if !self.live_tokens.contains(&token) {
            self.live_tokens.push(token.clone());
        }
        let device = self.push_token.clone();
        cx.background_spawn(async move {
            match agents::register_live_token(
                &account,
                kind,
                &token,
                notify::push_environment(),
                notify::BUNDLE_ID,
                device.as_deref(),
            ) {
                Ok(()) => log::info!("push: live activity token registered ({kind:?})"),
                Err(e) => log::warn!("push: live token register failed: {e}"),
            }
        })
        .detach();
    }

    pub(super) fn register_push(&mut self, cx: &mut Context<Self>) {
        if self.offline {
            return;
        }
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
        if self.offline {
            return;
        }
        let live = std::mem::take(&mut self.live_tokens);
        self.agents_status = None;
        crate::ios_ext::sync_live(&agents::AgentsState::default());
        if let Some(account) = self.account.clone() {
            cx.background_spawn(async move {
                for t in live {
                    let _ = agents::unregister_live_token(&account, &t);
                }
            })
            .detach();
        }
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
    pub(super) fn open_session(
        &mut self,
        session_id: String,
        reply: Option<String>,
        cx: &mut Context<Self>,
    ) {
        if self.account.is_none() {
            return;
        }
        if self.route_known_local_chat(&session_id, reply.clone(), cx) {
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
                self.session
                    .apply_intent(LocalIntent::UserQueuedInterrupt(text.clone()));
                self.send(|id| Request::SoftInterrupt {
                    id,
                    content: text,
                    urgent: false,
                });
            } else {
                self.session
                    .apply_intent(LocalIntent::UserSentMessage(text.clone()));
                self.send(|id| Request::Message { id, content: text });
            }
        }
        self.follow = true;
        cx.notify();
    }
}
