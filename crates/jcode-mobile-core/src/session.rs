//! Pure state machine turning connection output into session state.
//! Line-for-line port of `ios/Sources/JCodeKit/SessionReducer.swift` so both
//! apps behave identically.

use crate::wire::{HistoryPayload, ServerEvent};
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
    Event(ServerEvent),
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

    fn turn_over(&mut self) {
        self.is_processing = false;
        self.is_reasoning = false;
        self.server_phase = None;
        self.finish_streaming();
    }

    fn apply_event(&mut self, event: ServerEvent) {
        use ServerEvent::*;
        match event {
            TextDelta { text } => self.with_streaming_assistant(|e| e.text.push_str(&text)),
            ReasoningDelta { text } => {
                self.is_reasoning = true;
                self.with_streaming_assistant(|e| e.reasoning.push_str(&text));
            }
            ReasoningDone { .. } => self.is_reasoning = false,
            TextReplace { text } => self.with_streaming_assistant(|e| e.text = text),
            ToolStart { id, name } => self.with_streaming_assistant(|e| {
                e.tool_calls.push(ToolCall::new(id, name, ToolStatus::StreamingInput))
            }),
            ToolInput { delta } => self.with_streaming_assistant(|e| {
                if let Some(t) = e.tool_calls.last_mut() {
                    t.input.push_str(&delta);
                }
            }),
            // The server can send MessageEnd before ToolDone (the turn's text
            // ends while the tool still runs), so update a known call in place
            // instead of opening a fresh assistant entry for it.
            ToolExec { id, .. } if self.find_tool(&id).is_some() => {
                self.find_tool(&id).unwrap().status = ToolStatus::Running;
            }
            ToolDone { id, output, error, .. } if self.find_tool(&id).is_some() => {
                let t = self.find_tool(&id).unwrap();
                t.output = output;
                t.status = error.map(ToolStatus::Failed).unwrap_or(ToolStatus::Succeeded);
            }
            ToolExec { id, name } => self.with_streaming_assistant(|e| {
                match e.tool_calls.iter_mut().find(|t| t.id == id) {
                    Some(t) => t.status = ToolStatus::Running,
                    None => e.tool_calls.push(ToolCall::new(id, name, ToolStatus::Running)),
                }
            }),
            ToolDone { id, name, output, error } => self.with_streaming_assistant(|e| {
                let status = error.map(ToolStatus::Failed).unwrap_or(ToolStatus::Succeeded);
                match e.tool_calls.iter_mut().find(|t| t.id == id) {
                    Some(t) => {
                        t.output = output;
                        t.status = status;
                    }
                    None => {
                        let mut t = ToolCall::new(id, name, status);
                        t.output = output;
                        e.tool_calls.push(t);
                    }
                }
            }),
            MessageEnd => self.finish_streaming(),
            ConnectionPhase { phase } => {
                self.server_phase = if phase.is_empty() { None } else { Some(phase) }
            }
            SoftInterruptInjected { content, display_role, .. } => {
                self.resolve_pending_interrupt(content, display_role)
            }
            RetryRollback { attempt, max } => {
                if self.transcript.last().is_some_and(|e| e.is_streaming) {
                    self.transcript.pop();
                }
                self.is_reasoning = false;
                self.status_detail = Some(format!("Retrying ({attempt}/{max})"));
            }
            Done { .. } => {
                self.turn_over();
                self.drain_pending_interrupts();
            }
            Interrupted => {
                self.turn_over();
                self.drain_pending_interrupts();
                self.notice(NoticeKind::Info, "Interrupted");
            }
            Error { message, retry_after_secs, .. } => {
                self.turn_over();
                self.error_banner = Some(match retry_after_secs {
                    Some(r) => format!("{message} (retry in {r}s)"),
                    None => message,
                });
            }
            TokenUsage { input, output } => {
                self.token_input = input;
                self.token_output = output;
            }
            StatusDetail { detail } => self.status_detail = Some(detail),
            State { session_id, is_processing, .. } => {
                self.session_id = Some(session_id);
                self.is_processing = is_processing;
            }
            SessionId { session_id } => self.session_id = Some(session_id),
            SessionRenamed { session_id, display_title } => {
                if self.session_id.is_none() || self.session_id.as_deref() == Some(&session_id) {
                    self.session_title = Some(display_title.clone());
                }
                self.session_titles.insert(session_id, display_title);
            }
            History(payload) => self.apply_history(payload),
            ModelChanged { model, error, .. } => match error {
                Some(e) => self.error_banner = Some(e),
                None => self.model_name = Some(model),
            },
            ReasoningEffortChanged { effort, error, .. } => match error {
                Some(e) => self.error_banner = Some(e),
                None => self.reasoning_effort = effort,
            },
            CompactResult { message, success, .. } => {
                if success {
                    self.notice(NoticeKind::Compaction, message)
                } else {
                    self.error_banner = Some(message)
                }
            }
            AvailableModelsUpdated { models, provider_model } => {
                self.available_models = models;
                if provider_model.is_some() {
                    self.model_name = provider_model;
                }
            }
            Compaction { trigger, tokens_saved } => {
                if let Some(saved) = tokens_saved
                    && trigger != "background"
                {
                    self.notice(
                        NoticeKind::Compaction,
                        format!("Context compacted ({saved} tokens saved)"),
                    );
                }
            }
            Notification { from_name, message } => {
                let prefix = from_name.map(|n| format!("{n}: ")).unwrap_or_default();
                self.notice(NoticeKind::Notification, prefix + &message);
            }
            Reloading { .. } => {
                self.notice(NoticeKind::Info, "Server is updating, reconnecting shortly")
            }
            SessionCloseRequested { reason } => {
                self.is_processing = false;
                self.is_reasoning = false;
                self.finish_streaming();
                self.error_banner = Some(if reason.is_empty() {
                    "Server closed this session".into()
                } else {
                    reason
                });
            }
            Ack { .. } | Pong { .. } | Unknown { .. } => {}
        }
    }

    fn apply_history(&mut self, p: HistoryPayload) {
        self.session_id = Some(p.session_id.clone());
        if p.provider_name.is_some() {
            self.provider_name = p.provider_name;
        }
        if p.provider_model.is_some() {
            self.model_name = p.provider_model;
        }
        if !p.available_models.is_empty() {
            self.available_models = p.available_models;
        }
        if !p.all_sessions.is_empty() {
            self.all_sessions = p.all_sessions;
        }
        if p.server_version.is_some() {
            self.server_version = p.server_version;
        }
        if let Some(t) = &p.display_title {
            self.session_title = Some(t.clone());
            self.session_titles.insert(p.session_id.clone(), t.clone());
        }
        if p.reasoning_effort.is_some() {
            self.reasoning_effort = p.reasoning_effort;
        }
        self.pending_interrupts.clear();
        if let Some((i, o)) = p.total_tokens {
            self.token_input = i;
            self.token_output = o;
        }

        // Keep trailing optimistic user messages the server has not seen yet.
        let tail_len = self
            .transcript
            .iter()
            .rev()
            .take_while(|e| e.role == Role::User && !e.is_queued)
            .count();
        let tail: Vec<_> = self.transcript.split_off(self.transcript.len() - tail_len);
        let history_user: HashSet<&str> =
            p.messages.iter().filter(|m| m.role == "user").map(|m| m.content.as_str()).collect();
        let tail: Vec<_> =
            tail.into_iter().filter(|e| !history_user.contains(e.text.as_str())).collect();

        let mut transcript = Vec::with_capacity(p.messages.len() + tail.len());
        for m in &p.messages {
            let role = match m.role.as_str() {
                "user" => Role::User,
                "assistant" => Role::Assistant,
                "system" => Role::System,
                _ => continue,
            };
            let tool_calls: Vec<ToolCall> = match &m.tool_data {
                Some(d) => vec![ToolCall {
                    id: d.id.clone(),
                    name: d.name.clone(),
                    input: d.input.clone(),
                    output: d.output.clone().unwrap_or_default(),
                    status: match (&d.error, &d.output) {
                        (Some(e), _) => ToolStatus::Failed(e.clone()),
                        (None, Some(_)) => ToolStatus::Succeeded,
                        (None, None) => ToolStatus::Running,
                    },
                }],
                None => m
                    .tool_calls
                    .iter()
                    .map(|n| ToolCall::new(n.clone(), n.clone(), ToolStatus::Succeeded))
                    .collect(),
            };
            if m.content.is_empty() && tool_calls.is_empty() {
                continue;
            }
            let mut e = self.entry(role, m.content.clone());
            e.tool_calls = tool_calls;
            transcript.push(e);
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

    fn resolve_pending_interrupt(&mut self, content: String, display_role: Option<String>) {
        if let Some(i) = self.pending_interrupts.iter().position(|p| *p == content) {
            self.pending_interrupts.remove(i);
        }
        let idx = self
            .transcript
            .iter()
            .position(|e| e.is_queued && e.text == content)
            .or_else(|| self.transcript.iter().position(|e| e.is_queued));
        match idx {
            Some(i) => self.transcript[i].is_queued = false,
            None => {
                let role =
                    if display_role.as_deref() == Some("system") { Role::System } else { Role::User };
                let e = self.entry(role, content);
                self.transcript.push(e);
            }
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
    use crate::wire::{HistoryMessage, ServerEvent as E};

    fn ev(s: &mut SessionState, e: E) {
        s.apply(ConnectionOutput::Event(e));
    }

    #[test]
    fn streams_text_and_tools_into_one_entry() {
        let mut s = SessionState::default();
        s.apply_intent(LocalIntent::UserSentMessage("hi".into()));
        ev(&mut s, E::TextDelta { text: "Hel".into() });
        ev(&mut s, E::TextDelta { text: "lo".into() });
        ev(&mut s, E::ToolStart { id: "t1".into(), name: "bash".into() });
        ev(&mut s, E::ToolInput { delta: "ls".into() });
        ev(&mut s, E::ToolDone { id: "t1".into(), name: "bash".into(), output: "ok".into(), error: None });
        ev(&mut s, E::Done { id: 1 });
        assert_eq!(s.transcript.len(), 2);
        let a = &s.transcript[1];
        assert_eq!(a.text, "Hello");
        assert!(!a.is_streaming);
        assert_eq!(a.tool_calls[0].input, "ls");
        assert_eq!(a.tool_calls[0].status, ToolStatus::Succeeded);
        assert!(!s.is_processing);
    }

    #[test]
    fn tool_done_after_message_end_updates_existing_card() {
        // Real server order: tool_start, tool_exec, message_end, tool_done.
        let mut s = SessionState::default();
        s.apply_intent(LocalIntent::UserSentMessage("hi".into()));
        ev(&mut s, E::ToolStart { id: "t1".into(), name: "bash".into() });
        ev(&mut s, E::ToolExec { id: "t1".into(), name: "bash".into() });
        ev(&mut s, E::MessageEnd);
        ev(&mut s, E::ToolDone { id: "t1".into(), name: "bash".into(), output: "hi".into(), error: None });
        ev(&mut s, E::TextDelta { text: "ok".into() });
        let tools: Vec<_> = s.transcript.iter().flat_map(|e| &e.tool_calls).collect();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].status, ToolStatus::Succeeded);
        assert_eq!(tools[0].output, "hi");
    }

    #[test]
    fn retry_rollback_discards_partial_output() {
        let mut s = SessionState::default();
        ev(&mut s, E::TextDelta { text: "partial".into() });
        ev(&mut s, E::RetryRollback { attempt: 1, max: 3 });
        assert!(s.transcript.is_empty());
        assert_eq!(s.status_detail.as_deref(), Some("Retrying (1/3)"));
    }

    #[test]
    fn history_keeps_unseen_optimistic_tail() {
        let mut s = SessionState::default();
        s.apply_intent(LocalIntent::UserSentMessage("new one".into()));
        ev(
            &mut s,
            E::History(HistoryPayload {
                session_id: "s".into(),
                messages: vec![HistoryMessage {
                    role: "user".into(),
                    content: "old".into(),
                    tool_calls: vec![],
                    tool_data: None,
                }],
                ..Default::default()
            }),
        );
        let texts: Vec<_> = s.transcript.iter().map(|e| e.text.as_str()).collect();
        assert_eq!(texts, ["old", "new one"]);
    }

    #[test]
    fn queued_interrupt_resolves_on_injection() {
        let mut s = SessionState::default();
        s.apply_intent(LocalIntent::UserQueuedInterrupt("stop".into()));
        assert!(s.has_pending_interrupts());
        ev(
            &mut s,
            E::SoftInterruptInjected {
                content: "stop".into(),
                display_role: None,
                point: "x".into(),
                tools_skipped: None,
            },
        );
        assert!(!s.has_pending_interrupts());
        assert!(!s.transcript[0].is_queued);
    }

    #[test]
    fn disconnect_drops_empty_stub() {
        let mut s = SessionState::default();
        ev(&mut s, E::TextReplace { text: String::new() });
        s.apply(ConnectionOutput::Phase(ConnectionPhase::Reconnecting { attempt: 1 }));
        assert!(s.transcript.is_empty());
    }
}
