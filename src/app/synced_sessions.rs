//! Synced browsing, live computer chat and explicit cloud continuation.
#[path = "synced_chat.rs"]
mod chat;
#[cfg(test)]
#[path = "synced_account_isolation_tests.rs"]
mod account_isolation_tests;
use super::a11y::A11yExt as _;
use super::{MobileApp, Screen};
use crate::haptics::{self, Haptic};
use crate::theme::Theme;
use gpui::{Context, SharedString, div, prelude::*, px, relative};
use jcode_mobile_core::{
    Account,
    mobile_sync::{self, Command, SyncedSession},
};

const SESSION_PAGE_SIZE: usize = 5;

#[derive(Default)]
pub(super) struct State {
    chats: std::collections::HashMap<u64, chat::LocalChat>,
    sessions: Vec<SyncedSession>,
    commands: Vec<Command>,
    selected: Option<String>,
    loading: bool,
    submitting: bool,
    /// Only the submitted draft may be cleared by its asynchronous response.
    submitted_draft: Option<(String, String)>,
    uncertain_submission: bool,
    uncertain_command: Option<(String, String)>,
    uncertain_known_commands: Vec<String>,
    polling: Option<String>,
    error: Option<String>,
    /// Additional pages explicitly requested in the drawer.
    visible_pages: usize,
    external_open: u64,
    external_loading: bool,
    external_retry: Option<(String, Option<String>)>,
    transcript: Option<mobile_sync::SyncedTranscript>,
    transcript_loading: bool,
    transcript_generation: u64,
}

impl State {
    /// Refresh content in place. Server activity ordering must not move a row
    /// out from under the user's finger or change the overview scroll anchor.
    fn replace_sessions(&mut self, sessions: Vec<SyncedSession>) {
        let order: std::collections::HashMap<_, _> = self
            .sessions
            .iter()
            .enumerate()
            .map(|(i, s)| (s.session_id.clone(), i))
            .collect();
        let mut sessions: Vec<_> = sessions
            .into_iter()
            .filter(SyncedSession::can_transfer)
            .collect();
        sessions.sort_by_key(|s| order.get(&s.session_id).copied().unwrap_or(usize::MAX));
        self.sessions = sessions;
    }

    fn visible_limit(&self) -> usize {
        self.visible_pages
            .saturating_add(1)
            .saturating_mul(SESSION_PAGE_SIZE)
    }
}

fn same_account(current: Option<&Account>, expected: &Account) -> bool {
    current.is_some_and(|a| a.api_key == expected.api_key && a.api_base == expected.api_base)
}

#[cfg(test)]
impl State {
    pub(super) fn fixture() -> Self {
        let sessions = serde_json::from_value(serde_json::json!([
            {"session_id":"local-work","source":"local","title":"Local work","host":"laptop","last_message_preview":"Add tests for the new continuation flow"},
            {"session_id":"local-pending","source":"local","title":"Pending continuation"},
            {"session_id":"local-done","source":"local","title":"Completed continuation"}
        ])).unwrap();
        let commands = serde_json::from_value(serde_json::json!([
            {"command_id":"pending","kind":"transfer","session_id":"local-pending","status":"running"},
            {"command_id":"done","kind":"transfer","session_id":"local-done","status":"done","result_session_id":"cloud-new"}
        ])).unwrap();
        Self {
            sessions,
            commands,
            selected: Some("local-work".into()),
            ..Self::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn resolved_laptop_notification_never_opens_cloud_or_sends_inline_reply(cx: &mut gpui::TestAppContext) {
        cx.update(super::super::init);
        let (view, cx) = cx.add_window_view(super::super::lifecycle_coverage::fixture_app);
        view.update(cx, |app, cx| {
            let tab = app.active_tab;
            app.composer.update(cx, |input, cx| input.set_text("cloud draft", cx));
            app.drawer_search.update(cx, |input, cx| input.set_text("unrelated filter", cx));
            app.apply_external_sessions("local-work".into(), Some("reply from push".into()), State::fixture().sessions, cx);
            let local_tab = app.active_tab;
            assert_ne!(local_tab, tab);
            assert_eq!(app.tab_order, [tab, local_tab]);
            assert!(app.connection.is_none());
            assert!(app.auto_send.is_none());
            assert!(!app.drawer_open);
            assert!(app.is_local_chat());
            assert!(!app.local_chat_ready(&app.session));
            assert_eq!(app.synced.selected.as_deref(), Some("local-work"));
            assert_eq!(app.composer.read(cx).text(), "reply from push");
            assert!(app.session.transcript.is_empty());
            app.open_external_session("local-work".into(), Some("second push reply".into()), cx);
            assert_eq!(app.tab_order, [tab, local_tab]);
            assert!(app.connection.is_none());
            assert!(app.auto_send.is_none());
            assert_eq!(app.composer.read(cx).text(), "reply from push", "a second notification must not overwrite the local draft");
            app.focus_tab(tab, cx);
            assert_eq!(app.composer.read(cx).text(), "cloud draft");
        });
    }

    #[gpui::test]
    fn resolved_cloud_notification_keeps_existing_tab_opening_behavior(cx: &mut gpui::TestAppContext) {
        cx.update(super::super::init);
        let (view, cx) = cx.add_window_view(super::super::lifecycle_coverage::fixture_app);
        view.update(cx, |app, cx| {
            let mut sessions = State::fixture().sessions;
            sessions.push(serde_json::from_value(serde_json::json!({"session_id":"cloud-work","source":"cloud"})).unwrap());
            app.apply_external_sessions("cloud-work".into(), None, sessions, cx);
            assert_eq!(app.tab_order.len(), 2);
            assert!(!app.drawer_open);
            assert!(app.synced.selected.is_none());
        });
    }

    #[gpui::test]
    fn unsynced_notification_target_does_not_guess_a_cloud_host(cx: &mut gpui::TestAppContext) {
        cx.update(super::super::init);
        let (view, cx) = cx.add_window_view(super::super::lifecycle_coverage::fixture_app);
        view.update(cx, |app, cx| {
            let tab = app.active_tab;
            app.apply_external_sessions("not-synced-yet".into(), None, vec![], cx);
            assert_eq!(app.tab_order, [tab]);
            assert!(app.connection.is_none());
            assert!(app.drawer_open);
            assert!(app.synced.error.as_deref().unwrap().contains("not synced"));
            assert!(app.synced.external_retry.is_some());
        });
    }

    #[test]
    fn refresh_preserves_existing_row_order_and_updates_content() {
        let mut state = State::fixture();
        let mut refreshed = state.sessions.clone();
        refreshed.reverse();
        refreshed[2].title = Some("Updated work".into());
        state.replace_sessions(refreshed);
        assert_eq!(
            state
                .sessions
                .iter()
                .map(|s| s.session_id.as_str())
                .collect::<Vec<_>>(),
            ["local-work", "local-pending", "local-done"]
        );
        assert_eq!(state.sessions[0].label(), "Updated work");
    }

    #[gpui::test]
    fn refresh_and_error_leave_synced_row_bounds_unchanged(cx: &mut gpui::TestAppContext) {
        use super::super::navigation_coverage::{drawer, mount};
        let (view, cx) = mount(cx);
        view.update(cx, |app, cx| {
            app.synced = State::fixture();
            app.synced.selected = None;
            app.synced.commands.clear();
            for i in 0..9 {
                app.synced.sessions.push(serde_json::from_value(serde_json::json!({
                    "session_id": format!("extra-{i}"), "source": "local", "title": format!("Extra {i}")
                })).unwrap());
            }
            app.synced.visible_pages = 1;
            cx.notify();
        });
        drawer(&view, cx);
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let overview = cx.debug_bounds("drawer-overview").unwrap();
        cx.simulate_event(gpui::ScrollWheelEvent {
            position: overview.center(),
            delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-70.))),
            touch_phase: gpui::TouchPhase::Moved,
            modifiers: gpui::Modifiers::default(),
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let selectors = [
            "drawer-local-session-local-work",
            "drawer-local-session-local-pending",
            "drawer-local-session-local-done",
        ];
        let before = selectors.map(|s| cx.debug_bounds(s).unwrap());
        for (loading, error) in [(true, None), (false, Some("A long refresh error that should never wrap and move the computer rows below it".to_string())), (false, None)] {
            view.update(cx, |app, cx| {
                app.synced.loading = loading;
                app.synced.error = error;
                cx.notify();
            });
            cx.update(|window, cx| window.draw(cx).clear(cx));
            for (selector, bounds) in selectors.into_iter().zip(before) {
                assert_eq!(cx.debug_bounds(selector).unwrap(), bounds, "{selector} moved during refresh");
            }
            assert!(cx.debug_bounds("transfer-local-work").is_none(), "refresh must not insert continuation controls into unselected rows");
        }
    }

    #[gpui::test]
    fn continuation_controls_hit_test_offline_busy_uncertain_and_completed(
        cx: &mut gpui::TestAppContext,
    ) {
        use super::super::navigation_coverage::{drawer, mount, tap};
        let (view, cx) = mount(cx);
        view.update(cx, |app, cx| {
            app.synced = State::fixture();
            app.synced.sessions.truncate(1);
            app.synced.sessions[0].last_message_preview = None;
            app.synced.commands.clear();
            app.transfer_input
                .update(cx, |input, cx| input.set_text("Continue fixture", cx));
            cx.notify();
        });
        drawer(&view, cx);
        for state in ["ready", "loading", "submitting", "uncertain", "pending"] {
            view.update(cx, |app, cx| {
                app.synced.loading = state == "loading";
                app.synced.submitting = state == "submitting";
                app.synced.uncertain_submission = state == "uncertain";
                app.synced.commands = if state == "pending" {
                    serde_json::from_value(serde_json::json!([
                        {"command_id":"pending", "kind":"transfer", "session_id":"local-work", "status":"running"}
                    ])).unwrap()
                } else { vec![] };
                cx.notify();
            });
            let (_, feedback) = haptics::capture(|| tap(cx, "transfer-local-work"));
            assert!(
                feedback.is_empty(),
                "offline continuation {state} must not submit"
            );
            view.read_with(cx, |app, cx| {
                assert_eq!(app.tab_order, [0]);
                assert_eq!(app.transfer_input.read(cx).text(), "Continue fixture");
                assert!(app.connection.is_none());
            });
        }
        view.update(cx, |app, cx| {
            app.synced.loading = false;
            app.synced.submitting = false;
            app.synced.uncertain_submission = false;
            app.synced.commands = serde_json::from_value(serde_json::json!([
                {"command_id":"done", "kind":"transfer", "session_id":"local-work", "status":"done", "result_session_id":"cloud-fixture"}
            ])).unwrap();
            cx.notify();
        });
        tap(cx, "open-transfer-local-work");
        view.read_with(cx, |app, _| {
            assert_eq!(app.tab_order.len(), 2);
            assert!(!app.drawer_open);
            assert_eq!(
                app.synced.sessions[0].session_id, "local-work",
                "local original stays intact"
            );
            assert!(app.connection.is_none());
        });
    }

    #[test]
    fn local_sessions_reveal_five_at_a_time() {
        let mut state = State::default();
        assert_eq!(state.visible_limit(), 5);
        state.visible_pages += 1;
        assert_eq!(state.visible_limit(), 10);
        state.visible_pages = usize::MAX;
        assert_eq!(state.visible_limit(), usize::MAX);
    }

    #[gpui::test]
    fn local_sidebar_discloses_more_and_searches_hidden_sessions(cx: &mut gpui::TestAppContext) {
        cx.update(super::super::init);
        let (view, cx) = cx.add_window_view(|window, cx| {
            let mut app = MobileApp::new_offline(
                super::super::SafeArea {
                    top: 20.,
                    bottom: 0.,
                },
                window,
                cx,
            );
            app.screen = Screen::Chat;
            app.account = Some(Account {
                given_name: None,
                name: None,
                email: Some("preview@example.invalid".into()),
                api_key: "offline-fixture".into(),
                api_base: "https://example.invalid/v1".into(),
            });
            app.device = Some((390., 844.));
            app.drawer_open = true;
            app.drawer_t = 1.;
            app.synced.sessions = (0..12)
                .map(|i| {
                    serde_json::from_value(serde_json::json!({
                "session_id": format!("local-{i}"), "source": "local", "title": format!("Work {i}")
            })).unwrap()
                })
                .collect();
            app
        });
        cx.simulate_resize(gpui::size(px(390.), px(844.)));
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
        });
        assert!(cx.debug_bounds("drawer-local-session-local-4").is_some());
        assert!(cx.debug_bounds("drawer-local-session-local-5").is_none());
        // Debug bounds include clipped rows. Scroll the actual overview, not
        // an offscreen debug target, before tapping its disclosure control.
        let overview = cx.debug_bounds("drawer-overview").unwrap();
        let scroll_at = overview.center();
        for _ in 0..20 {
            let more = cx.debug_bounds("drawer-local-see-more").unwrap();
            if overview.contains(&more.center()) {
                break;
            }
            cx.simulate_event(gpui::ScrollWheelEvent {
                position: scroll_at,
                delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-60.))),
                touch_phase: gpui::TouchPhase::Moved,
                modifiers: gpui::Modifiers::default(),
            });
            cx.update(|window, cx| window.draw(cx).clear(cx));
        }
        let more = cx.debug_bounds("drawer-local-see-more").unwrap();
        assert!(
            overview.contains(&more.center()),
            "disclosure must be inside the actual overview viewport"
        );
        cx.simulate_click(more.center(), gpui::Modifiers::default());
        view.update(cx, |app, _| assert_eq!(app.synced.visible_limit(), 10));
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
        });
        assert!(cx.debug_bounds("drawer-local-session-local-9").is_some());
        assert!(cx.debug_bounds("drawer-local-session-local-10").is_none());
        view.update(cx, |app, cx| {
            app.drawer_search
                .update(cx, |input, cx| input.set_text("Work 11", cx));
            cx.notify();
        });
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
        });
        assert!(cx.debug_bounds("drawer-local-session-local-11").is_some());
        assert!(cx.debug_bounds("drawer-local-see-more").is_none());
        let row = cx.debug_bounds("drawer-local-session-local-11").unwrap();
        let overview = cx.debug_bounds("drawer-overview").unwrap();
        assert!(
            overview.contains(&row.center()),
            "filtered session must be reachable in the overview viewport"
        );
        cx.simulate_click(row.center(), gpui::Modifiers::default());
        view.update(cx, |app, _| {
            assert_eq!(app.synced.selected.as_deref(), Some("local-11"));
            assert!(app.is_local_chat());
            assert!(!app.drawer_open);
            assert!(!app.local_chat_ready(&app.session));
        });
    }

    #[gpui::test]
    fn unavailable_continuation_and_refresh_stay_silent(cx: &mut gpui::TestAppContext) {
        cx.update(super::super::init);
        let (view, cx) = cx.add_window_view(|window, cx| {
            MobileApp::new_offline(
                super::super::SafeArea {
                    top: 20.,
                    bottom: 0.,
                },
                window,
                cx,
            )
        });
        view.update(cx, |app, cx| {
            let (_, feedback) = haptics::capture(|| {
                app.continue_synced_in_cloud(cx);
                app.refresh_synced_sessions(cx);
            });
            assert!(feedback.is_empty(), "offline actions stay silent");
            app.offline = false;
            let (_, feedback) = haptics::capture(|| app.continue_synced_in_cloud(cx));
            assert!(feedback.is_empty(), "missing account stays silent");
            app.account = Some(Account {
                given_name: None,
                name: None,
                api_key: "fixture".into(),
                api_base: "http://127.0.0.1:9".into(),
                email: None,
            });
            app.synced = State::fixture();
            let (_, feedback) = haptics::capture(|| app.continue_synced_in_cloud(cx));
            assert!(feedback.is_empty(), "empty continuation stays silent");
            app.synced.submitting = true;
            let (_, feedback) = haptics::capture(|| app.continue_synced_in_cloud(cx));
            assert!(feedback.is_empty(), "busy continuation stays silent");
        });
    }

    #[test]
    fn stale_account_results_are_rejected() {
        let account = Account {
            given_name: None,
            name: None,
            api_key: "key-one".into(),
            api_base: "https://example.test/v1".into(),
            email: None,
        };
        let mut other = account.clone();
        assert!(same_account(Some(&account), &account));
        assert!(!same_account(None, &account));
        other.api_key = "key-two".into();
        assert!(!same_account(Some(&other), &account));
        other = account.clone();
        other.api_base = "https://other.test/v1".into();
        assert!(!same_account(Some(&other), &account));
    }
}

impl MobileApp {
    /// Resolve unknown notification/deep-link targets before opening SSH. A
    /// laptop session ID is not an attachable session on the cloud machine.
    pub(super) fn open_external_session(&mut self, id: String, reply: Option<String>, cx: &mut Context<Self>) {
        let Some(account) = self.account.clone() else { return };
        self.synced.external_open += 1;
        if self.route_known_local_chat(&id, reply.clone(), cx) {
            self.synced.external_loading = false;
            self.synced.external_retry = None;
            if !self.is_local_chat() {
                self.load_synced_transcript(id, cx);
            }
            return;
        }
        let already_open = self.session.session_id.as_deref() == Some(&id)
            || self.parked.iter().any(|p| p.session.session_id.as_deref() == Some(&id));
        if self.offline || already_open {
            self.synced.external_loading = false;
            self.synced.external_retry = None;
            self.screen = Screen::Chat;
            self.drawer_open = false;
            self.open_session(id, reply, cx);
            return;
        }
        let generation = self.synced.external_open;
        let origin_tab = self.active_tab;
        let origin_session = self.session.session_id.clone();
        let auth_generation = self.auth_generation;
        self.synced.external_retry = Some((id.clone(), reply.clone()));
        self.screen = Screen::Chat;
        self.drawer_open = true;
        self.synced.external_loading = true;
        self.synced.error = None;
        let request_account = account.clone();
        let request_id = id.clone();
        let task = cx.background_spawn(async move { mobile_sync::lookup(&request_account, &request_id) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.auth_generation != auth_generation || !same_account(this.account.as_ref(), &account) || this.synced.external_open != generation { return; }
                this.synced.external_loading = false;
                if !this.external_lookup_origin_current(origin_tab, &origin_session) {
                    this.synced.external_retry = None;
                    cx.notify();
                    return;
                }
                match result {
                    Ok(session) => {
                        this.synced.external_retry = None;
                        let mut sessions = this.synced.sessions.clone();
                        sessions.retain(|s| s.session_id != id);
                        sessions.push(session);
                        this.apply_external_sessions(id, reply, sessions, cx);
                    }
                    Err(e) => {
                        this.synced.error = Some(format!("Tap here to retry opening session: {e}"));
                    }
                }
                cx.notify();
            });
        }).detach();
        cx.notify();
    }

    fn external_lookup_origin_current(&self, tab: u64, session: &Option<String>) -> bool {
        self.active_tab == tab && &self.session.session_id == session
    }

    fn apply_external_sessions(&mut self, id: String, reply: Option<String>, sessions: Vec<SyncedSession>, cx: &mut Context<Self>) {
        let local = sessions.iter().find(|s| s.session_id == id && s.can_transfer()).cloned();
        let cloud = sessions.iter().any(|s| s.session_id == id && s.source == "cloud");
        self.synced.replace_sessions(sessions);
        if let Some(session) = local {
            self.open_local_chat(session, reply, cx);
        } else if cloud {
            self.drawer_open = false;
            self.open_session(id, reply, cx);
        } else {
            self.drawer_open = true;
            self.synced.external_retry = Some((id, reply));
            self.synced.error = Some("Session is not synced yet. Enable mobile sync on its computer, then tap here to retry.".into());
        }
    }

    fn load_synced_transcript(&mut self, id: String, cx: &mut Context<Self>) {
        self.synced.transcript_generation += 1;
        let generation = self.synced.transcript_generation;
        let auth_generation = self.auth_generation;
        self.synced.transcript = None;
        self.synced.transcript_loading = false;
        self.synced.error = None;
        if self.offline { return; }
        let Some(account) = self.account.clone() else { return };
        self.synced.transcript_loading = true;
        let request_account = account.clone();
        let task = cx.background_spawn(async move { mobile_sync::transcript(&request_account, &id) });
        let selected = self.synced.selected.clone();
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.auth_generation != auth_generation || !same_account(this.account.as_ref(), &account) || this.synced.selected != selected
                    || this.synced.transcript_generation != generation { return; }
                this.synced.transcript_loading = false;
                match result {
                    Ok(transcript) => this.synced.transcript = Some(transcript),
                    Err(e) => this.synced.error = Some(format!("Could not load synced conversation: {e}")),
                }
                cx.notify();
            });
        }).detach();
    }

    pub(super) fn refresh_synced_sessions(&mut self, cx: &mut Context<Self>) {
        if self.offline {
            return;
        }
        if self.synced.loading {
            return;
        }
        let Some(account) = self.account.clone() else {
            return;
        };
        self.synced.loading = true;
        self.synced.error = None;
        let generation = self.auth_generation;
        let request_account = account.clone();
        let task = cx.background_spawn(async move {
            Ok::<_, String>((
                mobile_sync::list(&request_account)?,
                mobile_sync::commands(&request_account)?,
            ))
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                this.complete_synced_refresh(&account, generation, result, cx);
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn continue_synced_in_cloud(&mut self, cx: &mut Context<Self>) {
        if self.offline {
            return;
        }
        if self.synced.loading
            || self.synced.submitting
            || self.synced.uncertain_submission
            || self.synced.polling.is_some()
        {
            return;
        }
        let Some(account) = self.account.clone() else {
            return;
        };
        let Some(session) = self
            .synced
            .sessions
            .iter()
            .find(|s| Some(&s.session_id) == self.synced.selected.as_ref())
            .cloned()
        else {
            return;
        };
        if self.synced.commands.iter().any(|c| {
            c.kind == "transfer"
                && c.session_id.as_deref() == Some(&session.session_id)
                && matches!(c.status.as_str(), "queued" | "running")
        }) {
            return;
        }
        let text = self.transfer_input.read(cx).text().trim().to_string();
        if text.is_empty() {
            return;
        }
        self.synced.submitting = true;
        self.synced.submitted_draft = Some((session.session_id.clone(), self.transfer_input.read(cx).text().to_string()));
        let generation = self.auth_generation;
        haptics::play(Haptic::Light);
        self.synced.error = None;
        let request_account = account.clone();
        let task =
            cx.background_spawn(
                async move { mobile_sync::transfer(&request_account, &session, &text) },
            );
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                this.complete_synced_transfer(&account, generation, result, cx);
            })
            .ok();
        })
        .detach();
    }

    fn synced_request_current(&self, account: &Account, generation: u64) -> bool {
        generation == self.auth_generation && same_account(self.account.as_ref(), account)
    }

    fn complete_synced_refresh(
        &mut self,
        account: &Account,
        generation: u64,
        result: Result<(Vec<SyncedSession>, Vec<Command>), String>,
        cx: &mut Context<Self>,
    ) {
        if !self.synced_request_current(account, generation) { return; }
        self.synced.loading = false;
        match result {
            Ok((sessions, commands)) => {
                self.synced.replace_sessions(sessions);
                // An empty/eventually-consistent command list cannot prove a lost
                // POST was rejected. Only a newly observed matching command can
                // reconcile an uncertain submission.
                if self.synced.uncertain_submission {
                    let reconciled = self.synced.uncertain_command.as_ref().is_some_and(|(id, source)| {
                        commands.iter().any(|c| &c.command_id == id && c.kind == "transfer"
                            && c.session_id.as_ref() == Some(source) && (c.status == "failed" || c.destination(source).is_ok()))
                    }) || self.synced.submitted_draft.as_ref().is_some_and(|(source, _)| {
                        commands.iter().any(|c| c.kind == "transfer"
                            && c.session_id.as_ref() == Some(source)
                            && !self.synced.uncertain_known_commands.contains(&c.command_id)
                            && (c.status == "failed" || c.destination(source).is_ok()))
                    });
                    self.synced.uncertain_submission = !reconciled;
                    if reconciled { self.synced.submitted_draft = None; self.synced.uncertain_command = None; }
                    else { self.synced.error = Some("Could not confirm whether cloud continuation was accepted. Retry refresh, not submission.".into()); }
                }
                self.synced.commands = commands;
                if let Some(command) = self.synced.commands.iter().find(|c| {
                    c.kind == "transfer" && matches!(c.status.as_str(), "queued" | "running")
                }).cloned() {
                    self.poll_transfer(command, account.clone(), cx);
                }
            }
            Err(error) => self.synced.error = Some(error),
        }
        cx.notify();
    }

    fn complete_synced_transfer(
        &mut self,
        account: &Account,
        generation: u64,
        result: Result<Command, String>,
        cx: &mut Context<Self>,
    ) {
        if !self.synced_request_current(account, generation) { return; }
        self.synced.submitting = false;
        let submitted_draft = self.synced.submitted_draft.take();
        match result {
            Ok(command) => {
                if submitted_draft.as_ref().is_some_and(|(source, text)| {
                    self.synced.selected.as_ref() == Some(source)
                        && self.transfer_input.read(cx).text() == *text
                }) {
                    self.transfer_input.update(cx, |input, cx| input.set_text("", cx));
                }
                self.synced.commands.insert(0, command.clone());
                self.poll_transfer(command, account.clone(), cx);
            }
            Err(error) => {
                // A lost response may have enqueued successfully. Reconcile before retrying.
                haptics::play(Haptic::Error);
                self.synced.submitted_draft = submitted_draft;
                self.synced.uncertain_known_commands = self.synced.commands.iter().map(|c| c.command_id.clone()).collect();
                self.synced.uncertain_submission = true;
                self.synced.error = Some(format!("{error}. Refresh sessions before retrying."));
                self.refresh_synced_sessions(cx);
            }
        }
        cx.notify();
    }

    fn complete_transfer_poll(
        &mut self,
        account: &Account,
        generation: u64,
        id: &str,
        source: &str,
        result: Result<Command, String>,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.synced_request_current(account, generation)
            || self.synced.polling.as_deref() != Some(id)
        {
            return true;
        }
        let result = match result {
            Ok(command) => {
                if command.command_id != id || command.kind != "transfer"
                    || command.session_id.as_deref() != Some(source)
                {
                    self.synced.polling = None;
                    self.synced.uncertain_submission = true;
                    self.synced.uncertain_command = Some((id.into(), source.into()));
                    self.synced.error = Some("Cloud continuation returned a different command, session or kind".into());
                    cx.notify();
                    return true;
                }
                self.synced.commands.retain(|c| c.command_id != command.command_id);
                self.synced.commands.insert(0, command.clone());
                command.destination(source)
            }
            Err(error) => {
                self.synced.error = Some(format!("Waiting for cloud: {error}"));
                cx.notify();
                return false;
            }
        };
        match result {
            Ok(None) => {
                self.synced.error = None;
                cx.notify();
                false
            }
            Ok(Some(destination)) => {
                self.synced.polling = None;
                self.synced.error = None;
                if self.drawer_open && self.synced.selected.as_deref() == Some(source)
                    && self.session.session_id.as_deref() == Some(source)
                {
                    self.synced.selected = None;
                    self.screen = Screen::Chat;
                    self.drawer_open_session(destination, cx);
                } else {
                    haptics::play(Haptic::Success);
                }
                cx.notify();
                true
            }
            Err(error) => {
                self.synced.polling = None;
                // A malformed terminal response is not evidence that the
                // transfer failed. Keep duplicate submissions blocked.
                self.synced.uncertain_submission = self.synced.commands.first().is_some_and(|c| c.status != "failed");
                self.synced.uncertain_command = self.synced.uncertain_submission.then(|| (id.into(), source.into()));
                haptics::play(Haptic::Error);
                self.synced.error = Some(error);
                cx.notify();
                true
            }
        }
    }

    fn poll_transfer(&mut self, command: Command, account: Account, cx: &mut Context<Self>) {
        if self.offline {
            return;
        }
        if self.synced.polling.is_some() {
            return;
        }
        let Some(source) = command.session_id.clone() else {
            return;
        };
        let id = command.command_id;
        self.synced.polling = Some(id.clone());
        let generation = self.auth_generation;
        cx.spawn(async move |this, cx| {
            // Polling resumes from the account's command list after reopening the app.
            for _ in 0..1800 {
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(2))
                    .await;
                let active = this
                    .update(cx, |this, _| {
                        this.synced_request_current(&account, generation)
                            && this.synced.polling.as_deref() == Some(&id)
                    })
                    .unwrap_or(false);
                if !active {
                    return;
                }
                let a = account.clone();
                let command_id = id.clone();
                let result = cx
                    .background_spawn(async move { mobile_sync::command(&a, &command_id) })
                    .await;
                let stop = this
                    .update(cx, |this, cx| {
                        this.complete_transfer_poll(&account, generation, &id, &source, result, cx)
                    })
                    .unwrap_or(true);
                if stop {
                    return;
                }
            }
            this.update(cx, |this, cx| {
                if this.synced_request_current(&account, generation)
                    && this.synced.polling.as_deref() == Some(&id)
                {
                    this.synced.polling = None;
                    this.synced.error = Some(
                        "Cloud continuation is still pending. Refresh to check its status.".into(),
                    );
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn render_synced_sessions(
        &self,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let query = self.drawer_search.read(cx).text().trim().to_lowercase();
        let mut section = div().flex_none().flex().flex_col();
        section = section.child(super::sidebar::section_label(
            "Synced from your computers",
            t,
        ));
        // A single reserved status line prevents loading and error transitions
        // from pushing every computer row (and the cloud sections below it).
        let status = self.synced.error.clone().unwrap_or_else(|| {
            if self.synced.external_loading {
                "Finding notification's session…".into()
            } else if self.synced.loading {
                if self.synced.sessions.is_empty() {
                    "Loading synced sessions…"
                } else {
                    "Refreshing synced sessions…"
                }
                .into()
            } else {
                String::new()
            }
        });
        section = section.child(
            div()
                .id("drawer-local-status")
                .debug_selector(|| "drawer-local-status".into())
                .on_click(cx.listener(|this, _, _, cx| {
                    if let Some((id, reply)) = this.synced.external_retry.clone().filter(|_| !this.synced.external_loading) {
                        this.open_external_session(id, reply, cx);
                    }
                }))
                .flex_none()
                .h(px(20.))
                .mx(px(16.))
                .truncate()
                .text_size(crate::system::fs(12.))
                .text_color(
                    if self
                        .synced
                        .error
                        .as_deref()
                        .is_some_and(|e| e != mobile_sync::SYNC_UNAVAILABLE)
                    {
                        t.error
                    } else {
                        t.text_muted
                    },
                )
                .child(status),
        );
        let matching: Vec<_> = self
            .synced
            .sessions
            .iter()
            .filter(|s| {
                query.is_empty()
                    || s.label().to_lowercase().contains(&query)
                    || s.session_id.to_lowercase().contains(&query)
                    || s.host
                        .as_deref()
                        .is_some_and(|h| h.to_lowercase().contains(&query))
            })
            .collect();
        // Search includes every local session, including those not yet revealed.
        let limit = if query.is_empty() {
            self.synced.visible_limit()
        } else {
            matching.len()
        };
        for session in matching.iter().take(limit) {
            let sid = session.session_id.clone();
            let selected = self.synced.selected.as_ref() == Some(&sid) && !session.can_message();
            let latest = self
                .synced
                .commands
                .iter()
                .find(|c| c.kind == "transfer" && c.session_id.as_ref() == Some(&sid));
            let destination = latest.and_then(|c| c.destination(&sid).ok().flatten());
            let pending = (selected
                && (self.synced.submitting || self.synced.uncertain_submission))
                || latest.is_some_and(|c| matches!(c.status.as_str(), "queued" | "running"));
            let mut row = div()
                .id(SharedString::from(format!("drawer-local-session-{sid}")))
                .debug_selector({
                    let sid = sid.clone();
                    move || format!("drawer-local-session-{sid}")
                })
                .mx(px(8.))
                .flex_none()
                .min_h(px(super::sidebar::SESSION_ROW_MIN))
                .flex()
                .flex_col()
                .justify_center()
                .gap(px(1.))
                .rounded(px(10.))
                .bg(if selected { t.surface } else { t.raised })
                .active(|el| el.bg(t.surface))
                .cursor_pointer()
                .px(px(10.))
                .py(px(3.))
                .on_click(cx.listener({
                    let sid = sid.clone();
                    move |this, _, _, cx| {
                        if let Some(session) = this.synced.sessions.iter().find(|s| s.session_id == sid && s.can_transfer()).cloned() {
                            this.open_local_chat(session, None, cx);
                            return;
                        }
                        if this.synced.selected.as_ref() != Some(&sid) {
                            this.synced.selected = Some(sid.clone());
                            haptics::play(Haptic::Selection);
                            this.transfer_input
                                .update(cx, |input, cx| input.set_text("", cx));
                            this.load_synced_transcript(sid.clone(), cx);
                            cx.notify();
                        }
                    }
                }))
                .child(
                    div()
                        .pl(px(12.))
                        .min_w_0()
                        .truncate()
                        .text_size(crate::system::fs(13.))
                        .line_height(relative(1.2))
                        .text_color(if selected { t.text_strong } else { t.text })
                        .child(format!("{}{}", session.label(), if session.can_message() { if session.busy { " · Working on computer" } else { " · Live on computer" } } else { "" })),
                )
                .child(
                    div()
                        .pl(px(12.))
                        .truncate()
                        .text_size(crate::system::fs(10.5))
                        .line_height(relative(1.15))
                        .text_color(t.text_muted)
                        .child(format!(
                            "Local · {}",
                            session.host.as_deref().unwrap_or("computer")
                        )),
                );
            // Keep the list dense. Selecting a continuation reveals its preview.
            if let Some(preview) = session.last_message_preview.as_ref().filter(|_| selected) {
                row = row.child(
                    div()
                        .max_h(px(60.))
                        .overflow_hidden()
                        .text_size(crate::system::fs(12.))
                        .text_color(t.text_secondary)
                        .child(preview.clone()),
                );
            }
            if let Some(destination) = destination {
                row = row.child(
                    div()
                        .id(SharedString::from(format!("open-transfer-{sid}")))
                        .debug_selector({
                            let sid = sid.clone();
                            move || format!("open-transfer-{sid}")
                        })
                        .a11y_text_button()
                        .min_h(px(28.))
                        .text_size(crate::system::fs(12.))
                        .cursor_pointer()
                        .text_color(t.accent)
                        .child("Open cloud continuation")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.screen = Screen::Chat;
                            this.drawer_open_session(destination.clone(), cx);
                        })),
                );
            }
            if let Some(command) = latest.filter(|c| c.status == "failed") {
                row =
                    row.child(
                        div()
                            .text_size(crate::system::fs(12.))
                            .text_color(t.error)
                            .child(command.error.clone().unwrap_or_else(|| {
                                "Cloud continuation failed. You can retry.".into()
                            })),
                    );
            }
            if selected {
                row = row.child(div().text_size(crate::system::fs(12.)).text_color(t.text_muted)
                    .child("Synced conversation · read-only on this phone"));
                // Transcript content belongs in the shared chat viewport, not
                // flattened into the narrow session drawer. The row opens that
                // read-only view, while continuation stays an explicit action.
                row = row.child(div().text_size(crate::system::fs(12.)).text_color(t.text_secondary).child("Creates a new cloud session from synced text. Local files and running tools are not moved. The original stays intact."))
                    .child(self.transfer_input.clone());
            }
            if selected || pending {
                row = row.child(
                    div()
                        .id(SharedString::from(format!("transfer-{sid}")))
                        .debug_selector({
                            let sid = sid.clone();
                            move || format!("transfer-{sid}")
                        })
                        .a11y_text_button()
                        .min_h(px(if selected { 44. } else { 26. }))
                        .text_size(crate::system::fs(12.))
                        .cursor_pointer()
                        .text_color(if pending { t.text_muted } else { t.accent })
                        .child(if pending {
                            "Continuing in cloud…"
                        } else {
                            "Continue in cloud"
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            if pending || this.synced.loading {
                                return;
                            }
                            if this.synced.selected.as_ref() == Some(&sid) {
                                this.continue_synced_in_cloud(cx);
                            } else {
                                this.synced.selected = Some(sid.clone());
                                haptics::play(Haptic::Selection);
                                this.transfer_input
                                    .update(cx, |input, cx| input.set_text("", cx));
                            }
                            cx.notify();
                        })),
                );
            }
            section = section.child(row);
        }
        if matching.len() > limit {
            section = section.child(
                div()
                    .id("drawer-local-see-more")
                    .a11y_text_button()
                    .debug_selector(|| "drawer-local-see-more".into())
                    .mx(px(8.))
                    .px(px(22.))
                    .min_h(px(44.))
                    .flex()
                    .items_center()
                    .cursor_pointer()
                    .text_size(crate::system::fs(12.))
                    .text_color(t.accent)
                    .child(format!("See more ({})", matching.len() - limit))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.synced.visible_pages = this.synced.visible_pages.saturating_add(1);
                        haptics::play(Haptic::Selection);
                        cx.notify();
                    })),
            );
        }
        section
    }
}
