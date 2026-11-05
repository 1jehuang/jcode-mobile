//! Empty-session quick nav: the blank middle of a fresh tab lists the cloud
//! directories used most recently, and tapping one restarts the tab there.

use gpui::{AnyElement, Context, div, prelude::*, px, svg};
use jcode_mobile_core::Request;
use jcode_mobile_core::jcode_sdk::SessionInfo;

use super::MobileApp;
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

    pub(super) fn render_recent_dirs(&self, t: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        // Any tab's list will do; the active one may not have fetched yet.
        let lists = std::iter::once(&self.session.session_list).chain(self.parked.iter().map(|p| &p.session.session_list));
        let list = lists.max_by_key(|l| l.len())?;
        let current = self.session.working_dir.as_deref().map(|d| d.trim_end_matches('/'));
        let dirs: Vec<String> = recent_dirs(list).into_iter().filter(|d| Some(d.trim_end_matches('/')) != current).collect();
        if dirs.is_empty() {
            return None;
        }
        let mut col = div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(div().pl(px(2.)).pb(px(2.)).text_size(crate::system::fs(11.)).text_color(t.text_muted).child("Recent directories"));
        for (i, dir) in dirs.into_iter().enumerate() {
            let (name, parent) = crate::labels::cloud_location(Some(&dir));
            col = col.child(
                div()
                    .id(("recent-dir", i))
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
