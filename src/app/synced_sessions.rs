//! Local synced sessions stay read-only. Continuing creates a separate cloud tab.
use super::a11y::A11yExt as _;
use super::{MobileApp, Screen};
use crate::theme::Theme;
use gpui::{Context, SharedString, div, prelude::*, px};
use jcode_mobile_core::{
    Account,
    mobile_sync::{self, Command, SyncedSession},
};

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
                                }
                                cx.notify();
                                true
                            }
                            Err(e) => {
                                this.synced.polling = None;
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
        let mut section = div().flex().flex_col().px(px(12.)).gap(px(6.));
        section = section.child(
            div()
                .pt(px(12.))
                .text_size(crate::system::fs(12.))
                .text_color(t.text_muted)
                .child("Synced from your computers"),
        );
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
                    .text_color(t.error)
                    .child(error.clone()),
            );
        }
        for session in self.synced.sessions.iter().filter(|s| {
            query.is_empty()
                || s.label().to_lowercase().contains(&query)
                || s.session_id.to_lowercase().contains(&query)
                || s.host
                    .as_deref()
                    .is_some_and(|h| h.to_lowercase().contains(&query))
        }) {
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
                .flex()
                .flex_col()
                .gap(px(5.))
                .rounded(px(10.))
                .bg(t.surface)
                .p(px(10.))
                .child(
                    div()
                        .text_size(crate::system::fs(14.))
                        .text_color(t.text_strong)
                        .child(session.label().to_string()),
                )
                .child(
                    div()
                        .text_size(crate::system::fs(11.))
                        .text_color(t.text_muted)
                        .child(format!(
                            "Local · {}",
                            session.host.as_deref().unwrap_or("computer")
                        )),
                );
            if let Some(preview) = &session.last_message_preview {
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
                        .a11y_text_button()
                        .min_h(px(44.))
                        .cursor_pointer()
                        .text_color(t.accent)
                        .child("Open cloud continuation")
                        .on_click(cx.listener(move |this, _, _, cx| {
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
            row = row.child(
                div()
                    .id(SharedString::from(format!("transfer-{sid}")))
                    .a11y_text_button()
                    .min_h(px(44.))
                    .cursor_pointer()
                    .text_color(if pending { t.text_muted } else { t.accent })
                    .child(if pending {
                        "Continuing in cloud…"
                    } else {
                        "Continue in cloud"
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if pending {
                            return;
                        }
                        if this.synced.selected.as_ref() == Some(&sid) {
                            this.continue_synced_in_cloud(cx);
                        } else {
                            this.synced.selected = Some(sid.clone());
                            this.transfer_input
                                .update(cx, |input, cx| input.set_text("", cx));
                        }
                        cx.notify();
                    })),
            );
            section = section.child(row);
        }
        section
    }
}
