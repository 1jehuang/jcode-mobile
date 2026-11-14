//! Chat transport for sessions still running on the user's computer.
use super::{MobileApp, same_account};
use gpui::{AppContext, Context};
use jcode_mobile_core::{
    mobile_sync::{self, Command, SyncedSession, SyncedTranscript},
    session::{ConnectionPhase, SessionState, synced_transcript_entries},
};
use std::time::Duration;

const DELIVERY_UNCERTAIN: &str = "Delivery is uncertain. Check the computer before sending again. This message will not be retried automatically.";
const COMPUTER_OFFLINE: &str =
    "Computer is offline. Open this session in jcode there to send messages.";

#[cfg(test)]
#[path = "synced_chat_tests.rs"]
mod tests;

pub(super) struct LocalChat {
    session: SyncedSession,
    submitting: bool,
    pending: Option<String>,
    uncertain: bool,
    delivery_error: Option<String>,
}

impl MobileApp {
    pub(in crate::app) fn forget_local_chat(&mut self, tab: u64) {
        self.synced.chats.remove(&tab);
    }

    pub(in crate::app) fn local_delivery_uncertain(&self, session: &SessionState) -> bool {
        self.synced.chats.values().any(|chat| {
            session.session_id.as_ref() == Some(&chat.session.session_id) && chat.uncertain
        })
    }

    /// Acknowledge uncertainty without resending anything. The user must check
    /// the computer and explicitly press send again if still needed.
    pub(in crate::app) fn acknowledge_local_delivery(&mut self, cx: &mut Context<Self>) {
        if let Some(chat) = self.synced.chats.get_mut(&self.active_tab) {
            chat.uncertain = false;
            // A newer poll may have replaced the uncertainty warning with a
            // connectivity error. Checking delivery does not resolve that error.
            if self.session.error_banner.as_deref().is_some_and(|error| {
                error == DELIVERY_UNCERTAIN || error.starts_with("Message delivery uncertain: ")
            }) {
                self.session.error_banner =
                    (!chat.session.live).then(|| COMPUTER_OFFLINE.to_owned());
            }
            cx.notify();
        }
    }

    /// Returning true means the local target was handled without SSH attach.
    pub(in crate::app) fn route_known_local_chat(
        &mut self,
        id: &str,
        reply: Option<String>,
        cx: &mut Context<Self>,
    ) -> bool {
        let session = self
            .synced
            .chats
            .values()
            .find(|chat| chat.session.session_id == id)
            .map(|chat| chat.session.clone())
            .or_else(|| {
                self.synced
                    .sessions
                    .iter()
                    .find(|session| session.session_id == id && session.can_transfer())
                    .cloned()
            });
        let Some(session) = session else {
            return false;
        };
        if session.can_message() {
            self.open_local_chat(session, reply, cx);
        } else {
            self.screen = super::Screen::Chat;
            self.synced.selected = Some(id.to_owned());
            self.synced
                .sessions
                .sort_by_key(|session| session.session_id != id);
            self.synced.visible_pages = 0;
            self.drawer_search
                .update(cx, |input, cx| input.set_text("", cx));
            self.drawer_open = true;
            self.drawer_last = std::time::Instant::now();
            if let Some(reply) = reply {
                self.transfer_input
                    .update(cx, |input, cx| input.set_text(reply, cx));
            }
            cx.notify();
        }
        true
    }

    pub(in crate::app) fn session_is_local_chat(&self, session: &SessionState) -> bool {
        session.session_id.as_ref().is_some_and(|id| {
            self.synced
                .chats
                .values()
                .any(|chat| &chat.session.session_id == id)
        })
    }

    pub(in crate::app) fn local_chat_ready(&self, session: &SessionState) -> bool {
        self.synced
            .chats
            .values()
            .find(|chat| session.session_id.as_ref() == Some(&chat.session.session_id))
            .is_some_and(|chat| {
                chat.session.live && !chat.submitting && chat.pending.is_none() && !chat.uncertain
            })
    }

    pub(in crate::app) fn is_local_chat(&self) -> bool {
        self.synced.chats.contains_key(&self.active_tab)
    }

    pub(in crate::app) fn local_chat_host(&self) -> Option<&str> {
        self.synced
            .chats
            .get(&self.active_tab)?
            .session
            .host
            .as_deref()
    }

    pub(in crate::app) fn open_local_chat(
        &mut self,
        session: SyncedSession,
        reply: Option<String>,
        cx: &mut Context<Self>,
    ) {
        if self.account.is_none() || !session.can_message() {
            return;
        }
        let existing = self
            .synced
            .chats
            .iter()
            .find(|(tab, chat)| {
                self.tab_order.contains(tab) && chat.session.session_id == session.session_id
            })
            .map(|(tab, _)| *tab);
        if let Some(tab) = existing {
            if tab != self.active_tab {
                self.focus_tab(tab, cx);
            }
        } else {
            self.synced
                .chats
                .retain(|tab, _| self.tab_order.contains(tab));
            let tab = self.next_tab;
            self.next_tab += 1;
            self.tab_order.push(tab);
            self.park_active(cx);
            self.active_tab = tab;
            self.follow = true;
            self.stream_motion.reset();
            self.session = SessionState::default();
            self.session.session_id = Some(session.session_id.clone());
            self.session.session_title = Some(session.label().to_owned());
            self.session.phase = ConnectionPhase::Connecting;
            self.synced.chats.insert(
                tab,
                LocalChat {
                    session,
                    submitting: false,
                    pending: None,
                    uncertain: false,
                    delivery_error: None,
                },
            );
            self.poll_local_chat(tab, cx);
        }
        // A notification reply is a draft, never an automatic command.
        if let Some(reply) = reply {
            if self.composer.read(cx).text().is_empty() {
                self.composer
                    .update(cx, |input, cx| input.set_text(reply, cx));
            }
        }
        self.screen = super::Screen::Chat;
        self.model_picker = false;
        self.dir_picker = false;
        self.close_drawer(cx);
        cx.notify();
    }

    fn local_session_mut(&mut self, tab: u64) -> Option<&mut SessionState> {
        if self.active_tab == tab {
            Some(&mut self.session)
        } else {
            self.parked
                .iter_mut()
                .find(|p| p.id == tab)
                .map(|p| &mut p.session)
        }
    }

    fn local_chat_current(
        &self,
        tab: u64,
        account: &jcode_mobile_core::Account,
        generation: u64,
    ) -> bool {
        self.auth_generation == generation
            && same_account(self.account.as_ref(), account)
            && self.tab_order.contains(&tab)
            && self.synced.chats.contains_key(&tab)
    }

    fn poll_local_chat(&mut self, tab: u64, cx: &mut Context<Self>) {
        if self.offline {
            return;
        }
        let Some(account) = self.account.clone() else {
            return;
        };
        let generation = self.auth_generation;
        cx.spawn(async move |this, cx| {
            loop {
                let state = this
                    .update(cx, |this, _| {
                        if !this.local_chat_current(tab, &account, generation) {
                            return None;
                        }
                        let chat = this.synced.chats.get(&tab)?;
                        Some((
                            chat.session.session_id.clone(),
                            chat.pending.clone(),
                            this.offline,
                        ))
                    })
                    .ok()
                    .flatten();
                let Some((id, pending, offline)) = state else {
                    break;
                };
                if !offline {
                    let request_account = account.clone();
                    let result = cx
                        .background_spawn(async move {
                            // A snapshot includes all pages, so seq updates and rewritten
                            // assistant messages replace rather than duplicate rows.
                            let transcript = mobile_sync::transcript(&request_account, &id);
                            let command = pending
                                .map(|command| mobile_sync::command(&request_account, &command));
                            (transcript, command)
                        })
                        .await;
                    let keep = this
                        .update(cx, |this, cx| {
                            if !this.local_chat_current(tab, &account, generation) {
                                return false;
                            }
                            this.apply_local_poll(tab, result.0, result.1);
                            cx.notify();
                            true
                        })
                        .unwrap_or(false);
                    if !keep {
                        break;
                    }
                }
                cx.background_executor().timer(Duration::from_secs(2)).await;
            }
        })
        .detach();
    }

    pub(in crate::app) fn apply_local_poll(
        &mut self,
        tab: u64,
        transcript: Result<SyncedTranscript, String>,
        command: Option<Result<Command, String>>,
    ) {
        let Some(chat) = self.synced.chats.get_mut(&tab) else {
            return;
        };
        let mut error = chat.delivery_error.clone();
        if let Some(command) = command {
            match command {
                Ok(command) if chat.pending.as_deref() != Some(&command.command_id) => {
                    error = Some("Computer returned a different message command".into());
                }
                Ok(command) => match command.message_complete(&chat.session.session_id) {
                    Ok(true) => chat.pending = None,
                    Ok(false) => {}
                    Err(e) => {
                        if command.kind == "message"
                            && command.session_id.as_deref() == Some(&chat.session.session_id)
                            && command.status == "failed"
                        {
                            chat.pending = None;
                            chat.delivery_error = Some(e.clone());
                        }
                        error = Some(e);
                    }
                },
                Err(e) => error = Some(e),
            }
        }
        match transcript {
            Ok(transcript) => {
                if transcript.session.source != "local"
                    || transcript.session.session_id != chat.session.session_id
                {
                    if let Some(state) = self.local_session_mut(tab) {
                        state.phase = ConnectionPhase::Disconnected;
                        state.error_banner =
                            Some("Computer sync returned a different session or source".into());
                    }
                    return;
                }
                chat.session = transcript.session;
                let busy = chat.session.busy;
                let uncertain = chat.uncertain;
                let title = Some(chat.session.label().to_owned());
                let live = chat.session.live;
                if let Some(state) = self.local_session_mut(tab) {
                    state.phase = if live {
                        ConnectionPhase::Connected
                    } else {
                        ConnectionPhase::Disconnected
                    };
                    state.session_title = title;
                    state.is_processing = busy;
                    state.transcript = synced_transcript_entries(transcript.messages);
                    state.error_banner = error.or_else(|| if uncertain {
                        Some(DELIVERY_UNCERTAIN.into())
                    } else if !live {
                        Some(COMPUTER_OFFLINE.into())
                    } else { None });
                }
            }
            Err(e) => {
                if let Some(state) = self.local_session_mut(tab) {
                    state.phase = ConnectionPhase::Disconnected;
                    state.error_banner = Some(format!("Computer sync unavailable: {e}"));
                }
            }
        }
    }

    pub(in crate::app) fn send_local_draft(&mut self, cx: &mut Context<Self>) {
        if self.offline || self.session.phase != ConnectionPhase::Connected {
            return;
        }
        let text = self.composer.read(cx).text().trim().to_string();
        if text.is_empty() {
            return;
        }
        let Some(account) = self.account.clone() else {
            return;
        };
        let tab = self.active_tab;
        let Some(chat) = self.synced.chats.get_mut(&tab) else {
            return;
        };
        if chat.submitting
            || chat.pending.is_some()
            || chat.uncertain
            || !chat.session.can_message()
        {
            return;
        }
        chat.submitting = true;
        chat.delivery_error = None;
        self.session.error_banner = None;
        let session = chat.session.clone();
        let generation = self.auth_generation;
        let expected = account.clone();
        let submitted = text.clone();
        let task =
            cx.background_spawn(async move { mobile_sync::message(&account, &session, &text) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.complete_local_submission(tab, &expected, generation, &submitted, result, cx);
            });
        })
        .detach();
        cx.notify();
    }

    fn complete_local_submission(
        &mut self,
        tab: u64,
        account: &jcode_mobile_core::Account,
        generation: u64,
        submitted: &str,
        result: Result<Command, String>,
        cx: &mut Context<Self>,
    ) {
        if !self.local_chat_current(tab, account, generation) {
            return;
        }
        let chat = self.synced.chats.get_mut(&tab).unwrap();
        chat.submitting = false;
        let result = result.and_then(|command| {
            let complete = command.message_complete(&chat.session.session_id)?;
            Ok((command, complete))
        });
        match result {
            Ok((command, complete)) => {
                // POST may already return a terminal acknowledgement. Only
                // queued/running commands need another status poll.
                chat.pending = (!complete).then_some(command.command_id);
                if self.active_tab == tab {
                    if self.composer.read(cx).text().trim() == submitted {
                        self.composer.update(cx, |input, cx| input.set_text("", cx));
                    }
                    self.feel_sent();
                    self.follow = true;
                } else if let Some(p) = self.parked.iter_mut().find(|p| p.id == tab) {
                    if p.preview_composer.read(cx).text().trim() == submitted {
                        p.draft = Default::default();
                        p.preview_composer
                            .update(cx, |input, cx| input.set_text("", cx));
                    }
                }
            }
            Err(e) => {
                chat.uncertain = true;
                if let Some(state) = self.local_session_mut(tab) {
                    state.error_banner = Some(format!(
                        "Message delivery uncertain: {e}. Check the computer before retrying."
                    ));
                }
            }
        }
        cx.notify();
    }
}
