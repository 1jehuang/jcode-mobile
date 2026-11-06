//! Local synced sessions stay read-only. Continuing creates a separate cloud tab.
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
    sessions: Vec<SyncedSession>,
    commands: Vec<Command>,
    selected: Option<String>,
    loading: bool,
    submitting: bool,
    uncertain_submission: bool,
    polling: Option<String>,
    error: Option<String>,
    /// Additional pages explicitly requested in the drawer.
    visible_pages: usize,
}

impl State {
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
    fn continuation_controls_hit_test_offline_busy_uncertain_and_completed(cx: &mut gpui::TestAppContext) {
        use super::super::navigation_coverage::{mount, drawer, tap};
        let (view, cx) = mount(cx);
        view.update(cx, |app, cx| {
            app.synced = State::fixture();
            app.synced.sessions.truncate(1);
            app.synced.sessions[0].last_message_preview = None;
            app.synced.commands.clear();
            app.transfer_input.update(cx, |input, cx| input.set_text("Continue fixture", cx));
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
            assert!(feedback.is_empty(), "offline continuation {state} must not submit");
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
            assert_eq!(app.synced.sessions[0].session_id, "local-work", "local original stays intact");
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
            assert_eq!(app.synced.selected.as_deref(), Some("local-11"))
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
                if !same_account(this.account.as_ref(), &account) {
                    return;
                }
                this.synced.loading = false;
                match result {
                    Ok((sessions, commands)) => {
                        this.synced.sessions = sessions
                            .into_iter()
                            .filter(SyncedSession::can_transfer)
                            .collect();
                        this.synced.commands = commands;
                        this.synced.uncertain_submission = false;
                        if let Some(command) = this
                            .synced
                            .commands
                            .iter()
                            .find(|c| {
                                c.kind == "transfer"
                                    && matches!(c.status.as_str(), "queued" | "running")
                            })
                            .cloned()
                        {
                            this.poll_transfer(command, account.clone(), cx);
                        }
                    }
                    Err(e) => this.synced.error = Some(e),
                }
                cx.notify();
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
                if !same_account(this.account.as_ref(), &account) {
                    return;
                }
                this.synced.submitting = false;
                match result {
                    Ok(command) => {
                        this.transfer_input
                            .update(cx, |input, cx| input.set_text("", cx));
                        this.synced.commands.insert(0, command.clone());
                        this.poll_transfer(command, account, cx);
                    }
                    Err(e) => {
                        // A lost response may have enqueued successfully. Reconcile before retrying.
                        haptics::play(Haptic::Error);
                        this.synced.uncertain_submission = true;
                        this.synced.error = Some(format!("{e}. Refresh sessions before retrying."));
                        this.refresh_synced_sessions(cx);
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
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
        cx.spawn(async move |this, cx| {
            // Polling resumes from the account's command list after reopening the app.
            for _ in 0..1800 {
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(2))
                    .await;
                let active = this
                    .update(cx, |this, _| {
                        same_account(this.account.as_ref(), &account)
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
                        if !same_account(this.account.as_ref(), &account) {
                            return true;
                        }
                        let result = match result {
                            Ok(command) => {
                                this.synced
                                    .commands
                                    .retain(|c| c.command_id != command.command_id);
                                this.synced.commands.insert(0, command.clone());
                                command.destination(&source)
                            }
                            Err(e) => {
                                this.synced.error = Some(format!("Waiting for cloud: {e}"));
                                cx.notify();
                                return false;
                            }
                        };
                        match result {
                            Ok(None) => {
                                this.synced.error = None;
                                cx.notify();
                                false
                            }
                            Ok(Some(destination)) => {
                                this.synced.polling = None;
                                this.synced.error = None;
                                // Do not steal focus if the user has left the continuation sheet.
                                if this.drawer_open
                                    && this.synced.selected.as_deref() == Some(&source)
                                {
                                    this.synced.selected = None;
                                    this.screen = Screen::Chat;
                                    this.drawer_open_session(destination, cx);
                                } else {
                                    haptics::play(Haptic::Success);
                                }
                                cx.notify();
                                true
                            }
                            Err(e) => {
                                this.synced.polling = None;
                                haptics::play(Haptic::Error);
                                this.synced.error = Some(e);
                                cx.notify();
                                true
                            }
                        }
                    })
                    .unwrap_or(true);
                if stop {
                    return;
                }
            }
            this.update(cx, |this, cx| {
                if same_account(this.account.as_ref(), &account) {
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
        let mut section = div().flex().flex_col();
        section = section.child(super::sidebar::section_label(
            "Synced from your computers",
            t,
        ));
        if self.synced.loading {
            section = section.child(
                div()
                    .text_color(t.text_muted)
                    .child("Loading synced sessions…"),
            );
        }
        if let Some(error) = &self.synced.error {
            section = section.child(
                div()
                    .text_size(crate::system::fs(12.))
                    .text_color(if error == mobile_sync::SYNC_UNAVAILABLE {
                        t.text_muted
                    } else {
                        t.error
                    })
                    .child(error.clone()),
            );
        }
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
            let selected = self.synced.selected.as_ref() == Some(&sid);
            let latest = self
                .synced
                .commands
                .iter()
                .find(|c| c.kind == "transfer" && c.session_id.as_ref() == Some(&sid));
            let destination = latest.and_then(|c| c.destination(&sid).ok().flatten());
            let pending = self.synced.submitting
                || self.synced.uncertain_submission
                || self.synced.loading
                || latest.is_some_and(|c| matches!(c.status.as_str(), "queued" | "running"));
            let mut row = div()
                .id(SharedString::from(format!("drawer-local-session-{sid}")))
                .debug_selector({
                    let sid = sid.clone();
                    move || format!("drawer-local-session-{sid}")
                })
                .mx(px(8.))
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
                        if this.synced.selected.as_ref() != Some(&sid) {
                            this.synced.selected = Some(sid.clone());
                            haptics::play(Haptic::Selection);
                            this.transfer_input
                                .update(cx, |input, cx| input.set_text("", cx));
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
                        .child(session.label().to_string()),
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
                        .debug_selector({ let sid = sid.clone(); move || format!("open-transfer-{sid}") })
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
                row = row.child(div().text_size(crate::system::fs(12.)).text_color(t.text_secondary).child("Creates a new cloud session from synced text. Local files and running tools are not moved. The original stays intact."))
                    .child(self.transfer_input.clone());
            }
            if selected || pending {
                row = row.child(
                    div()
                        .id(SharedString::from(format!("transfer-{sid}")))
                        .debug_selector({ let sid = sid.clone(); move || format!("transfer-{sid}") })
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
                            if pending {
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
