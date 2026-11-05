//! Empty-session quick nav: the blank middle of a fresh tab lists the cloud
//! directories used most recently, and tapping one restarts the tab there.

use gpui::{AnyElement, Context, Window, div, prelude::*, px, svg};
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

    /// Open the directory sheet from the composer's location label.
    pub(super) fn open_dir_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
        self.dir_picker = false;
        let dir = dir.trim().to_string();
        if dir.is_empty() {
            cx.notify();
            return;
        }
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
                    .on_click(cx.listener(move |this, _, _, cx| this.change_dir(dir.clone(), cx))),
            );
        }
        let note = if self.session.transcript.is_empty() {
            "Restarts this tab in the chosen directory."
        } else {
            "Opens a new tab there. This conversation stays put."
        };
        div()
            .absolute()
            .inset_0()
            .flex()
            .flex_col()
            .bg(gpui::black().opacity(0.5))
            .child(div().id("dir-dismiss").flex_1().on_click(cx.listener(|this, _, _, cx| {
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
                    .child(super::label("Directory", t.text_strong, 16.).px(px(16.)).pt(px(8.)))
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
                            .child(div().flex_1().min_w_0().child(self.dir_input.clone()))
                            .child(
                                div()
                                    .id("dir-go")
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
                    .child(super::label(note, t.text_muted, 11.).px(px(16.)))
                    .child(
                        div()
                            .id("dir-list")
                            .flex_1()
                            .overflow_y_scroll()
                            .px(px(12.))
                            .child(rows),
                    ),
            )
            .into_any_element()
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
