//! Pure state machine turning connection output into session state.
//! Driven by the harness API's `ApiEvent` (via jcode-sdk), so the app shares
//! one protocol with every other jcode client.

use jcode_sdk::{ApiEvent, HistoryMessage, TurnStopReason};
use std::collections::{HashMap, HashSet};

/// Stored history prefixes assistant text with terminal-specific reasoning
/// markup. Decode only the sentinel-tagged prefix, never ordinary Markdown or
/// examples in the answer. Keep this in sync with jcode-render-core/reasoning.
fn history_assistant_content(mut content: String) -> (String, String) {
    let mut reasoning = String::new();
    let mut consumed = 0;
    let mut tagged = false;
    for line in content.split_inclusive('\n') {
        let trimmed = line.trim_end_matches([' ', '\r', '\n']);
        if let Some(body) = trimmed.strip_prefix("*\u{2063}")
            .and_then(|s| s.strip_suffix("\u{2063}*"))
        {
            tagged = true;
            let mut chars = body.chars().peekable();
            while let Some(ch) = chars.next() {
                if ch == '\\' && chars.peek().is_some_and(|c| "\\*_`[]<>&~|$".contains(*c)) {
                    reasoning.push(chars.next().unwrap());
                } else {
                    reasoning.push(ch);
                }
            }
            if line.ends_with('\n') {
                reasoning.push('\n');
            }
        } else if trimmed.is_empty() {
            reasoning.push('\n');
        } else {
            break;
        }
        consumed += line.len();
    }
    if !tagged {
        return (content, String::new());
    }
    content.drain(..consumed);
    // A long reasoning prefix followed by a short answer should not keep a
    // second large allocation alive after the prefix has been decoded.
    if content.capacity() > 64 * 1024 && content.len() < content.capacity() / 2 {
        content.shrink_to_fit();
    }
    let trimmed_end = reasoning.trim_end_matches('\n').len();
    reasoning.truncate(trimmed_end);
    let leading = reasoning.len() - reasoning.trim_start_matches('\n').len();
    reasoning.drain(..leading);
    (content, reasoning)
}

/// Keep useful prose, not Markdown layout, in the small banner preview.
fn notification_preview(text: &str, limit: usize) -> String {
    let mut lines = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("```") {
            continue;
        }
        let line = line.trim_start_matches('#').trim_start();
        let line = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")).unwrap_or(line);
        let line = line.replace("**", "").replace('`', "");
        let line = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if !line.is_empty() && !lines.contains(&line) {
            lines.push(line);
        }
    }
    let text = lines.join(" · ");
    if text.chars().count() <= limit {
        return text;
    }
    let prefix: String = text.chars().take(limit.saturating_sub(1)).collect();
    // Avoid a dangling half-word, but don't discard most of a long token.
    let end = if text.chars().nth(limit.saturating_sub(1)).is_some_and(char::is_whitespace) {
        prefix.len()
    } else {
        prefix.rfind(char::is_whitespace)
            .filter(|&end| prefix[end..].chars().count() <= 24)
            .unwrap_or(prefix.len())
    };
    format!("{}…", prefix[..end].trim_end_matches([' ', '·']))
}

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AssistantPhase {
    Text,
    Reasoning,
    Tools,
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
    /// Session the alert is about (taps open it, replies go to it).
    pub session_id: Option<String>,
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
    /// Newest-first session list for the sidebar (from `ApiEvent::Sessions`).
    pub session_list: Vec<jcode_sdk::SessionInfo>,
    pub is_processing: bool,
    pub is_reasoning: bool,
    pub model_name: Option<String>,
    pub provider_name: Option<String>,
    pub available_models: Vec<String>,
    /// Every (model, provider, auth) route the session can switch to.
    pub routes: Vec<jcode_sdk::ModelRouteInfo>,
    pub reasoning_effort: Option<String>,
    /// Credential the daemon resolved (`oauth` or `api_key`).
    pub auth_method: Option<String>,
    /// OAuth account the daemon's account router picked for this session.
    pub account_label: Option<String>,
    /// Session directory on the cloud VM.
    pub working_dir: Option<String>,
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
    /// Server todo state (`GetTodoState`), the TUI's pinned card source.
    /// `None` on servers without the capability; the UI then replays tool calls.
    pub todo_state: Option<jcode_sdk::todo::TodoSnapshot>,
    next_id: u64,
    /// Transcript entries that came from the last History snapshot.
    history_len: usize,
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
            session_list: vec![],
            is_processing: false,
            is_reasoning: false,
            model_name: None,
            provider_name: None,
            working_dir: None,
            available_models: vec![],
            routes: vec![],
            reasoning_effort: None,
            auth_method: None,
            account_label: None,
            server_version: None,
            token_input: 0,
            token_output: 0,
            status_detail: None,
            server_phase: None,
            error_banner: None,
            notices: vec![],
            pending_interrupts: vec![],
            alerts: vec![],
            todo_state: None,
            next_id: 1,
            history_len: 0,
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
        let title = self.session_title.as_deref().map(str::trim)
            .filter(|title| !title.is_empty()).unwrap_or("jcode").to_owned();
        let body = notification_preview(&body.into(), 180);
        let session_id = self.session_id.clone();
        let alert = Alert { title, body, session_id };
        if self.alerts.last() != Some(&alert) {
            self.alerts.push(alert);
        }
    }

    fn last_reply_preview(&self) -> String {
        self.transcript
            .iter()
            .rev()
            .take_while(|e| e.role != Role::User || e.is_queued)
            .find(|e| e.role == Role::Assistant && !e.text.trim().is_empty())
            .map(|e| e.text.clone())
            .unwrap_or_else(|| "Finished without a text response".into())
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
            TodoState { session_id, state } => {
                if self.session_id.as_deref().is_none_or(|s| s == session_id) {
                    self.todo_state = Some(state);
                }
            }
            Attached { session } => {
                if self.session_id.as_deref() != Some(session.session_id.as_str()) {
                    self.todo_state = None;
                }
                if let Some(t) = session.title.clone() {
                    self.session_title = Some(t.clone());
                    self.session_titles.insert(session.session_id.clone(), t);
                }
                if let Some(d) = session.working_dir.clone().filter(|d| !d.trim().is_empty()) {
                    self.working_dir = Some(d);
                }
                self.is_processing = crate::connection::session_is_busy(&session.status);
                self.session_id = Some(session.session_id);
            }
            Sessions { sessions } => {
                self.all_sessions = sessions.iter().map(|s| s.session_id.clone()).collect();
                self.session_list = sessions.clone();
                for s in sessions {
                    if let Some(t) = s.title {
                        self.session_titles.insert(s.session_id, t);
                    }
                }
            }
            History { session_id, messages, .. } => self.apply_history(session_id, messages),
            TextDelta { text, .. } => {
                self.is_processing = true;
                self.with_streaming_assistant(AssistantPhase::Text, |e| e.text.push_str(&text))
            }
            TextReplace { text, .. } => self.with_streaming_assistant(AssistantPhase::Text, |e| e.text = text),
            // The text of this response ended; tools may still run, and they
            // update their cards in place.
            TextDone { .. } => self.finish_streaming(),
            ReasoningDelta { text, .. } => {
                self.is_reasoning = true;
                self.with_streaming_assistant(AssistantPhase::Reasoning, |e| e.reasoning.push_str(&text));
            }
            ReasoningDone { .. } => {
                self.is_reasoning = false;
                if self.transcript.last().is_some_and(|e| !e.reasoning.is_empty()) {
                    self.finish_streaming();
                }
            },
            ToolStart { call_id, name, .. } => {
                if self.find_tool(&call_id).is_none() {
                    self.with_streaming_assistant(AssistantPhase::Tools, |e| {
                        e.tool_calls.push(self::ToolCall::new(call_id, name, ToolStatus::StreamingInput))
                    })
                }
            }
            ToolInputDelta { call_id, delta, .. } => match self.find_tool(&call_id) {
                Some(t) => t.input.push_str(&delta),
                None => self.with_streaming_assistant(AssistantPhase::Tools, |e| {
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
                    Some(t) => {
                        t.input = input;
                        fill_name(t, name);
                    }
                    None => self.with_streaming_assistant(AssistantPhase::Tools, |e| {
                        let mut t = self::ToolCall::new(call_id, name, ToolStatus::Running);
                        t.input = input;
                        e.tool_calls.push(t);
                    }),
                }
            }
            ToolExec { call_id, name, .. } => match self.find_tool(&call_id) {
                Some(t) => {
                    t.status = ToolStatus::Running;
                    fill_name(t, name);
                }
                None => self.with_streaming_assistant(AssistantPhase::Tools, |e| {
                    e.tool_calls.push(self::ToolCall::new(call_id, name, ToolStatus::Running))
                }),
            },
            ToolDone { call_id, name, output, error, .. } => {
                let status = error.map(ToolStatus::Failed).unwrap_or(ToolStatus::Succeeded);
                match self.find_tool(&call_id) {
                    Some(t) => {
                        t.output = output;
                        t.status = status;
                        fill_name(t, name);
                    }
                    None => self.with_streaming_assistant(AssistantPhase::Tools, |e| {
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
                "running" | "processing" => self.is_processing = true,
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
            RuntimeInfo { provider, model, reasoning_effort, auth_method, account_label, routes, .. } => {
                if !routes.is_empty() {
                    self.routes = routes;
                }
                // Identity events are authoritative snapshots: an absent
                // account means the route has none (API key, or no OAuth).
                self.auth_method = auth_method;
                self.account_label = account_label;
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
            ModelInfo { provider, model, reasoning_effort, auth_method, account_label, .. } => {
                self.auth_method = auth_method;
                self.account_label = account_label;
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

        // Keep trailing optimistic user messages the server has not seen yet,
        // and, while a turn runs, the output streamed live since the last
        // snapshot. Stored history only holds completed messages, so a turn
        // joined mid-stream would otherwise lose its in-flight reply.
        let live_from = self.history_len.min(self.transcript.len());
        let processing = self.is_processing;
        let tail_len = self.transcript[live_from..]
            .iter()
            .rev()
            .take_while(|e| match e.role {
                Role::User => !e.is_queued,
                Role::Assistant => processing,
                Role::System => false,
            })
            .count();
        let tail: Vec<_> = self.transcript.split_off(self.transcript.len() - tail_len);
        let history_user: HashSet<&str> =
            messages.iter().filter(|m| m.role == "user").map(|m| m.content.as_str()).collect();
        let history_assistant: HashSet<&str> =
            messages.iter().filter(|m| m.role == "assistant").map(|m| m.content.as_str()).collect();
        let history_tools: HashSet<&str> =
            messages.iter().filter_map(|m| m.tool.as_ref()).map(|t| t.id.as_str()).collect();
        let tail: Vec<_> = tail
            .into_iter()
            .filter_map(|mut e| {
                e.tool_calls.retain(|t| !history_tools.contains(t.id.as_str()));
                (e.role == Role::User || !e.tool_calls.is_empty() || !e.text.is_empty() || !e.reasoning.is_empty() || e.is_streaming)
                    .then_some(e)
            })
            .filter(|e| match e.role {
                Role::User => !history_user.contains(e.text.as_str()),
                // A settled live message the snapshot already stored.
                _ => e.is_streaming
                    || !e.tool_calls.is_empty()
                    || e.text.is_empty()
                    || !history_assistant.iter().any(|h| h.ends_with(e.text.as_str())),
            })
            .collect();

        let mut transcript: Vec<TranscriptEntry> = Vec::with_capacity(messages.len() + tail.len());
        for m in messages {
            // Tool rows rebuild the cards a live client saw. Consecutive calls
            // share one entry, matching how live tool events group.
            if m.role == "tool" {
                let Some(tool) = m.tool else { continue };
                let mut card = ToolCall::new(tool.id, tool.name, ToolStatus::Succeeded);
                card.input = match tool.input {
                    serde_json::Value::Null => String::new(),
                    serde_json::Value::String(s) => s,
                    v => serde_json::to_string_pretty(&v).unwrap_or_default(),
                };
                card.output = m.content;
                match transcript.last_mut() {
                    Some(e) if e.role == Role::Assistant && !e.tool_calls.is_empty() => e.tool_calls.push(card),
                    _ => {
                        let mut e = self.entry(Role::Assistant, "");
                        e.tool_calls.push(card);
                        transcript.push(e);
                    }
                }
                continue;
            }
            let role = match m.role.as_str() {
                "user" => Role::User,
                "assistant" => Role::Assistant,
                "system" => Role::System,
                _ => continue,
            };
            if m.content.is_empty() {
                continue;
            }
            let (text, reasoning) = if role == Role::Assistant {
                history_assistant_content(m.content)
            } else {
                (m.content, String::new())
            };
            let mut entry = self.entry(role, text);
            entry.reasoning = reasoning;
            transcript.push(entry);
        }
        self.history_len = transcript.len();
        transcript.extend(tail);
        self.transcript = transcript;
    }

    // A running tool can outlive arbitrarily many text/reasoning phases.
    // Search settled entries too rather than manufacturing a duplicate card.
    fn find_tool(&mut self, id: &str) -> Option<&mut ToolCall> {
        self.transcript
            .iter_mut()
            .rev()
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

    /// Settle the previous phase before appending the next. An entry contains
    /// only one live phase, so renderers cannot move later reasoning or tools
    /// above text that has already arrived. Tool updates bypass this helper.
    fn with_streaming_assistant(
        &mut self,
        phase: AssistantPhase,
        f: impl FnOnce(&mut TranscriptEntry),
    ) {
        self.is_reasoning = phase == AssistantPhase::Reasoning;
        let streaming = self.transcript.last().is_some_and(|e| {
            e.role == Role::Assistant && e.is_streaming
                && match phase {
                    AssistantPhase::Text => e.reasoning.is_empty() && e.tool_calls.is_empty(),
                    AssistantPhase::Reasoning => e.text.is_empty() && e.tool_calls.is_empty(),
                    AssistantPhase::Tools => e.text.is_empty() && e.reasoning.is_empty(),
                }
        });
        if !streaming {
            self.finish_streaming();
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

/// A client that joined mid tool-input never saw `ToolStart` and its name.
fn fill_name(t: &mut ToolCall, name: String) {
    if t.name.is_empty() {
        t.name = name;
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
    fn history_reuses_incoming_text_allocations() {
        for role in ["user", "assistant", "system"] {
            let text = "界 answer **markup**".repeat(512);
            let ptr = text.as_ptr();
            let mut state = SessionState::default();
            state.apply_history(sid(), vec![HistoryMessage {
                response_stats: None, role: role.into(), content: text, tool: None
            }]);
            assert_eq!(state.transcript[0].text.as_ptr(), ptr);
        }
        let content = format!("\n*\u{2063}thought\u{2063}*\n\n{}", "回答".repeat(512));
        let ptr = content.as_ptr();
        let (answer, reasoning) = history_assistant_content(content);
        assert_eq!(answer.as_ptr(), ptr);
        assert_eq!(answer, "回答".repeat(512));
        assert_eq!(reasoning, "thought");
        let (answer, reasoning) = history_assistant_content(format!("*\u{2063}{}\u{2063}*\nOK", "r".repeat(128 * 1024)));
        assert_eq!(answer, "OK");
        assert_eq!(reasoning.len(), 128 * 1024);
        assert!(answer.capacity() <= 64 * 1024);
    }

    #[test]
    #[ignore = "controlled history buffer-transfer benchmark"]
    fn benchmark_history_buffer_transfer() {
        use std::{hint::black_box, time::{Duration, Instant}};
        for bytes in [4096, 65536] {
            let template = "a".repeat(bytes);
            let mut old = Duration::ZERO;
            let mut new = Duration::ZERO;
            for _ in 0..20 {
                // Incoming message creation is outside both timed sections.
                let messages: Vec<_> = (0..250).map(|_| HistoryMessage {
                    response_stats: None, role: "assistant".into(), content: template.clone(), tool: None
                }).collect();
                let start = Instant::now();
                let entries: Vec<_> = messages.iter().map(|m| {
                    let discarded = m.content.clone();
                    black_box(&discarded);
                    m.content.clone()
                }).collect();
                old += start.elapsed();
                black_box(entries);
                let mut state = SessionState::default();
                let start = Instant::now();
                state.apply_history(sid(), messages);
                new += start.elapsed();
                black_box(state);
            }
            eprintln!("history 250x{bytes} bytes: old double-copy only {:.3}ms/load, new full reducer {:.3}ms/load", old.as_secs_f64()*50.0, new.as_secs_f64()*50.0);
        }
    }

    #[test]
    fn attached_history_decodes_reasoning_and_preserves_answer_markdown() {
        let mut s = SessionState::default();
        let content = "*\u{2063}\\*\\*Adding TODO initialization\\*\\*\u{2063}*  \n\n*\u{2063}Checking `path` and C:\\work\u{2063}*  \n\nThe **answer** keeps \\*literal\\* markup.";
        ev(&mut s, E::History { session_id: sid(), messages: vec![HistoryMessage {
            response_stats: None, role: "assistant".into(), content: content.into(), tool: None
        }], images: vec![] });
        assert_eq!(s.transcript[0].reasoning, "**Adding TODO initialization**\n\nChecking `path` and C:\\work");
        assert_eq!(s.transcript[0].text, "The **answer** keeps \\*literal\\* markup.");
        assert!(!s.transcript[0].is_streaming);
    }

    #[test]
    fn history_decoder_preserves_literal_examples_and_partial_wrappers() {
        for content in ["\n\n**ordinary**", "**ordinary**", "*ordinary italic*", "*\u{2063}unfinished", "```\n*\u{2063}example\u{2063}*\n```", "Answer\n*\u{2063}example\u{2063}*"] {
            assert_eq!(history_assistant_content(content.into()), (content.into(), String::new()));
        }
        assert_eq!(history_assistant_content("*\u{2063}é文字 \\[docs\\] \\\\ \\*\u{2063}*".into()),
            (String::new(), "é文字 [docs] \\ *".into()));
        assert_eq!(history_assistant_content("\n\n*\u{2063}Thought\u{2063}*  \r\n\nAnswer".into()),
            ("Answer".into(), "Thought".into()));
    }

    #[test]
    fn user_history_is_not_interpreted_as_reasoning() {
        let mut s = SessionState::default();
        let content = "*\u{2063}literal\u{2063}*";
        ev(&mut s, E::History { session_id: sid(), messages: vec![HistoryMessage {
            response_stats: None, role: "user".into(), content: content.into(), tool: None
        }], images: vec![] });
        assert_eq!(s.transcript[0].text, content);
        assert!(s.transcript[0].reasoning.is_empty());
    }

    #[test]
    fn notification_preview_removes_layout_and_repeated_lines() {
        assert_eq!(notification_preview("## Fixed login\n\n- **Tests:** `12 passed`\n- **Tests:** `12 passed`", 180),
            "Fixed login · Tests: 12 passed");
        let preview = notification_preview(&"界".repeat(200), 180);
        assert_eq!(preview.chars().count(), 180);
        assert!(preview.ends_with('…'));
        assert_eq!(notification_preview("one two three four", 14), "one two three…");
    }

    #[test]
    fn notifications_do_not_reuse_previous_turn_reply() {
        let mut s = SessionState::default();
        s.apply_intent(LocalIntent::UserSentMessage("first".into()));
        ev(&mut s, E::TextDelta { session_id: sid(), text: "Old result".into(), message_id: None });
        ev(&mut s, E::TurnDone { session_id: sid(), pending_soft_interrupts: None });
        s.apply_intent(LocalIntent::UserSentMessage("second".into()));
        ev(&mut s, E::TurnDone { session_id: sid(), pending_soft_interrupts: None });
        assert_eq!(s.alerts[1].body, "Finished without a text response");
    }

    #[test]
    fn queued_followup_does_not_hide_current_result() {
        let mut s = SessionState::default();
        s.apply_intent(LocalIntent::UserSentMessage("first".into()));
        ev(&mut s, E::TextDelta { session_id: sid(), text: "Current result".into(), message_id: None });
        s.apply_intent(LocalIntent::UserQueuedInterrupt("next".into()));
        ev(&mut s, E::TurnDone { session_id: sid(), pending_soft_interrupts: None });
        assert_eq!(s.alerts[0].body, "Current result");
    }

    #[test]
    fn pending_duplicate_alerts_are_coalesced() {
        let mut s = SessionState { session_title: Some("  ".into()), ..Default::default() };
        s.alert("Same update");
        s.alert("Same update");
        assert_eq!(s.alerts.len(), 1);
        assert_eq!(s.alerts[0].title, "jcode");
        s.alerts.clear();
        s.alert("Same update");
        assert_eq!(s.alerts.len(), 1, "a later event must still be deliverable");
    }

    #[test]
    fn streams_text_and_tools_into_ordered_entries() {
        let mut s = SessionState::default();
        s.apply_intent(LocalIntent::UserSentMessage("hi".into()));
        ev(&mut s, E::TextDelta { session_id: sid(), text: "Hel".into(), message_id: None });
        ev(&mut s, E::TextDelta { session_id: sid(), text: "lo".into(), message_id: None });
        ev(&mut s, E::ToolStart { session_id: sid(), call_id: "t1".into(), name: "bash".into() });
        ev(&mut s, E::ToolInputDelta { session_id: sid(), call_id: "t1".into(), delta: "ls".into() });
        ev(&mut s, E::ToolDone { session_id: sid(), call_id: "t1".into(), name: "bash".into(), output: "ok".into(), error: None });
        ev(&mut s, E::TurnDone { session_id: sid(), pending_soft_interrupts: None });
        assert_eq!(s.transcript.len(), 3);
        let a = &s.transcript[1];
        assert_eq!(a.text, "Hello");
        assert!(!a.is_streaming);
        assert!(a.tool_calls.is_empty());
        assert_eq!(s.transcript[2].tool_calls[0].input, "ls");
        assert_eq!(s.transcript[2].tool_calls[0].status, ToolStatus::Succeeded);
        assert!(!s.is_processing);
        assert_eq!(s.alerts.len(), 1);
        assert_eq!(s.alerts[0].body, "Hello");
        ev(&mut s, E::TurnDone { session_id: sid(), pending_soft_interrupts: None });
        assert_eq!(s.alerts.len(), 1, "idle TurnDone must not re-alert");
    }

    fn text(s: &mut SessionState, value: &str) {
        ev(s, E::TextDelta { session_id: sid(), text: value.into(), message_id: None });
    }

    fn reasoning(s: &mut SessionState, value: &str) {
        ev(s, E::ReasoningDelta { session_id: sid(), text: value.into() });
    }

    #[test]
    fn resumed_reasoning_settles_text_in_event_order() {
        let mut s = SessionState::default();
        reasoning(&mut s, "first");
        reasoning(&mut s, " thought");
        text(&mut s, "answer");
        assert!(!s.is_reasoning, "text settles reasoning without ReasoningDone");
        reasoning(&mut s, "second");
        assert_eq!(s.transcript.len(), 3);
        assert_eq!(s.transcript[0].reasoning, "first thought");
        assert_eq!(s.transcript[1].text, "answer");
        assert_eq!(s.transcript[2].reasoning, "second");
        assert!(!s.transcript[0].is_streaming);
        assert!(!s.transcript[1].is_streaming);
        assert!(s.transcript[2].is_streaming);
        ev(&mut s, E::ReasoningDone { session_id: sid(), duration_secs: None });
        assert!(!s.is_reasoning);
        assert!(!s.transcript[2].is_streaming);
        reasoning(&mut s, "third");
        assert_eq!(s.transcript.len(), 4);
        text(&mut s, "final");
        ev(&mut s, E::TurnDone { session_id: sid(), pending_soft_interrupts: None });
        assert_eq!(s.transcript[4].text, "final");
        assert!(s.transcript.iter().all(|e| !e.is_streaming));
    }

    #[test]
    fn old_tool_updates_do_not_reorder_or_split_current_phase() {
        let mut s = SessionState::default();
        text(&mut s, "before tool");
        ev(&mut s, E::ToolStart { session_id: sid(), call_id: "old".into(), name: "bash".into() });
        assert!(!s.is_reasoning);
        assert_eq!(s.transcript[0].text, "before tool");
        assert!(s.transcript[0].tool_calls.is_empty());
        assert_eq!(s.transcript[1].tool_calls[0].id, "old");
        for _ in 0..6 {
            reasoning(&mut s, "thinking");
            text(&mut s, "answer");
        }
        let len = s.transcript.len();
        let current_id = s.transcript.last().unwrap().id;
        ev(&mut s, E::ToolStart { session_id: sid(), call_id: "old".into(), name: "bash".into() });
        ev(&mut s, E::ToolInputDelta { session_id: sid(), call_id: "old".into(), delta: "draft".into() });
        ev(&mut s, E::ToolCall { session_id: sid(), call_id: "old".into(), name: "bash".into(), input: serde_json::json!({"command": "ls"}) });
        ev(&mut s, E::ToolExec { session_id: sid(), call_id: "old".into(), name: "bash".into() });
        ev(&mut s, E::ToolDone { session_id: sid(), call_id: "old".into(), name: "bash".into(), output: "failed output".into(), error: Some("failure".into()) });
        text(&mut s, " continued");
        assert_eq!(s.transcript.len(), len);
        assert_eq!(s.transcript.last().unwrap().id, current_id);
        assert_eq!(s.transcript.last().unwrap().text, "answer continued");
        let tools: Vec<_> = s.transcript.iter().flat_map(|e| &e.tool_calls).collect();
        assert_eq!(tools.len(), 1);
        assert!(tools[0].input.contains("command"));
        assert_eq!(tools[0].output, "failed output");
        assert_eq!(tools[0].status, ToolStatus::Failed("failure".into()));
    }

    #[test]
    fn replacement_only_replaces_current_text_phase() {
        let mut s = SessionState::default();
        reasoning(&mut s, "first");
        text(&mut s, "draft");
        let text_id = s.transcript[1].id;
        ev(&mut s, E::TextReplace { session_id: sid(), text: "replacement".into(), message_id: None });
        assert_eq!(s.transcript[1].id, text_id);
        assert_eq!(s.transcript[1].text, "replacement");
        reasoning(&mut s, "second");
        ev(&mut s, E::TextReplace { session_id: sid(), text: "new phase".into(), message_id: None });
        assert_eq!(s.transcript.len(), 4);
        assert_eq!(s.transcript[0].reasoning, "first");
        assert_eq!(s.transcript[1].text, "replacement");
        assert_eq!(s.transcript[2].reasoning, "second");
        assert_eq!(s.transcript[3].text, "new phase");
        ev(&mut s, E::TextReplace { session_id: sid(), text: String::new(), message_id: None });
        ev(&mut s, E::TextDone { session_id: sid(), message_id: None });
        assert_eq!(s.transcript.len(), 3, "empty replaced phase is removed");
        assert_eq!(s.transcript[1].text, "replacement");
    }

    #[test]
    fn tools_settle_reasoning_and_late_updates_keep_reasoning_active() {
        let mut s = SessionState::default();
        reasoning(&mut s, "before");
        for id in ["one", "two"] {
            ev(&mut s, E::ToolStart { session_id: sid(), call_id: id.into(), name: "bash".into() });
            assert!(!s.is_reasoning, "tool phase ends the reasoning orb");
        }
        assert_eq!(s.transcript.len(), 2);
        assert!(!s.transcript[0].is_streaming);
        assert_eq!(s.transcript[1].tool_calls.len(), 2);
        reasoning(&mut s, "after");
        ev(&mut s, E::ToolDone { session_id: sid(), call_id: "one".into(), name: "bash".into(), output: "ok".into(), error: None });
        assert!(s.is_reasoning, "old tool completion is not a new phase");
        assert_eq!(s.transcript.len(), 3);
        assert_eq!(s.transcript[2].reasoning, "after");
        assert!(s.transcript[2].is_streaming);
    }

    #[test]
    fn missing_tool_start_still_settles_text_for_every_tool_event() {
        for event in [
            E::ToolInputDelta { session_id: sid(), call_id: "t".into(), delta: "input".into() },
            E::ToolCall { session_id: sid(), call_id: "t".into(), name: "bash".into(), input: serde_json::json!("input") },
            E::ToolExec { session_id: sid(), call_id: "t".into(), name: "bash".into() },
            E::ToolDone { session_id: sid(), call_id: "t".into(), name: "bash".into(), output: "done".into(), error: None },
        ] {
            let mut s = SessionState::default();
            text(&mut s, "before");
            ev(&mut s, event);
            reasoning(&mut s, "after");
            assert_eq!(s.transcript.len(), 3);
            assert_eq!(s.transcript[0].text, "before");
            assert_eq!(s.transcript[1].tool_calls.len(), 1);
            assert_eq!(s.transcript[2].reasoning, "after");
        }
    }

    #[test]
    fn cancellation_and_history_replace_settled_phases_cleanly() {
        let mut s = SessionState::default();
        reasoning(&mut s, "first");
        text(&mut s, "answer");
        reasoning(&mut s, "second");
        ev(&mut s, E::SessionStatus { session_id: sid(), status: "cancelled".into(), pending_soft_interrupts: None });
        assert!(!s.is_processing);
        assert!(!s.is_reasoning);
        assert_eq!(s.transcript.len(), 3);
        assert!(s.transcript.iter().all(|e| !e.is_streaming));
        ev(&mut s, E::History { session_id: sid(), messages: vec![HistoryMessage {
            response_stats: None, role: "assistant".into(), content: "persisted answer".into(), tool: None
        }], images: vec![] });
        assert_eq!(s.transcript.len(), 1);
        assert_eq!(s.transcript[0].text, "persisted answer");
        text(&mut s, "next response");
        assert_eq!(s.transcript.len(), 2);
        assert_eq!(s.transcript[0].text, "persisted answer");
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

    fn hm(role: &str, content: &str) -> HistoryMessage {
        HistoryMessage { response_stats: None, role: role.into(), content: content.into(), tool: None }
    }

    fn attach_running(s: &mut SessionState) {
        let session: jcode_sdk::SessionInfo =
            serde_json::from_value(serde_json::json!({"session_id":"s","status":"processing"})).unwrap();
        ev(s, E::Attached { session });
    }

    /// Attaching to a running turn: live deltas can arrive before the history
    /// snapshot. The snapshot must not drop them, nor be dropped itself.
    #[test]
    fn history_after_live_deltas_keeps_both() {
        let mut s = SessionState::default();
        attach_running(&mut s);
        assert!(s.is_processing, "an attached 'processing' session is busy");
        text(&mut s, " and over long spans");
        ev(&mut s, E::History {
            session_id: sid(),
            messages: vec![hm("user", "earlier"), hm("assistant", "done"), hm("user", "write about rivers")],
            images: vec![],
        });
        let texts: Vec<_> = s.transcript.iter().map(|e| e.text.as_str()).collect();
        assert_eq!(texts, ["earlier", "done", "write about rivers", " and over long spans"]);
        assert!(s.transcript[3].is_streaming);
        text(&mut s, " of time");
        assert_eq!(s.transcript.len(), 4);
        assert_eq!(s.transcript[3].text, " and over long spans of time");
        // A later full snapshot during the turn replaces history but keeps the
        // live tail exactly once.
        ev(&mut s, E::History {
            session_id: sid(),
            messages: vec![hm("user", "earlier"), hm("assistant", "done"), hm("user", "write about rivers")],
            images: vec![],
        });
        assert_eq!(s.transcript.len(), 4);
        // After the turn the authoritative history replaces the truncated tail.
        ev(&mut s, E::TurnDone { session_id: sid(), pending_soft_interrupts: None });
        ev(&mut s, E::History {
            session_id: sid(),
            messages: vec![
                hm("user", "earlier"),
                hm("assistant", "done"),
                hm("user", "write about rivers"),
                hm("assistant", "Rivers carve stone and over long spans of time"),
            ],
            images: vec![],
        });
        let texts: Vec<_> = s.transcript.iter().map(|e| e.text.as_str()).collect();
        assert_eq!(texts, ["earlier", "done", "write about rivers", "Rivers carve stone and over long spans of time"]);
    }

    /// Live tool cards streamed before the snapshot survive it.
    #[test]
    fn history_keeps_live_tools_of_running_turn() {
        let mut s = SessionState::default();
        attach_running(&mut s);
        ev(&mut s, E::ToolStart { session_id: sid(), call_id: "t1".into(), name: "bash".into() });
        ev(&mut s, E::History { session_id: sid(), messages: vec![hm("user", "go")], images: vec![] });
        assert_eq!(s.transcript.len(), 2);
        assert_eq!(s.transcript[1].tool_calls.len(), 1);
        ev(&mut s, E::ToolDone { session_id: sid(), call_id: "t1".into(), name: "bash".into(), output: "x".into(), error: None });
        assert_eq!(s.transcript.len(), 2);
        assert_eq!(s.transcript[1].tool_calls[0].status, ToolStatus::Succeeded);
    }

    /// A settled live message the snapshot also contains is not duplicated.
    #[test]
    fn history_drops_live_text_it_already_stores() {
        let mut s = SessionState::default();
        attach_running(&mut s);
        text(&mut s, "I'll run it now.");
        ev(&mut s, E::TextDone { session_id: sid(), message_id: None });
        ev(&mut s, E::History {
            session_id: sid(),
            messages: vec![hm("user", "go"), hm("assistant", "I'll run it now.")],
            images: vec![],
        });
        let texts: Vec<_> = s.transcript.iter().map(|e| e.text.as_str()).collect();
        assert_eq!(texts, ["go", "I'll run it now."]);
    }

    /// Idle snapshot: stale assistant entries are replaced, never kept.
    #[test]
    fn idle_history_replaces_assistant_entries() {
        let mut s = SessionState::default();
        text(&mut s, "stale");
        ev(&mut s, E::TurnDone { session_id: sid(), pending_soft_interrupts: None });
        ev(&mut s, E::History { session_id: sid(), messages: vec![hm("assistant", "fresh")], images: vec![] });
        let texts: Vec<_> = s.transcript.iter().map(|e| e.text.as_str()).collect();
        assert_eq!(texts, ["fresh"]);
    }

    #[test]
    fn tool_joined_mid_input_gets_its_name() {
        let mut s = SessionState::default();
        attach_running(&mut s);
        ev(&mut s, E::ToolInputDelta { session_id: sid(), call_id: "t1".into(), delta: "{}".into() });
        ev(&mut s, E::ToolExec { session_id: sid(), call_id: "t1".into(), name: "bash".into() });
        assert_eq!(s.transcript[0].tool_calls[0].name, "bash");
    }

    #[test]
    fn history_tool_rows_rebuild_cards() {
        let mut s = SessionState::default();
        let tool = |id: &str, out: &str| HistoryMessage {
            response_stats: None,
            role: "tool".into(),
            content: out.into(),
            tool: Some(jcode_sdk::HistoryTool { id: id.into(), name: "bash".into(), input: serde_json::json!({"command":"ls"}) }),
        };
        ev(&mut s, E::History {
            session_id: sid(),
            messages: vec![hm("user", "go"), hm("assistant", "on it"), tool("a", "x"), tool("b", "y"), hm("assistant", "done"),
                // A row from an older server without tool identity is skipped.
                hm("tool", "orphan")],
            images: vec![],
        });
        assert_eq!(s.transcript.len(), 4);
        let cards = &s.transcript[2].tool_calls;
        assert_eq!(cards.iter().map(|c| (c.id.as_str(), c.output.as_str())).collect::<Vec<_>>(), [("a", "x"), ("b", "y")]);
        assert!(cards[0].input.contains("ls"));
        assert_eq!(s.transcript[3].text, "done");
    }

    #[test]
    fn processing_status_event_marks_busy() {
        let mut s = SessionState::default();
        ev(&mut s, E::SessionStatus { session_id: sid(), status: "processing".into(), pending_soft_interrupts: None });
        assert!(s.is_processing);
    }

    #[test]
    fn history_keeps_unseen_optimistic_tail() {
        let mut s = SessionState::default();
        s.apply_intent(LocalIntent::UserSentMessage("new one".into()));
        let old = HistoryMessage { response_stats: None, role: "user".into(), content: "old".into(), tool: None };
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
