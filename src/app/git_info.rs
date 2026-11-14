//! Dedicated right-edge pull tab. Git panel gesture handling. Git reads never enter the conversation.
use super::*;
use gpui::{Animation, AnimationExt as _, Bounds, Pixels, Point, Transformation, percentage, svg};
use jcode_mobile_core::git_info::{self, GitSnapshot};
use std::{cell::Cell, rc::Rc};

#[cfg(test)]
#[path = "git_info_tests.rs"]
mod tests;

const TRIGGER: f32 = 48.;

/// Folded section length. Longer lists expand in place.
const FOLD: usize = 5;

/// The pinned todo card's palette, so both overlays read as one family:
/// purple for refs, blue for local-only work.
fn rgb8(r: u8, g: u8, b: u8) -> Hsla {
    gpui::Rgba { r: r as f32 / 255., g: g as f32 / 255., b: b as f32 / 255., a: 1. }.into()
}
fn ref_color() -> Hsla {
    if crate::system::is_light() { rgb8(118, 82, 190) } else { rgb8(190, 165, 235) }
}
fn local_color() -> Hsla {
    if crate::system::is_light() { rgb8(36, 98, 210) } else { rgb8(110, 160, 245) }
}
/// Opaque card fill, lifted off the transcript in both appearances.
fn card_fill(t: &Theme) -> Hsla {
    if crate::system::is_light() { gpui::white() } else { t.surface }
}

/// Fixed row height, scaled with Dynamic Type, so the commit graph's rail
/// stays continuous from row to row.
fn row_height() -> f32 {
    f32::from(crate::system::fs(28.))
}

/// Width of `chars` monospace glyphs, for right-aligned numeric columns.
fn mono_width(chars: usize, size: f32) -> Pixels {
    px((chars as f32 * 0.6 * f32::from(crate::system::fs(size))).ceil() + 1.)
}

/// One titled section of the widget: an uppercase caption with a count and a
/// right-aligned summary, then its rows.
fn git_section(
    id: &'static str,
    title: &'static str,
    count: Option<usize>,
    summary: Option<AnyElement>,
    t: &Theme,
) -> gpui::Div {
    div()
        .debug_selector(move || id.into())
        .w_full()
        .min_w_0()
        .flex()
        .flex_col()
        .px(px(14.))
        .pt(px(10.))
        .pb(px(4.))
        .child(
            git_row()
                .h(px(24.))
                .gap(px(6.))
                .child(label(title, t.text_muted, 10.5).font_weight(gpui::FontWeight::BOLD))
                .when_some(count, |d, n| d.child(label(n.to_string(), t.text_muted, 10.5)))
                .child(div().flex_1())
                .children(summary),
        )
}

fn git_row() -> gpui::Div {
    div().w_full().min_w_0().flex().items_center().gap(px(8.))
}

/// Porcelain XY code to a single badge letter and tint: the staged side wins,
/// untracked reads as U, any unmerged pair as a red `!`.
fn status_badge(code: &str, t: &Theme) -> (&'static str, Hsla) {
    let bytes = code.as_bytes();
    let (x, y) = (bytes.first().copied().unwrap_or(b' '), bytes.get(1).copied().unwrap_or(b' '));
    if x == b'U' || y == b'U' || (x == y && matches!(x, b'A' | b'D')) {
        return ("!", t.error);
    }
    match if x == b' ' { y } else { x } {
        b'?' => ("U", t.ok),
        b'A' => ("A", t.ok),
        b'D' => ("D", t.error),
        b'R' => ("R", local_color()),
        b'C' => ("C", local_color()),
        b'M' | b'T' => ("M", t.warning),
        _ => ("·", t.text_muted),
    }
}

/// Split a status path into (directory, file name), following renames and
/// keeping the trailing slash of untracked directories on the name.
fn split_path(path: &str) -> (&str, &str) {
    let path = path.rsplit_once(" -> ").map_or(path, |(_, new)| new);
    let base = path.strip_suffix('/').unwrap_or(path);
    match base.rfind('/') {
        Some(i) => (&path[..i], &path[i + 1..]),
        None => ("", path),
    }
}

fn diff_totals(stats: &str) -> (u64, u64) {
    stats.lines().fold((0, 0), |(a, r), line| {
        let mut columns = line.split('\t');
        let mut next = || columns.next().and_then(|n| n.parse::<u64>().ok()).unwrap_or(0);
        (a.saturating_add(next()), r.saturating_add(next()))
    })
}

/// Numstat cells for one file: `+a` (or `bin`) and `−r`, blank for zero.
fn stat_cells(stat: Option<(&str, &str)>) -> (String, String) {
    match stat {
        Some(("-", _)) => ("bin".into(), String::new()),
        Some((a, r)) => (
            if a == "0" { String::new() } else { format!("+{a}") },
            if r == "0" { String::new() } else { format!("−{r}") },
        ),
        None => (String::new(), String::new()),
    }
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
    /// Preview-only fixtures for `JCODE_MOBILE_DEMO=git[-expanded|-loading|-error|-clean]`.
    pub fn seed_git_demo(&mut self, demo: &str) {
        self.git_info.open = true;
        let expanded = demo == "git-expanded";
        self.git_info.show_all_files = expanded;
        self.git_info.show_all_commits = expanded;
        match demo {
            "git-loading" => self.git_info.loading = true,
            "git-error" => {
                self.git_info.error = Some("Could not read Git information. Check the connection and that this directory is in a Git repository, then refresh.".into())
            }
            _ => {
                let clean = demo == "git-clean";
                self.git_info.snapshot = Some(GitSnapshot {
                    root: "/home/demo/jcode-mobile".into(),
                    branch: if clean { "main".into() } else { "feat/git-widget-redesign".into() },
                    status: if clean {
                        String::new()
                    } else {
                        " M src/app/git_info.rs\nM  src/main.rs\nA  assets/icons/git-branch.svg\n D docs/old-notes.md\nR  src/app/a.rs -> src/app/git_widget.rs\n?? assets/icons/refresh.svg\n?? scratch/\n M Cargo.toml".into()
                    },
                    recent_commits: "09567d4\tfeat(git): timeline commit graph\t12 minutes ago\n9435fa3\tfeat(git): branch header with sync chips\t2 hours ago\nc3fb286\ttest(voice): await asynchronous snapshot acknowledgment\t1 day ago\nd72341b\tFix iOS voice certificate trust integration\t2 days ago\n9636cfa\tRecord verified cloud-only voice release\t3 days ago\n19e6367\tRebuild tool cards from history tool rows\t1 week ago\n1a2b3c4\tInitial pull tab\t2 weeks ago".into(),
                    diff_stat: if clean {
                        String::new()
                    } else {
                        "120\t48\tsrc/app/git_info.rs\n6\t1\tsrc/main.rs\n3\t0\tassets/icons/git-branch.svg\n0\t40\tdocs/old-notes.md\n2\t2\tsrc/app/git_widget.rs\n1\t1\tCargo.toml".into()
                    },
                    ahead: if clean { 0 } else { 2 },
                    behind: if clean { 0 } else { 1 },
                });
            }
        }
    }

    /// Changed files: a status badge, the file name bright with its directory
    /// muted, and right-aligned `+`/`−` columns sized to the widest value.
    fn render_git_changes(&self, snapshot: &GitSnapshot, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let files: Vec<_> = snapshot.status.lines().filter_map(status_columns).collect();
        let (added, removed) = diff_totals(&snapshot.diff_stat);
        let totals = (!snapshot.diff_stat.is_empty()).then(|| {
            div()
                .flex()
                .gap(px(6.))
                .child(label(format!("+{added}"), t.ok, 10.5))
                .child(label(format!("−{removed}"), t.error, 10.5))
                .into_any_element()
        });
        let mut section = git_section("git-overview", "CHANGES", Some(files.len()).filter(|n| *n > 0), totals, t);
        if files.is_empty() {
            return section
                .child(
                    git_row()
                        .h(px(row_height()))
                        .child(label("✓", t.ok, 12.).flex_none().w(px(18.)).text_center())
                        .child(label("Working tree clean", t.text_secondary, 12.5)),
                )
                .into_any_element();
        }
        let rows: Vec<_> = files
            .iter()
            .map(|(code, path)| {
                let lookup = path.rsplit_once(" -> ").map_or(*path, |(_, new)| new);
                (*code, *path, stat_cells(file_stat(lookup, &snapshot.diff_stat)))
            })
            .collect();
        // Shared column widths keep every row's numbers on one vertical line.
        let plus_w = rows.iter().map(|(_, _, (a, _))| a.chars().count()).max().unwrap_or(0);
        let minus_w = rows.iter().map(|(_, _, (_, r))| r.chars().count()).max().unwrap_or(0);
        let limit = if self.git_info.show_all_files { rows.len() } else { FOLD };
        for (code, path, (plus, minus)) in rows.iter().take(limit) {
            let (badge, tint) = status_badge(code, t);
            let (dir, name) = split_path(path);
            let deleted = badge == "D";
            section = section.child(
                git_row()
                    .h(px(row_height()))
                    .child(
                        div()
                            .flex_none()
                            .size(px(18.))
                            .rounded(px(4.))
                            .bg(with_alpha(tint, 0.14))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(label(badge, tint, 10.5).font_weight(gpui::FontWeight::BOLD)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .items_baseline()
                            .gap(px(6.))
                            .overflow_hidden()
                            .child(
                                label(name.to_owned(), if deleted { t.text_muted } else { t.text }, 12.5)
                                    .when(deleted, |d| d.line_through())
                                    .flex_none()
                                    .max_w_full()
                                    .truncate(),
                            )
                            .when(!dir.is_empty(), |d| {
                                d.child(
                                    label(dir.to_owned(), t.text_muted, 11.)
                                        .flex_1()
                                        .min_w_0()
                                        .overflow_hidden()
                                        .whitespace_nowrap()
                                        .text_ellipsis_start(),
                                )
                            }),
                    )
                    .when(plus_w > 0, |d| {
                        d.child(
                            label(plus.clone(), if plus == "bin" { t.text_muted } else { t.ok }, 11.)
                                .flex_none()
                                .w(mono_width(plus_w, 11.))
                                .text_right(),
                        )
                    })
                    .when(minus_w > 0, |d| {
                        d.child(label(minus.clone(), t.error, 11.).flex_none().w(mono_width(minus_w, 11.)).text_right())
                    }),
            );
        }
        if rows.len() > FOLD {
            section = section.child(self.git_more_toggle(
                "git-more-files",
                "Toggle all changed files",
                self.git_info.show_all_files,
                rows.len() - FOLD,
                "files",
                t,
                cx,
                |this| this.git_info.show_all_files = !this.git_info.show_all_files,
            ));
        }
        section.into_any_element()
    }

    /// Recent commits as a timeline: a continuous rail with one node per
    /// commit. Local commits the upstream lacks are blue.
    fn render_git_commits(&self, snapshot: &GitSnapshot, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let commits: Vec<_> = snapshot.recent_commits.lines().filter(|l| !l.trim().is_empty()).collect();
        let summary = (snapshot.ahead > 0).then(|| {
            label(format!("{} unpushed", snapshot.ahead), local_color(), 10.5).into_any_element()
        });
        let mut section = git_section("git-commits", "COMMITS", None, summary, t);
        if commits.is_empty() {
            return section
                .child(git_row().h(px(row_height())).child(label("No commits yet", t.text_muted, 12.)))
                .into_any_element();
        }
        let limit = if self.git_info.show_all_commits { commits.len() } else { FOLD };
        let shown = commits.len().min(limit);
        let h = row_height();
        let rail = t.border_focus;
        for (i, line) in commits.iter().take(limit).enumerate() {
            let (hash, subject, age) = commit_columns(line);
            let local = (i as u32) < snapshot.ahead;
            let head = i == 0;
            let node = if local { local_color() } else if head { t.text_secondary } else { t.text_muted };
            // The rail continues below the node while older history exists.
            let more_below = i + 1 < shown || commits.len() > shown;
            let segment = |top: bool, color: Hsla| {
                div().absolute().when(top, |d| d.top_0()).when(!top, |d| d.bottom_0()).h(px(h / 2.)).left(px(8.)).w(px(2.)).bg(color)
            };
            section = section.child(
                git_row()
                    .h(px(h))
                    .child(
                        div()
                            .flex_none()
                            .w(px(18.))
                            .h_full()
                            .relative()
                            .flex()
                            .items_center()
                            .justify_center()
                            .when(i > 0, |d| d.child(segment(true, if local { local_color() } else { rail })))
                            .when(more_below, |d| {
                                d.child(segment(false, if (i as u32 + 1) < snapshot.ahead { local_color() } else { rail }))
                            })
                            .child(
                                div()
                                    .size(px(if head { 10. } else { 8. }))
                                    .rounded_full()
                                    .border_2()
                                    .border_color(node)
                                    .bg(if head { node } else { card_fill(t) }),
                            ),
                    )
                    .child(label(hash.chars().take(7).collect::<String>(), if local { local_color() } else { ref_color() }, 11.).flex_none())
                    .child(label(subject.to_owned(), if head { t.text } else { t.text_secondary }, 12.5).flex_1().min_w_0().truncate())
                    .child(label(compact_age(age), t.text_muted, 10.5).flex_none()),
            );
        }
        if commits.len() > FOLD {
            section = section.child(self.git_more_toggle(
                "git-more-commits",
                "Toggle all recent commits",
                self.git_info.show_all_commits,
                commits.len() - FOLD,
                "commits",
                t,
                cx,
                |this| this.git_info.show_all_commits = !this.git_info.show_all_commits,
            ));
        }
        section.into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn git_more_toggle(
        &self,
        id: &'static str,
        a11y: &'static str,
        expanded: bool,
        hidden: usize,
        noun: &'static str,
        t: &Theme,
        cx: &mut Context<Self>,
        toggle: fn(&mut Self),
    ) -> gpui::Stateful<gpui::Div> {
        git_row()
            .id(id)
            .debug_selector(move || id.into())
            .a11y_button(a11y)
            .h(px(32.))
            .pl(px(26.))
            .cursor_pointer()
            .child(label(
                if expanded { "Show fewer ▴".to_string() } else { format!("{hidden} more {noun} ▾") },
                t.text_muted,
                11.,
            ))
            .on_click(cx.listener(move |this, _, _, cx| {
                toggle(this);
                cx.notify();
            }))
    }

    /// Placeholder rows shaped like the loaded widget, breathing gently.
    fn render_git_skeleton(&self, t: &Theme) -> AnyElement {
        let block = |title: &'static str, widths: &[f32]| {
            div()
                .px(px(14.))
                .pt(px(10.))
                .flex()
                .flex_col()
                .child(git_row().h(px(24.)).child(label(title, t.text_muted, 10.5).font_weight(gpui::FontWeight::BOLD)))
                .children(widths.iter().map(|w| {
                    git_row()
                        .h(px(row_height()))
                        .child(div().flex_none().size(px(16.)).rounded(px(4.)).bg(t.border_focus))
                        .child(div().flex_1().child(div().h(px(9.)).w(gpui::relative(*w)).rounded(px(3.)).bg(t.border_focus)))
                }))
        };
        let body = div()
            .debug_selector(|| "git-loading".into())
            .flex()
            .flex_col()
            .pb(px(8.))
            .child(block("CHANGES", &[0.7, 0.5, 0.62]))
            .child(block("COMMITS", &[0.8, 0.66, 0.74, 0.5]));
        if crate::system::reduce_motion() {
            return body.opacity(0.5).into_any_element();
        }
        body.with_animation(
            "git-skeleton",
            gpui::Animation::new(std::time::Duration::from_millis(1400))
                .repeat()
                .with_easing(gpui::pulsating_between(0.3, 0.65)),
            |d, o| d.opacity(o),
        )
        .into_any_element()
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
        // Pulled leftward out of the right edge. Mostly vertical drags are
        // scrolling, not a pull.
        let left = -f32::from(point.x - origin.x);
        self.git_info.pull = if dy.abs() > left.max(12.) {
            0.
        } else {
            left.clamp(0., 96.)
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
        if !super::account_profile::same_account(self.account.as_ref(), expected)
            || self.git_info.generation != generation
            || &self.git_context() != context
        {
            return;
        }
        self.git_info.loading = false;
        match result {
            Ok(snapshot) => self.git_info.snapshot = Some(snapshot),
            Err(e) if e == jcode_mobile_core::git_info::NOT_A_REPOSITORY => {
                self.git_info.error = Some(format!(
                    "{e}. Open a session in a project folder to see its Git status."
                ))
            }
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
        let armed = pull >= TRIGGER;
        // Edge tab: a slim grip tucked against the right edge. Pulling it left
        // slides it out; release past TRIGGER (or tap) opens the Git panel.
        // The touch target stays 28x72 even though only the grip is visible.
        div()
            .id("git-pull-handle")
            .debug_selector(|| "git-pull-handle".into())
            .a11y_button("Show Git information. Pull left from the edge or activate.")
            .absolute()
            .right_0()
            .top(px(132.))
            .h(px(72.))
            .w(px(28. + pull))
            .flex()
            .justify_end()
            .items_center()
            .cursor_pointer()
            .on_click(cx.listener(|this, _, _, cx| this.open_git(cx)))
            .child(
                div()
                    .h(px(56.))
                    .w(px(10. + pull))
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap(px(4.))
                    .overflow_hidden()
                    .rounded_l(px(8.))
                    .border_1()
                    .border_r_0()
                    .border_color(if armed { t.border_focus } else { t.border })
                    .bg(if armed { t.surface } else { t.background })
                    .text_color(t.text_muted)
                    .text_size(crate::system::fs(11.))
                    .when(pull > 24., |d| d.child(if armed { "Release" } else { "Git" }))
                    .child(div().flex_none().w(px(3.)).h(px(24.)).rounded_full().bg(t.border_focus)),
            )
            .child(
                gpui::canvas(move |b, _, _| bounds.set(Some(b)), |_, _, _, _| {})
                    .absolute()
                    .inset_0(),
            )
            .into_any_element()
    }

    /// The Git widget: a card that drops in below the header over a dimmed
    /// transcript. Tapping the dimmed area, or ✕, dismisses it.
    pub(super) fn render_git_panel(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let state = &self.git_info;
        let snapshot = state.snapshot.as_ref();
        let calm = crate::system::reduce_motion();
        let pressed = t.surface_elevated;
        // Ids stay literal at each call site so coverage inventory names them.
        let icon_button = move |el: gpui::Stateful<gpui::Div>, a11y: &'static str| {
            el.a11y_button(a11y)
                .flex_none()
                .size(px(44.))
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .active(move |s| s.bg(pressed))
        };
        let chip = |text: String, color: Hsla| {
            div()
                .flex_none()
                .h(px(20.))
                .px(px(7.))
                .rounded_full()
                .flex()
                .items_center()
                .bg(with_alpha(color, 0.15))
                .child(label(text, color, 11.).font_weight(gpui::FontWeight::BOLD))
        };

        let refresh_icon = svg()
            .path("icons/refresh.svg")
            .size(px(17.))
            .text_color(if state.loading { t.text_muted } else { t.text_secondary });
        let refresh_icon = if state.loading && !calm {
            refresh_icon
                .with_animation(
                    "git-refresh-spin",
                    Animation::new(std::time::Duration::from_millis(900)).repeat(),
                    |s, p| s.with_transformation(Transformation::rotate(percentage(p))),
                )
                .into_any_element()
        } else {
            refresh_icon.into_any_element()
        };
        let header = div()
            .flex_none()
            .h(px(52.))
            .pl(px(14.))
            .pr(px(4.))
            .flex()
            .items_center()
            .gap(px(7.))
            .border_b_1()
            .border_color(t.border)
            .child(svg().path("icons/git-branch.svg").size(px(16.)).flex_none().text_color(ref_color()))
            .child(
                label(
                    snapshot.map_or_else(|| "Git".to_owned(), |s| s.branch.clone()),
                    if snapshot.is_some() { ref_color() } else { t.text_secondary },
                    14.,
                )
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .min_w_0()
                .truncate(),
            )
            .when_some(snapshot.filter(|s| s.ahead > 0), |d, s| d.child(chip(format!("↑{}", s.ahead), local_color())))
            .when_some(snapshot.filter(|s| s.behind > 0), |d, s| d.child(chip(format!("↓{}", s.behind), t.warning)))
            .child(div().flex_1())
            .child(
                icon_button(
                    div().id("refresh-git").debug_selector(|| "refresh-git".into()),
                    "Refresh Git information",
                )
                    .when(!state.loading, |d| {
                        d.on_click(cx.listener(|this, _, _, cx| {
                            this.refresh_git(cx);
                            cx.notify();
                        }))
                    })
                    .child(refresh_icon),
            )
            .child(
                icon_button(
                    div().id("close-git").debug_selector(|| "close-git".into()),
                    "Close Git information",
                )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.git_info.open = false;
                        cx.notify();
                    }))
                    .child(label("✕", t.text_secondary, 15.)),
            );

        let body = div()
            .id("git-info-panel")
            .debug_selector(|| "git-info-panel".into())
            .min_h_0()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .pb(px(8.));
        let body = if let Some(error) = state.error.clone() {
            body.child(
                div()
                    .px(px(14.))
                    .py(px(16.))
                    .flex()
                    .items_start()
                    .gap(px(10.))
                    .child(
                        div()
                            .flex_none()
                            .size(px(20.))
                            .rounded_full()
                            .bg(with_alpha(t.error, 0.15))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(label("!", t.error, 12.).font_weight(gpui::FontWeight::BOLD)),
                    )
                    .child(label(error, t.text_secondary, 12.5).flex_1().min_w_0()),
            )
        } else if let Some(snapshot) = snapshot {
            body.child(self.render_git_changes(snapshot, t, cx))
                .child(div().mx(px(14.)).my(px(4.)).h(px(1.)).bg(t.border))
                .child(self.render_git_commits(snapshot, t, cx))
        } else if state.loading {
            body.child(self.render_git_skeleton(t))
        } else {
            body
        };

        let card = div()
            .debug_selector(|| "git-overlay".into())
            .occlude()
            .relative()
            .w_full()
            .max_w(px(420.))
            .max_h(gpui::relative(0.74))
            .min_h_0()
            .flex()
            .flex_col()
            .rounded(px(16.))
            .border_1()
            .border_color(t.border)
            .bg(card_fill(t))
            .shadow_lg()
            .overflow_hidden()
            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
            .child(header)
            .child(body);
        let scrim = div()
            .id("git-dismiss")
            .debug_selector(|| "git-dismiss".into())
            .a11y_button("Close Git information")
            .occlude()
            .absolute()
            .inset_0()
            .bg(gpui::black().opacity(if crate::system::is_light() { 0.16 } else { 0.5 }))
            .on_click(cx.listener(|this, _, _, cx| {
                this.git_info.open = false;
                cx.notify();
            }));
        let (scrim, card) = if calm {
            (scrim.into_any_element(), card.into_any_element())
        } else {
            let enter = || Animation::new(std::time::Duration::from_millis(240)).with_easing(gpui::ease_out_quint());
            (
                scrim.with_animation("git-scrim-in", enter(), |d, p| d.opacity(p)).into_any_element(),
                card.with_animation("git-card-in", enter(), |d, p| d.top(px(-10. * (1. - p))).opacity(p))
                    .into_any_element(),
            )
        };
        // Below the 48px header, inset from the edges, never past ~3/4 height
        // so the composer stays visible.
        div()
            .absolute()
            .inset_0()
            .flex()
            .flex_col()
            .items_end()
            .pt(px(54.))
            .px(px(8.))
            .child(scrim)
            .child(card)
            .into_any_element()
    }
}
