//! Faithful, inert neighboring session panels. Message and composer rendering
//! is shared with the active panel, using the parked tab's own draft and tools.
use super::*;

impl MobileApp {
    pub(super) fn render_panel_preview(&self, id: u64, t: &Theme, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let parked = self.parked.iter().find(|p| p.id == id)?;
        let session = &parked.session;
        let status = || Self::connection_status_text_for(session)
            .map(|text| orb_row(&parked.preview_status_orb, text, t.text_muted));
        let empty = session.transcript.is_empty() && !session.is_reasoning;
        let body = if empty {
            div().flex_1().min_h_0().flex().flex_col().justify_end()
                .px(px(16.)).pb(px(14.)).gap(px(12.))
                .children(status())
                .when_some(session.error_banner.clone(), |d, error| d.child(banner(&error, t.error, t)))
                .child(label("What would you like to work on?", t.text, 20.).line_height(crate::system::fs(28.)))
                .children(self.render_recent_dirs_for(session, t, cx))
                .into_any_element()
        } else {
            let mut prompt_n = 0;
            let mut rows = Vec::with_capacity(session.transcript.len() + 2);
            let mut top = Some(px(12.));
            // Preview layout may be taller than the saved tab (keyboard or
            // resize) and clamp its private offset. Cover that entire possible
            // shift conservatively, without changing the parked scroll handle.
            let viewport = px(self.device.map_or_else(|| f32::from(window.viewport_size().height), |(_, height)| height));
            let off = (-parked.scroll.offset().y - viewport).max(px(0.));
            for entry in &session.transcript {
                if entry.role == Role::User { prompt_n += 1; }
                let key = super::transcript_layout::key(id, entry, prompt_n, &parked.expanded_tools, self.content_width());
                let height = self.transcript_heights.get(key);
                let skip = height.is_some_and(|height| super::transcript_layout::offscreen(top, height, off, viewport * 3.));
                #[cfg(test)]
                let skip = skip && !self.disable_transcript_windowing;
                let row = div().id(("preview-entry", entry.id))
                    .debug_selector({ let id = entry.id; move || format!("preview-entry-{id}") });
                if skip {
                    rows.push(row.h(height.unwrap()).flex_none().into_any_element());
                } else {
                    let heights = self.transcript_heights.clone();
                    rows.push(row.relative()
                        .child(self.render_entry(entry, prompt_n, &parked.expanded_tools, t, cx))
                        .child(gpui::canvas(move |bounds, _, _| heights.record(key, bounds.size.height), |_, _, _, _| {}).absolute().inset_0())
                        .into_any_element());
                }
                top = top.zip(height).map(|(top, height)| top + height + px(14.));
            }
            if let Some(activity) = Self::activity_for(session) {
                parked.preview_activity_orb.update(cx, |orb, cx| orb.set_state(orb_state(activity), cx));
                rows.push(orb_row(&parked.preview_activity_orb, activity, t.text_muted).into_any_element());
            }
            if let Some(error) = &session.error_banner {
                rows.push(banner(error, t.error, t).into_any_element());
            }
            // Use a private scroll handle. Laying out a preview must not clamp or
            // otherwise alter the real parked scroll when keyboard/size changes.
            let scroll = ScrollHandle::new();
            scroll.set_offset(parked.scroll.offset());
            let show_jump = parked.scroll.max_offset().y + parked.scroll.offset().y >= px(56.);
            div().relative().flex_1().min_h_0()
                .child(div().id("preview-transcript").debug_selector(|| "preview-transcript".into())
                    .size_full().overflow_y_scroll().track_scroll(&scroll)
                    .child(div().w_full().min_w_0().flex().flex_col().gap(px(14.))
                        .px(px(14.)).py(px(12.)).children(rows)))
                .when(show_jump, |d| d.child(div().absolute().right(px(14.)).bottom(px(8.))
                    .size(px(44.)).rounded_full().flex().items_center().justify_center()
                    .bg(t.background).border_1().border_color(t.border_focus)
                    .text_color(t.text_strong).child("↓")))
                .into_any_element()
        };
        Some(div().id(("panel-preview", id)).debug_selector(move || format!("panel-preview-{id}"))
            .relative().size_full().flex().flex_col()
            .when(!empty && !matches!(session.phase, ConnectionPhase::Connected | ConnectionPhase::Failed { .. }), |d| {
                d.child(div().px(px(14.)).py(px(6.)).border_b_1().border_color(t.border).children(status()))
            })
            .child(div().relative().flex_1().min_h_0().flex().flex_col().overflow_hidden().child(body))
            .child(div().id("preview-composer").debug_selector(|| "preview-composer".into())
                .flex_none().child(self.render_composer_for(session, &parked.preview_composer, true, t, window, cx)))
            // Paint an opaque hit-test layer last. The matching visuals are not
            // live controls until navigation commits, and never take IME focus.
            .child(div().absolute().inset_0().occlude()
                .on_scroll_wheel(cx.listener(|this, event, _, cx| {
                    this.panel_wheel(event, cx);
                    cx.stop_propagation();
                })))
            .into_any_element())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::navigation_coverage::{draw, mount};
    use gpui::{Modifiers, TestAppContext, VisualTestContext, point};

    const DRAFT: &str = "Neighbor draft
Second line with emoji 👩🏽‍💻
Third line
Fourth line";

    fn history(id: u64) -> TranscriptEntry {
        TranscriptEntry {
            id,
            role: Role::Assistant,
            text: format!("History item {id}
A second line to preserve measured height."),
            reasoning: String::new(),
            tool_calls: vec![],
            is_streaming: false,
            is_queued: false,
        }
    }

    fn prepare_destination(view: &Entity<MobileApp>, cx: &mut VisualTestContext) {
        view.update(cx, |app, cx| {
            app.session.phase = ConnectionPhase::Connected;
            app.session.session_id = Some("preview-fixture".into());
            app.session.session_title = Some("Neighbor session".into());
            app.session.transcript = (0..18).map(history).collect();
            app.session.transcript[3].tool_calls.push(jcode_mobile_core::ToolCall {
                id: "preview-fixture".into(),
                name: "Read".into(),
                input: "{\"path\":\"fixture.rs\"}".into(),
                output: "First output line
Second output line
Third output line".into(),
                status: ToolStatus::Succeeded,
            });
            app.expanded_tools.insert("preview-fixture".into());
            app.follow = false;
            app.composer.update(cx, |input, cx| input.set_text(DRAFT, cx));
            cx.notify();
        });
        draw(cx);
        view.update(cx, |app, cx| {
            assert!(app.transcript_scroll.max_offset().y > px(180.));
            app.transcript_scroll.set_offset(point(px(0.), px(-180.)));
            cx.notify();
        });
        draw(cx);
    }

    fn park_and_preview(view: &Entity<MobileApp>, cx: &mut VisualTestContext) {
        cx.update(|window, cx| {
            window.blur(cx);
            view.update(cx, |app, cx| {
                app.park_active(cx);
                let second = app.next_tab;
                app.next_tab += 1;
                app.active_tab = second;
                app.tab_order.push(second);
                app.composer.update(cx, |input, cx| input.set_text("Current draft", cx));
                assert!(app.swipe.feed(100., 0., std::time::Instant::now()));
                cx.notify();
            });
        });
        draw(cx);
    }

    fn near(actual: gpui::Pixels, expected: gpui::Pixels, what: &str) {
        assert!((f32::from(actual - expected)).abs() < 1., "{what}: {actual:?} != {expected:?}");
    }

    #[gpui::test]
    fn saved_scroll_and_multiline_draft_preview_match_destination_geometry(cx: &mut TestAppContext) {
        let (view, cx) = mount(cx);
        prepare_destination(&view, cx);
        let destination_composer = cx.debug_bounds("chat-composer").unwrap();
        let destination_left = destination_composer.left();
        let destination_entries: Vec<_> = ["entry-0", "entry-3", "entry-17"].into_iter()
            .map(|selector| cx.debug_bounds(selector).expect("destination entry rendered"))
            .collect();
        let header = cx.debug_bounds("menu").unwrap();
        let git = cx.debug_bounds("git-pull-handle").unwrap();
        park_and_preview(&view, cx);
        let panel = cx.debug_bounds("panel-preview-0").unwrap();
        let composer = cx.debug_bounds("preview-composer").unwrap();
        near(composer.origin.y, destination_composer.origin.y, "composer vertical position");
        near(composer.size.height, destination_composer.size.height, "multiline composer height");
        near(composer.size.width, destination_composer.size.width, "composer width");
        for (selector, destination) in ["preview-entry-0", "preview-entry-3", "preview-entry-17"].into_iter().zip(destination_entries) {
            let preview = cx.debug_bounds(selector).expect("all history remains in preview");
            near(preview.origin.y, destination.origin.y, "saved scroll message offset");
            near(preview.origin.x - panel.origin.x, destination.origin.x - destination_left, "message horizontal inset");
            near(preview.size.width, destination.size.width, "message width");
            near(preview.size.height, destination.size.height, "message height");
        }
        assert_eq!(cx.debug_bounds("menu").unwrap(), header);
        assert_eq!(cx.debug_bounds("git-pull-handle").unwrap(), git);
        cx.update(|window, cx| view.update(cx, |app, cx| {
            for _ in 0..20 {
                assert!(app.render_panel_preview(0, &Theme::pure_black(), window, cx).is_some());
                assert!(app.render_panel_preview(u64::MAX, &Theme::pure_black(), window, cx).is_none());
            }
            assert_eq!(app.active_tab, 1);
            assert_eq!(app.composer.read(cx).text(), "Current draft");
            assert_eq!(app.parked.len(), 1);
            let parked = &app.parked[0];
            assert_eq!(parked.scroll.offset().y, px(-180.));
            assert_eq!(parked.preview_composer.read(cx).text(), DRAFT);
            assert!(parked.expanded_tools.contains("preview-fixture"));
            assert!(!parked.follow);
            app.swipe = Default::default();
            app.focus_tab(0, cx);
            assert_eq!(app.composer.read(cx).text(), DRAFT);
            assert_eq!(app.session.session_title.as_deref(), Some("Neighbor session"));
            assert_eq!(app.transcript_scroll.offset().y, px(-180.));
        }));
        draw(cx);
        near(cx.debug_bounds("chat-composer").unwrap().origin.y, composer.origin.y, "focused destination composer");
    }

    #[gpui::test]
    fn wheel_over_preview_continues_and_commits_without_idle_timeout(cx: &mut TestAppContext) {
        let (view, cx) = mount(cx);
        prepare_destination(&view, cx);
        park_and_preview(&view, cx);
        let panel = cx.debug_bounds("panel-preview-0").unwrap();
        let at = point(panel.right() - px(30.), panel.top() + px(100.));
        assert!(at.x > px(0.) && at.x < px(390.));
        for (phase, dx) in [(gpui::TouchPhase::Moved, 40.), (gpui::TouchPhase::Ended, 0.)] {
            cx.simulate_event(gpui::ScrollWheelEvent {
                position: at,
                delta: gpui::ScrollDelta::Pixels(point(px(dx), px(0.))),
                modifiers: Modifiers::default(),
                touch_phase: phase,
            });
            draw(cx);
            if phase == gpui::TouchPhase::Moved {
                view.read_with(cx, |app, _| {
                    assert_eq!(app.active_tab, 1);
                    assert_eq!(app.swipe.dx, 140., "preview wheel must not bubble into a second dispatch");
                });
            }
        }
        view.read_with(cx, |app, cx| {
            assert_eq!(app.active_tab, 0, "release over the preview must commit immediately");
            assert!(!app.swipe.active && app.swipe.last_event.is_none());
            assert_eq!(app.composer.read(cx).text(), DRAFT);
            assert_eq!(app.parked.len(), 1);
        });
    }

    #[gpui::test]
    fn preview_taps_do_not_expand_send_or_focus_either_editor(cx: &mut TestAppContext) {
        let (view, cx) = mount(cx);
        prepare_destination(&view, cx);
        park_and_preview(&view, cx);
        let panel = cx.debug_bounds("panel-preview-0").unwrap();
        let composer = cx.debug_bounds("preview-composer").unwrap();
        let tool = cx.debug_bounds("tool-preview-fixture").unwrap();
        let visible_x = (panel.right() - px(30.)).min(px(100.));
        assert!(visible_x > px(0.));
        for position in [
            point(visible_x, tool.origin.y + px(16.)),
            point(visible_x, composer.origin.y + px(25.)),
            point(panel.right() - px(32.), composer.origin.y + px(30.)),
        ] {
            assert!(panel.contains(&position), "tap must hit the visible preview");
            cx.simulate_click(position, Modifiers::default());
            draw(cx);
        }
        cx.update(|window, cx| view.read_with(cx, |app, cx| {
            assert_eq!(app.active_tab, 1);
            assert_eq!(app.composer.read(cx).text(), "Current draft");
            assert!(app.session.transcript.is_empty());
            assert!(app.expanded_tools.is_empty());
            assert!(!app.composer.read(cx).is_focused(window));
            let parked = &app.parked[0];
            assert_eq!(parked.session.transcript.len(), 18);
            assert_eq!(parked.preview_composer.read(cx).text(), DRAFT);
            assert!(!parked.preview_composer.read(cx).is_focused(window));
            assert!(parked.expanded_tools.contains("preview-fixture"));
            assert_eq!(parked.scroll.offset().y, px(-180.));
        }));
    }
}
