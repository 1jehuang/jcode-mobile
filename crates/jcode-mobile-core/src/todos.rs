//! Pinned todo board, the mobile counterpart of the TUI's pinned todo card.
//!
//! The authoritative source is the server's persisted todo state
//! (`GetTodoState`), the same snapshot the TUI renders: phases, lanes,
//! blockers, owners, plan and goals. Older servers without that capability
//! fall back to replaying `todo` tool calls from the transcript.

use crate::session::{ToolStatus, TranscriptEntry};
use jcode_sdk::todo::{self as policy, TodoItem, TodoSnapshot, stages};
use serde_json::Value;

/// Display state of one item, mirroring the TUI glyph rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    Done,
    Cancelled,
    /// In progress, owned by the coordinator itself (amber in the TUI).
    SelfActive,
    Active,
    /// Waiting on another todo or an external event (⊘).
    Blocked,
    /// Cannot proceed without someone else (✋).
    CompletelyBlocked,
    Open,
}

pub fn mark(t: &TodoItem) -> Mark {
    if policy::todo_requires_user_auth(t) { return Mark::CompletelyBlocked; }
    let incomplete = policy::is_incomplete_todo(t);
    if incomplete && (!t.waiting_on.is_empty() || !t.blocked_by.is_empty()) { return Mark::Blocked; }
    if policy::todo_status_is_completed(&t.status) { return Mark::Done; }
    if policy::todo_status_is_cancelled(&t.status) { return Mark::Cancelled; }
    match t.status.as_str() {
        "in_progress" if t.assigned_to.as_deref() == Some("@self") => Mark::SelfActive,
        "in_progress" => Mark::Active,
        _ => Mark::Open,
    }
}

impl Mark {
    pub fn finished(self) -> bool { matches!(self, Mark::Done | Mark::Cancelled) }
    pub fn active(self) -> bool { matches!(self, Mark::Active | Mark::SelfActive) }
}

#[derive(Debug, Clone)]
pub struct Lane {
    pub name: Option<String>,
    /// Worker ownership (`lane_owner: worker`) or assignee shown as a chip.
    pub owner: Option<String>,
    pub items: Vec<TodoItem>,
}

#[derive(Debug, Clone)]
pub struct Phase {
    pub number: Option<u32>,
    pub lanes: Vec<Lane>,
}

impl Phase {
    pub fn items(&self) -> impl Iterator<Item = &TodoItem> { self.lanes.iter().flat_map(|l| &l.items) }
    pub fn done(&self) -> bool { self.items().all(|t| mark(t).finished()) }
}

#[derive(Debug, Clone, Default)]
pub struct Board {
    pub title: Option<String>,
    pub intention: Option<String>,
    pub phases: Vec<Phase>,
    /// One-line goal/feedback-loop notes keyed by lane (`None` = whole plan).
    pub goals: Vec<(Option<String>, String)>,
}

impl Board {
    pub fn items(&self) -> impl Iterator<Item = &TodoItem> { self.phases.iter().flat_map(|p| p.items()) }
    pub fn is_empty(&self) -> bool { self.items().next().is_none() }
    pub fn staged(&self) -> bool { self.phases.iter().any(|p| p.number.is_some()) }
    /// Index of the first phase with unfinished work.
    pub fn current_phase(&self) -> Option<usize> { self.phases.iter().position(|p| !p.done()) }
    pub fn focus(&self) -> Option<&TodoItem> {
        self.items().find(|t| mark(t).active()).or_else(|| self.items().find(|t| !mark(t).finished()))
    }
    pub fn blockers(&self) -> impl Iterator<Item = (&TodoItem, &str)> {
        self.items().filter_map(|t| t.requires_user_auth.as_ref().filter(|_| policy::todo_requires_user_auth(t)).map(|b| (t, b.reason.as_str())))
    }
}

fn clean(s: Option<&str>) -> Option<String> { s.map(str::trim).filter(|s| !s.is_empty()).map(String::from) }

// These are display-only migrations. Never send the synthesized legacy wait
// back to the server as a real background-task dependency.
const LEGACY_EXTERNAL_WAIT: &str = "mobile:legacy-external-blocker";

fn normalize_item(value: &mut Value) {
    let Some(fields) = value.as_object_mut() else { return };
    if !fields.contains_key("requires_user_auth") {
        if let Some(blocker) = fields.get("completely_blocked").cloned() {
            fields.insert("requires_user_auth".into(), blocker);
        }
    }
    if let Some(wait) = fields.get("waiting_on").filter(|v| v.is_object()).cloned() {
        fields.insert("waiting_on".into(), wait.get("ids").cloned().unwrap_or(Value::Null));
        fields.insert("waiting_reason".into(), wait.get("reason").cloned().unwrap_or(Value::Null));
    } else if !fields.contains_key("waiting_on") {
        if let Some(blocker) = fields.get("external_blocker").cloned() {
            let ids = if blocker.is_null() { serde_json::json!([]) } else { serde_json::json!([LEGACY_EXTERNAL_WAIT]) };
            fields.insert("waiting_on".into(), ids);
            fields.insert("waiting_reason".into(), blocker.get("reason").cloned().unwrap_or(Value::Null));
        }
    }
}

/// Preserve old mobile caches before the modern SDK discards legacy blockers.
pub fn deserialize_optional_snapshot<'de, D>(deserializer: D) -> Result<Option<TodoSnapshot>, D::Error>
where D: serde::Deserializer<'de> {
    use serde::Deserialize;
    let Some(mut value) = Option::<Value>::deserialize(deserializer)? else { return Ok(None) };
    if let Some(todos) = value.get_mut("todos").and_then(Value::as_array_mut) {
        for todo in todos { normalize_item(todo); }
    }
    serde_json::from_value(value).map(Some).map_err(serde::de::Error::custom)
}

fn apply_pause_patch(todo: &mut TodoItem, patch: &Value) {
    let mut patch = patch.clone();
    normalize_item(&mut patch);
    if let Some(auth) = patch.get("requires_user_auth") {
        todo.requires_user_auth = serde_json::from_value(auth.clone()).ok();
    }
    if let Some(wait) = patch.get("waiting_on") {
        todo.waiting_on = serde_json::from_value(wait.clone()).unwrap_or_default();
        todo.waiting_reason = patch.get("waiting_reason").and_then(Value::as_str).map(String::from);
    } else if let Some(reason) = patch.get("waiting_reason") {
        todo.waiting_reason = reason.as_str().map(String::from);
    }
}

pub fn board(snapshot: &TodoSnapshot) -> Board {
    let refs: Vec<&TodoItem> = snapshot.todos.iter().collect();
    let phases = stages::stage_layout(&refs).into_iter().map(|stage| Phase {
        number: stage.number,
        lanes: stage.lanes.into_iter().map(|lane| {
            let items: Vec<TodoItem> = lane.items.iter().map(|&i| refs[i].clone()).collect();
            let owner = items.iter().find_map(|t| t.assigned_to.clone().filter(|a| a != "@self"));
            Lane { name: lane.group, owner, items }
        }).collect(),
    }).collect();
    let goals = snapshot.goals.iter().filter_map(|g| {
        clean(g.feedback_loop.as_deref()).map(|f| (clean(g.group.as_deref()), f))
    }).collect();
    Board {
        title: clean(snapshot.plan.title.as_deref()),
        intention: clean(snapshot.plan.user_intention.as_deref()),
        phases,
        goals,
    }
}

/// Fallback for servers without `todo_state`: replay todo tool calls.
pub fn from_transcript(transcript: &[TranscriptEntry]) -> TodoSnapshot {
    let mut snap = TodoSnapshot::default();
    for call in transcript.iter().flat_map(|e| &e.tool_calls) {
        if (call.name.eq_ignore_ascii_case("todo") || call.name.eq_ignore_ascii_case("todowrite"))
            && !matches!(call.status, ToolStatus::StreamingInput | ToolStatus::Failed(_))
        {
            apply(&mut snap, &call.input);
        }
    }
    snap
}

fn item(v: &Value, phase: Option<u32>, lane: Option<&str>, owner: Option<&str>) -> Option<TodoItem> {
    let mut normalized = v.clone();
    normalize_item(&mut normalized);
    // Tool inputs omit required persisted fields such as id and priority.
    let mut fields = serde_json::to_value(TodoItem::default()).ok()?;
    fields.as_object_mut()?.extend(normalized.as_object()?.clone());
    let mut t: TodoItem = serde_json::from_value(fields).ok()?;
    t.content = v.get("name").or_else(|| v.get("content")).or_else(|| v.get("title"))?.as_str()?.to_string();
    if t.status.is_empty() { t.status = "pending".into(); }
    if t.id.is_empty() { t.id = t.content.clone(); }
    if t.stage.is_none() { t.stage = phase; }
    if t.group.is_none() { t.group = lane.map(String::from); }
    if t.assigned_to.is_none() { t.assigned_to = owner.map(String::from); }
    Some(t)
}

/// Apply one todo tool input. Unparseable input leaves the snapshot unchanged.
pub fn apply(snap: &mut TodoSnapshot, input: &str) {
    let Ok(v) = serde_json::from_str::<Value>(input) else { return };
    let list = &mut snap.todos;
    if let Some(todos) = v.get("todos").and_then(Value::as_array) {
        *list = todos.iter().filter_map(|t| item(t, None, None, None)).collect();
    }
    if let Some(phases) = v.get("phases").and_then(Value::as_array) {
        *list = phases.iter().flat_map(|p| {
            let n = p.get("phase").and_then(Value::as_u64).map(|n| n as u32);
            p.get("lanes").and_then(Value::as_array).into_iter().flatten().flat_map(move |l| {
                let lane = l.get("lane").and_then(Value::as_str);
                let owner = l.get("owner").and_then(Value::as_str);
                l.get("items").and_then(Value::as_array).into_iter().flatten().filter_map(move |i| item(i, n, lane, owner))
            })
        }).collect();
    }
    for r in v.get("remove").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
        list.retain(|t| t.content != r);
    }
    for a in v.get("add").and_then(Value::as_array).into_iter().flatten() {
        let after = a.get("after").and_then(Value::as_str).and_then(|n| list.iter().position(|t| t.content == n));
        let inherit = after.map(|i| (list[i].stage, list[i].group.clone()));
        let phase = a.get("phase").and_then(Value::as_u64).map(|n| n as u32).or(inherit.as_ref().and_then(|x| x.0));
        let lane = inherit.as_ref().and_then(|x| x.1.clone());
        let Some(t) = item(a, phase, lane.as_deref(), None) else { continue };
        match after { Some(i) => list.insert(i + 1, t), None => list.push(t) }
    }
    for u in v.get("update").and_then(Value::as_array).into_iter().flatten() {
        let Some(name) = u.get("name").and_then(Value::as_str) else { continue };
        let Some(t) = list.iter_mut().find(|t| t.content == name) else { continue };
        if let Some(s) = u.get("status").and_then(Value::as_str) { t.status = s.into(); }
        if let Some(n) = u.get("rename_to").and_then(Value::as_str) { t.content = n.into(); }
        if let Some(l) = u.get("lane").and_then(Value::as_str) { t.group = Some(l.into()); }
        if let Some(p) = u.get("phase").and_then(Value::as_u64) { t.stage = Some(p as u32); }
        apply_pause_patch(t, u);
        if let Some(b) = u.get("blocked_by") { t.blocked_by = serde_json::from_value(b.clone()).unwrap_or_default(); }
    }
    if let Some(plan) = v.get("plan") {
        if let Some(i) = plan.get("user_intention").and_then(Value::as_str) { snap.plan.user_intention = Some(i.into()); }
        if let Some(i) = plan.get("title").and_then(Value::as_str) { snap.plan.title = Some(i.into()); }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modern_pauses_replay_and_clear_without_losing_item_fields() {
        let mut s = TodoSnapshot::default();
        apply(&mut s, r#"{"todos":[{"name":"login","status":"in_progress","assigned_to":"@self","requires_user_auth":{"reason":"2FA"}},{"name":"job","status":"pending","waiting_on":{"ids":["task-1"],"reason":"build"}}]}"#);
        assert_eq!(mark(&s.todos[0]), Mark::CompletelyBlocked);
        assert_eq!(board(&s).blockers().next().unwrap().1, "2FA");
        assert_eq!(s.todos[0].assigned_to.as_deref(), Some("@self"));
        assert_eq!(mark(&s.todos[1]), Mark::Blocked);
        assert_eq!(s.todos[1].waiting_reason.as_deref(), Some("build"));
        apply(&mut s, r#"{"update":[{"name":"login","requires_user_auth":null},{"name":"job","waiting_on":null}]}"#);
        assert_eq!(mark(&s.todos[0]), Mark::SelfActive);
        assert_eq!(mark(&s.todos[1]), Mark::Open);
        assert!(s.todos[1].waiting_reason.is_none());
    }

    #[test]
    fn legacy_external_blockers_and_finished_items() {
        let mut s = TodoSnapshot::default();
        apply(&mut s, r#"{"todos":[{"name":"wait","status":"pending","external_blocker":{"reason":"API unavailable"}}]}"#);
        assert_eq!(mark(&s.todos[0]), Mark::Blocked);
        apply(&mut s, r#"{"update":[{"name":"wait","external_blocker":null}]}"#);
        assert_eq!(mark(&s.todos[0]), Mark::Open);
        apply(&mut s, r#"{"update":[{"name":"wait","completely_blocked":{"reason":"credential"},"status":"completed"}]}"#);
        assert_eq!(mark(&s.todos[0]), Mark::Done);
        assert_eq!(board(&s).blockers().count(), 0);
    }

    #[test]
    fn legacy_snapshot_cache_adapter_and_modern_precedence() {
        #[derive(serde::Deserialize)]
        struct Cache {
            #[serde(default, deserialize_with = "deserialize_optional_snapshot")]
            snapshot: Option<TodoSnapshot>,
        }
        let mut snapshot = serde_json::to_value(TodoSnapshot::default()).unwrap();
        let mut todo = serde_json::to_value(TodoItem::default()).unwrap();
        todo["content"] = serde_json::json!("cached");
        todo["status"] = serde_json::json!("pending");
        todo["completely_blocked"] = serde_json::json!({"reason":"old credential"});
        snapshot["todos"] = serde_json::json!([todo]);
        let cache: Cache = serde_json::from_value(serde_json::json!({"snapshot":snapshot})).unwrap();
        let s = cache.snapshot.unwrap();
        assert_eq!(mark(&s.todos[0]), Mark::CompletelyBlocked);
        assert_eq!(board(&s).blockers().next().unwrap().1, "old credential");
        snapshot["todos"][0]["requires_user_auth"] = Value::Null;
        let cache: Cache = serde_json::from_value(serde_json::json!({"snapshot":snapshot})).unwrap();
        assert_eq!(mark(&cache.snapshot.unwrap().todos[0]), Mark::Open);
        assert!(serde_json::from_str::<Cache>(r#"{"snapshot":null}"#).unwrap().snapshot.is_none());
        assert!(serde_json::from_str::<Cache>("{}").unwrap().snapshot.is_none());
    }

    #[test]
    fn phases_lanes_and_patches() {
        let mut s = TodoSnapshot::default();
        apply(&mut s, r#"{"plan":{"user_intention":"ship it"},"phases":[
            {"phase":1,"lanes":[{"lane":"core","items":[{"name":"x","status":"pending"},{"name":"y","status":"pending"}]}]},
            {"phase":2,"lanes":[{"lane":"ui","owner":"worker","items":[{"name":"z","status":"pending"}]},{"lane":"docs","items":[{"name":"w","status":"pending"}]}]}]}"#);
        let b = board(&s);
        assert_eq!(b.intention.as_deref(), Some("ship it"));
        assert_eq!(b.phases.len(), 2);
        assert_eq!(b.phases[1].lanes.len(), 2);
        assert_eq!(b.phases[1].lanes[0].owner.as_deref(), Some("worker"));
        apply(&mut s, r#"{"update":[{"name":"x","status":"completed"},{"name":"y","status":"in_progress"},{"name":"w","completely_blocked":{"reason":"needs API key"}}],"add":[{"name":"v","status":"pending","after":"y"}]}"#);
        let b = board(&s);
        assert_eq!(b.phases[0].lanes[0].items.len(), 3);
        assert_eq!(mark(&b.phases[0].lanes[0].items[0]), Mark::Done);
        assert_eq!(b.focus().unwrap().content, "y");
        assert_eq!(b.current_phase(), Some(0));
        assert_eq!(b.blockers().next().unwrap().1, "needs API key");
        apply(&mut s, r#"{"remove":["v"]}"#);
        assert_eq!(board(&s).phases[0].lanes[0].items.len(), 2);
        apply(&mut s, "not json");
        assert_eq!(s.todos.len(), 4);
    }
}
