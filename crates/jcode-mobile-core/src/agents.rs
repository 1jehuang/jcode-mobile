//! Account-wide agent status for the Live Activity (docs/PUSH.md, "Many agents").
//!
//! The user may run dozens of agents across hosts. The Live Activity is one
//! aggregate view: counts plus the most urgent few sessions. The Worker pushes
//! it via APNs while the app is suspended. In the foreground the app merges
//! the Worker's view with its own open tabs, which are fresher.

use crate::cloud::Account;
use serde::{Deserialize, Serialize};
use std::time::Duration;

pub const MAX_ITEMS: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, PartialOrd, Ord, Hash)]
#[serde(rename_all = "lowercase")]
pub enum AgentState {
    // Order is urgency: waiting first.
    Waiting,
    Error,
    Running,
}

/// One row on the Lock Screen. JSON keys match the Swift Codable type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentItem {
    pub id: String,
    pub title: String,
    pub state: AgentState,
    #[serde(default)]
    pub detail: String,
}

/// `JcodeAgentsAttributes.ContentState`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentsState {
    pub running: u32,
    pub waiting: u32,
    pub errored: u32,
    pub items: Vec<AgentItem>,
    #[serde(default)]
    pub updated_at: i64,
}

impl AgentsState {
    pub fn is_empty(&self) -> bool {
        self.running + self.waiting + self.errored == 0
    }

    /// Equal apart from the timestamp.
    pub fn same_content(&self, other: &Self) -> bool {
        self.running == other.running
            && self.waiting == other.waiting
            && self.errored == other.errored
            && self.items == other.items
    }
}

/// A session known locally (an open tab), with its current state or None if idle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalAgent {
    pub id: String,
    pub title: String,
    pub state: Option<AgentState>,
    pub detail: String,
}

/// Overlay local tabs on the server's account-wide view.
///
/// Local sessions are authoritative for themselves: an idle local tab removes
/// that session from the server's items. Counts are adjusted by the difference
/// between what the server believed (only visible through `items`) and the
/// local truth. Sessions the server counted but did not list cannot be
/// corrected, so counts never drop below the number of listed items.
pub fn merge(server: Option<&AgentsState>, local: &[LocalAgent], now: i64) -> AgentsState {
    let mut out = server.cloned().unwrap_or_default();
    for l in local {
        if l.id.is_empty() {
            continue;
        }
        let prev = out.items.iter().position(|i| i.id == l.id);
        let prev_state = prev.map(|i| out.items[i].state);
        if let Some(i) = prev {
            out.items.remove(i);
        }
        if let Some(s) = prev_state {
            dec(&mut out, s);
        }
        if let Some(s) = l.state {
            // An unlisted local session may already be in the server's count.
            if prev_state.is_some() || server.is_none() || !server_counted_unlisted(server, s) {
                inc(&mut out, s);
            }
            out.items.push(AgentItem {
                id: l.id.clone(),
                title: l.title.clone(),
                state: s,
                detail: l.detail.clone(),
            });
        }
    }
    out.items.sort_by_key(|i| i.state);
    out.items.truncate(MAX_ITEMS);
    out.updated_at = now;
    out
}

/// True when the server's count for `s` exceeds its listed items, so an
/// unlisted local session is probably already included in the count.
fn server_counted_unlisted(server: Option<&AgentsState>, s: AgentState) -> bool {
    let Some(srv) = server else { return false };
    let listed = srv.items.iter().filter(|i| i.state == s).count() as u32;
    count(srv, s) > listed
}

fn count(s: &AgentsState, st: AgentState) -> u32 {
    match st {
        AgentState::Running => s.running,
        AgentState::Waiting => s.waiting,
        AgentState::Error => s.errored,
    }
}

fn slot(s: &mut AgentsState, st: AgentState) -> &mut u32 {
    match st {
        AgentState::Running => &mut s.running,
        AgentState::Waiting => &mut s.waiting,
        AgentState::Error => &mut s.errored,
    }
}

fn inc(s: &mut AgentsState, st: AgentState) {
    *slot(s, st) += 1;
}

fn dec(s: &mut AgentsState, st: AgentState) {
    let c = slot(s, st);
    *c = c.saturating_sub(1);
}

/// Short Lock Screen headline, e.g. "7 running, 1 needs input".
pub fn headline(s: &AgentsState) -> String {
    let mut parts = Vec::new();
    if s.waiting > 0 {
        parts.push(format!("{} need{} input", s.waiting, if s.waiting == 1 { "s" } else { "" }));
    }
    if s.errored > 0 {
        parts.push(format!("{} failed", s.errored));
    }
    if s.running > 0 {
        parts.push(format!("{} running", s.running));
    }
    if parts.is_empty() { "All agents idle".into() } else { parts.join(", ") }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum LiveTokenKind {
    /// Per-activity update token.
    Update,
    /// iOS 17.2+ push-to-start token.
    Start,
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(15)))
        .http_status_as_error(false)
        .user_agent(crate::cloud::CLIENT_NAME)
        .build()
        .into()
}

fn url(account: &Account, path: &str) -> String {
    format!("{}{}", account.api_base.trim_end_matches('/'), path)
}

fn check(mut resp: ureq::http::Response<ureq::Body>) -> Result<serde_json::Value, String> {
    let status = resp.status().as_u16();
    let text = resp.body_mut().read_to_string().unwrap_or_default();
    let body: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
    if (200..300).contains(&status) {
        Ok(body)
    } else {
        Err(format!("HTTP {status} {}", body["error"]["code"].as_str().unwrap_or("")))
    }
}

/// Register a Live Activity push token with the account.
pub fn register_live_token(
    account: &Account,
    kind: LiveTokenKind,
    token_hex: &str,
    environment: crate::cloud::PushEnvironment,
    bundle_id: &str,
    device_token: Option<&str>,
) -> Result<(), String> {
    let resp = agent()
        .post(&url(account, "/push/live-tokens"))
        .header("Authorization", &format!("Bearer {}", account.api_key))
        .send_json(serde_json::json!({
            "token": token_hex,
            "kind": kind,
            "environment": environment,
            "bundle_id": bundle_id,
            "device_token": device_token,
        }))
        .map_err(|e| e.to_string())?;
    check(resp).map(|_| ())
}

pub fn unregister_live_token(account: &Account, token_hex: &str) -> Result<(), String> {
    let resp = agent()
        .delete(&url(account, "/push/live-tokens"))
        .header("Authorization", &format!("Bearer {}", account.api_key))
        .force_send_body()
        .send_json(serde_json::json!({ "token": token_hex }))
        .map_err(|e| e.to_string())?;
    check(resp).map(|_| ())
}

/// The account-wide state the Worker last computed.
pub fn fetch_status(account: &Account) -> Result<AgentsState, String> {
    let resp = agent()
        .get(&url(account, "/agents/status"))
        .header("Authorization", &format!("Bearer {}", account.api_key))
        .call()
        .map_err(|e| e.to_string())?;
    let body = check(resp)?;
    serde_json::from_value(body["state"].clone()).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, s: AgentState) -> AgentItem {
        AgentItem { id: id.into(), title: id.into(), state: s, detail: String::new() }
    }
    fn local(id: &str, s: Option<AgentState>) -> LocalAgent {
        LocalAgent { id: id.into(), title: id.into(), state: s, detail: String::new() }
    }

    #[test]
    fn local_only_counts_and_orders_by_urgency() {
        let m = merge(
            None,
            &[
                local("a", Some(AgentState::Running)),
                local("b", Some(AgentState::Waiting)),
                local("c", None),
                local("d", Some(AgentState::Error)),
                local("e", Some(AgentState::Running)),
            ],
            5,
        );
        assert_eq!((m.running, m.waiting, m.errored), (2, 1, 1));
        let ids: Vec<_> = m.items.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(ids, ["b", "d", "a"]);
        assert_eq!(m.updated_at, 5);
        assert_eq!(headline(&m), "1 needs input, 1 failed, 2 running");
    }

    #[test]
    fn local_overrides_listed_server_items() {
        let srv = AgentsState {
            running: 9,
            waiting: 1,
            errored: 0,
            items: vec![item("x", AgentState::Waiting), item("y", AgentState::Running)],
            updated_at: 1,
        };
        // x was answered locally and is now running, y finished.
        let m = merge(Some(&srv), &[local("x", Some(AgentState::Running)), local("y", None)], 2);
        assert_eq!((m.running, m.waiting), (9, 0));
        assert_eq!(m.items, vec![item("x", AgentState::Running)]);
    }

    #[test]
    fn unlisted_local_session_is_not_double_counted() {
        let srv = AgentsState { running: 12, items: vec![item("y", AgentState::Running)], ..Default::default() };
        let m = merge(Some(&srv), &[local("z", Some(AgentState::Running))], 0);
        assert_eq!(m.running, 12);
        let srv = AgentsState { running: 1, items: vec![item("y", AgentState::Running)], ..Default::default() };
        let m = merge(Some(&srv), &[local("z", Some(AgentState::Running))], 0);
        assert_eq!(m.running, 2);
    }

    #[test]
    fn json_matches_swift_codable() {
        let s = merge(None, &[local("a", Some(AgentState::Waiting))], 7);
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(
            v,
            serde_json::json!({"running":0,"waiting":1,"errored":0,"items":[{"id":"a","title":"a","state":"waiting","detail":""}],"updatedAt":7})
        );
        assert!(merge(None, &[], 0).is_empty());
        assert_eq!(headline(&AgentsState::default()), "All agents idle");
    }
}
