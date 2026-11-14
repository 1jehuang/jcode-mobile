//! Empty-session quick nav: resume recent/live conversations or start in a
//! recently used cloud directory.

use gpui::{AnyElement, Context, Window, div, prelude::*, px, svg};
use jcode_mobile_core::Request;
use jcode_mobile_core::jcode_sdk::SessionInfo;

use super::MobileApp;
use super::a11y::A11yExt as _;
use crate::theme::Theme;

const MAX_DIRS: usize = 5;

impl MobileApp {
    /// Ask once per connected session for the session list the dirs come from.
    pub(super) fn ensure_recent_dirs(&mut self) {
        let Some(id) = self.session.session_id.clone() else { return };
        if self.recent_dirs_asked.as_deref() == Some(id.as_str()) {
            return;
        }
        self.recent_dirs_asked = Some(id);
        self.send(|id| Request::ListSessions { id });
    }

    /// Fresh session in `dir`, replacing the current (still empty) one.
    fn start_in_dir(&mut self, dir: String, cx: &mut Context<Self>) {
        crate::haptics::play(crate::haptics::Haptic::Selection);
        if let Some(c) = self.connection.take() {
            c.stop();
        }
        let list = std::mem::take(&mut self.session.session_list);
        self.session = Default::default();
        self.session.session_list = list;
        self.session.working_dir = Some(dir.clone());
        self.start_connection_in(None, Some(dir), cx);
        cx.notify();
    }

    /// Open the directory sheet from the composer's location label.
    pub(super) fn open_dir_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.dir_picker {
            crate::haptics::play(crate::haptics::Haptic::Light);
        }
        self.ensure_recent_dirs();
        let current = self.session.working_dir.clone().unwrap_or_default();
        self.dir_input.update(cx, |t, cx| t.set_text(current, cx));
        self.dir_picker = true;
        window.blur(cx);
        cx.notify();
    }

    /// Move to `dir`. A session's working dir is fixed once it starts, so an
    /// empty tab restarts there and a tab with history opens a new tab instead
    /// of discarding the conversation.
    pub(super) fn change_dir(&mut self, dir: String, cx: &mut Context<Self>) {
        let dir = dir.trim().to_string();
        if dir.is_empty() {
            cx.notify();
            return;
        }
        self.dir_picker = false;
        let same = self.session.working_dir.as_deref().map(|d| d.trim_end_matches('/')) == Some(dir.trim_end_matches('/'));
        if same {
            cx.notify();
        } else if self.session.transcript.is_empty() {
            self.start_in_dir(dir, cx);
        } else {
            self.new_tab_in_dir(Some(dir), cx);
        }
    }

    /// Bottom sheet: a path field plus recent cloud dirs.
    pub(super) fn render_dir_picker(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let lists = std::iter::once(&self.session.session_list).chain(self.parked.iter().map(|p| &p.session.session_list));
        let dirs = lists.max_by_key(|l| l.len()).map(|l| recent_dirs(l)).unwrap_or_default();
        let current = self.session.working_dir.as_deref().map(|d| d.trim_end_matches('/').to_string());
        let mut rows = div().flex().flex_col().gap(px(2.));
        for (i, dir) in dirs.into_iter().enumerate() {
            let selected = current.as_deref() == Some(dir.trim_end_matches('/'));
            let (name, parent) = crate::labels::cloud_location(Some(&dir));
            rows = rows.child(
                super::picker_row(("dir", i), name, parent.unwrap_or_default(), selected, true, t)
                    .debug_selector(move || format!("dir-row-{i}"))
                    .on_click(cx.listener(move |this, _, _, cx| this.change_dir(dir.clone(), cx))),
            );
        }
        let note = if self.session.transcript.is_empty() {
            "Restarts this tab in the chosen directory."
        } else {
            "Opens a new tab there. This conversation stays put."
        };
        div()
            .occlude()
            .absolute()
            .inset_0()
            .flex()
            .flex_col()
            .bg(gpui::black().opacity(0.5))
            .child(div().id("dir-dismiss").debug_selector(|| "dir-dismiss".into()).flex_1().on_click(cx.listener(|this, _, _, cx| {
                this.dir_picker = false;
                cx.notify()
            })))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .max_h(px(520.))
                    .bg(t.surface_elevated)
                    .rounded_t(px(18.))
                    .border_t_1()
                    .border_color(t.border)
                    .pt(px(8.))
                    .pb(px(12.))
                    .child(div().w_full().flex().justify_center().child(div().w(px(36.)).h(px(5.)).rounded_full().bg(t.text_muted)))
                    .when(!self.keyboard_up, |d| d.child(super::label("Directory", t.text_strong, 16.).px(px(16.)).pt(px(8.))))
                    .child(
                        div()
                            .mx(px(12.))
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .pl(px(12.))
                            .pr(px(4.))
                            .min_h(px(40.))
                            .rounded(px(12.))
                            .bg(t.field)
                            .border_1()
                            .border_color(t.input_border)
                            .child(div().id("dir-input").debug_selector(|| "dir-input".into()).flex_1().min_w_0().child(self.dir_input.clone()))
                            .child(
                                div()
                                    .id("dir-go")
                                    .debug_selector(|| "dir-go".into())
                                    .px(px(10.))
                                    .py(px(6.))
                                    .rounded(px(10.))
                                    .bg(t.surface)
                                    .cursor_pointer()
                                    .child(super::label("Go", t.text_strong, 13.))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        let dir = this.dir_input.read(cx).text().to_string();
                                        this.change_dir(dir, cx)
                                    })),
                            ),
                    )
                    .when(!self.keyboard_up, |d| d.child(super::label(note, t.text_muted, 11.).px(px(16.))))
                    .child(
                        div()
                            .id("dir-list")
                            .debug_selector(|| "dir-list".into())
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .px(px(12.))
                            .child(rows),
                    ),
            )
            .into_any_element()
    }

    pub(super) fn render_recent_dirs(&self, t: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        self.render_recent_dirs_for(&self.session, t, cx)
    }

    pub(super) fn render_recent_dirs_for(&self, session: &jcode_mobile_core::SessionState, t: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        // Any tab's list will do; the active one may not have fetched yet.
        let lists = std::iter::once(&self.session.session_list).chain(self.parked.iter().map(|p| &p.session.session_list));
        let list = lists.max_by_key(|l| l.len())?;
        let current = session.working_dir.as_deref().map(|d| d.trim_end_matches('/'));
        let dirs: Vec<String> = recent_dirs(list).into_iter().filter(|d| Some(d.trim_end_matches('/')) != current).collect();
        let open_sessions: Vec<_> = std::iter::once(self.session.session_id.as_deref())
            .chain(self.parked.iter().map(|p| p.session.session_id.as_deref()))
            .chain(std::iter::once(session.session_id.as_deref()))
            .flatten()
            .collect();
        let sessions = suggested_sessions(list, &open_sessions);
        if dirs.is_empty() && sessions.is_empty() {
            return None;
        }
        let mut col = div()
            .id("start-suggestions")
            .debug_selector(|| "start-suggestions".into())
            .min_h_0()
            .max_h(px(360.))
            .overflow_y_scroll()
            // Only the live panel's list decides whether a pull may begin.
            .when(std::ptr::eq(session, &self.session), |d| d.track_scroll(&self.suggestions_scroll))
            .flex()
            .flex_col()
            .gap(px(6.));
        if !sessions.is_empty() {
            col = col.child(super::label("Continue a session", t.text_muted, 11.).pl(px(2.)).pb(px(2.)));
        }
        for (i, session) in sessions.into_iter().enumerate() {
            let sid = session.session_id.clone();
            let live = session.status == "running" || session.user_connected;
            let action = if live { "Attach" } else { "Resume" };
            let title = super::sidebar::session_title(session);
            let (location, _) = crate::labels::cloud_location(session.working_dir.as_deref());
            col = col.child(
                div()
                    .id(("suggested-session", i))
                    .debug_selector(move || format!("suggested-session-{i}"))
                    .a11y_button(format!("{action} {title}"))
                    .flex_none()
                    .min_h(px(44.))
                    .px(px(14.))
                    .py(px(4.))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .rounded(px(22.))
                    .bg(t.surface)
                    .cursor_pointer()
                    .hover(|d| d.bg(t.surface_elevated))
                    .active(|d| d.bg(t.surface_elevated))
                    .child(div().flex_none().size(px(5.)).rounded_full().bg(if live { t.ok } else { t.text_muted }))
                    .child(div().flex_1().min_w_0().flex().flex_col().gap(px(1.))
                        .child(super::label(title, t.text, 13.5).truncate())
                        .child(super::label(location, t.text_muted, 11.).truncate()))
                    .child(super::label("›", t.text_muted, 16.).flex_none())
                    .on_click(cx.listener(move |this, _, _, cx| this.drawer_open_session(sid.clone(), cx))),
            );
        }
        if !dirs.is_empty() {
            col = col.child(super::label("Recent directories", t.text_muted, 11.).pl(px(2.)).pt(px(16.)).pb(px(2.)));
        }
        for (i, dir) in dirs.into_iter().enumerate() {
            let (name, parent) = crate::labels::cloud_location(Some(&dir));
            col = col.child(
                div()
                    .id(("recent-dir", i))
                    .debug_selector(move || format!("recent-dir-{i}"))
                    .flex_none()
                    .h(px(44.))
                    .px(px(12.))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .rounded(px(12.))
                    .bg(t.surface)
                    .cursor_pointer()
                    .child(svg().path("icons/cloud.svg").size(px(14.)).flex_none().text_color(t.text_muted))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .items_baseline()
                            .gap(px(6.))
                            .child(div().flex_none().text_size(crate::system::fs(13.5)).text_color(t.text).child(name))
                            .when_some(parent, |d, p| {
                                d.child(div().min_w_0().truncate().text_size(crate::system::fs(11.)).text_color(t.text_muted).child(p))
                            }),
                    )
                    .child(div().flex_none().text_size(crate::system::fs(14.)).text_color(t.text_muted).child("›"))
                    .on_click(cx.listener(move |this, _, _, cx| this.start_in_dir(dir.clone(), cx))),
            );
        }
        Some(col.into_any_element())
    }
}

/// Keep running agents ahead of ready/connected sessions, then sort by activity.
/// Sessions already represented by any open tab are never suggestions.
fn suggested_sessions<'a>(sessions: &'a [SessionInfo], open_sessions: &[&str]) -> Vec<&'a SessionInfo> {
    let mut rows: Vec<_> = sessions.iter()
        .filter(|s| !s.archived && s.parent_session_id.is_none() && !open_sessions.contains(&s.session_id.as_str()))
        .collect();
    rows.sort_by_key(|s| std::cmp::Reverse((
        s.status == "running",
        s.user_connected,
        s.updated_at_ms.or(s.last_active_at_ms).map(|v| v.max(0) as u64)
            .or_else(|| super::sidebar::created_ms(&s.session_id)).unwrap_or(0),
    )));
    rows.dedup_by(|a, b| a.session_id == b.session_id);
    rows.truncate(3);
    rows
}

/// Distinct working dirs, newest session first.
fn recent_dirs(sessions: &[SessionInfo]) -> Vec<String> {
    let mut rows: Vec<&SessionInfo> = sessions.iter().filter(|s| s.parent_session_id.is_none()).collect();
    rows.sort_by_key(|s| std::cmp::Reverse(super::sidebar::created_ms(&s.session_id).unwrap_or(0)));
    let mut out: Vec<String> = vec![];
    for s in rows {
        let Some(d) = s.working_dir.as_deref().map(str::trim).filter(|d| !d.is_empty()) else { continue };
        if !out.iter().any(|o| o.trim_end_matches('/') == d.trim_end_matches('/')) {
            out.push(d.to_string());
        }
        if out.len() == MAX_DIRS {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn directory_haptics_confirm_changes_not_invalid_or_unchanged_paths(cx: &mut gpui::TestAppContext) {
        use crate::haptics::{capture, Haptic};
        cx.update(super::super::init);
        let (view, cx) = cx.add_window_view(|window, cx| {
            MobileApp::new_offline(super::super::SafeArea { top: 20., bottom: 0. }, window, cx)
        });
        view.update(cx, |app, cx| {
            app.account = Some(jcode_mobile_core::Account {
                given_name: None,
                name: None,
                api_key: "offline-fixture".into(), api_base: "https://offline.invalid".into(), email: None,
            });
            app.session.working_dir = Some("/repo".into());
            let (_, events) = capture(|| {
                app.change_dir("  ".into(), cx);
                app.change_dir(" /repo/ ".into(), cx);
            });
            assert!(events.is_empty());
            let (_, events) = capture(|| app.change_dir("/other".into(), cx));
            assert_eq!(events, [Haptic::Selection]);
            assert_eq!(app.session.working_dir.as_deref(), Some("/other"));
        });
    }

    #[gpui::test]
    fn edge_swipe_opens_session_directly_and_preserves_draft(cx: &mut gpui::TestAppContext) {
        cx.update(super::super::init);
        let (view, cx) = cx.add_window_view(|window, cx| {
            MobileApp::new_offline(super::super::SafeArea { top: 20., bottom: 0. }, window, cx)
        });
        view.update(cx, |app, cx| {
            app.account = Some(jcode_mobile_core::Account {
                given_name: None,
                name: None,
                api_key: "offline-fixture".into(), api_base: "https://offline.invalid".into(), email: None,
            });
            let original = app.active_tab;
            app.session.working_dir = Some("/repo".into());
            app.composer.update(cx, |input, cx| input.set_text("Keep my draft", cx));
            app.swipe.feed(-crate::swipe::COMMIT, 0., std::time::Instant::now());
            app.end_swipe(cx);
            assert!(!app.dir_picker);
            assert_eq!(app.active_tab, original);
            assert_eq!(app.tab_order.len(), 1);
            app.swipe.feed(-crate::swipe::NEW_SESSION_COMMIT, 0., std::time::Instant::now());
            app.end_swipe(cx);
            assert!(!app.dir_picker);
            assert_ne!(app.active_tab, original);
            assert_eq!(app.tab_order.len(), 2);
            assert!(app.session.working_dir.is_none());
            assert!(app.composer.read(cx).text().is_empty());
            assert!(app.connection.is_none());
            app.focus_tab(original, cx);
            assert_eq!(app.composer.read(cx).text(), "Keep my draft");
        });
    }

    #[test]
    fn suggestions_prioritize_live_then_activity_and_hide_ineligible_sessions() {
        let sessions: Vec<SessionInfo> = serde_json::from_value(serde_json::json!([
            {"session_id":"current", "status":"running", "updated_at_ms":1000},
            {"session_id":"archived", "status":"running", "archived":true},
            {"session_id":"agent", "status":"running", "parent_session_id":"owner"},
            {"session_id":"old", "status":"idle", "updated_at_ms":1},
            {"session_id":"recent", "status":"idle", "updated_at_ms":100},
            {"session_id":"running", "status":"running", "last_active_at_ms":2},
            {"session_id":"connected", "status":"idle", "user_connected":true, "updated_at_ms":3}
        ])).unwrap();
        let ids: Vec<_> = suggested_sessions(&sessions, &["current"])
            .iter().map(|s| s.session_id.as_str()).collect();
        assert_eq!(ids, ["running", "connected", "recent"]);
    }

    #[test]
    fn suggestions_exclude_all_open_tabs_before_limiting_results() {
        let sessions: Vec<SessionInfo> = serde_json::from_value(serde_json::json!([
            {"session_id":"active", "status":"running", "updated_at_ms":1000},
            {"session_id":"parked-running", "status":"running", "updated_at_ms":900},
            {"session_id":"parked-ready", "status":"idle", "user_connected":true, "updated_at_ms":800},
            {"session_id":"ready", "status":"idle", "user_connected":true, "updated_at_ms":700},
            {"session_id":"justrust-agent", "status":"running", "updated_at_ms":1},
            {"session_id":"recent", "status":"idle", "updated_at_ms":600},
            {"session_id":"older", "status":"idle", "updated_at_ms":500}
        ])).unwrap();
        let ids: Vec<_> = suggested_sessions(&sessions, &["active", "parked-running", "parked-ready"])
            .iter().map(|s| s.session_id.as_str()).collect();
        assert_eq!(ids, ["justrust-agent", "ready", "recent"]);
        let all_open: Vec<_> = sessions.iter().map(|s| s.session_id.as_str()).collect();
        assert!(suggested_sessions(&sessions, &all_open).is_empty());
    }

    #[test]
    fn suggestions_allow_missing_metadata_and_empty_lists() {
        assert!(suggested_sessions(&[], &[]).is_empty());
        let sessions = serde_json::from_value::<Vec<SessionInfo>>(serde_json::json!([
            {"session_id":"session_older_1700000000000", "status":"idle"},
            {"session_id":"session_newer_1700000001000", "status":"idle"}
        ])).unwrap();
        assert_eq!(suggested_sessions(&sessions, &[])[0].session_id, "session_newer_1700000001000");
    }
}
