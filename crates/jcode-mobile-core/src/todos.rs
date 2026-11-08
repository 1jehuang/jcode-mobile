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
    if policy::todo_completely_blocked(t) { return Mark::CompletelyBlocked; }
    let incomplete = policy::is_incomplete_todo(t);
    if incomplete && (policy::todo_has_external_blocker(t) || !t.blocked_by.is_empty()) { return Mark::Blocked; }
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
        self.items().filter_map(|t| t.completely_blocked.as_ref().filter(|_| policy::todo_completely_blocked(t)).map(|b| (t, b.reason.as_str())))
    }
}

fn clean(s: Option<&str>) -> Option<String> { s.map(str::trim).filter(|s| !s.is_empty()).map(String::from) }

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
    let mut t: TodoItem = serde_json::from_value(v.clone()).unwrap_or_default();
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
        if let Some(b) = u.get("completely_blocked") { t.completely_blocked = serde_json::from_value(b.clone()).ok(); }
        if let Some(b) = u.get("blocked_by") { t.blocked_by = serde_json::from_value(b.clone()).unwrap_or_default(); }
        if let Some(b) = u.get("external_blocker") { t.external_blocker = serde_json::from_value(b.clone()).ok(); }
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
