//! Cold exact-height regression fixtures. These test geometry, not native shaping speed.
use super::super::navigation_coverage::{draw, mount};
use super::super::*;
use gpui::{Pixels, TestAppContext, VisualTestContext, point};

fn long_entry(index: usize) -> TranscriptEntry {
    let prefix = format!("Cold row {index}: **formatted** `inline code` 界🦀\n\n");
    let paragraph = "A unique transcript keeps all its history accessible and wraps naturally. ";
    let mut text = prefix;
    while text.len() < 4096 {
        text.push_str(paragraph);
    }
    text.truncate(4096); // The repeated ASCII suffix makes this a UTF-8 boundary.
    TranscriptEntry {
        id: index as u64,
        role: if index % 5 == 0 {
            Role::User
        } else {
            Role::Assistant
        },
        text,
        reasoning: if index % 11 == 0 {
            "Reasoning that also wraps across the phone viewport. ".repeat(8)
        } else {
            String::new()
        },
        tool_calls: vec![],
        is_streaming: false,
        is_queued: false,
    }
}

fn compare_with_full_layout(view: &Entity<MobileApp>, cx: &mut VisualTestContext, count: usize) {
    let expected = view.read_with(cx, |app, _| {
        (
            app.transcript_scroll.max_offset(),
            app.transcript_scroll.offset(),
        )
    });
    let bounds: Vec<_> = (0..count)
        .filter_map(|index| {
            let selector: &'static str = Box::leak(format!("entry-{index}").into_boxed_str());
            cx.debug_bounds(selector).map(|bounds| (selector, bounds))
        })
        .collect();
    assert!(
        !bounds.is_empty(),
        "the visible destination must be materialized"
    );
    view.update(cx, |app, cx| {
        app.disable_transcript_windowing = true;
        cx.notify();
    });
    draw(cx);
    view.read_with(cx, |app, _| {
        assert_eq!(
            app.rendered_transcript_rows, count,
            "full history must remain available"
        );
        assert_eq!(app.transcript_scroll.max_offset(), expected.0);
        assert_eq!(app.transcript_scroll.offset(), expected.1);
    });
    for (selector, expected) in bounds {
        assert_eq!(
            cx.debug_bounds(selector),
            Some(expected),
            "{selector} shifted"
        );
    }
    view.update(cx, |app, cx| {
        app.disable_transcript_windowing = false;
        cx.notify();
    });
    draw(cx);
}

#[gpui::test]
fn cold_long_rows_preserve_exact_geometry_through_jumps_edits_and_resize(cx: &mut TestAppContext) {
    const COUNT: usize = 40;
    let (view, cx) = mount(cx);
    view.update(cx, |app, cx| {
        app.session.phase = ConnectionPhase::Connected;
        app.follow = false;
        app.session.transcript = (0..COUNT).map(long_entry).collect();
        app.session.transcript[1]
            .tool_calls
            .push(jcode_mobile_core::ToolCall {
                id: "cold-layout-tool".into(),
                name: "read".into(),
                input: r#"{"path":"fixture.rs"}"#.into(),
                output: "An expanded output line.\n".repeat(40),
                status: jcode_mobile_core::ToolStatus::Running,
            });
        app.transcript_heights = Default::default();
        cx.notify();
    });
    draw(cx);
    view.read_with(cx, |app, _| {
        assert_eq!(
            app.rendered_transcript_rows, COUNT,
            "cold geometry cannot use guessed spacers"
        );
        assert!(
            app.session
                .transcript
                .iter()
                .all(|entry| entry.text.len() == 4096)
        );
    });
    view.update(cx, |_, cx| cx.notify());
    draw(cx);
    compare_with_full_layout(&view, cx, COUNT);
    for fraction in [0.5, 1., 0.] {
        view.update(cx, |app, cx| {
            let max = app.transcript_scroll.max_offset().y;
            app.transcript_scroll
                .set_offset(point(px(0.), -max * fraction));
            cx.notify();
        });
        draw(cx);
        compare_with_full_layout(&view, cx, COUNT);
    }
    view.update(cx, |app, cx| {
        app.expanded_tools.insert("cold-layout-tool".into());
        cx.notify();
    });
    draw(cx);
    view.update(cx, |_, cx| cx.notify());
    draw(cx);
    compare_with_full_layout(&view, cx, COUNT);
    view.update(cx, |app, cx| {
        let call = &mut app.session.transcript[1].tool_calls[0];
        call.output = "Replacement output with different wrapping and height. ".repeat(200);
        call.status = jcode_mobile_core::ToolStatus::Succeeded;
        cx.notify();
    });
    draw(cx);
    view.update(cx, |_, cx| cx.notify());
    draw(cx);
    compare_with_full_layout(&view, cx, COUNT);
    view.update(cx, |app, cx| {
        app.expanded_tools.remove("cold-layout-tool");
        // Same byte length, different line structure: byte counts cannot stand in for geometry.
        app.session.transcript[1].text = "\n".repeat(app.session.transcript[1].text.len());
        app.session.transcript[COUNT - 1]
            .text
            .push_str("\n\nA newly appended tail.");
        cx.notify();
    });
    draw(cx);
    view.update(cx, |_, cx| cx.notify());
    draw(cx);
    compare_with_full_layout(&view, cx, COUNT);
    view.update(cx, |app, cx| {
        app.device = Some((320., 844.));
        cx.notify();
    });
    cx.simulate_resize(gpui::size(px(320.), px(844.)));
    draw(cx);
    view.update(cx, |_, cx| cx.notify());
    draw(cx);
    compare_with_full_layout(&view, cx, COUNT);
}

#[gpui::test]
#[ignore = "opt-in cold 500x4096-byte fixture, headless timings are not native presentation"]
fn benchmark_cold_500_long_rows(cx: &mut TestAppContext) {
    let (view, cx) = mount(cx);
    view.update(cx, |app, cx| {
        app.session.phase = ConnectionPhase::Connected;
        app.follow = false;
        app.session.transcript = (0..500).map(long_entry).collect();
        app.transcript_heights = Default::default();
        cx.notify();
    });
    let started = std::time::Instant::now();
    draw(cx);
    eprintln!(
        "cold exact-height 500x4096-byte headless draw: {:.3}ms",
        started.elapsed().as_secs_f64() * 1000.
    );
    view.read_with(cx, |app, _| assert_eq!(app.rendered_transcript_rows, 500));
    view.update(cx, |_, cx| cx.notify());
    draw(cx);
    compare_with_full_layout(&view, cx, 500);
}

/// Directly render identical real entry subtrees using the production observer
/// or the previous absolute canvas. This bypasses transcript windowing so the
/// reference cannot accidentally compare the new observer against itself.
struct ObserverFixture {
    app: Entity<MobileApp>,
    entries: Vec<TranscriptEntry>,
    expanded: std::collections::HashSet<String>,
    heights: super::Heights,
    scroll: gpui::ScrollHandle,
    observer: bool,
    tab: u64,
    width: f32,
}

impl Render for ObserverFixture {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::pure_black();
        let mut rows = Vec::new();
        let mut prompt = 0;
        for entry in &self.entries {
            if entry.role == Role::User {
                prompt += 1;
            }
            let key = super::key(self.tab, entry, prompt, &self.expanded, self.width - 28.);
            let body = self.app.update(cx, |app, cx| {
                app.render_entry(entry, prompt, &self.expanded, None, &[], &theme, cx)
            });
            let child_selector: &'static str =
                Box::leak(format!("causal-child-{}", entry.id).into_boxed_str());
            let row_selector: &'static str =
                Box::leak(format!("causal-row-{}", entry.id).into_boxed_str());
            let row = div()
                .relative()
                .id(("causal-row", entry.id))
                .debug_selector(move || row_selector.into())
                .child(
                    div()
                        .debug_selector(move || child_selector.into())
                        .child(body),
                );
            if self.observer {
                rows.push(
                    super::measured_row(row.into_any_element(), key, self.heights.clone())
                        .into_any_element(),
                );
            } else {
                let heights = self.heights.clone();
                rows.push(
                    row.child(
                        gpui::canvas(
                            move |bounds, _, _| heights.record(key, bounds.size.height),
                            |_, _, _, _| {},
                        )
                        .absolute()
                        .inset_0(),
                    )
                    .into_any_element(),
                );
            }
        }
        div().size_full().child(
            div()
                .id("causal-scroll")
                .w(px(self.width))
                .h_full()
                .overflow_y_scroll()
                .track_scroll(&self.scroll)
                .child(
                    div()
                        .w_full()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(14.))
                        .px(px(14.))
                        .py(px(12.))
                        .children(rows),
                ),
        )
    }
}

#[derive(Debug, PartialEq)]
struct ObserverSnapshot {
    max: gpui::Point<Pixels>,
    offset: gpui::Point<Pixels>,
    bounds: Vec<(gpui::Bounds<Pixels>, gpui::Bounds<Pixels>)>,
    heights: Vec<Pixels>,
}

#[gpui::test]
fn identical_prose_reuses_real_geometry_across_ids_and_tabs_but_not_changed_inputs(
    cx: &mut TestAppContext,
) {
    let _settings = crate::system::settings_fixture(1., false, false);
    cx.update(init);
    let (view, cx) = cx.add_window_view(|window, cx| {
        let app = cx.new(|cx| {
            MobileApp::new_offline(
                SafeArea {
                    top: 20.,
                    bottom: 0.,
                },
                window,
                cx,
            )
        });
        let entries = (0..4)
            .map(|index| {
                let mut entry = long_entry(index);
                entry.role = if index % 2 == 0 {
                    Role::User
                } else {
                    Role::Assistant
                };
                entry.reasoning.clear();
                entry
            })
            .collect();
        ObserverFixture {
            app,
            entries,
            expanded: Default::default(),
            heights: Default::default(),
            scroll: Default::default(),
            observer: true,
            tab: 7,
            width: 390.,
        }
    });
    cx.simulate_resize(gpui::size(px(390.), px(844.)));
    draw(cx);
    let original = observer_snapshot(&view, cx);
    assert!(
        original.max.y > px(0.),
        "fixture must exercise actual scrolling geometry"
    );
    view.update(cx, |fixture, cx| {
        fixture.tab = 91;
        let mut prompt = 0;
        for (index, entry) in fixture.entries.iter_mut().enumerate() {
            entry.id += 1000;
            if entry.role == Role::User {
                prompt += 1;
            }
            let key = super::key(
                fixture.tab,
                entry,
                prompt,
                &fixture.expanded,
                fixture.width - 28.,
            );
            assert!(
                !fixture
                    .heights
                    .0
                    .borrow()
                    .contains_key(&(fixture.tab, entry.id)),
                "new identities cannot hit the primary cache"
            );
            assert_eq!(
                fixture.heights.get(key),
                Some(original.heights[index]),
                "secondary hit must contain the exact height measured by real layout"
            );
        }
        cx.notify();
    });
    draw(cx);
    assert_eq!(
        observer_snapshot(&view, cx),
        original,
        "cross-tab/id reuse must agree with a subsequent actual native layout"
    );
    view.read_with(cx, |fixture, _| {
        let entry = &fixture.entries[0];
        let heights = &fixture.heights;
        let key = |entry: &TranscriptEntry, prompt, width| {
            super::key(123, entry, prompt, &fixture.expanded, width)
        };
        assert_eq!(
            heights.get(key(entry, 9, 362.)),
            Some(original.heights[0]),
            "same-width prompt badges can reuse measured geometry"
        );
        assert_eq!(
            heights.get(key(entry, 10, 362.)),
            None,
            "extra badge digit changes available text width"
        );
        assert_eq!(
            heights.get(key(entry, 1, 334.)),
            None,
            "resizing cannot reuse old wrapping"
        );
        assert_eq!(
            heights.get(key(entry, 1, 362.).with_window_scale(2.)),
            None,
            "device pixel density must invalidate pixel-snapped shared geometry"
        );
        let mut edited = entry.clone();
        edited.text.replace_range(..1, "X");
        assert_eq!(edited.text.len(), entry.text.len());
        assert_eq!(
            heights.get(key(&edited, 1, 362.)),
            None,
            "same-byte-length edits invalidate geometry"
        );
        let mut role = entry.clone();
        role.role = Role::Assistant;
        assert_eq!(
            heights.get(key(&role, 1, 362.)),
            None,
            "role changes alter rendered structure"
        );
        let _scale = crate::system::settings_fixture(1.5, false, false);
        assert_eq!(
            heights.get(key(entry, 1, 362.)),
            None,
            "font scale must invalidate secondary geometry"
        );
    });
    view.read_with(cx, |fixture, _| {
        let base = fixture.entries[1].clone();
        let mut excluded = Vec::new();
        let mut reasoning = base.clone();
        reasoning.reasoning = "Identity-specific reasoning".into();
        excluded.push(reasoning);
        let mut streaming = base.clone();
        streaming.is_streaming = true;
        excluded.push(streaming);
        let mut system = base.clone();
        system.role = Role::System;
        excluded.push(system);
        let mut tools = base.clone();
        tools.tool_calls.push(jcode_mobile_core::ToolCall {
            id: "identity-tool".into(),
            name: "read".into(),
            input: "{}".into(),
            output: String::new(),
            status: jcode_mobile_core::ToolStatus::Running,
        });
        excluded.push(tools);
        for mut entry in excluded {
            let measured = super::Heights::default();
            let source = super::key(1, &entry, 1, &fixture.expanded, 362.);
            assert!(
                source.geometry().is_none(),
                "ineligible rows must not enter the secondary cache"
            );
            measured.record(source, original.heights[1]);
            assert_eq!(
                measured.get(source),
                Some(original.heights[1]),
                "primary cache remains usable"
            );
            entry.id += 9000;
            assert_eq!(
                measured.get(super::key(2, &entry, 1, &fixture.expanded, 362.)),
                None,
                "identity-sensitive rows must be freshly measured for another identity"
            );
        }
        let images = [jcode_mobile_core::SessionImage::new(
            "image/png",
            vec![1, 2, 3],
        )];
        let measured = super::Heights::default();
        let source = super::key(1, &base, 1, &fixture.expanded, 362.).with_images(&images);
        assert!(
            source.geometry().is_none(),
            "asynchronous image dimensions exclude shared geometry"
        );
        measured.record(source, original.heights[1]);
        let mut another = base;
        another.id += 9000;
        assert_eq!(
            measured.get(super::key(2, &another, 1, &fixture.expanded, 362.).with_images(&images)),
            None
        );
    });
}

fn observer_snapshot(
    view: &Entity<ObserverFixture>,
    cx: &mut VisualTestContext,
) -> ObserverSnapshot {
    let (max, offset, heights, ids) = view.read_with(cx, |fixture, _| {
        let mut prompt = 0;
        let heights = fixture
            .entries
            .iter()
            .map(|entry| {
                if entry.role == Role::User {
                    prompt += 1;
                }
                fixture
                    .heights
                    .get(super::key(
                        fixture.tab,
                        entry,
                        prompt,
                        &fixture.expanded,
                        fixture.width - 28.,
                    ))
                    .expect("every real row must have an exact measurement")
            })
            .collect();
        (
            fixture.scroll.max_offset(),
            fixture.scroll.offset(),
            heights,
            fixture
                .entries
                .iter()
                .map(|entry| entry.id)
                .collect::<Vec<_>>(),
        )
    });
    let bounds = ids
        .into_iter()
        .map(|id| {
            let row: &'static str = Box::leak(format!("causal-row-{id}").into_boxed_str());
            let child: &'static str = Box::leak(format!("causal-child-{id}").into_boxed_str());
            (
                cx.debug_bounds(row).expect("row bounds"),
                cx.debug_bounds(child).expect("child bounds"),
            )
        })
        .collect();
    ObserverSnapshot {
        max,
        offset,
        bounds,
        heights,
    }
}

fn assert_observer_matches_canvas(view: &Entity<ObserverFixture>, cx: &mut VisualTestContext) {
    view.update(cx, |fixture, cx| {
        fixture.observer = false;
        fixture.heights = Default::default();
        cx.notify();
    });
    draw(cx);
    let legacy = observer_snapshot(view, cx);
    view.update(cx, |fixture, cx| {
        fixture.observer = true;
        fixture.heights = Default::default();
        cx.notify();
    });
    draw(cx);
    assert_eq!(
        observer_snapshot(view, cx),
        legacy,
        "observer must preserve exact row/child bounds, scroll geometry and measured heights"
    );
}

#[gpui::test]
fn observer_matches_legacy_canvas_across_changes_widths_and_font_scales(cx: &mut TestAppContext) {
    cx.update(init);
    let (view, cx) = cx.add_window_view(|window, cx| {
        let app = cx.new(|cx| {
            MobileApp::new_offline(
                SafeArea {
                    top: 20.,
                    bottom: 0.,
                },
                window,
                cx,
            )
        });
        let mut entries: Vec<_> = (0..12).map(long_entry).collect();
        entries[1].tool_calls.push(jcode_mobile_core::ToolCall {
            id: "causal-tool".into(),
            name: "read".into(),
            input: r#"{"path":"fixture.rs"}"#.into(),
            output: "Unicode tool output 界🦀 and wrapping.\n".repeat(30),
            status: jcode_mobile_core::ToolStatus::Running,
        });
        entries[2].role = Role::System;
        entries[2].text = "A system message that also wraps across the viewport. ".repeat(8);
        ObserverFixture {
            app,
            entries,
            expanded: Default::default(),
            heights: Default::default(),
            scroll: Default::default(),
            observer: false,
            tab: 7,
            width: 390.,
        }
    });
    cx.simulate_resize(gpui::size(px(390.), px(844.)));
    for scale in [1., 1.5, 2.] {
        let _settings = crate::system::settings_fixture(scale, false, false);
        for width in [390., 320., 460.] {
            view.update(cx, |fixture, cx| {
                fixture.width = width;
                cx.notify();
            });
            cx.simulate_resize(gpui::size(px(width), px(844.)));
            assert_observer_matches_canvas(&view, cx);
            view.update(cx, |fixture, cx| {
                fixture
                    .scroll
                    .set_offset(point(px(0.), -fixture.scroll.max_offset().y * 0.5));
                cx.notify();
            });
            assert_observer_matches_canvas(&view, cx);
            view.update(cx, |fixture, cx| {
                fixture.expanded.insert("causal-tool".into());
                fixture.entries[3].text =
                    "**Replacement**\n\n- bullet 界\n\n```rust\nfn updated() {}\n```".repeat(9);
                fixture.entries[4]
                    .reasoning
                    .push_str("\n\nAdditional reasoning after an edit.");
                cx.notify();
            });
            assert_observer_matches_canvas(&view, cx);
            view.update(cx, |fixture, cx| {
                fixture.entries[1].tool_calls[0]
                    .output
                    .push_str(&"\nA changed result".repeat(50));
                fixture.entries[1].tool_calls[0].status = jcode_mobile_core::ToolStatus::Succeeded;
                fixture.entries.push(long_entry(fixture.entries.len()));
                cx.notify();
            });
            assert_observer_matches_canvas(&view, cx);
            view.update(cx, |fixture, cx| {
                fixture.expanded.clear();
                cx.notify();
            });
            assert_observer_matches_canvas(&view, cx);
        }
    }
}
