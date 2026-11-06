//! Dedicated top-right pull target. Git panel gesture handling. Git reads never enter the conversation.
use super::*;
use gpui::{Bounds, Pixels, Point};
use jcode_mobile_core::git_info::{self, GitSnapshot};
use std::{cell::Cell, rc::Rc};

#[cfg(test)]
#[path = "git_info_tests.rs"]
mod tests;

const TRIGGER: f32 = 48.;

#[cfg(test)]
mod lifecycle_tests {
    use super::super::lifecycle_coverage::{fixture_account, fixture_app};
    use super::*;
    use gpui::{Modifiers, ScrollDelta, ScrollWheelEvent, TestAppContext, TouchPhase, point, size};

    fn snapshot(branch: &str) -> GitSnapshot {
        GitSnapshot {
            root: "/fixture".into(),
            branch: branch.into(),
            status: String::new(),
            recent_commits: "fixture commit".into(),
        }
    }

    #[gpui::test]
    fn mocked_git_results_reject_stale_refresh_account_tab_session_and_directory(
        cx: &mut TestAppContext,
    ) {
        cx.update(init);
        let (view, cx) = cx.add_window_view(fixture_app);
        view.update(cx, |app, cx| {
            app.session.working_dir = Some("/fixture".into());
            app.session.session_id = Some("fixture-session".into());
            app.open_git(cx);
            assert!(app.git_info.open && !app.git_info.loading);
            assert!(
                app.git_info.error.is_some(),
                "offline refresh must short-circuit"
            );
            let old_generation = app.git_info.generation;
            app.refresh_git(cx);
            assert_ne!(app.git_info.generation, old_generation);
            let generation = app.git_info.generation;
            let context = app.git_context();
            let account = fixture_account();
            // Fake service completions exercise the actual completion seam.
            app.git_info.error = None;
            app.git_info.loading = true;
            app.finish_git(&account, old_generation, &context, Ok(snapshot("old")), cx);
            assert!(app.git_info.loading && app.git_info.snapshot.is_none());
            let mut wrong_account = account.clone();
            wrong_account.api_key = "different fixture".into();
            app.finish_git(
                &wrong_account,
                generation,
                &context,
                Ok(snapshot("wrong-account")),
                cx,
            );
            assert!(app.git_info.loading && app.git_info.snapshot.is_none());
            for stale in [
                (context.0 + 1, context.1.clone(), context.2.clone()),
                (context.0, Some("other-session".into()), context.2.clone()),
                (
                    context.0,
                    context.1.clone(),
                    Some("/other-directory".into()),
                ),
            ] {
                app.finish_git(
                    &account,
                    generation,
                    &stale,
                    Ok(snapshot("wrong-context")),
                    cx,
                );
                assert!(app.git_info.loading && app.git_info.snapshot.is_none());
            }
            app.finish_git(&account, generation, &context, Ok(snapshot("main")), cx);
            assert!(!app.git_info.loading);
            assert_eq!(app.git_info.snapshot.as_ref().unwrap().branch, "main");
            app.refresh_git(cx);
            assert!(app.git_info.snapshot.is_none());
            app.git_info.loading = true;
            app.finish_git(
                &account,
                app.git_info.generation,
                &context,
                Err("secret transport detail".into()),
                cx,
            );
            assert!(!app.git_info.loading);
            assert!(!app.git_info.error.as_ref().unwrap().contains("secret"));
            app.sign_out(cx);
            app.finish_git(
                &account,
                generation,
                &context,
                Ok(snapshot("after-logout")),
                cx,
            );
            assert!(!app.git_info.open && app.git_info.snapshot.is_none());
        });
    }

    #[gpui::test]
    fn git_overlay_wheel_at_boundaries_never_scrolls_or_releases_transcript(
        cx: &mut TestAppContext,
    ) {
        cx.update(init);
        let (view, cx) = cx.add_window_view(|window, cx| {
            let mut app = fixture_app(window, cx);
            app.session.transcript.push(TranscriptEntry {
                id: 1,
                role: Role::Assistant,
                text: "Long fixture transcript\n\n".repeat(100),
                reasoning: String::new(),
                tool_calls: Vec::new(),
                is_streaming: false,
                is_queued: false,
            });
            app.open_git(cx);
            app
        });
        cx.simulate_resize(size(px(390.), px(844.)));
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
        });
        view.update(cx, |app, cx| {
            assert!(app.transcript_scroll.max_offset().y > px(100.));
            app.transcript_scroll.scroll_to_bottom();
            app.follow = true;
            cx.notify();
        });
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
        });
        let before = view.read_with(cx, |app, _| app.transcript_scroll.offset());
        let panel = cx.debug_bounds("git-info-panel").unwrap();
        for position in [panel.center(), point(px(100.), px(30.))] {
            for delta in [-80., 80.] {
                for touch_phase in [TouchPhase::Moved, TouchPhase::Ended, TouchPhase::Cancelled] {
                    cx.simulate_event(ScrollWheelEvent {
                        position,
                        delta: ScrollDelta::Pixels(point(px(0.), px(delta))),
                        touch_phase,
                        modifiers: Modifiers::default(),
                    });
                    cx.update(|window, cx| {
                        window.draw(cx).clear(cx);
                    });
                    view.read_with(cx, |app, _| {
                        assert!(app.follow && app.git_info.open);
                        assert_eq!(app.transcript_scroll.offset(), before);
                        assert!(!app.voice.busy && !app.voice.starting);
                        assert!(app.feel.g.refreshing.is_none());
                    });
                }
            }
        }
    }
}

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
        (
            self.active_tab,
            self.session.session_id.clone(),
            self.session.working_dir.clone(),
        )
    }

    pub(super) fn git_down(&mut self, point: Point<Pixels>) -> bool {
        if self.screen != Screen::Chat
            || self.drawer_open
            || self.model_picker
            || self.dir_picker
            || self.tab_menu.is_some()
            || self.git_info.open
        {
            return false;
        }
        if !self
            .git_info
            .bounds
            .get()
            .is_some_and(|b| b.contains(&point))
        {
            return false;
        }
        self.feel.g.cancel();
        self.git_info.origin = Some(point);
        self.git_info.pull = 0.;
        self.git_info.moved = false;
        true
    }

    pub(super) fn git_move(&mut self, point: Point<Pixels>, cx: &mut Context<Self>) -> bool {
        let Some(origin) = self.git_info.origin else {
            return false;
        };
        let dx = f32::from(point.x - origin.x).abs();
        let dy = f32::from(point.y - origin.y);
        self.git_info.moved |= dx > 6. || dy.abs() > 6.;
        self.git_info.pull = if dx > dy.abs().max(12.) {
            0.
        } else {
            dy.clamp(0., 96.)
        };
        cx.notify();
        true
    }

    pub(super) fn git_up(&mut self, cancelled: bool, cx: &mut Context<Self>) -> bool {
        if self.git_info.origin.take().is_none() {
            return false;
        }
        let open = !cancelled && (self.git_info.pull >= TRIGGER || !self.git_info.moved);
        self.git_info.pull = 0.;
        if open {
            self.open_git(cx);
        }
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
        static NEXT_REQUEST: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let generation = NEXT_REQUEST.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.git_info.generation = generation;
        let Some(directory) = self.session.working_dir.clone() else {
            self.git_info.error =
                Some("Choose a session directory to inspect its Git repository.".into());
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
                this.finish_git(&expected, generation, &context, result, cx);
            })
            .ok();
        })
        .detach();
    }

    /// Shared by service completion and deterministic fake-result tests.
    fn finish_git(
        &mut self,
        expected: &Account,
        generation: u64,
        context: &(u64, Option<String>, Option<String>),
        result: Result<GitSnapshot, String>,
        cx: &mut Context<Self>,
    ) {
        if self.account.as_ref() != Some(expected)
            || self.git_info.generation != generation
            || &self.git_context() != context
        {
            return;
        }
        self.git_info.loading = false;
        match result {
            Ok(snapshot) => self.git_info.snapshot = Some(snapshot),
            Err(_) => self.git_info.error = Some("Could not read Git information. Check the connection and that this directory is in a Git repository, then refresh.".into()),
        }
        cx.notify();
    }

    pub(super) fn render_git_handle(&mut self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        // Session changes invalidate both cached content and in-flight responses.
        if self
            .git_info
            .context
            .as_ref()
            .is_some_and(|c| *c != self.git_context())
        {
            self.git_info = GitState::default();
        }
        let bounds = self.git_info.bounds.clone();
        let pull = self.git_info.pull;
        div()
            .flex_none()
            .w_full()
            .flex()
            .justify_end()
            .px(px(10.))
            .child(
                div()
                    .id("git-pull-handle")
                    .debug_selector(|| "git-pull-handle".into())
                    .a11y_button("Show Git information. Pull down or activate.")
                    .relative()
                    .w(px(88.))
                    .h(px(44. + pull * 0.4))
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap(px(3.))
                    .rounded(px(12.))
                    .bg(if pull >= TRIGGER {
                        t.surface
                    } else {
                        t.background
                    })
                    .cursor_pointer()
                    .text_color(t.text_muted)
                    .text_size(crate::system::fs(11.))
                    .child(if pull >= TRIGGER {
                        "Release · Git"
                    } else {
                        "Git ↓"
                    })
                    .child(div().w(px(28.)).h(px(3.)).rounded_full().bg(t.border_focus))
                    .on_click(cx.listener(|this, _, _, cx| this.open_git(cx)))
                    .child(
                        gpui::canvas(move |b, _, _| bounds.set(Some(b)), |_, _, _, _| {})
                            .absolute()
                            .inset_0(),
                    ),
            )
            .into_any_element()
    }

    pub(super) fn render_git_panel(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let state = &self.git_info;
        div()
            .occlude()
            .absolute()
            .inset_0()
            .flex()
            .flex_col()
            .bg(with_alpha(t.background, 0.98))
            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px(px(16.))
                    .py(px(10.))
                    .child(label("Git information", t.text_strong, 18.))
                    .child(
                        pill("close-git", "Done", t).on_click(cx.listener(|this, _, _, cx| {
                            this.git_info.open = false;
                            cx.notify();
                        })),
                    ),
            )
            .child(
                div()
                    .id("git-info-panel")
                    .debug_selector(|| "git-info-panel".into())
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px(px(16.))
                    .pb(px(16.))
                    .child(
                        div()
                            .w_full()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(12.))
                            .child(label(
                                self.session
                                    .working_dir
                                    .clone()
                                    .unwrap_or_else(|| "No session directory".into()),
                                t.text_muted,
                                12.,
                            ))
                            .child(label(
                                "Read-only snapshot. No fetch, commits, or file changes.",
                                t.text_muted,
                                11.,
                            ))
                            .child(
                                outline_button(
                                    "refresh-git",
                                    if state.loading {
                                        "Reading Git…"
                                    } else {
                                        "Refresh"
                                    },
                                    !state.loading,
                                    t,
                                )
                                .when(!state.loading, |d| {
                                    d.on_click(cx.listener(|this, _, _, cx| {
                                        this.refresh_git(cx);
                                        cx.notify();
                                    }))
                                }),
                            )
                            .when_some(state.error.clone(), |d, error| {
                                d.child(label(error, t.error, 12.))
                            })
                            .when_some(state.snapshot.as_ref(), |mut d, snapshot| {
                                for (title, text) in [
                                    ("Repository", snapshot.root.as_str()),
                                    ("Branch", snapshot.branch.as_str()),
                                    (
                                        "Working tree",
                                        if snapshot.status.is_empty() {
                                            "Clean working tree"
                                        } else {
                                            snapshot.status.as_str()
                                        },
                                    ),
                                    (
                                        "Recent commits",
                                        if snapshot.recent_commits.is_empty() {
                                            "No commits yet"
                                        } else {
                                            snapshot.recent_commits.as_str()
                                        },
                                    ),
                                ] {
                                    d = d.child(label(title, t.text_strong, 13.)).child(label(
                                        text.to_owned(),
                                        t.text_secondary,
                                        12.,
                                    ));
                                }
                                d
                            }),
                    ),
            )
            .into_any_element()
    }
}
