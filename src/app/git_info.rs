//! Dedicated top-right pull target. Git panel gesture handling. Git reads never enter the conversation.
use super::*;
use gpui::{Bounds, Pixels, Point};
use jcode_mobile_core::git_info::{self, GitSnapshot};
use std::{cell::Cell, rc::Rc};

#[cfg(test)]
#[path = "git_info_tests.rs"]
mod tests;

const TRIGGER: f32 = 48.;

/// A terminal-style frame with its title interrupting the top border.
fn git_frame(id: &'static str, title: &'static str, t: &Theme) -> gpui::Div {
    div()
        .debug_selector(move || id.into())
        .relative()
        .w_full()
        .min_w_0()
        .flex()
        .flex_col()
        .gap(px(6.))
        .mt(px(8.))
        .px(px(9.))
        .pt(px(15.))
        .pb(px(9.))
        .border_1()
        .border_color(t.border_focus)
        .rounded(px(5.))
        .child(
            label(title, t.text_secondary, 12.)
                .absolute()
                .top(px(-9.))
                .left(px(8.))
                .px(px(5.))
                .bg(t.background),
        )
}

fn git_row() -> gpui::Div {
    div().w_full().min_w_0().flex().items_center().gap(px(6.))
}

fn commit_columns(line: &str) -> (&str, &str, &str) {
    let mut columns = line.splitn(3, '\t');
    let hash = columns.next().unwrap_or_default();
    if let Some(subject) = columns.next() {
        (hash, subject, columns.next().unwrap_or_default())
    } else {
        let (hash, subject) = line.split_once(' ').unwrap_or((line, ""));
        (hash, subject, "")
    }
}

fn compact_age(age: &str) -> String {
    let mut words = age.split_whitespace();
    let first = words.next().unwrap_or_default();
    let number = if first == "a" || first == "an" {
        "1"
    } else {
        first
    };
    if number.parse::<u32>().is_ok() {
        let unit = match words.next().unwrap_or_default() {
            "second" | "seconds" => "s",
            "minute" | "minutes" => "m",
            "hour" | "hours" => "h",
            "day" | "days" => "d",
            "week" | "weeks" => "w",
            "month" | "months" => "mo",
            "year" | "years" => "y",
            _ => return age.to_owned(),
        };
        return format!("{number}{unit}");
    }
    age.to_owned()
}

fn status_columns(line: &str) -> Option<(&str, &str)> {
    // Porcelain status codes are two ASCII bytes, followed by a space.
    if line.len() < 4 || !line.is_char_boundary(2) || line.as_bytes()[2] != b' ' {
        return None;
    }
    Some((&line[..2], &line[3..]))
}

fn file_stat<'a>(path: &str, stats: &'a str) -> Option<(&'a str, &'a str)> {
    stats.lines().find_map(|line| {
        let mut columns = line.splitn(3, '\t');
        let added = columns.next()?;
        let removed = columns.next()?;
        (columns.next()? == path).then_some((added, removed))
    })
}

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
            ..Default::default()
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
    show_all_files: bool,
    show_all_commits: bool,
}

impl MobileApp {
    fn render_git_commits(
        &self,
        snapshot: &GitSnapshot,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let commits: Vec<_> = snapshot.recent_commits.lines().collect();
        let limit = if self.git_info.show_all_commits {
            commits.len()
        } else {
            5
        };
        let mut panel = git_frame("git-commits", "Commits", t);
        for line in commits.iter().take(limit) {
            let (hash, subject, age) = commit_columns(line);
            panel = panel.child(
                git_row()
                    .child(label("●", gpui::rgb(0x8fb8ff).into(), 11.).flex_none())
                    .child(label(hash.to_owned(), t.warning, 12.).flex_none())
                    .child(
                        label(subject.to_owned(), t.text, 12.)
                            .flex_1()
                            .min_w_0()
                            .truncate(),
                    )
                    .child(label(compact_age(age), t.text_muted, 10.).flex_none()),
            );
        }
        if commits.is_empty() {
            panel = panel.child(label("No commits yet", t.text_muted, 12.));
        }
        let mut footer = git_row().flex_wrap().justify_between();
        if commits.len() > 5 {
            footer = footer.child(
                div()
                    .id("git-more-commits")
                    .debug_selector(|| "git-more-commits".into())
                    .a11y_button("Toggle all recent commits")
                    .cursor_pointer()
                    .py(px(6.))
                    .child(label(
                        if self.git_info.show_all_commits {
                            "Show fewer".into()
                        } else {
                            format!("+{} more", commits.len() - 5)
                        },
                        t.text_muted,
                        11.,
                    ))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.git_info.show_all_commits = !this.git_info.show_all_commits;
                        cx.notify();
                    })),
            );
        }
        if snapshot.ahead > 0 || snapshot.behind > 0 {
            footer = footer.child(label(
                format!("↑{} unpushed  ↓{} behind", snapshot.ahead, snapshot.behind),
                gpui::rgb(0x8fb8ff).into(),
                11.,
            ));
        }
        panel.child(footer).into_any_element()
    }

    fn render_git_overview(
        &self,
        snapshot: &GitSnapshot,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let files: Vec<_> = snapshot.status.lines().filter_map(status_columns).collect();
        let limit = if self.git_info.show_all_files {
            files.len()
        } else {
            5
        };
        let mut panel = git_frame("git-overview", "Overview", t).child(
            git_row()
                .child(label("◆", gpui::rgb(0x8fb8ff).into(), 12.))
                .child(
                    label(snapshot.branch.clone(), t.text_secondary, 12.)
                        .min_w_0()
                        .flex_1()
                        .truncate(),
                )
                .child(label(format!("{} changed", files.len()), t.text_muted, 11.).flex_none()),
        );
        for (code, path) in files.iter().take(limit) {
            let color = if code.contains('?') {
                t.text_muted
            } else if code.contains('D') {
                t.error
            } else if code.contains('A') {
                t.ok
            } else {
                t.warning
            };
            let mut row = git_row()
                .child(label(code.to_owned(), color, 12.).flex_none())
                .child(
                    label(path.to_owned(), t.text_secondary, 12.)
                        .flex_1()
                        .min_w_0()
                        .truncate(),
                );
            if let Some((added, removed)) = file_stat(path, &snapshot.diff_stat) {
                row = row.child(
                    label(
                        if added == "-" {
                            "binary".into()
                        } else {
                            format!("+{added}")
                        },
                        t.ok,
                        11.,
                    )
                    .flex_none(),
                );
                if removed != "-" {
                    row = row.child(label(format!("−{removed}"), t.error, 11.).flex_none());
                }
            }
            panel = panel.child(row);
        }
        if files.is_empty() {
            panel = panel.child(label("✓ Clean working tree", t.ok, 12.));
        }
        if files.len() > 5 {
            panel = panel.child(
                div()
                    .id("git-more-files")
                    .debug_selector(|| "git-more-files".into())
                    .a11y_button("Toggle all changed files")
                    .cursor_pointer()
                    .py(px(6.))
                    .child(label(
                        if self.git_info.show_all_files {
                            "Show fewer".into()
                        } else {
                            format!("+{} more", files.len() - 5)
                        },
                        t.text_muted,
                        11.,
                    ))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.git_info.show_all_files = !this.git_info.show_all_files;
                        cx.notify();
                    })),
            );
        }
        let (added, removed) = snapshot
            .diff_stat
            .lines()
            .fold((0_u64, 0_u64), |(a, r), line| {
                let mut columns = line.split('\t');
                (
                    a.saturating_add(
                        columns
                            .next()
                            .and_then(|n| n.parse::<u64>().ok())
                            .unwrap_or(0),
                    ),
                    r.saturating_add(
                        columns
                            .next()
                            .and_then(|n| n.parse::<u64>().ok())
                            .unwrap_or(0),
                    ),
                )
            });
        if !snapshot.diff_stat.is_empty() {
            panel = panel.child(
                git_row()
                    .justify_end()
                    .child(label(format!("+{added}"), t.ok, 11.))
                    .child(label(format!("−{removed}"), t.error, 11.))
                    .child(label("all tracked", t.text_muted, 11.)),
            );
        }
        panel.into_any_element()
    }

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
                    .border_1()
                    .border_color(t.border)
                    .rounded(px(6.))
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
                        "◇ Git ↓"
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
                    .child(label("Repository", t.text_strong, 16.))
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
                            .when_some(state.snapshot.as_ref(), |d, snapshot| {
                                d.child(self.render_git_commits(snapshot, t, cx))
                                    .child(self.render_git_overview(snapshot, t, cx))
                                    .child(
                                        label(snapshot.root.clone(), t.text_muted, 11.).truncate(),
                                    )
                            }),
                    ),
            )
            .into_any_element()
    }
}
