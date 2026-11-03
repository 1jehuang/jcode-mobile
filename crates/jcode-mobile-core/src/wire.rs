//! Wire protocol. Mirrors `jcode-protocol/src/wire.rs` (`#[serde(tag = "type")]`,
//! snake_case tags), but only the subset the phone uses, decoded leniently so a
//! newer or older server never breaks the app.

use serde_json::{Map, Value, json};

#[derive(Debug, Clone, PartialEq)]
pub enum Request {
    Subscribe { id: u64, target_session_id: Option<String>, working_dir: Option<String> },
    Message { id: u64, content: String },
    Cancel { id: u64 },
    SoftInterrupt { id: u64, content: String, urgent: bool },
    CancelSoftInterrupts { id: u64 },
    Ping { id: u64 },
    GetHistory { id: u64 },
    ResumeSession { id: u64, session_id: String },
    SetModel { id: u64, model: String },
    SetReasoningEffort { id: u64, effort: String },
    Compact { id: u64 },
    RenameSession { id: u64, title: Option<String> },
    Clear { id: u64 },
}

impl Request {
    pub fn id(&self) -> u64 {
        use Request::*;
        match self {
            Subscribe { id, .. } | Message { id, .. } | Cancel { id } | SoftInterrupt { id, .. }
            | CancelSoftInterrupts { id } | Ping { id } | GetHistory { id }
            | ResumeSession { id, .. } | SetModel { id, .. } | SetReasoningEffort { id, .. }
            | Compact { id } | RenameSession { id, .. } | Clear { id } => *id,
        }
    }

    /// One JSON line, no trailing newline. Keys are sorted (serde_json's Map is a BTreeMap).
    pub fn encoded_line(&self) -> String {
        use Request::*;
        let mut o = Map::new();
        o.insert("id".into(), json!(self.id()));
        let mut put = |k: &str, v: Value| {
            o.insert(k.into(), v);
        };
        match self {
            Subscribe { target_session_id, working_dir, .. } => {
                put("type", json!("subscribe"));
                if let Some(s) = target_session_id {
                    put("target_session_id", json!(s));
                }
                if let Some(w) = working_dir {
                    put("working_dir", json!(w));
                }
            }
            Message { content, .. } => {
                put("type", json!("message"));
                put("content", json!(content));
            }
            Cancel { .. } => put("type", json!("cancel")),
            SoftInterrupt { content, urgent, .. } => {
                put("type", json!("soft_interrupt"));
                put("content", json!(content));
                put("urgent", json!(urgent));
            }
            CancelSoftInterrupts { .. } => put("type", json!("cancel_soft_interrupts")),
            Ping { .. } => put("type", json!("ping")),
            GetHistory { .. } => put("type", json!("get_history")),
            ResumeSession { session_id, .. } => {
                put("type", json!("resume_session"));
                put("session_id", json!(session_id));
            }
            SetModel { model, .. } => {
                put("type", json!("set_model"));
                put("model", json!(model));
            }
            SetReasoningEffort { effort, .. } => {
                put("type", json!("set_reasoning_effort"));
                put("effort", json!(effort));
            }
            Compact { .. } => put("type", json!("compact")),
            RenameSession { title, .. } => {
                put("type", json!("rename_session"));
                if let Some(t) = title {
                    put("title", json!(t));
                }
            }
            Clear { .. } => put("type", json!("clear")),
        }
        Value::Object(o).to_string()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolCallRecord {
    pub id: String,
    pub name: String,
    pub input: String,
    pub output: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HistoryMessage {
    pub role: String,
    pub content: String,
    pub tool_calls: Vec<String>,
    pub tool_data: Option<ToolCallRecord>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct HistoryPayload {
    pub id: u64,
    pub session_id: String,
    pub messages: Vec<HistoryMessage>,
    pub provider_name: Option<String>,
    pub provider_model: Option<String>,
    pub available_models: Vec<String>,
    pub total_tokens: Option<(u64, u64)>,
    pub all_sessions: Vec<String>,
    pub server_version: Option<String>,
    pub display_title: Option<String>,
    pub reasoning_effort: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ServerEvent {
    Ack { id: u64 },
    TextDelta { text: String },
    ReasoningDelta { text: String },
    ReasoningDone { duration_secs: Option<f64> },
    TextReplace { text: String },
    ToolStart { id: String, name: String },
    ToolInput { delta: String },
    ToolExec { id: String, name: String },
    ToolDone { id: String, name: String, output: String, error: Option<String> },
    TokenUsage { input: u64, output: u64 },
    StatusDetail { detail: String },
    ConnectionPhase { phase: String },
    SoftInterruptInjected { content: String, display_role: Option<String>, point: String, tools_skipped: Option<i64> },
    RetryRollback { attempt: i64, max: i64 },
    MessageEnd,
    Interrupted,
    Done { id: u64 },
    Error { id: u64, message: String, retry_after_secs: Option<u64> },
    Pong { id: u64 },
    State { id: u64, session_id: String, message_count: i64, is_processing: bool },
    SessionId { session_id: String },
    SessionRenamed { session_id: String, display_title: String },
    History(HistoryPayload),
    ModelChanged { id: u64, model: String, error: Option<String> },
    ReasoningEffortChanged { id: u64, effort: Option<String>, error: Option<String> },
    CompactResult { id: u64, message: String, success: bool },
    AvailableModelsUpdated { models: Vec<String>, provider_model: Option<String> },
    Compaction { trigger: String, tokens_saved: Option<u64> },
    Notification { from_name: Option<String>, message: String },
    Reloading { new_socket: Option<String> },
    SessionCloseRequested { reason: String },
    Unknown { kind: String },
}

#[derive(Debug, Clone, PartialEq)]
pub enum WireError {
    InvalidJson(String),
    MissingType(String),
}

/// Lenient accessor: absent or mistyped fields fall back to defaults.
struct J<'a>(&'a Map<String, Value>);

impl<'a> J<'a> {
    fn s(&self, k: &str) -> String {
        self.os(k).unwrap_or_default()
    }
    fn os(&self, k: &str) -> Option<String> {
        self.0.get(k).and_then(Value::as_str).map(str::to_owned)
    }
    fn b(&self, k: &str) -> bool {
        self.0.get(k).and_then(Value::as_bool).unwrap_or(false)
    }
    fn oi(&self, k: &str) -> Option<i64> {
        self.0.get(k).and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)))
    }
    fn i(&self, k: &str) -> i64 {
        self.oi(k).unwrap_or(0)
    }
    fn ou(&self, k: &str) -> Option<u64> {
        self.0.get(k).and_then(coerce_u64)
    }
    fn u(&self, k: &str) -> u64 {
        self.ou(k).unwrap_or(0)
    }
    fn of(&self, k: &str) -> Option<f64> {
        self.0.get(k).and_then(Value::as_f64)
    }
    fn strs(&self, k: &str) -> Vec<String> {
        self.0
            .get(k)
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect())
            .unwrap_or_default()
    }
    fn objs(&self, k: &str) -> Vec<J<'a>> {
        self.0
            .get(k)
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(|v| v.as_object().map(J)).collect())
            .unwrap_or_default()
    }
    fn obj(&self, k: &str) -> Option<J<'a>> {
        self.0.get(k).and_then(Value::as_object).map(J)
    }
}

fn coerce_u64(v: &Value) -> Option<u64> {
    v.as_u64().or_else(|| v.as_f64().filter(|f| *f >= 0.0).map(|f| f as u64))
}

impl ServerEvent {
    pub fn decode(line: &str) -> Result<ServerEvent, WireError> {
        let value: Value =
            serde_json::from_str(line).map_err(|_| WireError::InvalidJson(line.into()))?;
        let Some(obj) = value.as_object() else {
            return Err(WireError::InvalidJson(line.into()));
        };
        let j = J(obj);
        let Some(kind) = j.os("type") else {
            return Err(WireError::MissingType(line.into()));
        };
        use ServerEvent::*;
        Ok(match kind.as_str() {
            "ack" => Ack { id: j.u("id") },
            "text_delta" => TextDelta { text: j.s("text") },
            "reasoning_delta" => ReasoningDelta { text: j.s("text") },
            "reasoning_done" => ReasoningDone { duration_secs: j.of("duration_secs") },
            "text_replace" => TextReplace { text: j.s("text") },
            "tool_start" => ToolStart { id: j.s("id"), name: j.s("name") },
            "tool_input" => ToolInput { delta: j.s("delta") },
            "tool_exec" => ToolExec { id: j.s("id"), name: j.s("name") },
            "tool_done" => ToolDone {
                id: j.s("id"),
                name: j.s("name"),
                output: j.s("output"),
                error: j.os("error"),
            },
            "tokens" => TokenUsage { input: j.u("input"), output: j.u("output") },
            "status_detail" => StatusDetail { detail: j.s("detail") },
            "connection_phase" => ConnectionPhase { phase: j.s("phase") },
            "soft_interrupt_injected" => SoftInterruptInjected {
                content: j.s("content"),
                display_role: j.os("display_role"),
                point: j.s("point"),
                tools_skipped: j.oi("tools_skipped"),
            },
            "retry_rollback" => RetryRollback { attempt: j.i("attempt"), max: j.i("max") },
            "message_end" => MessageEnd,
            "interrupted" => Interrupted,
            "done" => Done { id: j.u("id") },
            "error" => Error {
                id: j.u("id"),
                message: j.s("message"),
                retry_after_secs: j.ou("retry_after_secs"),
            },
            "pong" => Pong { id: j.u("id") },
            "state" => State {
                id: j.u("id"),
                session_id: j.s("session_id"),
                message_count: j.i("message_count"),
                is_processing: j.b("is_processing"),
            },
            "session" => SessionId { session_id: j.s("session_id") },
            "session_renamed" => SessionRenamed {
                session_id: j.s("session_id"),
                display_title: j.s("display_title"),
            },
            "history" => History(decode_history(&j)),
            "model_changed" => ModelChanged {
                id: j.u("id"),
                model: j.s("model"),
                error: j.os("error"),
            },
            "reasoning_effort_changed" => ReasoningEffortChanged {
                id: j.u("id"),
                effort: j.os("effort"),
                error: j.os("error"),
            },
            "compact_result" => CompactResult {
                id: j.u("id"),
                message: j.s("message"),
                success: j.b("success"),
            },
            "available_models_updated" => AvailableModelsUpdated {
                models: j.strs("available_models"),
                provider_model: j.os("provider_model"),
            },
            "compaction" => Compaction { trigger: j.s("trigger"), tokens_saved: j.ou("tokens_saved") },
            "notification" => Notification { from_name: j.os("from_name"), message: j.s("message") },
            "reloading" => Reloading { new_socket: j.os("new_socket") },
            "session_close_requested" => SessionCloseRequested { reason: j.s("reason") },
            _ => Unknown { kind },
        })
    }
}

fn decode_history(j: &J) -> HistoryPayload {
    let messages = j
        .objs("messages")
        .into_iter()
        .map(|m| HistoryMessage {
            role: m.s("role"),
            content: m.s("content"),
            tool_calls: m.strs("tool_calls"),
            tool_data: m.obj("tool_data").map(|td| ToolCallRecord {
                id: td.s("id"),
                name: td.s("name"),
                input: td.s("input"),
                output: td.os("output"),
                error: td.os("error"),
            }),
        })
        .collect();
    let total_tokens = j.0.get("total_tokens").and_then(Value::as_array).and_then(|a| {
        if a.len() == 2 { Some((coerce_u64(&a[0])?, coerce_u64(&a[1])?)) } else { None }
    });
    HistoryPayload {
        id: j.u("id"),
        session_id: j.s("session_id"),
        messages,
        provider_name: j.os("provider_name"),
        provider_model: j.os("provider_model"),
        available_models: j.strs("available_models"),
        total_tokens,
        all_sessions: j.strs("all_sessions"),
        server_version: j.os("server_version"),
        display_title: j.os("display_title"),
        reasoning_effort: j.os("reasoning_effort"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_subscribe_with_sorted_keys() {
        let r = Request::Subscribe { id: 1, target_session_id: None, working_dir: Some("/w".into()) };
        assert_eq!(r.encoded_line(), r#"{"id":1,"type":"subscribe","working_dir":"/w"}"#);
    }

    #[test]
    fn decodes_events_leniently() {
        assert_eq!(
            ServerEvent::decode(r#"{"type":"text_delta","text":"hi"}"#).unwrap(),
            ServerEvent::TextDelta { text: "hi".into() }
        );
        assert_eq!(
            ServerEvent::decode(r#"{"type":"brand_new"}"#).unwrap(),
            ServerEvent::Unknown { kind: "brand_new".into() }
        );
        assert!(matches!(ServerEvent::decode("nope"), Err(WireError::InvalidJson(_))));
        assert!(matches!(ServerEvent::decode("{}"), Err(WireError::MissingType(_))));
    }

    #[test]
    fn decodes_history() {
        let line = r#"{"type":"history","id":2,"session_id":"s","total_tokens":[10,20],
            "messages":[{"role":"user","content":"q"},
            {"role":"assistant","content":"","tool_data":{"id":"t","name":"bash","input":"ls","output":"ok"}}]}"#;
        let ServerEvent::History(h) = ServerEvent::decode(line).unwrap() else { panic!() };
        assert_eq!(h.total_tokens, Some((10, 20)));
        assert_eq!(h.messages.len(), 2);
        assert_eq!(h.messages[1].tool_data.as_ref().unwrap().output.as_deref(), Some("ok"));
    }
}
