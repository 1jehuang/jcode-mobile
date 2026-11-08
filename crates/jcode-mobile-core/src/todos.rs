//! Derives the current todo list from `todo` tool calls in the transcript, so
//! the UI can pin it above the composer. Supports the full-list form
//! (`todos: [...]`), plan replacement (`phases[].lanes[].items[]`), and
//! patches by name (`add`, `update`, `remove`).

use crate::session::{ToolStatus, TranscriptEntry};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TodoItem {
    pub name: String,
    pub status: String,
    /// Plan lane (or `group`), shown as a section header like the TUI card.
    pub lane: Option<String>,
}

impl TodoItem {
    pub fn is_done(&self) -> bool { matches!(self.status.as_str(), "completed" | "cancelled") }
    pub fn is_active(&self) -> bool { self.status == "in_progress" }
    pub fn is_cancelled(&self) -> bool { self.status == "cancelled" }
}

fn item(v: &Value) -> Option<TodoItem> {
    let name = v.get("name").or_else(|| v.get("content")).or_else(|| v.get("title"))?.as_str()?.to_string();
    let status = v.get("status").and_then(Value::as_str).unwrap_or("pending").to_string();
    let lane = v.get("lane").or_else(|| v.get("group")).and_then(Value::as_str)
        .map(str::trim).filter(|l| !l.is_empty()).map(String::from);
    Some(TodoItem { name, status, lane })
}

fn lane_item(v: &Value, lane: Option<&str>) -> Option<TodoItem> {
    let mut t = item(v)?;
    if t.lane.is_none() { t.lane = lane.map(String::from); }
    Some(t)
}

/// Items grouped by lane in first-seen order. One `None` group when no lanes.
pub fn lanes(list: &[TodoItem]) -> Vec<(Option<&str>, Vec<&TodoItem>)> {
    let mut out: Vec<(Option<&str>, Vec<&TodoItem>)> = Vec::new();
    for t in list {
        let key = t.lane.as_deref();
        match out.iter_mut().find(|(k, _)| *k == key) {
            Some((_, v)) => v.push(t),
            None => out.push((key, vec![t])),
        }
    }
    out.sort_by_key(|(k, _)| k.is_none());
    out
}

/// Apply one tool input to the list. Unparseable input leaves it unchanged.
pub fn apply(list: &mut Vec<TodoItem>, input: &str) {
    let Ok(v) = serde_json::from_str::<Value>(input) else { return };
    if let Some(todos) = v.get("todos").and_then(Value::as_array) {
        *list = todos.iter().filter_map(item).collect();
    }
    if let Some(phases) = v.get("phases").and_then(Value::as_array) {
        *list = phases.iter()
            .filter_map(|p| p.get("lanes").and_then(Value::as_array))
            .flatten()
            .flat_map(|l| {
                let lane = l.get("lane").and_then(Value::as_str);
                l.get("items").and_then(Value::as_array).into_iter().flatten().filter_map(move |i| lane_item(i, lane))
            })
            .collect();
    }
    for r in v.get("remove").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
        list.retain(|t| t.name != r);
    }
    for a in v.get("add").and_then(Value::as_array).into_iter().flatten().filter_map(item) {
        let after = v.get("add").and_then(Value::as_array)
            .and_then(|adds| adds.iter().find(|x| x.get("name").and_then(Value::as_str) == Some(&a.name)))
            .and_then(|x| x.get("after")).and_then(Value::as_str)
            .and_then(|n| list.iter().position(|t| t.name == n));
        match after { Some(i) => list.insert(i + 1, a), None => list.push(a) }
    }
    for u in v.get("update").and_then(Value::as_array).into_iter().flatten() {
        let Some(name) = u.get("name").and_then(Value::as_str) else { continue };
        let Some(t) = list.iter_mut().find(|t| t.name == name) else { continue };
        if let Some(s) = u.get("status").and_then(Value::as_str) { t.status = s.into(); }
        if let Some(n) = u.get("rename_to").and_then(Value::as_str) { t.name = n.into(); }
        if let Some(l) = u.get("lane").and_then(Value::as_str) { t.lane = Some(l.into()); }
    }
}

/// Current todo list implied by the transcript's todo tool calls.
pub fn current(transcript: &[TranscriptEntry]) -> Vec<TodoItem> {
    let mut list = Vec::new();
    for call in transcript.iter().flat_map(|e| &e.tool_calls) {
        if call.name.eq_ignore_ascii_case("todo") || call.name.eq_ignore_ascii_case("todowrite") {
            if !matches!(call.status, ToolStatus::StreamingInput | ToolStatus::Failed(_)) {
                apply(&mut list, &call.input);
            }
        }
    }
    list
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_list_then_patches() {
        let mut l = Vec::new();
        apply(&mut l, r#"{"phases":[{"phase":1,"lanes":[{"lane":"a","items":[{"name":"x","status":"pending"},{"name":"y","status":"pending"}]}]}]}"#);
        assert_eq!(l.len(), 2);
        assert_eq!(l[0].lane.as_deref(), Some("a"));
        apply(&mut l, r#"{"update":[{"name":"x","status":"completed"}],"add":[{"name":"z","status":"in_progress","after":"x"}]}"#);
        assert!(l[0].is_done());
        assert_eq!(l[1].name, "z");
        assert!(l[1].is_active());
        apply(&mut l, r#"{"remove":["y"]}"#);
        assert_eq!(l.len(), 2);
        apply(&mut l, r#"{"todos":[{"content":"only","status":"pending"}]}"#);
        assert_eq!(l, vec![TodoItem { name: "only".into(), status: "pending".into(), lane: None }]);
        apply(&mut l, "not json");
        assert_eq!(l.len(), 1);
    }
}
