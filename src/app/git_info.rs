//! Dedicated top-right pull target. Git panel gesture handling. Git reads never enter the conversation.
use super::*;
use std::{cell::Cell, rc::Rc};
use gpui::{Bounds, Pixels, Point};
use jcode_mobile_core::git_info::{self, GitSnapshot};

const TRIGGER: f32 = 48.;

#[derive(Default)]
pub(super) struct GitState {
    pub open: bool,
    bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    origin: Option<Point<Pixels>>,
    pull: f32,
    moved: bool,
    loading: bool,
    snapshot: Option<GitSnapshot>,
    error: Option<String>,
    context: Option<(u64, Option<String>, Option<String>)>,
    generation: u64,
}

impl MobileApp {
    fn git_context(&self) -> (u64, Option<String>, Option<String>) {
        (self.active_tab, self.session.session_id.clone(), self.session.working_dir.clone())
    }

    pub(super) fn git_down(&mut self, point: Point<Pixels>) -> bool {
        if self.screen != Screen::Chat || self.drawer_open || self.model_picker || self.dir_picker
            || self.tab_menu.is_some() || self.git_info.open { return false; }
        if !self.git_info.bounds.get().is_some_and(|b| b.contains(&point)) { return false; }
        self.feel.g.cancel();
        self.git_info.origin = Some(point);
        self.git_info.pull = 0.;
        self.git_info.moved = false;
        true
    }

    pub(super) fn git_move(&mut self, point: Point<Pixels>, cx: &mut Context<Self>) -> bool {
        let Some(origin) = self.git_info.origin else { return false; };
        let dx = f32::from(point.x - origin.x).abs();
        let dy = f32::from(point.y - origin.y);
        self.git_info.moved |= dx > 6. || dy.abs() > 6.;
        self.git_info.pull = if dx > dy.abs().max(12.) { 0. } else { dy.clamp(0., 96.) };
        cx.notify();
        true
    }

    pub(super) fn git_up(&mut self, cancelled: bool, cx: &mut Context<Self>) -> bool {
        if self.git_info.origin.take().is_none() { return false; }
        let open = !cancelled && (self.git_info.pull >= TRIGGER || !self.git_info.moved);
        self.git_info.pull = 0.;
        if open { self.open_git(cx); }
        cx.notify();
        true
    }

    fn open_git(&mut self, cx: &mut Context<Self>) {
        self.git_info.open = true;
        haptics::play(Haptic::Light);
        self.refresh_git(cx);
        cx.notify();
    }

    fn refresh_git(&mut self, cx: &mut Context<Self>) {
        let context = self.git_context();
        self.git_info.context = Some(context.clone());
        self.git_info.snapshot = None;
        self.git_info.error = None;
        self.git_info.loading = false;
        self.git_info.generation += 1;
        let generation = self.git_info.generation;
        let Some(directory) = self.session.working_dir.clone() else {
            self.git_info.error = Some("Choose a session directory to inspect its Git repository.".into());
            return;
        };
        let Some(account) = self.account.clone().filter(|_| !self.offline) else {
            self.git_info.error = Some("Connect to Jcode Cloud to inspect this repository.".into());
            return;
        };
        self.git_info.loading = true;
        let expected = account.clone();
        let task = cx.background_spawn(async move { git_info::inspect(&account, &directory) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                if this.account.as_ref() != Some(&expected) || this.git_info.generation != generation
                    || this.git_context() != context { return; }
                this.git_info.loading = false;
                match result {
                    Ok(snapshot) => this.git_info.snapshot = Some(snapshot),
                    Err(_) => this.git_info.error = Some("Could not read Git information. Check the connection and that this directory is in a Git repository, then refresh.".into()),
                }
                cx.notify();
            }).ok();
        }).detach();
    }

    pub(super) fn render_git_handle(&mut self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        // Session changes invalidate both cached content and in-flight responses.
        if self.git_info.context.as_ref().is_some_and(|c| *c != self.git_context()) {
            self.git_info = GitState::default();
        }
        let bounds = self.git_info.bounds.clone();
        let pull = self.git_info.pull;
        div().flex_none().w_full().flex().justify_end().px(px(10.))
            .child(div().id("git-pull-handle").debug_selector(|| "git-pull-handle".into())
                .a11y_button("Show Git information. Pull down or activate.")
                .relative().w(px(88.)).h(px(44. + pull * 0.4)).flex().flex_col()
                .items_center().justify_center().gap(px(3.)).rounded(px(12.))
                .bg(if pull >= TRIGGER { t.surface } else { t.background })
                .cursor_pointer().text_color(t.text_muted).text_size(crate::system::fs(11.))
                .child(if pull >= TRIGGER { "Release · Git" } else { "Git ↓" })
                .child(div().w(px(28.)).h(px(3.)).rounded_full().bg(t.border_focus))
                .on_click(cx.listener(|this, _, _, cx| this.open_git(cx)))
                .child(gpui::canvas(move |b, _, _| bounds.set(Some(b)), |_, _, _, _| {})
                    .absolute().inset_0()))
            .into_any_element()
    }

    pub(super) fn render_git_panel(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let state = &self.git_info;
        div().absolute().inset_0().flex().flex_col().bg(with_alpha(t.background, 0.98))
            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
            .child(div().flex_none().flex().items_center().justify_between().px(px(16.)).py(px(10.))
                .child(label("Git information", t.text_strong, 18.))
                .child(pill("close-git", "Done", t).on_click(cx.listener(|this, _, _, cx| {
                    this.git_info.open = false;
                    cx.notify();
                }))))
            .child(div().id("git-info-panel").debug_selector(|| "git-info-panel".into())
                .flex_1().min_h_0().overflow_y_scroll().px(px(16.)).pb(px(16.))
                .child(div().w_full().min_w_0().flex().flex_col().gap(px(12.))
                    .child(label(self.session.working_dir.clone().unwrap_or_else(|| "No session directory".into()), t.text_muted, 12.))
                    .child(label("Read-only snapshot. No fetch, commits, or file changes.", t.text_muted, 11.))
                    .child(outline_button("refresh-git", if state.loading { "Reading Git…" } else { "Refresh" }, !state.loading, t)
                        .when(!state.loading, |d| d.on_click(cx.listener(|this, _, _, cx| {
                            this.refresh_git(cx);
                            cx.notify();
                        }))))
                    .when_some(state.error.clone(), |d, error| d.child(label(error, t.error, 12.)))
                    .when_some(state.snapshot.as_ref(), |mut d, snapshot| {
                        for (title, text) in [("Repository", snapshot.root.as_str()), ("Branch", snapshot.branch.as_str()),
                            ("Working tree", snapshot.status.as_str()), ("Recent commits", snapshot.recent_commits.as_str())] {
                            d = d.child(label(title, t.text_strong, 13.))
                                .child(label(text.to_owned(), t.text_secondary, 12.));
                        }
                        d
                    })))
            .into_any_element()
    }
}
