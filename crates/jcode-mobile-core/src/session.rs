//! Pure state machine turning connection output into session state.
//! Driven by the harness API's `ApiEvent` (via jcode-sdk), so the app shares
//! one protocol with every other jcode client.

use jcode_sdk::{ApiEvent, HistoryMessage, TurnStopReason};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, PartialEq)]
pub enum ConnectionPhase {
    Disconnected,
    Connecting,
    /// Cloud host is waking or provisioning.
    Provisioning { message: String },
    Connected,
    Reconnecting { attempt: u32 },
    Failed { reason: String },
}

#[derive(Debug, Clone, PartialEq)]
pub enum ConnectionOutput {
    Phase(ConnectionPhase),
    Event(ApiEvent),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
    System,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ToolStatus {
    StreamingInput,
    Running,
    Succeeded,
    Failed(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub input: String,
    pub output: String,
    pub status: ToolStatus,
}

impl ToolCall {
    fn new(id: impl Into<String>, name: impl Into<String>, status: ToolStatus) -> Self {
        Self { id: id.into(), name: name.into(), input: String::new(), output: String::new(), status }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TranscriptEntry {
    pub id: u64,
    pub role: Role,
    pub text: String,
    pub reasoning: String,
    pub tool_calls: Vec<ToolCall>,
    pub is_streaming: bool,
    pub is_queued: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoticeKind {
    Info,
    Notification,
    Compaction,
}

/// A user-facing system notification (iOS banner, desktop toast).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Alert {
    pub title: String,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Notice {
    pub id: u64,
    pub kind: NoticeKind,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SessionState {
    pub phase: ConnectionPhase,
    pub transcript: Vec<TranscriptEntry>,
    pub session_id: Option<String>,
    pub session_title: Option<String>,
    pub all_sessions: Vec<String>,
    pub session_titles: HashMap<String, String>,
    pub is_processing: bool,
    pub is_reasoning: bool,
    pub model_name: Option<String>,
    pub provider_name: Option<String>,
    pub available_models: Vec<String>,
    pub reasoning_effort: Option<String>,
    pub server_version: Option<String>,
    pub token_input: u64,
    pub token_output: u64,
    pub status_detail: Option<String>,
    pub server_phase: Option<String>,
    pub error_banner: Option<String>,
    pub notices: Vec<Notice>,
    pub pending_interrupts: Vec<String>,
    /// System notifications for the shell to post (drain after `apply`).
    pub alerts: Vec<Alert>,
    next_id: u64,
}

impl Default for SessionState {
    fn default() -> Self {
        Self {
            phase: ConnectionPhase::Disconnected,
            transcript: vec![],
            session_id: None,
            session_title: None,
            all_sessions: vec![],
            session_titles: HashMap::new(),
            is_processing: false,
            is_reasoning: false,
            model_name: None,
            provider_name: None,
            available_models: vec![],
            reasoning_effort: None,
            server_version: None,
            token_input: 0,
            token_output: 0,
            status_detail: None,
            server_phase: None,
            error_banner: None,
            notices: vec![],
            pending_interrupts: vec![],
            alerts: vec![],
            next_id: 1,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum LocalIntent {
    UserSentMessage(String),
    UserQueuedInterrupt(String),
    CancelledQueuedInterrupts,
    DismissError,
    DismissNotice(u64),
    ClearedConversation,
    Reset,
}

impl SessionState {
    pub fn title_for(&self, session_id: &str) -> Option<&str> {
        self.session_titles.get(session_id).map(String::as_str)
    }

    pub fn has_pending_interrupts(&self) -> bool {
        !self.pending_interrupts.is_empty()
    }

    fn fresh_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    fn entry(&mut self, role: Role, text: impl Into<String>) -> TranscriptEntry {
        TranscriptEntry {
            id: self.fresh_id(),
            role,
            text: text.into(),
            reasoning: String::new(),
            tool_calls: vec![],
            is_streaming: false,
            is_queued: false,
        }
    }

    fn notice(&mut self, kind: NoticeKind, message: impl Into<String>) {
        let id = self.fresh_id();
        self.notices.push(Notice { id, kind, message: message.into() });
    }

    pub fn apply_intent(&mut self, intent: LocalIntent) {
        match intent {
            LocalIntent::UserSentMessage(text) => {
                let e = self.entry(Role::User, text);
                self.transcript.push(e);
                self.is_processing = true;
                self.error_banner = None;
            }
            LocalIntent::UserQueuedInterrupt(text) => {
                let mut e = self.entry(Role::User, text.clone());
                e.is_queued = true;
                self.transcript.push(e);
                self.pending_interrupts.push(text);
            }
            LocalIntent::CancelledQueuedInterrupts => {
                self.transcript.retain(|e| !e.is_queued);
                self.pending_interrupts.clear();
            }
            LocalIntent::DismissError => self.error_banner = None,
            LocalIntent::DismissNotice(id) => self.notices.retain(|n| n.id != id),
            LocalIntent::ClearedConversation => {
                self.transcript.clear();
                self.is_processing = false;
                self.is_reasoning = false;
                self.error_banner = None;
            }
            LocalIntent::Reset => {
                let next_id = self.next_id;
                *self = SessionState { next_id, ..Default::default() };
            }
        }
    }

    pub fn apply(&mut self, output: ConnectionOutput) {
        match output {
            ConnectionOutput::Phase(p) => self.apply_phase(p),
            ConnectionOutput::Event(e) => self.apply_event(e),
        }
    }

    fn apply_phase(&mut self, phase: ConnectionPhase) {
        self.phase = phase.clone();
        match phase {
            ConnectionPhase::Connected => self.error_banner = None,
            ConnectionPhase::Failed { reason } => {
                self.error_banner = Some(reason);
                self.is_processing = false;
                self.is_reasoning = false;
            }
            ConnectionPhase::Disconnected | ConnectionPhase::Reconnecting { .. } => {
                self.is_processing = false;
                self.is_reasoning = false;
                self.server_phase = None;
                self.finish_streaming();
            }
            ConnectionPhase::Connecting | ConnectionPhase::Provisioning { .. } => {}
        }
    }

    fn alert(&mut self, body: impl Into<String>) {
        let title = self.session_title.clone().unwrap_or_else(|| "jcode".into());
        let mut body: String = body.into();
        if body.chars().count() > 180 {
            body = body.chars().take(179).collect::<String>() + "…";
        }
        self.alerts.push(Alert { title, body });
    }

    fn last_reply_preview(&self) -> String {
        self.transcript
            .iter()
            .rev()
            .find(|e| e.role == Role::Assistant && !e.text.trim().is_empty())
            .map(|e| e.text.split_whitespace().collect::<Vec<_>>().join(" "))
            .unwrap_or_else(|| "Finished".into())
    }

    fn turn_over(&mut self) {
        self.is_processing = false;
        self.is_reasoning = false;
        self.server_phase = None;
        self.status_detail = None;
        self.finish_streaming();
    }

    fn apply_event(&mut self, event: ApiEvent) {
        use ApiEvent::*;
        match event {
            HelloOk { server, .. } => self.server_version = Some(server),
            Attached { session } => {
                if let Some(t) = session.title.clone() {
                    self.session_title = Some(t.clone());
                    self.session_titles.insert(session.session_id.clone(), t);
                }
                self.is_processing = session.status == "running";
                self.session_id = Some(session.session_id);
            }
            Sessions { sessions } => {
                self.all_sessions = sessions.iter().map(|s| s.session_id.clone()).collect();
                for s in sessions {
                    if let Some(t) = s.title {
                        self.session_titles.insert(s.session_id, t);
                    }
                }
            }
            History { session_id, messages, .. } => self.apply_history(session_id, messages),
            TextDelta { text, .. } => {
                self.is_processing = true;
                self.with_streaming_assistant(|e| e.text.push_str(&text))
            }
            TextReplace { text, .. } => self.with_streaming_assistant(|e| e.text = text),
            // The text of this response ended; tools may still run, and they
            // update their cards in place.
            TextDone { .. } => self.finish_streaming(),
            ReasoningDelta { text, .. } => {
                self.is_reasoning = true;
                self.with_streaming_assistant(|e| e.reasoning.push_str(&text));
            }
            ReasoningDone { .. } => self.is_reasoning = false,
            ToolStart { call_id, name, .. } => {
                if self.find_tool(&call_id).is_none() {
                    self.with_streaming_assistant(|e| {
                        e.tool_calls.push(self::ToolCall::new(call_id, name, ToolStatus::StreamingInput))
                    })
                }
            }
            ToolInputDelta { call_id, delta, .. } => match self.find_tool(&call_id) {
                Some(t) => t.input.push_str(&delta),
                None => self.with_streaming_assistant(|e| {
                    let mut t = self::ToolCall::new(call_id, "", ToolStatus::StreamingInput);
                    t.input = delta;
                    e.tool_calls.push(t);
                }),
            },
            ToolCall { call_id, name, input, .. } => {
                let input = match input {
                    serde_json::Value::String(s) => s,
                    v => serde_json::to_string_pretty(&v).unwrap_or_default(),
                };
                match self.find_tool(&call_id) {
                    Some(t) => t.input = input,
                    None => self.with_streaming_assistant(|e| {
                        let mut t = self::ToolCall::new(call_id, name, ToolStatus::Running);
                        t.input = input;
                        e.tool_calls.push(t);
                    }),
                }
            }
            ToolExec { call_id, name, .. } => match self.find_tool(&call_id) {
                Some(t) => t.status = ToolStatus::Running,
                None => self.with_streaming_assistant(|e| {
                    e.tool_calls.push(self::ToolCall::new(call_id, name, ToolStatus::Running))
                }),
            },
            ToolDone { call_id, name, output, error, .. } => {
                let status = error.map(ToolStatus::Failed).unwrap_or(ToolStatus::Succeeded);
                match self.find_tool(&call_id) {
                    Some(t) => {
                        t.output = output;
                        t.status = status;
                    }
                    None => self.with_streaming_assistant(|e| {
                        let mut t = self::ToolCall::new(call_id, name, status);
                        t.output = output;
                        e.tool_calls.push(t);
                    }),
                }
            }
            TokenUsage { input, output, .. } => {
                self.token_input = input;
                self.token_output = output;
            }
            ConnectionPhase { phase, .. } => {
                self.server_phase = if phase.is_empty() { None } else { Some(phase) }
            }
            SessionStatus { status, .. } => match status.as_str() {
                "running" => self.is_processing = true,
                "idle" | "cancelled" => self.turn_over(),
                _ => {}
            },
            MessageAccepted { .. } => self.is_processing = true,
            SoftInterruptInjected { client_ids, display_role, .. } => {
                for _ in 0..client_ids.len().max(1) {
                    self.resolve_pending_interrupt(display_role.as_deref());
                }
            }
            SoftInterruptsCancelled { .. } => {
                self.transcript.retain(|e| !e.is_queued);
                self.pending_interrupts.clear();
            }
            TurnDone { .. } => {
                let was_busy = self.is_processing;
                self.turn_over();
                if was_busy {
                    let preview = self.last_reply_preview();
                    self.alert(preview);
                }
                self.drain_pending_interrupts();
            }
            TurnStopped { reason, message, .. } => {
                self.turn_over();
                self.drain_pending_interrupts();
                match reason {
                    TurnStopReason::Interrupted => self.notice(NoticeKind::Info, "Interrupted"),
                    _ => {
                        self.error_banner =
                            Some(if message.is_empty() { "The turn stopped".into() } else { message })
                    }
                }
            }
            Error { message, .. } => {
                self.turn_over();
                self.alert(format!("Error: {message}"));
                self.error_banner = Some(message);
            }
            ModelInfo { provider, model, reasoning_effort, .. }
            | RuntimeInfo { provider, model, reasoning_effort, .. } => {
                if provider.is_some() {
                    self.provider_name = provider;
                }
                if model.is_some() {
                    self.model_name = model;
                }
                if reasoning_effort.is_some() {
                    self.reasoning_effort = reasoning_effort;
                }
            }
            Models { models, current, .. } => {
                self.available_models = models;
                if current.is_some() {
                    self.model_name = current;
                }
            }
            Compacted { message, .. } => self.notice(NoticeKind::Compaction, message),
            SessionRenamed { session_id, display_title, .. } => {
                if self.session_id.is_none() || self.session_id.as_deref() == Some(&session_id) {
                    self.session_title = Some(display_title.clone());
                }
                self.session_titles.insert(session_id, display_title);
            }
            WakeRequested { notification, .. } => {
                self.alert(notification.clone());
                self.notice(NoticeKind::Notification, notification)
            }
            SessionRecovery { reconnect_notice: Some(n), .. } => self.notice(NoticeKind::Info, n),
            BackgroundProgress { label, summary, done, .. } => {
                self.status_detail = if done { None } else { Some(format!("{label}: {summary}")) }
            }
            _ => {}
        }
    }

    fn apply_history(&mut self, session_id: String, messages: Vec<HistoryMessage>) {
        self.session_id = Some(session_id);
        self.pending_interrupts.clear();

        // Keep trailing optimistic user messages the server has not seen yet.
        let tail_len = self
            .transcript
            .iter()
            .rev()
            .take_while(|e| e.role == Role::User && !e.is_queued)
            .count();
        let tail: Vec<_> = self.transcript.split_off(self.transcript.len() - tail_len);
        let history_user: HashSet<&str> =
            messages.iter().filter(|m| m.role == "user").map(|m| m.content.as_str()).collect();
        let tail: Vec<_> =
            tail.into_iter().filter(|e| !history_user.contains(e.text.as_str())).collect();

        let mut transcript = Vec::with_capacity(messages.len() + tail.len());
        for m in &messages {
            let role = match m.role.as_str() {
                "user" => Role::User,
                "assistant" => Role::Assistant,
                "system" => Role::System,
                _ => continue,
            };
            if m.content.is_empty() {
                continue;
            }
            transcript.push(self.entry(role, m.content.clone()));
        }
        transcript.extend(tail);
        self.transcript = transcript;
    }

    fn find_tool(&mut self, id: &str) -> Option<&mut ToolCall> {
        self.transcript
            .iter_mut()
            .rev()
            .take(8)
            .flat_map(|e| e.tool_calls.iter_mut())
            .find(|t| t.id == id)
    }

    /// Soft interrupts are injected in the order they were queued.
    fn resolve_pending_interrupt(&mut self, display_role: Option<&str>) {
        if !self.pending_interrupts.is_empty() {
            self.pending_interrupts.remove(0);
        }
        if let Some(i) = self.transcript.iter().position(|e| e.is_queued) {
            let mut e = self.transcript.remove(i);
            e.is_queued = false;
            if display_role == Some("system") {
                e.role = Role::System;
            }
            // Injected mid-turn: it belongs before the reply now streaming.
            self.finish_streaming();
            self.transcript.push(e);
        }
    }

    fn drain_pending_interrupts(&mut self) {
        self.pending_interrupts.clear();
        for e in &mut self.transcript {
            e.is_queued = false;
        }
    }

    fn with_streaming_assistant(&mut self, f: impl FnOnce(&mut TranscriptEntry)) {
        let streaming = self
            .transcript
            .last()
            .is_some_and(|e| e.role == Role::Assistant && e.is_streaming);
        if !streaming {
            let mut e = self.entry(Role::Assistant, "");
            e.is_streaming = true;
            self.transcript.push(e);
        }
        f(self.transcript.last_mut().unwrap());
    }

    fn finish_streaming(&mut self) {
        if let Some(last) = self.transcript.last_mut()
            && last.is_streaming
        {
            last.is_streaming = false;
            if last.text.is_empty() && last.tool_calls.is_empty() && last.reasoning.is_empty() {
                self.transcript.pop();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ApiEvent as E;

    fn ev(s: &mut SessionState, e: E) {
        s.apply(ConnectionOutput::Event(e));
    }
    fn sid() -> String {
        "s".into()
    }

    #[test]
    fn streams_text_and_tools_into_one_entry() {
        let mut s = SessionState::default();
        s.apply_intent(LocalIntent::UserSentMessage("hi".into()));
        ev(&mut s, E::TextDelta { session_id: sid(), text: "Hel".into(), message_id: None });
        ev(&mut s, E::TextDelta { session_id: sid(), text: "lo".into(), message_id: None });
        ev(&mut s, E::ToolStart { session_id: sid(), call_id: "t1".into(), name: "bash".into() });
        ev(&mut s, E::ToolInputDelta { session_id: sid(), call_id: "t1".into(), delta: "ls".into() });
        ev(&mut s, E::ToolDone { session_id: sid(), call_id: "t1".into(), name: "bash".into(), output: "ok".into(), error: None });
        ev(&mut s, E::TurnDone { session_id: sid(), pending_soft_interrupts: None });
        assert_eq!(s.transcript.len(), 2);
        let a = &s.transcript[1];
        assert_eq!(a.text, "Hello");
        assert!(!a.is_streaming);
        assert_eq!(a.tool_calls[0].input, "ls");
        assert_eq!(a.tool_calls[0].status, ToolStatus::Succeeded);
        assert!(!s.is_processing);
    }

    #[test]
    fn tool_done_after_text_done_updates_existing_card() {
        let mut s = SessionState::default();
        s.apply_intent(LocalIntent::UserSentMessage("hi".into()));
        ev(&mut s, E::ToolStart { session_id: sid(), call_id: "t1".into(), name: "bash".into() });
        ev(&mut s, E::ToolExec { session_id: sid(), call_id: "t1".into(), name: "bash".into() });
        ev(&mut s, E::TextDone { session_id: sid(), message_id: None });
        ev(&mut s, E::ToolDone { session_id: sid(), call_id: "t1".into(), name: "bash".into(), output: "hi".into(), error: None });
        ev(&mut s, E::TextDelta { session_id: sid(), text: "ok".into(), message_id: None });
        let tools: Vec<_> = s.transcript.iter().flat_map(|e| &e.tool_calls).collect();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].status, ToolStatus::Succeeded);
        assert_eq!(tools[0].output, "hi");
    }

    #[test]
    fn history_keeps_unseen_optimistic_tail() {
        let mut s = SessionState::default();
        s.apply_intent(LocalIntent::UserSentMessage("new one".into()));
        let old = HistoryMessage { response_stats: None, role: "user".into(), content: "old".into() };
        ev(&mut s, E::History { session_id: sid(), messages: vec![old], images: vec![] });
        let texts: Vec<_> = s.transcript.iter().map(|e| e.text.as_str()).collect();
        assert_eq!(texts, ["old", "new one"]);
        assert_eq!(s.session_id.as_deref(), Some("s"));
    }

    #[test]
    fn queued_interrupt_resolves_on_injection() {
        let mut s = SessionState::default();
        s.apply_intent(LocalIntent::UserQueuedInterrupt("stop".into()));
        assert!(s.has_pending_interrupts());
        ev(
            &mut s,
            E::SoftInterruptInjected {
                session_id: sid(),
                client_ids: vec!["q1".into()],
                point: "x".into(),
                tools_skipped: None,
                display_role: None,
            },
        );
        assert!(!s.has_pending_interrupts());
        assert!(!s.transcript[0].is_queued);
    }

    #[test]
    fn interrupted_turn_leaves_notice_not_error() {
        let mut s = SessionState::default();
        s.apply_intent(LocalIntent::UserSentMessage("hi".into()));
        ev(&mut s, E::TurnStopped { session_id: sid(), reason: TurnStopReason::Interrupted, message: String::new(), provider_stop_reason: None });
        assert!(!s.is_processing && s.error_banner.is_none());
        assert_eq!(s.notices[0].message, "Interrupted");
    }

    #[test]
    fn disconnect_drops_empty_stub() {
        let mut s = SessionState::default();
        ev(&mut s, E::TextReplace { session_id: sid(), text: String::new(), message_id: None });
        s.apply(ConnectionOutput::Phase(ConnectionPhase::Reconnecting { attempt: 1 }));
        assert!(s.transcript.is_empty());
    }
}
